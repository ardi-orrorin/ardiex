use super::*;
use crate::config::{BackupHistoryEntry, BackupHistoryType, SourceMetadata};
use chrono::{DateTime, NaiveDateTime, Utc};
use log::warn;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
struct BackupDirEntry {
    backup_name: String,
    backup_type: BackupHistoryType,
    created_at: DateTime<Utc>,
    backup_path: PathBuf,
}

impl BackupManager {
    pub(crate) fn load_source_metadata(metadata_path: &Path) -> SourceMetadata {
        if !metadata_path.exists() {
            return SourceMetadata::default();
        }

        match fs::read_to_string(metadata_path) {
            Ok(content) => match serde_json::from_str::<SourceMetadata>(&content) {
                Ok(metadata) => metadata,
                Err(e) => {
                    warn!(
                        "Failed to parse metadata file {:?}, using default: {}",
                        metadata_path, e
                    );
                    SourceMetadata::default()
                }
            },
            Err(e) => {
                warn!(
                    "Failed to read metadata file {:?}, using default: {}",
                    metadata_path, e
                );
                SourceMetadata::default()
            }
        }
    }

    fn backup_history_type_from_name(backup_name: &str) -> Option<BackupHistoryType> {
        if backup_name.starts_with("full_") {
            Some(BackupHistoryType::Full)
        } else if backup_name.starts_with("inc_") {
            Some(BackupHistoryType::Incremental)
        } else {
            None
        }
    }

    fn parse_backup_created_at(backup_name: &str) -> Option<DateTime<Utc>> {
        let ts = backup_name
            .strip_prefix("full_")
            .or_else(|| backup_name.strip_prefix("inc_"))?;

        for fmt in ["%Y%m%d_%H%M%S%3f", "%Y%m%d_%H%M%S"] {
            if let Ok(naive) = NaiveDateTime::parse_from_str(ts, fmt) {
                return Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
            }
        }

        None
    }

    fn collect_backup_dir_stats(backup_path: &Path) -> Result<(usize, u64)> {
        let mut files = 0usize;
        let mut bytes = 0u64;

        for entry in WalkDir::new(backup_path).into_iter() {
            let entry = entry?;
            if entry.file_type().is_file() {
                files += 1;
                bytes += entry.metadata()?.len();
            }
        }

        Ok((files, bytes))
    }

