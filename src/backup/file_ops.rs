use super::*;
use crate::config::{BackupMode, SourceMetadata};
use log::{info, warn};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

impl BackupManager {
    pub(super) fn scan_for_changes(
        source_dir: &Path,
        metadata: &SourceMetadata,
        exclude_patterns: &[String],
    ) -> Result<(BackupType, Vec<PathBuf>, HashMap<String, String>)> {
        let mut files_to_backup = Vec::new();
        let mut current_hashes = HashMap::new();

        if !source_dir.exists() {
            return Err(anyhow::anyhow!(
                "Source directory does not exist: {:?}",
                source_dir
            ));
        }

        Self::collect_files(
            source_dir,
            source_dir,
            &mut files_to_backup,
            &mut current_hashes,
            exclude_patterns,
        )?;

        let backup_type = if metadata.last_full_backup.is_none() {
            BackupType::Full
        } else {
            BackupType::Incremental
        };

        if matches!(backup_type, BackupType::Full) {
            return Ok((backup_type, files_to_backup, current_hashes));
        }

        let changed_files: Vec<PathBuf> = files_to_backup
            .into_iter()
            .filter(|path| {
                let relative_path = path.strip_prefix(source_dir).unwrap_or(path.as_path());
                let path_str = relative_path.to_string_lossy();

                let current_hash = current_hashes.get(path_str.as_ref());
                let stored_hash = metadata.file_hashes.get(path_str.as_ref());

                current_hash != stored_hash
            })
            .collect();

        Ok((backup_type, changed_files, current_hashes))
    }

    pub(super) fn collect_all_files(
        source_dir: &Path,
        exclude_patterns: &[String],
    ) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let mut hashes = HashMap::new();
        Self::collect_files(
            source_dir,
            source_dir,
            &mut files,
            &mut hashes,
            exclude_patterns,
        )?;
        Ok(files)
    }

    fn collect_files(
        base_dir: &Path,
        dir: &Path,
        files: &mut Vec<PathBuf>,
        hashes: &mut HashMap<String, String>,
        exclude_patterns: &[String],
    ) -> Result<()> {
        let entries = fs::read_dir(dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if Self::should_exclude(&path, exclude_patterns) {
                continue;
            }

            if path.is_dir() {
                Self::collect_files(base_dir, &path, files, hashes, exclude_patterns)?;
            } else {
                let relative_path = path.strip_prefix(base_dir).unwrap_or(path.as_path());
                let path_str = relative_path.to_string_lossy();
                let hash = Self::calculate_file_hash(&path)?;
                hashes.insert(path_str.to_string(), hash);
                files.push(path);
            }
        }

        Ok(())
    }

    fn should_exclude(path: &Path, patterns: &[String]) -> bool {
        let path_str = path.to_string_lossy();

        for pattern in patterns {
            if pattern.contains('*') {
                let parts: Vec<&str> = pattern.split('*').collect();
                if parts.len() == 2
                    && path_str.starts_with(parts[0])
                    && path_str.ends_with(parts[1])
                {
                    return true;
                }
            } else if path_str.contains(pattern) {
                return true;
            }
        }

        false
    }

    pub(super) fn calculate_file_hash(path: &Path) -> Result<String> {
        let file = fs::File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut hasher = Sha256::new();
        let mut buffer = [0; 8192];

        loop {
            let bytes_read = reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }

        Ok(format!("{:x}", hasher.finalize()))
    }

    pub(super) fn find_latest_backup_file(
        backup_dir: &Path,
        relative_path: &Path,
    ) -> Option<PathBuf> {
        let mut backup_dirs: Vec<_> = fs::read_dir(backup_dir)
            .ok()?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                (name.starts_with("full_") || name.starts_with("inc_")) && entry.path().is_dir()
            })
            .collect();

        backup_dirs.sort_by(|a, b| {
            let a_time = a
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            let b_time = b
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            b_time.cmp(&a_time)
        });

        for dir in backup_dirs {
            let candidate = dir.path().join(relative_path);
            if candidate.exists() && candidate.is_file() {
                return Some(candidate);
            }
        }

        None
    }

    pub(super) fn cleanup_old_backups(
        backup_dir: &Path,
        max_backups: usize,
        backup_mode: &BackupMode,
    ) -> Result<()> {
        let mut backups: Vec<_> = fs::read_dir(backup_dir)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                name.starts_with("full_") || name.starts_with("inc_")
            })
            .collect();

        backups.sort_by(|a, b| {
            let a_time = a
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            let b_time = b
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            a_time.cmp(&b_time)
        });

        if backups.len() <= max_backups {
            return Ok(());
        }

        let keep_count = if matches!(backup_mode, BackupMode::Delta) {
            let latest_full_idx = backups
                .iter()
                .rposition(|entry| entry.file_name().to_string_lossy().starts_with("full_"));

            let protect_count = match latest_full_idx {
                Some(idx) => backups.len() - idx,
                None => backups.len(),
            };

            max_backups.max(protect_count)
        } else {
            max_backups
        };

        if backups.len() <= keep_count {
            return Ok(());
        }

        let to_remove = backups.len() - keep_count;
        for old_backup in backups.iter().take(to_remove) {
            let path = old_backup.path();
            if path.is_dir() {
                if let Err(e) = fs::remove_dir_all(&path) {
                    warn!("Failed to remove old backup {:?}: {}", path, e);
                } else {
                    info!("Removed old backup: {:?}", path);
                }
            }
        }

        if keep_count > max_backups {
            warn!(
                "Keeping {} backups (> max_backups={}) to preserve delta restore chain",
                keep_count, max_backups
            );
        }

        Ok(())
    }
}
