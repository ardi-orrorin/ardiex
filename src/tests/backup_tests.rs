use super::*;
use crate::config::{BackupConfig, BackupHistoryType, BackupMode, SourceConfig, SourceMetadata};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_nanos();
    std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), ts))
}

fn contains_delta_file(dir: &Path) -> Result<bool> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if contains_delta_file(&path)? {
                return Ok(true);
            }
        } else {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            if file_name.ends_with(".delta") {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn make_source(source_dir: &Path, backup_dirs: Vec<PathBuf>, enabled: bool) -> SourceConfig {
    SourceConfig {
        source_dir: source_dir.to_path_buf(),
        backup_dirs,
        enabled,
        exclude_patterns: None,
        max_backups: None,
        backup_mode: None,
        cron_schedule: None,
        enable_event_driven: None,
        enable_periodic: None,
    }
}

fn make_config(
    sources: Vec<SourceConfig>,
    backup_mode: BackupMode,
    max_backups: usize,
    exclude_patterns: Vec<String>,
) -> BackupConfig {
    BackupConfig {
        sources,
        enable_periodic: true,
        enable_event_driven: false,
        exclude_patterns,
        max_backups,
        backup_mode,
        cron_schedule: "0 0 * * * *".to_string(),
        enable_min_interval_by_size: false,
        max_log_file_size_mb: 20,
        metadata: HashMap::new(),
    }
}

fn list_backup_dirs(backup_dir: &Path) -> Result<Vec<String>> {
    let mut entries: Vec<String> = fs::read_dir(backup_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with("full_") || name.starts_with("inc_"))
        .collect();
    entries.sort();
    Ok(entries)
}

fn find_latest_dir_with_prefix(backup_dir: &Path, prefix: &str) -> Result<PathBuf> {
    let mut candidates: Vec<String> = list_backup_dirs(backup_dir)?
        .into_iter()
        .filter(|name| name.starts_with(prefix))
        .collect();
    candidates.sort();

    let selected = candidates
        .last()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{} backup directory not found", prefix))?;
    Ok(backup_dir.join(selected))
}

#[tokio::test]
async fn clears_force_full_after_first_full_in_same_process() -> Result<()> {
    let base = unique_temp_dir("ardiex_force_full_test");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let file_path = source_dir.join("sample.txt");
    fs::write(&file_path, b"v1")?;

    let source = SourceConfig {
        source_dir: source_dir.clone(),
        backup_dirs: vec![backup_dir.clone()],
        enabled: true,
        exclude_patterns: None,
        max_backups: None,
        backup_mode: None,
        cron_schedule: None,
        enable_event_driven: None,
        enable_periodic: None,
    };

    let config = BackupConfig {
        sources: vec![source],
        enable_periodic: true,
        enable_event_driven: true,
        exclude_patterns: vec![],
        max_backups: 10,
        backup_mode: BackupMode::Delta,
        cron_schedule: "0 0 * * * *".to_string(),
        enable_min_interval_by_size: true,
        max_log_file_size_mb: 20,
        metadata: HashMap::new(),
    };

    let mut manager = BackupManager::new(config);
    manager.validate_all_sources()?;

    let first = manager.backup_all_sources().await?;
    assert_eq!(first.len(), 1);
    assert!(matches!(first[0].backup_type, BackupType::Full));

    std::thread::sleep(Duration::from_millis(5));
    fs::write(&file_path, b"v2")?;

    let second = manager.backup_all_sources().await?;
    assert_eq!(second.len(), 1);
    assert!(matches!(second[0].backup_type, BackupType::Incremental));

    let entries: Vec<String> = fs::read_dir(&backup_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    assert!(entries.iter().any(|n| n.starts_with("full_")));
    assert!(entries.iter().any(|n| n.starts_with("inc_")));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[tokio::test]
async fn copy_mode_creates_incremental_copy_without_delta_file() -> Result<()> {
    let base = unique_temp_dir("ardiex_copy_mode_test");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let file_path = source_dir.join("sample.txt");
    fs::write(&file_path, b"copy_v1")?;

    let source = SourceConfig {
        source_dir: source_dir.clone(),
        backup_dirs: vec![backup_dir.clone()],
        enabled: true,
        exclude_patterns: None,
        max_backups: None,
        backup_mode: None,
        cron_schedule: None,
        enable_event_driven: None,
        enable_periodic: None,
    };

    let config = BackupConfig {
        sources: vec![source],
        enable_periodic: true,
        enable_event_driven: false,
        exclude_patterns: vec![],
        max_backups: 10,
        backup_mode: BackupMode::Copy,
        cron_schedule: "0 0 * * * *".to_string(),
        enable_min_interval_by_size: true,
        max_log_file_size_mb: 20,
        metadata: HashMap::new(),
    };

    let mut manager = BackupManager::new(config);
    manager.validate_all_sources()?;

    let first = manager.backup_all_sources().await?;
    assert_eq!(first.len(), 1);
    assert!(matches!(first[0].backup_type, BackupType::Full));

    std::thread::sleep(Duration::from_millis(5));
    fs::write(&file_path, b"copy_v2")?;

    let second = manager.backup_all_sources().await?;
    assert_eq!(second.len(), 1);
    assert!(matches!(second[0].backup_type, BackupType::Incremental));

    let entries: Vec<String> = fs::read_dir(&backup_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    let inc_name = entries
        .iter()
        .find(|n| n.starts_with("inc_"))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("incremental backup directory not found"))?;
    let inc_path = backup_dir.join(inc_name);
    let copied_file = inc_path.join("sample.txt");

    assert!(copied_file.exists());
    assert_eq!(fs::read(&copied_file)?, b"copy_v2");
    assert!(!contains_delta_file(&backup_dir)?);

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[tokio::test]
async fn validates_incremental_checksum_from_metadata_on_startup() -> Result<()> {
    let base = unique_temp_dir("ardiex_inc_checksum_test");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let file_path = source_dir.join("sample.txt");
    fs::write(&file_path, b"checksum_v1")?;

    let source = SourceConfig {
        source_dir: source_dir.clone(),
        backup_dirs: vec![backup_dir.clone()],
        enabled: true,
        exclude_patterns: None,
        max_backups: None,
        backup_mode: None,
        cron_schedule: None,
        enable_event_driven: None,
        enable_periodic: None,
    };

    let config = BackupConfig {
        sources: vec![source],
        enable_periodic: true,
        enable_event_driven: false,
        exclude_patterns: vec![],
        max_backups: 10,
        backup_mode: BackupMode::Copy,
        cron_schedule: "0 0 * * * *".to_string(),
        enable_min_interval_by_size: true,
        max_log_file_size_mb: 20,
        metadata: HashMap::new(),
    };

    let mut manager = BackupManager::new(config.clone());
    manager.validate_all_sources()?;
    manager.backup_all_sources().await?;

    std::thread::sleep(Duration::from_millis(5));
    fs::write(&file_path, b"checksum_v2")?;
    manager.backup_all_sources().await?;

    let metadata_path = backup_dir.join("metadata.json");
    let metadata_content = fs::read_to_string(&metadata_path)?;
    let metadata: SourceMetadata = serde_json::from_str(&metadata_content)?;
    let inc_entry = metadata
        .backup_history
        .iter()
        .find(|entry| matches!(entry.backup_type, BackupHistoryType::Incremental))
        .ok_or_else(|| anyhow::anyhow!("incremental backup history entry not found"))?;
    assert!(inc_entry.inc_checksum.is_some());

    let inc_dir_name = metadata
        .backup_history
        .iter()
        .find(|entry| matches!(entry.backup_type, BackupHistoryType::Incremental))
        .map(|entry| entry.backup_name.clone())
        .ok_or_else(|| anyhow::anyhow!("incremental backup directory name not found"))?;
    let inc_file = backup_dir.join(inc_dir_name).join("sample.txt");
    fs::write(&inc_file, b"tampered_incremental_backup")?;

    let mut restarted_manager = BackupManager::new(config);
    restarted_manager.validate_all_sources()?;
    let result = restarted_manager.backup_all_sources().await?;
    assert_eq!(result.len(), 1);
    assert!(matches!(result[0].backup_type, BackupType::Full));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_rejects_relative_source_path() -> Result<()> {
    let backup_dir = unique_temp_dir("ardiex_relative_backup_dir");
    fs::create_dir_all(&backup_dir)?;

    let source = SourceConfig {
        source_dir: PathBuf::from("relative/source"),
        backup_dirs: vec![backup_dir.clone()],
        enabled: true,
        exclude_patterns: None,
        max_backups: None,
        backup_mode: None,
        cron_schedule: None,
        enable_event_driven: None,
        enable_periodic: None,
    };

    let config = make_config(vec![source], BackupMode::Copy, 10, vec![]);
    let mut manager = BackupManager::new(config);
    let err = manager
        .validate_all_sources()
        .expect_err("relative source path must be rejected");
    assert!(err.to_string().contains("Source path must be absolute"));

    fs::remove_dir_all(&backup_dir)?;
    Ok(())
}

#[test]
fn validate_all_sources_auto_creates_missing_backup_dir() -> Result<()> {
    let base = unique_temp_dir("ardiex_autocreate_backup_dir");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup_new");
    fs::create_dir_all(&source_dir)?;

    let source = make_source(&source_dir, vec![backup_dir.clone()], true);
    let config = make_config(vec![source], BackupMode::Delta, 10, vec![]);
    let mut manager = BackupManager::new(config);
    manager.validate_all_sources()?;

    assert!(backup_dir.exists());
    assert!(backup_dir.is_dir());

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_marks_force_full_when_metadata_is_missing() -> Result<()> {
    let base = unique_temp_dir("ardiex_force_full_metadata_missing");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;
    fs::create_dir_all(backup_dir.join("full_20260224_120000123"))?;

    let source = make_source(&source_dir, vec![backup_dir.clone()], true);
    let config = make_config(vec![source], BackupMode::Delta, 10, vec![]);
    let mut manager = BackupManager::new(config);
    manager.validate_all_sources()?;

    assert_eq!(manager.force_full_dirs.get(&backup_dir), Some(&true));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_backup_metadata_history_fails_when_metadata_json_missing() -> Result<()> {
    let backup_dir = unique_temp_dir("ardiex_history_validation_missing_metadata");
    fs::create_dir_all(backup_dir.join("full_20260224_120000123"))?;

    let err = BackupManager::validate_backup_metadata_history(&backup_dir)
        .expect_err("history validation must fail when metadata.json is missing");
    assert!(err.to_string().contains("metadata.json is missing"));

    fs::remove_dir_all(&backup_dir)?;
    Ok(())
}

#[test]
fn load_source_metadata_returns_default_for_invalid_json() -> Result<()> {
    let backup_dir = unique_temp_dir("ardiex_invalid_metadata_json");
    fs::create_dir_all(&backup_dir)?;
    let metadata_path = backup_dir.join("metadata.json");
    fs::write(&metadata_path, "{invalid json")?;

    let metadata = BackupManager::load_source_metadata(&metadata_path);
    assert!(metadata.last_full_backup.is_none());
    assert!(metadata.last_backup.is_none());
    assert!(metadata.file_hashes.is_empty());
    assert!(metadata.backup_history.is_empty());

    fs::remove_dir_all(&backup_dir)?;
    Ok(())
}

#[test]
fn synchronize_metadata_history_with_disk_rebuilds_history_and_checksum() -> Result<()> {
    let backup_dir = unique_temp_dir("ardiex_sync_metadata_history");
    let full_dir = backup_dir.join("full_20260224_120000123");
    let inc_dir = backup_dir.join("inc_20260224_120100456");
    fs::create_dir_all(&full_dir)?;
    fs::create_dir_all(&inc_dir)?;
    fs::write(full_dir.join("a.txt"), b"full-data")?;
    fs::write(inc_dir.join("a.txt"), b"inc-data")?;

    let mut metadata = SourceMetadata::default();
    BackupManager::synchronize_metadata_history_with_disk(&backup_dir, &mut metadata)?;

    assert_eq!(metadata.backup_history.len(), 2);
    assert!(matches!(
        metadata.backup_history[0].backup_type,
        BackupHistoryType::Full
    ));
    assert!(matches!(
        metadata.backup_history[1].backup_type,
        BackupHistoryType::Incremental
    ));
    assert!(metadata.backup_history[1].inc_checksum.is_some());
    assert!(metadata.last_full_backup.is_some());
    assert!(metadata.last_backup.is_some());

    fs::remove_dir_all(&backup_dir)?;
    Ok(())
}

#[tokio::test]
async fn copy_mode_prunes_old_backups_to_max_backups() -> Result<()> {
    let base = unique_temp_dir("ardiex_copy_prune_old_backups");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let file_path = source_dir.join("sample.txt");
    fs::write(&file_path, b"v1")?;

    let source = make_source(&source_dir, vec![backup_dir.clone()], true);
    let config = make_config(vec![source], BackupMode::Copy, 2, vec![]);
    let mut manager = BackupManager::new(config);
    manager.validate_all_sources()?;

    manager.backup_all_sources().await?;
    std::thread::sleep(Duration::from_millis(5));

    fs::write(&file_path, b"v2")?;
    manager.backup_all_sources().await?;
    std::thread::sleep(Duration::from_millis(5));

    fs::write(&file_path, b"v3")?;
    manager.backup_all_sources().await?;

    let entries = list_backup_dirs(&backup_dir)?;
    assert_eq!(entries.len(), 2);

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[tokio::test]
async fn full_backup_respects_exclude_patterns() -> Result<()> {
    let base = unique_temp_dir("ardiex_full_exclude_patterns");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    fs::write(source_dir.join("keep.txt"), b"keep")?;
    fs::write(source_dir.join("drop.tmp"), b"drop")?;

    let source = make_source(&source_dir, vec![backup_dir.clone()], true);
    let config = make_config(
        vec![source],
        BackupMode::Copy,
        10,
        vec!["*.tmp".to_string()],
    );
    let mut manager = BackupManager::new(config);
    manager.validate_all_sources()?;
    manager.backup_all_sources().await?;

    let full_dir = find_latest_dir_with_prefix(&backup_dir, "full_")?;
    assert!(full_dir.join("keep.txt").exists());
    assert!(!full_dir.join("drop.tmp").exists());

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[tokio::test]
async fn disabled_source_is_skipped_in_backup_all_sources() -> Result<()> {
    let base = unique_temp_dir("ardiex_disabled_source_skip");
    let source_enabled = base.join("source_enabled");
    let source_disabled = base.join("source_disabled");
    let backup_enabled = base.join("backup_enabled");
    let backup_disabled = base.join("backup_disabled");
    fs::create_dir_all(&source_enabled)?;
    fs::create_dir_all(&source_disabled)?;
    fs::create_dir_all(&backup_enabled)?;
    fs::create_dir_all(&backup_disabled)?;
    fs::write(source_enabled.join("a.txt"), b"enabled")?;
    fs::write(source_disabled.join("b.txt"), b"disabled")?;

    let enabled_source = make_source(&source_enabled, vec![backup_enabled.clone()], true);
    let disabled_source = make_source(&source_disabled, vec![backup_disabled.clone()], false);
    let config = make_config(
        vec![enabled_source, disabled_source],
        BackupMode::Copy,
        10,
        vec![],
    );

    let mut manager = BackupManager::new(config);
    manager.validate_all_sources()?;
    let results = manager.backup_all_sources().await?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].backup_dir, backup_enabled);

    let disabled_entries = list_backup_dirs(&backup_disabled)?;
    assert!(disabled_entries.is_empty());

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[tokio::test]
async fn max_backups_interval_forces_full_after_restart_in_delta_mode() -> Result<()> {
    let base = unique_temp_dir("ardiex_auto_full_interval_restart");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;
    let file_path = source_dir.join("sample.txt");
    fs::write(&file_path, b"v1")?;

    let source = make_source(&source_dir, vec![backup_dir.clone()], true);
    let config = make_config(vec![source], BackupMode::Delta, 3, vec![]);

    let mut manager1 = BackupManager::new(config.clone());
    manager1.validate_all_sources()?;
    let result1 = manager1.backup_all_sources().await?;
    assert!(matches!(result1[0].backup_type, BackupType::Full));

    std::thread::sleep(Duration::from_millis(5));
    fs::write(&file_path, b"v2")?;
    let mut manager2 = BackupManager::new(config.clone());
    manager2.validate_all_sources()?;
    let result2 = manager2.backup_all_sources().await?;
    assert!(matches!(result2[0].backup_type, BackupType::Incremental));

    std::thread::sleep(Duration::from_millis(5));
    fs::write(&file_path, b"v3")?;
    let mut manager3 = BackupManager::new(config.clone());
    manager3.validate_all_sources()?;
    let result3 = manager3.backup_all_sources().await?;
    assert!(matches!(result3[0].backup_type, BackupType::Incremental));

    std::thread::sleep(Duration::from_millis(5));
    fs::write(&file_path, b"v4")?;
    let mut manager4 = BackupManager::new(config);
    manager4.validate_all_sources()?;
    let result4 = manager4.backup_all_sources().await?;
    assert!(matches!(result4[0].backup_type, BackupType::Full));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn calculate_min_interval_by_size_respects_thresholds() -> Result<()> {
    let base = unique_temp_dir("ardiex_interval_by_size");
    fs::create_dir_all(&base)?;

    // <= 10MB => 1s
    let tiny = base.join("tiny.bin");
    fs::write(&tiny, vec![0u8; 1024])?;
    assert_eq!(
        BackupManager::calculate_min_interval_by_size(&base),
        Duration::from_secs(1)
    );

    // >10MB && <=100MB => 60s
    let mid = base.join("mid.bin");
    let mid_file = fs::File::create(&mid)?;
    mid_file.set_len(11 * 1024 * 1024)?;
    assert_eq!(
        BackupManager::calculate_min_interval_by_size(&base),
        Duration::from_secs(60)
    );

    // >100MB && <=1GB => 3600s
    let large = base.join("large.bin");
    let large_file = fs::File::create(&large)?;
    large_file.set_len(110 * 1024 * 1024)?;
    assert_eq!(
        BackupManager::calculate_min_interval_by_size(&base),
        Duration::from_secs(3600)
    );

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn scan_for_changes_returns_full_without_last_full_metadata() -> Result<()> {
    let base = unique_temp_dir("ardiex_scan_full_without_marker");
    fs::create_dir_all(&base)?;
    fs::write(base.join("a.txt"), b"v1")?;
    let metadata = SourceMetadata::default();

    let (backup_type, files, _current_hashes) =
        BackupManager::scan_for_changes(&base, &metadata, &[])?;
    assert!(matches!(backup_type, BackupType::Full));
    assert_eq!(files.len(), 1);

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn scan_for_changes_returns_only_modified_files_for_incremental() -> Result<()> {
    let base = unique_temp_dir("ardiex_scan_incremental_changes");
    fs::create_dir_all(&base)?;
    let a = base.join("a.txt");
    let b = base.join("b.txt");
    fs::write(&a, b"same")?;
    fs::write(&b, b"old")?;

    let a_hash = BackupManager::calculate_file_hash(&a)?;
    let b_hash_old = BackupManager::calculate_file_hash(&b)?;
    fs::write(&b, b"new")?;

    let mut metadata = SourceMetadata {
        last_full_backup: Some(chrono::Utc::now()),
        last_backup: Some(chrono::Utc::now()),
        file_hashes: HashMap::new(),
        backup_history: vec![],
    };
    metadata.file_hashes.insert("a.txt".to_string(), a_hash);
    metadata.file_hashes.insert("b.txt".to_string(), b_hash_old);

    let (backup_type, files, _current_hashes) =
        BackupManager::scan_for_changes(&base, &metadata, &[])?;
    assert!(matches!(backup_type, BackupType::Incremental));
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_name().unwrap_or_default(), "b.txt");

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn find_latest_backup_file_prefers_most_recent_backup() -> Result<()> {
    let base = unique_temp_dir("ardiex_find_latest_backup");
    fs::create_dir_all(&base)?;
    let full = base.join("full_20260224_120000");
    let inc = base.join("inc_20260224_121000");
    fs::create_dir_all(&full)?;
    fs::create_dir_all(&inc)?;
    fs::write(full.join("a.txt"), b"full")?;
    std::thread::sleep(Duration::from_millis(5));
    fs::write(inc.join("a.txt"), b"inc")?;

    let found = BackupManager::find_latest_backup_file(&base, Path::new("a.txt"))
        .ok_or_else(|| anyhow::anyhow!("latest backup file not found"))?;
    assert_eq!(fs::read(found)?, b"inc");

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn cleanup_old_backups_delta_mode_preserves_latest_full_chain() -> Result<()> {
    let base = unique_temp_dir("ardiex_cleanup_delta_chain");
    fs::create_dir_all(&base)?;

    let full1 = base.join("full_20260224_100000");
    let inc1 = base.join("inc_20260224_101000");
    let full2 = base.join("full_20260224_110000");
    let inc2 = base.join("inc_20260224_111000");
    let inc3 = base.join("inc_20260224_112000");
    for dir in [&full1, &inc1, &full2, &inc2, &inc3] {
        fs::create_dir_all(dir)?;
        fs::write(dir.join("marker.txt"), dir.to_string_lossy().as_bytes())?;
        std::thread::sleep(Duration::from_millis(5));
    }

    BackupManager::cleanup_old_backups(&base, 2, &BackupMode::Delta)?;
    let entries = list_backup_dirs(&base)?;
    assert_eq!(
        entries,
        vec![
            "full_20260224_110000".to_string(),
            "inc_20260224_111000".to_string(),
            "inc_20260224_112000".to_string(),
        ]
    );

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_forces_full_when_chain_has_no_full_backup() -> Result<()> {
    let base = unique_temp_dir("ardiex_validate_chain_no_full");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    let inc = backup_dir.join("inc_20260224_121000");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&inc)?;
    fs::write(inc.join("a.txt"), b"v1")?;

    let mut metadata = SourceMetadata::default();
    BackupManager::synchronize_metadata_history_with_disk(&backup_dir, &mut metadata)?;
    fs::write(
        backup_dir.join("metadata.json"),
        serde_json::to_string_pretty(&metadata)?,
    )?;

    let source = make_source(&source_dir, vec![backup_dir.clone()], true);
    let config = make_config(vec![source], BackupMode::Delta, 10, vec![]);
    let mut manager = BackupManager::new(config);
    manager.validate_all_sources()?;
    assert_eq!(manager.force_full_dirs.get(&backup_dir), Some(&true));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_forces_full_when_auto_interval_is_reached() -> Result<()> {
    let base = unique_temp_dir("ardiex_force_full_auto_interval");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    for name in [
        "full_20260224_100000",
        "inc_20260224_101000",
        "inc_20260224_102000",
    ] {
        let dir = backup_dir.join(name);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("a.txt"), name.as_bytes())?;
        std::thread::sleep(Duration::from_millis(5));
    }

    let mut metadata = SourceMetadata::default();
    BackupManager::synchronize_metadata_history_with_disk(&backup_dir, &mut metadata)?;
    fs::write(
        backup_dir.join("metadata.json"),
        serde_json::to_string_pretty(&metadata)?,
    )?;

    let source = make_source(&source_dir, vec![backup_dir.clone()], true);
    // max_backups=3 => auto interval = 2. current inc tail is 2, so startup should force full.
    let config = make_config(vec![source], BackupMode::Delta, 3, vec![]);
    let mut manager = BackupManager::new(config);
    manager.validate_all_sources()?;
    assert_eq!(manager.force_full_dirs.get(&backup_dir), Some(&true));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[tokio::test]
async fn multi_backup_dirs_receive_full_and_incremental_backups() -> Result<()> {
    let base = unique_temp_dir("ardiex_multi_backup_dirs");
    let source_dir = base.join("source");
    let backup_a = base.join("backup_a");
    let backup_b = base.join("backup_b");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_a)?;
    fs::create_dir_all(&backup_b)?;
    let file_path = source_dir.join("sample.txt");
    fs::write(&file_path, b"v1")?;

    let source = make_source(&source_dir, vec![backup_a.clone(), backup_b.clone()], true);
    let config = make_config(vec![source], BackupMode::Copy, 10, vec![]);
    let mut manager = BackupManager::new(config);
    manager.validate_all_sources()?;

    let first = manager.backup_all_sources().await?;
    assert_eq!(first.len(), 2);
    assert!(
        first
            .iter()
            .all(|r| matches!(r.backup_type, BackupType::Full))
    );

    std::thread::sleep(Duration::from_millis(5));
    fs::write(&file_path, b"v2")?;
    let second = manager.backup_all_sources().await?;
    assert_eq!(second.len(), 2);
    assert!(
        second
            .iter()
            .all(|r| matches!(r.backup_type, BackupType::Incremental))
    );

    for backup_dir in [&backup_a, &backup_b] {
        let entries = list_backup_dirs(backup_dir)?;
        assert!(entries.iter().any(|name| name.starts_with("full_")));
        assert!(entries.iter().any(|name| name.starts_with("inc_")));
    }

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_forces_full_on_metadata_history_mismatch() -> Result<()> {
    let base = unique_temp_dir("ardiex_metadata_mismatch_force_full");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let full_dir = backup_dir.join("full_20260224_100000123");
    let inc_dir = backup_dir.join("inc_20260224_101000456");
    fs::create_dir_all(&full_dir)?;
    fs::create_dir_all(&inc_dir)?;
    fs::write(full_dir.join("a.txt"), b"full")?;
    fs::write(inc_dir.join("a.txt"), b"inc")?;

    let mut metadata = SourceMetadata::default();
    BackupManager::synchronize_metadata_history_with_disk(&backup_dir, &mut metadata)?;
    metadata.backup_history.pop();
    fs::write(
        backup_dir.join("metadata.json"),
        serde_json::to_string_pretty(&metadata)?,
    )?;

    let source = make_source(&source_dir, vec![backup_dir.clone()], true);
    let config = make_config(vec![source], BackupMode::Delta, 10, vec![]);
    let mut manager = BackupManager::new(config);
    manager.validate_all_sources()?;
    assert_eq!(manager.force_full_dirs.get(&backup_dir), Some(&true));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[tokio::test]
async fn deleted_file_is_removed_from_metadata_hashes() -> Result<()> {
    let base = unique_temp_dir("ardiex_deleted_file_metadata_cleanup");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let keep = source_dir.join("keep.txt");
    let remove = source_dir.join("remove.txt");
    fs::write(&keep, b"keep-v1")?;
    fs::write(&remove, b"remove-v1")?;

    let source = make_source(&source_dir, vec![backup_dir.clone()], true);
    let config = make_config(vec![source], BackupMode::Copy, 10, vec![]);
    let mut manager = BackupManager::new(config.clone());
    manager.validate_all_sources()?;
    manager.backup_all_sources().await?;

    fs::remove_file(&remove)?;
    std::thread::sleep(Duration::from_millis(5));
    let result = manager.backup_all_sources().await?;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].files_backed_up, 0);

    let metadata_path = backup_dir.join("metadata.json");
    let metadata: SourceMetadata = serde_json::from_str(&fs::read_to_string(&metadata_path)?)?;
    assert!(metadata.file_hashes.contains_key("keep.txt"));
    assert!(!metadata.file_hashes.contains_key("remove.txt"));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[tokio::test]
async fn delta_mode_creates_delta_file_when_previous_backup_exists() -> Result<()> {
    let base = unique_temp_dir("ardiex_delta_file_creation");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let file_path = source_dir.join("sample.txt");
    fs::write(&file_path, b"delta-v1")?;

    let source = make_source(&source_dir, vec![backup_dir.clone()], true);
    let config = make_config(vec![source], BackupMode::Delta, 10, vec![]);
    let mut manager = BackupManager::new(config);
    manager.validate_all_sources()?;
    manager.backup_all_sources().await?;

    std::thread::sleep(Duration::from_millis(5));
    fs::write(&file_path, b"delta-v2")?;
    manager.backup_all_sources().await?;

    let inc_dir = find_latest_dir_with_prefix(&backup_dir, "inc_")?;
    assert!(contains_delta_file(&inc_dir)?);

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_rejects_duplicate_sources() -> Result<()> {
    let base = unique_temp_dir("ardiex_duplicate_sources");
    let source_dir = base.join("source");
    let backup_a = base.join("backup_a");
    let backup_b = base.join("backup_b");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_a)?;
    fs::create_dir_all(&backup_b)?;

    let source1 = make_source(&source_dir, vec![backup_a], true);
    let source2 = make_source(&source_dir, vec![backup_b], true);
    let config = make_config(vec![source1, source2], BackupMode::Copy, 10, vec![]);
    let mut manager = BackupManager::new(config);

    let err = manager
        .validate_all_sources()
        .expect_err("duplicate sources must be rejected");
    assert!(err.to_string().contains("Duplicate source directory"));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_rejects_duplicate_backup_dirs() -> Result<()> {
    let base = unique_temp_dir("ardiex_duplicate_backup_dirs");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let source = make_source(&source_dir, vec![backup_dir.clone(), backup_dir], true);
    let config = make_config(vec![source], BackupMode::Copy, 10, vec![]);
    let mut manager = BackupManager::new(config);

    let err = manager
        .validate_all_sources()
        .expect_err("duplicate backup dirs must be rejected");
    assert!(err.to_string().contains("Duplicate backup directory"));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_rejects_same_source_and_backup_path() -> Result<()> {
    let base = unique_temp_dir("ardiex_source_equals_backup");
    let source_dir = base.join("source");
    fs::create_dir_all(&source_dir)?;

    let source = make_source(&source_dir, vec![source_dir.clone()], true);
    let config = make_config(vec![source], BackupMode::Copy, 10, vec![]);
    let mut manager = BackupManager::new(config);

    let err = manager
        .validate_all_sources()
        .expect_err("same source/backup path must be rejected");
    assert!(
        err.to_string()
            .contains("Backup directory cannot be the same as source")
    );

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_rejects_nonexistent_source_dir() -> Result<()> {
    let base = unique_temp_dir("ardiex_nonexistent_source");
    let source_dir = base.join("missing_source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&backup_dir)?;

    let source = make_source(&source_dir, vec![backup_dir], true);
    let config = make_config(vec![source], BackupMode::Copy, 10, vec![]);
    let mut manager = BackupManager::new(config);

    let err = manager
        .validate_all_sources()
        .expect_err("nonexistent source must be rejected");
    assert!(err.to_string().contains("Source directory does not exist"));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_rejects_source_file_path() -> Result<()> {
    let base = unique_temp_dir("ardiex_source_file_path");
    let source_file = base.join("source_file.txt");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&base)?;
    fs::write(&source_file, b"not a dir")?;
    fs::create_dir_all(&backup_dir)?;

    let source = make_source(&source_file, vec![backup_dir], true);
    let config = make_config(vec![source], BackupMode::Copy, 10, vec![]);
    let mut manager = BackupManager::new(config);

    let err = manager
        .validate_all_sources()
        .expect_err("source file path must be rejected");
    assert!(err.to_string().contains("Source path is not a directory"));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_rejects_source_max_backups_zero() -> Result<()> {
    let base = unique_temp_dir("ardiex_source_max_backups_zero");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let mut source = make_source(&source_dir, vec![backup_dir], true);
    source.max_backups = Some(0);
    let config = make_config(vec![source], BackupMode::Copy, 10, vec![]);
    let mut manager = BackupManager::new(config);

    let err = manager
        .validate_all_sources()
        .expect_err("source max_backups=0 must be rejected");
    assert!(err.to_string().contains("max_backups must be > 0"));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_rejects_source_invalid_cron() -> Result<()> {
    let base = unique_temp_dir("ardiex_source_invalid_cron");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let mut source = make_source(&source_dir, vec![backup_dir], true);
    source.cron_schedule = Some("invalid cron".to_string());
    let config = make_config(vec![source], BackupMode::Copy, 10, vec![]);
    let mut manager = BackupManager::new(config);

    let err = manager
        .validate_all_sources()
        .expect_err("invalid source cron must be rejected");
    assert!(err.to_string().contains("invalid cron_schedule"));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_rejects_invalid_global_cron() -> Result<()> {
    let base = unique_temp_dir("ardiex_global_invalid_cron");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let source = make_source(&source_dir, vec![backup_dir], true);
    let mut config = make_config(vec![source], BackupMode::Copy, 10, vec![]);
    config.cron_schedule = "invalid cron".to_string();
    let mut manager = BackupManager::new(config);

    let err = manager
        .validate_all_sources()
        .expect_err("invalid global cron must be rejected");
    assert!(err.to_string().contains("Invalid global cron_schedule"));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_rejects_global_max_backups_zero() -> Result<()> {
    let base = unique_temp_dir("ardiex_global_max_backups_zero");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let source = make_source(&source_dir, vec![backup_dir], true);
    let mut config = make_config(vec![source], BackupMode::Copy, 10, vec![]);
    config.max_backups = 0;
    let mut manager = BackupManager::new(config);

    let err = manager
        .validate_all_sources()
        .expect_err("global max_backups=0 must be rejected");
    assert!(err.to_string().contains("Global max_backups must be > 0"));

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn validate_all_sources_rejects_global_log_size_zero() -> Result<()> {
    let base = unique_temp_dir("ardiex_global_log_size_zero");
    let source_dir = base.join("source");
    let backup_dir = base.join("backup");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let source = make_source(&source_dir, vec![backup_dir], true);
    let mut config = make_config(vec![source], BackupMode::Copy, 10, vec![]);
    config.max_log_file_size_mb = 0;
    let mut manager = BackupManager::new(config);

    let err = manager
        .validate_all_sources()
        .expect_err("global log size 0 must be rejected");
    assert!(
        err.to_string()
            .contains("Global max_log_file_size_mb must be > 0")
    );

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn scan_for_changes_fails_for_missing_source_directory() {
    let missing = PathBuf::from("/tmp/ardiex_missing_source_for_scan");
    let metadata = SourceMetadata::default();
    let err = BackupManager::scan_for_changes(&missing, &metadata, &[])
        .expect_err("missing source must return error");
    assert!(err.to_string().contains("Source directory does not exist"));
}

#[test]
fn validate_backup_metadata_history_fails_when_only_incremental_exists() -> Result<()> {
    let backup_dir = unique_temp_dir("ardiex_history_only_incremental");
    let inc_dir = backup_dir.join("inc_20260224_100000123");
    fs::create_dir_all(&inc_dir)?;
    fs::write(inc_dir.join("a.txt"), b"v1")?;

    let mut metadata = SourceMetadata::default();
    BackupManager::synchronize_metadata_history_with_disk(&backup_dir, &mut metadata)?;
    fs::write(
        backup_dir.join("metadata.json"),
        serde_json::to_string_pretty(&metadata)?,
    )?;

    let err = BackupManager::validate_backup_metadata_history(&backup_dir)
        .expect_err("incremental before full must be rejected");
    assert!(
        err.to_string()
            .contains("incremental backup appears before any full backup")
    );

    fs::remove_dir_all(&backup_dir)?;
    Ok(())
}
