use super::*;
use crate::config::{BackupConfig, BackupMode, SourceConfig};
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
