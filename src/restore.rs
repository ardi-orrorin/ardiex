use anyhow::{Context, Result};
use log::info;
use std::fs;
use std::path::{Path, PathBuf};

use crate::delta;

#[derive(Debug)]
pub struct BackupEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_full: bool,
    pub timestamp: String,
}

pub struct RestoreManager;

impl RestoreManager {
    pub fn list_backups(backup_dir: &Path) -> Result<Vec<BackupEntry>> {
        let mut entries: Vec<BackupEntry> = fs::read_dir(backup_dir)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                let path = entry.path();

                if !path.is_dir() {
                    return None;
                }

                let is_full = name.starts_with("full_");
                let is_inc = name.starts_with("inc_");

                if !is_full && !is_inc {
                    return None;
                }

                let timestamp = if is_full {
                    name.strip_prefix("full_").unwrap_or("").to_string()
                } else {
                    name.strip_prefix("inc_").unwrap_or("").to_string()
                };

                Some(BackupEntry {
                    name,
                    path,
                    is_full,
                    timestamp,
                })
            })
            .collect();

        entries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        Ok(entries)
    }

    pub fn restore_to_point(
        backup_dir: &Path,
        target_dir: &Path,
        restore_point: Option<&str>,
    ) -> Result<usize> {
        let backups = Self::list_backups(backup_dir)?;

        if backups.is_empty() {
            return Err(anyhow::anyhow!("No backups found in {:?}", backup_dir));
        }

        // Determine which backups to apply
        let backups_to_apply = Self::select_backups(&backups, restore_point)?;

        fs::create_dir_all(target_dir)
            .with_context(|| format!("Failed to create restore directory: {:?}", target_dir))?;

        let mut total_files_restored = 0;
        let total_backups = backups_to_apply.len();

        for (i, backup) in backups_to_apply.iter().enumerate() {
            let files_restored = Self::apply_backup(backup, target_dir)?;
            total_files_restored += files_restored;
            let progress = ((i + 1) * 100) / total_backups;
            info!(
                "Restore progress: {}% - Applied backup '{}': {} files restored",
                progress, backup.name, files_restored
            );
        }

        info!(
            "Restore completed: {} total files restored to {:?}",
            total_files_restored, target_dir
        );

        Ok(total_files_restored)
    }

    fn select_backups<'a>(
        backups: &'a [BackupEntry],
        restore_point: Option<&str>,
    ) -> Result<Vec<&'a BackupEntry>> {
        // Find the latest full backup before the restore point
        let cutoff = restore_point.unwrap_or("99999999_999999");

        let latest_full = backups
            .iter()
            .rfind(|b| b.is_full && b.timestamp.as_str() <= cutoff);

        let full_backup = match latest_full {
            Some(b) => b,
            None => return Err(anyhow::anyhow!("No full backup found before restore point")),
        };

        let mut result = vec![full_backup];

        // Add incremental backups after the full backup and before the restore point
        for backup in backups {
            if !backup.is_full
                && backup.timestamp.as_str() > full_backup.timestamp.as_str()
                && backup.timestamp.as_str() <= cutoff
            {
                result.push(backup);
            }
        }

        Ok(result)
    }

    fn apply_backup(backup: &BackupEntry, target_dir: &Path) -> Result<usize> {
        // Count total files first for progress tracking
        let total_files = Self::count_files(&backup.path)?;
        let mut files_restored = 0;
        let mut last_progress = 0;

        Self::restore_dir_recursive(
            &backup.path,
            &backup.path,
            target_dir,
            &mut files_restored,
            total_files,
            &mut last_progress,
        )?;

        Ok(files_restored)
    }

    fn count_files(dir: &Path) -> Result<usize> {
        let mut count = 0;
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                count += Self::count_files(&path)?;
            } else {
                count += 1;
            }
        }
        Ok(count)
    }

    fn restore_dir_recursive(
        base_backup_path: &Path,
        current_path: &Path,
        target_dir: &Path,
        files_restored: &mut usize,
        total_files: usize,
        last_progress: &mut usize,
    ) -> Result<()> {
        for entry in fs::read_dir(current_path)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                Self::restore_dir_recursive(
                    base_backup_path,
                    &path,
                    target_dir,
                    files_restored,
                    total_files,
                    last_progress,
                )?;
            } else {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();

                if file_name.ends_with(".delta") {
                    // Delta file: apply delta to restore
                    let relative_path = path.strip_prefix(base_backup_path)?;
                    // Remove .delta extension to get the original relative path
                    let original_rel = Self::strip_delta_extension(relative_path);
                    let target_file = target_dir.join(&original_rel);

                    let delta_data = delta::load_delta(&path)?;

                    if target_file.exists() {
                        // Apply delta on top of existing restored file
                        let temp_file = target_file.with_extension("tmp_restore");
                        delta::apply_delta(&target_file, &delta_data, &temp_file)?;
                        fs::rename(&temp_file, &target_file)?;
                    } else {
                        // Apply delta with empty base
                        let empty_path = target_file.with_extension("tmp_empty");
                        delta::apply_delta(&empty_path, &delta_data, &target_file)?;
                    }

                    *files_restored += 1;
                } else {
                    // Regular file: copy directly
                    let relative_path = path.strip_prefix(base_backup_path)?;
                    let target_file = target_dir.join(relative_path);

                    if let Some(parent) = target_file.parent() {
                        fs::create_dir_all(parent)?;
                    }

                    fs::copy(&path, &target_file)?;
                    *files_restored += 1;
                }

                // Log progress every 10%
                if total_files > 0 {
                    let progress = (*files_restored * 100) / total_files;
                    if progress / 10 > *last_progress / 10 {
                        *last_progress = progress;
                        info!(
                            "Restore file progress: {}% ({}/{} files)",
                            progress, files_restored, total_files
                        );
                    }
                }
            }
        }

        Ok(())
    }

    fn strip_delta_extension(path: &Path) -> PathBuf {
        let path_str = path.to_string_lossy();
        // e.g. "file.bin.delta" -> "file.bin"
        // e.g. "file.txt.delta" -> "file.txt"
        if let Some(stripped) = path_str.strip_suffix(".delta") {
            PathBuf::from(stripped)
        } else {
            path.to_path_buf()
        }
    }
}
