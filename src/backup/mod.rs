use crate::config::{BackupConfig, BackupMode, ResolvedSourceConfig, SourceConfig};
use crate::delta;
use anyhow::{Context, Result};
use chrono::Utc;
use log::{error, info};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tokio::task;

mod file_ops;
mod metadata;
mod validation;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub enum BackupType {
    Full,
    Incremental,
}

#[derive(Debug)]
pub struct BackupResult {
    pub backup_dir: PathBuf,
    pub backup_type: BackupType,
    pub files_backed_up: usize,
    pub bytes_processed: u64,
    pub duration_ms: u64,
}

pub struct BackupManager {
    config: BackupConfig,
    force_full_dirs: HashMap<PathBuf, bool>,
}

impl BackupManager {
    pub fn new(config: BackupConfig) -> Self {
        Self {
            config,
            force_full_dirs: HashMap::new(),
        }
    }

    pub async fn backup_all_sources(&mut self) -> Result<Vec<BackupResult>> {
        let config = self.config.clone();
        let mut results = Vec::new();

        let tasks: Vec<_> = config
            .sources
            .iter()
            .filter(|s| s.enabled)
            .map(|source| {
                let source = source.clone();
                let resolved = source.resolve(&config);
                let backup_dirs = source.effective_backup_dirs();

                let force_full_dirs = self.force_full_dirs.clone();
                task::spawn(async move {
                    Self::backup_source(source, backup_dirs, resolved, force_full_dirs).await
                })
            })
            .collect();

        for task in tasks {
            match task.await {
                Ok(Ok(result)) => {
                    for r in result {
                        info!("Backup completed: {:?}", r.backup_dir);
                        results.push(r);
                    }
                }
                Ok(Err(e)) => error!("Backup failed: {}", e),
                Err(e) => error!("Task join error: {}", e),
            }
        }

        // Startup validation can mark a backup dir as force-full once.
        // After a successful full backup, clear that flag so subsequent
        // backups in the same process can proceed as incremental.
        for result in &results {
            if matches!(result.backup_type, BackupType::Full) {
                self.force_full_dirs.remove(&result.backup_dir);
            }
        }

        Ok(results)
    }

    async fn backup_source(
        source: SourceConfig,
        backup_dirs: Vec<PathBuf>,
        resolved: ResolvedSourceConfig,
        force_full_dirs: HashMap<PathBuf, bool>,
    ) -> Result<Vec<BackupResult>> {
        let mut results = Vec::new();

        for backup_dir in &backup_dirs {
            let force_full = force_full_dirs.get(backup_dir).copied().unwrap_or(false);
            let result = Self::perform_backup_to_dir(
                &source.source_dir,
                backup_dir,
                &resolved.exclude_patterns,
                resolved.max_backups,
                &resolved.backup_mode,
                force_full,
            )
            .await?;
            results.push(result);
        }

        Ok(results)
    }

