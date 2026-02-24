use super::collect_event_watch_paths;
use crate::config::{BackupConfig, BackupMode, SourceConfig};
use std::collections::HashMap;
use std::path::PathBuf;

fn make_source(path: &str) -> SourceConfig {
    SourceConfig {
        source_dir: PathBuf::from(path),
        backup_dirs: vec![PathBuf::from("/tmp/backup")],
        enabled: true,
        exclude_patterns: None,
        max_backups: None,
        backup_mode: None,
        cron_schedule: None,
        enable_event_driven: None,
        enable_periodic: None,
    }
}

fn make_source_with_flags(
    path: &str,
    enabled: bool,
    enable_event_driven: Option<bool>,
) -> SourceConfig {
    SourceConfig {
        source_dir: PathBuf::from(path),
        backup_dirs: vec![PathBuf::from("/tmp/backup")],
        enabled,
        exclude_patterns: None,
        max_backups: None,
        backup_mode: None,
        cron_schedule: None,
        enable_event_driven,
        enable_periodic: None,
    }
}

fn base_config(backup_mode: BackupMode, enable_event_driven: bool) -> BackupConfig {
    BackupConfig {
        sources: vec![make_source("/tmp/source")],
        enable_periodic: true,
        enable_event_driven,
        exclude_patterns: vec![],
        max_backups: 10,
        backup_mode,
        cron_schedule: "0 0 * * * *".to_string(),
        enable_min_interval_by_size: false,
        max_log_file_size_mb: 20,
        metadata: HashMap::new(),
    }
}

#[test]
fn collect_event_watch_paths_includes_copy_mode_source() {
    let config = base_config(BackupMode::Copy, true);
    let paths = collect_event_watch_paths(&config);
    assert_eq!(paths, vec![PathBuf::from("/tmp/source")]);
}

#[test]
fn collect_event_watch_paths_respects_source_override_disable() {
    let mut config = base_config(BackupMode::Delta, true);
    config.sources[0].enable_event_driven = Some(false);
    let paths = collect_event_watch_paths(&config);
    assert!(paths.is_empty());
}

#[test]
fn collect_event_watch_paths_empty_when_global_disabled() {
    let config = base_config(BackupMode::Copy, false);
    let paths = collect_event_watch_paths(&config);
    assert!(paths.is_empty());
}

#[test]
fn collect_event_watch_paths_skips_disabled_sources() {
    let mut config = base_config(BackupMode::Copy, true);
    config.sources = vec![
        make_source_with_flags("/tmp/source_enabled", true, None),
        make_source_with_flags("/tmp/source_disabled", false, None),
    ];

    let paths = collect_event_watch_paths(&config);
    assert_eq!(paths, vec![PathBuf::from("/tmp/source_enabled")]);
}

#[test]
fn collect_event_watch_paths_filters_mixed_source_overrides() {
    let mut config = base_config(BackupMode::Delta, true);
    config.sources = vec![
        make_source_with_flags("/tmp/source_a", true, Some(true)),
        make_source_with_flags("/tmp/source_b", true, Some(false)),
        make_source_with_flags("/tmp/source_c", false, Some(true)),
    ];

    let paths = collect_event_watch_paths(&config);
    assert_eq!(paths, vec![PathBuf::from("/tmp/source_a")]);
}

#[test]
fn config_snapshot_pretty_json_contains_phase_and_config() {
    let config = base_config(BackupMode::Delta, true);
    let json = super::config_snapshot_pretty_json(&config, "startup");

    assert!(json.contains('\n'));

    let parsed: serde_json::Value =
        serde_json::from_str(&json).expect("snapshot must be valid json");
    assert_eq!(parsed["phase"], "startup");
    assert!(parsed.get("config").is_some());
}

#[test]
fn config_fingerprint_is_stable_and_changes_on_mutation() {
    let config = base_config(BackupMode::Copy, true);
    let fp1 = super::config_fingerprint(&config).expect("fingerprint must be created");
    let fp2 = super::config_fingerprint(&config).expect("fingerprint must be stable");
    assert_eq!(fp1, fp2);

    let mut changed = config.clone();
    changed.enable_event_driven = false;
    let fp3 = super::config_fingerprint(&changed).expect("fingerprint must be created");
    assert_ne!(fp1, fp3);
}

#[test]
fn spawn_runtime_handles_returns_empty_when_all_triggers_disabled() {
    let mut config = base_config(BackupMode::Copy, false);
    config.enable_periodic = false;
    config.sources = vec![make_source_with_flags("/tmp/source", true, Some(false))];
    let (tx, _rx) = tokio::sync::mpsc::channel::<()>(1);

    let mut handles = super::spawn_runtime_handles(&config, tx)
        .expect("spawning runtime handles without triggers must succeed");
    assert!(handles.cron_tasks.is_empty());
    assert!(handles.watcher_task.is_none());
    handles.abort_all();
}

#[test]
fn spawn_runtime_handles_rejects_invalid_source_cron() {
    let mut config = base_config(BackupMode::Copy, false);
    config.enable_periodic = true;
    config.sources = vec![make_source_with_flags("/tmp/source", true, None)];
    config.sources[0].cron_schedule = Some("invalid cron expression".to_string());
    let (tx, _rx) = tokio::sync::mpsc::channel::<()>(1);

    match super::spawn_runtime_handles(&config, tx) {
        Ok(mut handles) => {
            handles.abort_all();
            panic!("invalid source cron must return error");
        }
        Err(err) => assert!(err.to_string().contains("Invalid cron")),
    }
}

#[test]
fn should_skip_hot_reload_when_latest_matches_active_fingerprint() {
    assert!(super::should_skip_hot_reload("active", None, "active"));
}

#[test]
fn should_skip_hot_reload_when_latest_matches_failed_fingerprint() {
    assert!(super::should_skip_hot_reload(
        "active",
        Some("failed"),
        "failed"
    ));
}

#[test]
fn should_not_skip_hot_reload_for_new_fingerprint() {
    assert!(!super::should_skip_hot_reload(
        "active",
        Some("failed"),
        "new"
    ));
}

#[test]
fn spawn_runtime_handles_rejects_invalid_global_cron_when_source_has_no_override() {
    let mut config = base_config(BackupMode::Copy, false);
    config.enable_periodic = true;
    config.cron_schedule = "invalid global cron".to_string();
    config.sources = vec![make_source_with_flags("/tmp/source", true, None)];
    let (tx, _rx) = tokio::sync::mpsc::channel::<()>(1);

    match super::spawn_runtime_handles(&config, tx) {
        Ok(mut handles) => {
            handles.abort_all();
            panic!("invalid global cron must return error when source has no override");
        }
        Err(err) => assert!(err.to_string().contains("Invalid cron")),
    }
}
