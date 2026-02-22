use super::*;
use log::{info, warn};
use std::cmp::Reverse;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration as StdDuration;

impl BackupManager {
    /// Validate all sources at program startup.
    /// Checks delta chain integrity and auto full-backup interval for each backup directory.
    /// Sets force_full flag per backup_dir so subsequent backups know to force full.
    pub fn validate_all_sources(&mut self) -> Result<()> {
        info!("Starting pre-flight validation of all backup sources...");
        let config = self.config.clone();

        // ── Global config validation ──

        // Validate global cron_schedule
        cron::Schedule::from_str(&config.cron_schedule).map_err(|e| {
            anyhow::anyhow!(
                "Invalid global cron_schedule '{}': {}",
                config.cron_schedule,
                e
            )
        })?;

        // Validate global numeric values
        if config.max_backups == 0 {
            return Err(anyhow::anyhow!("Global max_backups must be > 0"));
        }
        if config.max_log_file_size_mb == 0 {
            return Err(anyhow::anyhow!("Global max_log_file_size_mb must be > 0"));
        }

        // ── Per-source validation ──

        let mut seen_sources: HashSet<PathBuf> = HashSet::new();

        for source in &config.sources {
            // Duplicate source check
            if !seen_sources.insert(source.source_dir.clone()) {
                return Err(anyhow::anyhow!(
                    "Duplicate source directory: {:?}",
                    source.source_dir
                ));
            }

            // Source path must be absolute
            if !source.source_dir.is_absolute() {
                return Err(anyhow::anyhow!(
                    "Source path must be absolute: {:?}",
                    source.source_dir
                ));
            }

            if !source.enabled {
                continue;
            }

            // Source directory must exist
            if !source.source_dir.exists() {
                return Err(anyhow::anyhow!(
                    "Source directory does not exist: {:?}",
                    source.source_dir
                ));
            }

            // Source must be a directory
            if !source.source_dir.is_dir() {
                return Err(anyhow::anyhow!(
                    "Source path is not a directory: {:?}",
                    source.source_dir
                ));
            }

            // Validate source-level overrides
            if let Some(mb) = source.max_backups
                && mb == 0
            {
                return Err(anyhow::anyhow!(
                    "Source {:?}: max_backups must be > 0",
                    source.source_dir
                ));
            }
            if let Some(ref cs) = source.cron_schedule {
                cron::Schedule::from_str(cs).map_err(|e| {
                    anyhow::anyhow!(
                        "Source {:?}: invalid cron_schedule '{}': {}",
                        source.source_dir,
                        cs,
                        e
                    )
                })?;
            }

            // Backup dirs validation
            let backup_dirs = source.effective_backup_dirs();

            let mut seen_backup_dirs: HashSet<PathBuf> = HashSet::new();
            for backup_dir in &backup_dirs {
                // Backup path must be absolute
                if !backup_dir.is_absolute() {
                    return Err(anyhow::anyhow!(
                        "Backup path must be absolute: {:?} (source: {:?})",
                        backup_dir,
                        source.source_dir
                    ));
                }

                // Duplicate backup dir check
                if !seen_backup_dirs.insert(backup_dir.clone()) {
                    return Err(anyhow::anyhow!(
                        "Duplicate backup directory: {:?} (source: {:?})",
                        backup_dir,
                        source.source_dir
                    ));
                }

                // Source and backup must not be the same
                if *backup_dir == source.source_dir {
                    return Err(anyhow::anyhow!(
                        "Backup directory cannot be the same as source: {:?}",
                        backup_dir
                    ));
                }

                // Auto-create backup directory if it doesn't exist
                if !backup_dir.exists() {
                    fs::create_dir_all(backup_dir).map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to create backup directory {:?} (source: {:?}): {}",
                            backup_dir,
                            source.source_dir,
                            e
                        )
                    })?;
                    info!("Auto-created backup directory: {:?}", backup_dir);
                } else if !backup_dir.is_dir() {
                    return Err(anyhow::anyhow!(
                        "Backup path is not a directory: {:?} (source: {:?})",
                        backup_dir,
                        source.source_dir
                    ));
                }
            }

            // ── Delta chain / full interval validation ──
            let resolved = source.resolve(&config);
            for backup_dir in &backup_dirs {
                let mut needs_full = false;

                if let Err(e) = Self::validate_backup_metadata_history(backup_dir) {
                    warn!(
                        "[{:?}] Metadata history validation failed: {}. Will force full backup.",
                        backup_dir, e
                    );
                    needs_full = true;
                }

                if matches!(resolved.backup_mode, BackupMode::Delta) {
                    let inc_count = Self::count_inc_since_last_full(backup_dir);
                    if inc_count >= resolved.full_backup_interval {
                        info!(
                            "[{:?}] Full backup interval reached ({} inc backups), will force full",
                            backup_dir, inc_count
                        );
                        needs_full = true;
                    }

                    if !needs_full {
                        let chain_valid = Self::validate_delta_chain(backup_dir);
                        if !chain_valid {
                            warn!(
                                "[{:?}] Delta chain integrity check failed, will force full",
                                backup_dir
                            );
                            needs_full = true;
                        }
                    }
                }

                if needs_full {
                    self.force_full_dirs.insert(backup_dir.clone(), true);
                }
                info!(
                    "[{:?}] Validation complete (force_full: {})",
                    backup_dir, needs_full
                );
            }
        }
        info!("Pre-flight validation complete.");
        Ok(())
    }

    fn count_inc_since_last_full(backup_dir: &Path) -> usize {
        let mut entries: Vec<_> = match fs::read_dir(backup_dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    (name.starts_with("full_") || name.starts_with("inc_")) && e.path().is_dir()
                })
                .collect(),
            Err(_) => return 0,
        };

        // Sort by name (timestamp) descending
        entries.sort_by_key(|entry| Reverse(entry.file_name()));

        let mut count = 0;
        for entry in &entries {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("full_") {
                break;
            }
            if name.starts_with("inc_") {
                count += 1;
            }
        }
        count
    }

    fn validate_delta_chain(backup_dir: &Path) -> bool {
        let mut entries: Vec<_> = match fs::read_dir(backup_dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.starts_with("inc_") && e.path().is_dir()
                })
                .collect(),
            Err(_) => return false,
        };

        // Sort by name (timestamp) ascending
        entries.sort_by_key(|entry| entry.file_name());

        // Find the latest full backup as the chain start
        let full_exists = match fs::read_dir(backup_dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).any(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.starts_with("full_") && e.path().is_dir()
            }),
            Err(_) => false,
        };

        if !full_exists {
            return false;
        }

        // Validate each delta file in inc backups can be loaded
        for entry in &entries {
            let inc_path = entry.path();
            if Self::validate_delta_files_in_dir(&inc_path).is_err() {
                warn!("Corrupted delta found in {:?}", inc_path);
                return false;
            }
        }

        true
    }

    /// Calculate minimum backup interval based on source directory size.
    /// - up to 10MB: 1 second
    /// - up to 100MB: 1 minute
    /// - up to 1GB: 1 hour
    /// - above 1GB: 1 hour per GB
    pub fn calculate_min_interval_by_size(source_dir: &Path) -> StdDuration {
        let total_bytes = Self::calculate_dir_size(source_dir);
        let mb = total_bytes as f64 / (1024.0 * 1024.0);
        let gb = total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);

        if mb <= 10.0 {
            StdDuration::from_secs(1)
        } else if mb <= 100.0 {
            StdDuration::from_secs(60)
        } else if gb <= 1.0 {
            StdDuration::from_secs(3600)
        } else {
            let hours = gb.ceil() as u64;
            StdDuration::from_secs(hours * 3600)
        }
    }

    fn calculate_dir_size(dir: &Path) -> u64 {
        let mut total: u64 = 0;
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    total += Self::calculate_dir_size(&path);
                } else if let Ok(meta) = fs::metadata(&path) {
                    total += meta.len();
                }
            }
        }
        total
    }

    fn validate_delta_files_in_dir(dir: &Path) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                Self::validate_delta_files_in_dir(&path)?;
            } else {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name.ends_with(".delta") {
                    // Try to load and verify the delta file
                    delta::load_delta(&path)
                        .with_context(|| format!("Failed to load delta: {:?}", path))?;
                }
            }
        }
        Ok(())
    }
}