    async fn perform_backup_to_dir(
        source_dir: &Path,
        backup_dir: &Path,
        exclude_patterns: &[String],
        max_backups: usize,
        backup_mode: &BackupMode,
        force_full: bool,
    ) -> Result<BackupResult> {
        let start_time = std::time::Instant::now();

        fs::create_dir_all(backup_dir)
            .with_context(|| format!("Failed to create backup directory: {:?}", backup_dir))?;

        let metadata_path = backup_dir.join("metadata.json");
        let mut metadata = Self::load_source_metadata(&metadata_path);
        Self::synchronize_metadata_history_with_disk(backup_dir, &mut metadata)?;

        let (mut backup_type, mut files_to_backup) =
            Self::scan_for_changes(source_dir, &metadata, exclude_patterns)?;

        // Apply force_full flag from startup validation
        if force_full && matches!(backup_type, BackupType::Incremental) {
            info!("Forcing full backup based on startup validation");
            backup_type = BackupType::Full;
            // Re-collect full file set. scan_for_changes() returned only changed
            // files for incremental mode, which could create an incomplete full.
            files_to_backup = Self::collect_all_files(source_dir, exclude_patterns)?;
        }

        // Skip incremental backup if no files changed
        if matches!(backup_type, BackupType::Incremental) && files_to_backup.is_empty() {
            info!(
                "[{:?}] No changes detected, skipping incremental backup",
                backup_dir
            );
            return Ok(BackupResult {
                backup_dir: backup_dir.to_path_buf(),
                backup_type,
                files_backed_up: 0,
                bytes_processed: 0,
                duration_ms: start_time.elapsed().as_millis() as u64,
            });
        }

        // Copy mode: always use file copy (no delta)
        let use_delta = matches!(backup_mode, BackupMode::Delta)
            && matches!(backup_type, BackupType::Incremental);

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S%3f");
        let backup_name = format!(
            "{}_{}",
            match backup_type {
                BackupType::Full => "full",
                BackupType::Incremental => "inc",
            },
            timestamp
        );
        let backup_path = backup_dir.join(&backup_name);

        fs::create_dir_all(&backup_path)?;

        let mut files_backed_up = 0;
        let mut bytes_processed = 0;
        let total_files = files_to_backup.len();
        let mut last_progress = 0;

        for file_path in &files_to_backup {
            let relative_path = file_path.strip_prefix(source_dir)?;
            let backup_file_path = backup_path.join(relative_path);

            if let Some(parent) = backup_file_path.parent() {
                fs::create_dir_all(parent)?;
            }

            match backup_type {
                BackupType::Full => {
                    fs::copy(file_path, &backup_file_path)?;
                    let file_size = fs::metadata(file_path)?.len();
                    bytes_processed += file_size;
                }
                BackupType::Incremental => {
                    if use_delta {
                        // Delta mode: find previous backup and create delta
                        let prev_backup = Self::find_latest_backup_file(backup_dir, relative_path);

                        if let Some(prev_path) = prev_backup {
                            let delta_data = delta::create_delta(&prev_path, file_path)?;
                            let delta_bytes = delta::delta_size(&delta_data);
                            let file_size = fs::metadata(file_path)?.len();
                            let delta_file_path = backup_file_path.with_extension(format!(
                                "{}.delta",
                                backup_file_path
                                    .extension()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                            ));
                            delta::save_delta(&delta_data, &delta_file_path)?;
                            bytes_processed += delta_bytes as u64;
                            info!(
                                "Delta backup: {:?} ({} bytes delta vs {} bytes full, {}/{} blocks changed)",
                                relative_path,
                                delta_bytes,
                                file_size,
                                delta_data.changed_blocks.len(),
                                delta_data.total_blocks
                            );
                        } else {
                            fs::copy(file_path, &backup_file_path)?;
                            let file_size = fs::metadata(file_path)?.len();
                            bytes_processed += file_size;
                        }
                    } else {
                        // Copy mode: always copy full file
                        fs::copy(file_path, &backup_file_path)?;
                        let file_size = fs::metadata(file_path)?.len();
                        bytes_processed += file_size;
                    }
                }
            }

            files_backed_up += 1;

            // Log progress every 10%
            if total_files > 0 {
                let progress = (files_backed_up * 100) / total_files;
                if progress / 10 > last_progress / 10 {
                    last_progress = progress;
                    info!(
                        "Backup progress: {}% ({}/{} files)",
                        progress, files_backed_up, total_files
                    );
                }
            }

            let hash = Self::calculate_file_hash(file_path)?;
            metadata
                .file_hashes
                .insert(relative_path.to_string_lossy().to_string(), hash);
        }

        let now = Utc::now();
        Self::append_backup_history_entry(
            &mut metadata,
            &backup_name,
            &backup_type,
            now,
            files_backed_up,
            bytes_processed,
        );

        Self::cleanup_old_backups(backup_dir, max_backups, backup_mode)?;
        Self::synchronize_metadata_history_with_disk(backup_dir, &mut metadata)?;

        let metadata_content = serde_json::to_string_pretty(&metadata)?;
        fs::write(&metadata_path, metadata_content)?;

        let duration = start_time.elapsed();

        Ok(BackupResult {
            backup_dir: backup_dir.to_path_buf(),
            backup_type,
            files_backed_up,
            bytes_processed,
            duration_ms: duration.as_millis() as u64,
        })
    }
}
