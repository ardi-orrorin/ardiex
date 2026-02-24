use super::*;

#[test]
fn auto_full_backup_interval_has_expected_bounds() {
    assert_eq!(auto_full_backup_interval(0), 1);
    assert_eq!(auto_full_backup_interval(1), 1);
    assert_eq!(auto_full_backup_interval(2), 1);
    assert_eq!(auto_full_backup_interval(10), 9);
}

#[test]
fn effective_backup_dirs_uses_source_backup_dir_when_empty() {
    let source = SourceConfig {
        source_dir: PathBuf::from("/tmp/source"),
        backup_dirs: vec![],
        enabled: true,
        exclude_patterns: None,
        max_backups: None,
        backup_mode: None,
        cron_schedule: None,
        enable_event_driven: None,
        enable_periodic: None,
    };

    assert_eq!(
        source.effective_backup_dirs(),
        vec![PathBuf::from("/tmp/source/.backup")]
    );
}

#[test]
fn effective_backup_dirs_keeps_explicit_backup_dirs() {
    let source = SourceConfig {
        source_dir: PathBuf::from("/tmp/source"),
        backup_dirs: vec![PathBuf::from("/tmp/backup1"), PathBuf::from("/tmp/backup2")],
        enabled: true,
        exclude_patterns: None,
        max_backups: None,
        backup_mode: None,
        cron_schedule: None,
        enable_event_driven: None,
        enable_periodic: None,
    };

    assert_eq!(
        source.effective_backup_dirs(),
        vec![PathBuf::from("/tmp/backup1"), PathBuf::from("/tmp/backup2")]
    );
}

#[test]
fn resolve_uses_global_defaults_when_source_overrides_are_absent() {
    let global = BackupConfig {
        sources: vec![],
        enable_periodic: true,
        enable_event_driven: true,
        exclude_patterns: vec!["*.tmp".to_string()],
        max_backups: 10,
        backup_mode: BackupMode::Delta,
        cron_schedule: "0 0 * * * *".to_string(),
        enable_min_interval_by_size: true,
        max_log_file_size_mb: 20,
        metadata: HashMap::new(),
    };

    let source = SourceConfig {
        source_dir: PathBuf::from("/tmp/source"),
        backup_dirs: vec![],
        enabled: true,
        exclude_patterns: None,
        max_backups: None,
        backup_mode: None,
        cron_schedule: None,
        enable_event_driven: None,
        enable_periodic: None,
    };

    let resolved = source.resolve(&global);
    assert_eq!(resolved.exclude_patterns, vec!["*.tmp".to_string()]);
    assert_eq!(resolved.max_backups, 10);
    assert!(matches!(resolved.backup_mode, BackupMode::Delta));
    assert_eq!(resolved.full_backup_interval, 9);
    assert_eq!(resolved.cron_schedule, "0 0 * * * *");
    assert!(resolved.enable_event_driven);
    assert!(resolved.enable_periodic);
}

#[test]
fn resolve_source_overrides_take_precedence() {
    let global = BackupConfig {
        sources: vec![],
        enable_periodic: true,
        enable_event_driven: true,
        exclude_patterns: vec!["*.tmp".to_string()],
        max_backups: 10,
        backup_mode: BackupMode::Delta,
        cron_schedule: "0 0 * * * *".to_string(),
        enable_min_interval_by_size: true,
        max_log_file_size_mb: 20,
        metadata: HashMap::new(),
    };

    let source = SourceConfig {
        source_dir: PathBuf::from("/tmp/source"),
        backup_dirs: vec![],
        enabled: true,
        exclude_patterns: Some(vec!["*.cache".to_string()]),
        max_backups: Some(3),
        backup_mode: Some(BackupMode::Copy),
        cron_schedule: Some("0 */5 * * * *".to_string()),
        enable_event_driven: Some(false),
        enable_periodic: Some(false),
    };

    let resolved = source.resolve(&global);
    assert_eq!(resolved.exclude_patterns, vec!["*.cache".to_string()]);
    assert_eq!(resolved.max_backups, 3);
    assert!(matches!(resolved.backup_mode, BackupMode::Copy));
    assert_eq!(resolved.full_backup_interval, 2);
    assert_eq!(resolved.cron_schedule, "0 */5 * * * *");
    assert!(!resolved.enable_event_driven);
    assert!(!resolved.enable_periodic);
}

#[test]
fn resolve_full_backup_interval_is_never_below_one() {
    let global = BackupConfig::default();
    let source = SourceConfig {
        source_dir: PathBuf::from("/tmp/source"),
        backup_dirs: vec![],
        enabled: true,
        exclude_patterns: None,
        max_backups: Some(1),
        backup_mode: None,
        cron_schedule: None,
        enable_event_driven: None,
        enable_periodic: None,
    };

    let resolved = source.resolve(&global);
    assert_eq!(resolved.full_backup_interval, 1);
}

#[test]
fn backup_config_default_values_are_consistent() {
    let config = BackupConfig::default();
    assert!(config.enable_periodic);
    assert!(config.enable_event_driven);
    assert!(config.enable_min_interval_by_size);
    assert_eq!(config.max_backups, 10);
    assert_eq!(config.max_log_file_size_mb, 20);
    assert!(matches!(config.backup_mode, BackupMode::Delta));
    assert_eq!(config.cron_schedule, "0 0 * * * *");
}

#[test]
fn backup_config_deserialization_fails_for_invalid_backup_mode() {
    let json = r#"{
        "sources": [],
        "enable_periodic": true,
        "enable_event_driven": true,
        "exclude_patterns": [],
        "max_backups": 10,
        "backup_mode": "invalid_mode",
        "cron_schedule": "0 0 * * * *",
        "enable_min_interval_by_size": true,
        "max_log_file_size_mb": 20,
        "metadata": {}
    }"#;

    let err = serde_json::from_str::<BackupConfig>(json)
        .expect_err("invalid backup_mode must fail deserialization");
    assert!(!err.to_string().is_empty());
}

#[test]
fn backup_config_deserialization_fails_when_required_field_is_missing() {
    let json = r#"{
        "sources": [],
        "enable_periodic": true,
        "enable_event_driven": true,
        "exclude_patterns": [],
        "backup_mode": "delta",
        "cron_schedule": "0 0 * * * *",
        "enable_min_interval_by_size": true,
        "max_log_file_size_mb": 20,
        "metadata": {}
    }"#;

    let err = serde_json::from_str::<BackupConfig>(json)
        .expect_err("missing max_backups must fail deserialization");
    assert!(!err.to_string().is_empty());
}