    fn scan_backup_entries_from_disk(backup_dir: &Path) -> Result<Vec<BackupDirEntry>> {
        if !backup_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(backup_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let backup_name = entry.file_name().to_string_lossy().to_string();
            let Some(backup_type) = Self::backup_history_type_from_name(&backup_name) else {
                continue;
            };

            let created_at = Self::parse_backup_created_at(&backup_name).ok_or_else(|| {
                anyhow::anyhow!("Invalid backup directory timestamp format: {}", backup_name)
            })?;

            entries.push(BackupDirEntry {
                backup_name,
                backup_type,
                created_at,
                backup_path: path,
            });
        }

        entries.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.backup_name.cmp(&b.backup_name))
        });

        Ok(entries)
    }

    fn build_history_from_entries(entries: &[BackupDirEntry]) -> Result<Vec<BackupHistoryEntry>> {
        let mut history = Vec::with_capacity(entries.len());

        for entry in entries {
            let (files_backed_up, bytes_processed) =
                Self::collect_backup_dir_stats(&entry.backup_path)?;

            history.push(BackupHistoryEntry {
                backup_name: entry.backup_name.clone(),
                backup_type: entry.backup_type.clone(),
                created_at: entry.created_at,
                files_backed_up,
                bytes_processed,
            });
        }

        Ok(history)
    }

    fn refresh_metadata_markers(metadata: &mut SourceMetadata) {
        metadata.backup_history.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.backup_name.cmp(&b.backup_name))
        });

        metadata.last_backup = metadata.backup_history.last().map(|entry| entry.created_at);
        metadata.last_full_backup = metadata
            .backup_history
            .iter()
            .rev()
            .find(|entry| matches!(entry.backup_type, BackupHistoryType::Full))
            .map(|entry| entry.created_at);
    }

    pub(crate) fn synchronize_metadata_history_with_disk(
        backup_dir: &Path,
        metadata: &mut SourceMetadata,
    ) -> Result<()> {
        let entries = Self::scan_backup_entries_from_disk(backup_dir)?;
        metadata.backup_history = Self::build_history_from_entries(&entries)?;
        Self::refresh_metadata_markers(metadata);
        Ok(())
    }

    pub(crate) fn append_backup_history_entry(
        metadata: &mut SourceMetadata,
        backup_name: &str,
        backup_type: &BackupType,
        created_at: DateTime<Utc>,
        files_backed_up: usize,
        bytes_processed: u64,
    ) {
        let backup_type = match backup_type {
            BackupType::Full => BackupHistoryType::Full,
            BackupType::Incremental => BackupHistoryType::Incremental,
        };

        metadata
            .backup_history
            .retain(|entry| entry.backup_name != backup_name);
        metadata.backup_history.push(BackupHistoryEntry {
            backup_name: backup_name.to_string(),
            backup_type,
            created_at,
            files_backed_up,
            bytes_processed,
        });

        Self::refresh_metadata_markers(metadata);
    }

    pub(crate) fn validate_backup_metadata_history(backup_dir: &Path) -> Result<()> {
        let disk_entries = Self::scan_backup_entries_from_disk(backup_dir)?;
        if disk_entries.is_empty() {
            return Ok(());
        }

        let metadata_path = backup_dir.join("metadata.json");
        if !metadata_path.exists() {
            return Err(anyhow::anyhow!(
                "metadata.json is missing while backup directories exist: {:?}",
                backup_dir
            ));
        }

        let metadata = Self::load_source_metadata(&metadata_path);
        if metadata.backup_history.is_empty() {
            return Err(anyhow::anyhow!(
                "metadata.backup_history is empty while backup directories exist: {:?}",
                backup_dir
            ));
        }

        let mut metadata_history = metadata.backup_history.clone();
        metadata_history.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.backup_name.cmp(&b.backup_name))
        });

        let disk_history = Self::build_history_from_entries(&disk_entries)?;
        if metadata_history.len() != disk_history.len() {
            return Err(anyhow::anyhow!(
                "metadata history count mismatch: metadata={}, disk={}",
                metadata_history.len(),
                disk_history.len()
            ));
        }

        for (index, (meta, disk)) in metadata_history.iter().zip(disk_history.iter()).enumerate() {
            if meta.backup_name != disk.backup_name {
                return Err(anyhow::anyhow!(
                    "metadata history mismatch at index {}: backup_name metadata='{}', disk='{}'",
                    index,
                    meta.backup_name,
                    disk.backup_name
                ));
            }
            if meta.backup_type != disk.backup_type {
                return Err(anyhow::anyhow!(
                    "metadata history mismatch at index {}: backup_type metadata='{:?}', disk='{:?}'",
                    index,
                    meta.backup_type,
                    disk.backup_type
                ));
            }
            if meta.files_backed_up != disk.files_backed_up {
                return Err(anyhow::anyhow!(
                    "metadata history mismatch at index {}: files_backed_up metadata={}, disk={}",
                    index,
                    meta.files_backed_up,
                    disk.files_backed_up
                ));
            }
            if meta.bytes_processed != disk.bytes_processed {
                return Err(anyhow::anyhow!(
                    "metadata history mismatch at index {}: bytes_processed metadata={}, disk={}",
                    index,
                    meta.bytes_processed,
                    disk.bytes_processed
                ));
            }
        }

        let mut seen_full = false;
        for entry in &metadata_history {
            match entry.backup_type {
                BackupHistoryType::Full => seen_full = true,
                BackupHistoryType::Incremental if !seen_full => {
                    return Err(anyhow::anyhow!(
                        "incremental backup appears before any full backup: {}",
                        entry.backup_name
                    ));
                }
                BackupHistoryType::Incremental => {}
            }
        }

        let expected_last_full = metadata_history
            .iter()
            .rev()
            .find(|entry| matches!(entry.backup_type, BackupHistoryType::Full))
            .map(|entry| entry.created_at);
        let expected_last_backup = metadata_history.last().map(|entry| entry.created_at);

        if metadata.last_full_backup != expected_last_full {
            return Err(anyhow::anyhow!(
                "metadata.last_full_backup mismatch: metadata={:?}, expected={:?}",
                metadata.last_full_backup,
                expected_last_full
            ));
        }
        if metadata.last_backup != expected_last_backup {
            return Err(anyhow::anyhow!(
                "metadata.last_backup mismatch: metadata={:?}, expected={:?}",
                metadata.last_backup,
                expected_last_backup
            ));
        }

        Ok(())
    }
}
