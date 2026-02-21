use anyhow::{Context, Result};
use chrono::Utc;
use log::{error, info, warn};
use serde_json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use tokio::task;
use crate::config::{BackupConfig, BackupMode, ResolvedSourceConfig, SourceConfig, SourceMetadata};
use crate::delta;
use std::time::Duration as StdDuration;

#[derive(Debug)]
pub enum BackupType {
    Full,
    Incremental,
}

#[derive(Debug)]
pub struct BackupResult {
    pub source_dir: PathBuf,
    pub backup_dir: PathBuf,
    pub backup_type: BackupType,
    pub files_backed_up: usize,
    pub total_files: usize,
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

    /// Validate all sources at program startup.
    /// Checks delta chain integrity and full_backup_interval for each backup directory.
    /// Sets force_full flag per backup_dir so subsequent backups know to force full.
    pub fn validate_all_sources(&mut self) -> Result<()> {
        use std::collections::HashSet;
        use std::str::FromStr;

        info!("Starting pre-flight validation of all backup sources...");
        let config = self.config.clone();

        // ── Global config validation ──

        // Validate global cron_schedule
        cron::Schedule::from_str(&config.cron_schedule)
            .map_err(|e| anyhow::anyhow!(
                "Invalid global cron_schedule '{}': {}",
                config.cron_schedule, e
            ))?;

        // Validate global numeric values
        if config.max_backups == 0 {
            return Err(anyhow::anyhow!("Global max_backups must be > 0"));
        }
        if config.full_backup_interval == 0 {
            return Err(anyhow::anyhow!("Global full_backup_interval must be > 0"));
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
            if let Some(mb) = source.max_backups {
                if mb == 0 {
                    return Err(anyhow::anyhow!(
                        "Source {:?}: max_backups must be > 0",
                        source.source_dir
                    ));
                }
            }
            if let Some(fbi) = source.full_backup_interval {
                if fbi == 0 {
                    return Err(anyhow::anyhow!(
                        "Source {:?}: full_backup_interval must be > 0",
                        source.source_dir
                    ));
                }
            }
            if let Some(ref cs) = source.cron_schedule {
                cron::Schedule::from_str(cs)
                    .map_err(|e| anyhow::anyhow!(
                        "Source {:?}: invalid cron_schedule '{}': {}",
                        source.source_dir, cs, e
                    ))?;
            }

            // Backup dirs validation
            let backup_dirs = if source.backup_dirs.is_empty() {
                vec![source.source_dir.join(".backup")]
            } else {
                source.backup_dirs.clone()
            };

            let mut seen_backup_dirs: HashSet<PathBuf> = HashSet::new();
            for backup_dir in &backup_dirs {
                // Backup path must be absolute
                if !backup_dir.is_absolute() {
                    return Err(anyhow::anyhow!(
                        "Backup path must be absolute: {:?} (source: {:?})",
                        backup_dir, source.source_dir
                    ));
                }

                // Duplicate backup dir check
                if !seen_backup_dirs.insert(backup_dir.clone()) {
                    return Err(anyhow::anyhow!(
                        "Duplicate backup directory: {:?} (source: {:?})",
                        backup_dir, source.source_dir
                    ));
                }

                // Source and backup must not be the same
                if *backup_dir == source.source_dir {
                    return Err(anyhow::anyhow!(
                        "Backup directory cannot be the same as source: {:?}",
                        backup_dir
                    ));
                }

                // Backup directory must exist
                if !backup_dir.exists() {
                    return Err(anyhow::anyhow!(
                        "Backup directory does not exist: {:?} (source: {:?})",
                        backup_dir, source.source_dir
                    ));
                }

                // Backup must be a directory
                if !backup_dir.is_dir() {
                    return Err(anyhow::anyhow!(
                        "Backup path is not a directory: {:?} (source: {:?})",
                        backup_dir, source.source_dir
                    ));
                }
            }

            // ── Delta chain / full interval validation ──
            let resolved = source.resolve(&config);
            for backup_dir in &backup_dirs {
                let mut needs_full = false;

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
                info!("[{:?}] Validation complete (force_full: {})", backup_dir, needs_full);
            }
        }
        info!("Pre-flight validation complete.");
        Ok(())
    }

    pub async fn backup_all_sources(&mut self) -> Result<Vec<BackupResult>> {
        let config = self.config.clone();
        let mut results = Vec::new();

        let tasks: Vec<_> = config.sources
            .iter()
            .filter(|s| s.enabled)
            .map(|source| {
                let source = source.clone();
                let resolved = source.resolve(&config);
                let backup_dirs = if source.backup_dirs.is_empty() {
                    vec![source.source_dir.join(".backup")]
                } else {
                    source.backup_dirs.clone()
                };
                
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
            ).await?;
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
        let mut metadata = if metadata_path.exists() {
            let content = fs::read_to_string(&metadata_path)?;
            serde_json::from_str(&content).unwrap_or_else(|_| SourceMetadata {
                last_full_backup: None,
                last_backup: None,
                file_hashes: HashMap::new(),
            })
        } else {
            SourceMetadata {
                last_full_backup: None,
                last_backup: None,
                file_hashes: HashMap::new(),
            }
        };

        let (mut backup_type, files_to_backup) = Self::scan_for_changes(
            source_dir,
            &metadata,
            exclude_patterns,
        )?;

        // Apply force_full flag from startup validation
        if force_full && matches!(backup_type, BackupType::Incremental) {
            info!("Forcing full backup based on startup validation");
            backup_type = BackupType::Full;
        }

        // Skip incremental backup if no files changed
        if matches!(backup_type, BackupType::Incremental) && files_to_backup.is_empty() {
            info!("[{:?}] No changes detected, skipping incremental backup", backup_dir);
            return Ok(BackupResult {
                source_dir: source_dir.to_path_buf(),
                backup_dir: backup_dir.to_path_buf(),
                backup_type,
                files_backed_up: 0,
                total_files: 0,
                bytes_processed: 0,
                duration_ms: start_time.elapsed().as_millis() as u64,
            });
        }

        // Copy mode: always use file copy (no delta)
        let use_delta = matches!(backup_mode, BackupMode::Delta) && matches!(backup_type, BackupType::Incremental);

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S%3f");
        let backup_name = format!("{}_{}", 
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
                        let prev_backup = Self::find_latest_backup_file(
                            backup_dir, relative_path,
                        );

                        if let Some(prev_path) = prev_backup {
                            let delta_data = delta::create_delta(&prev_path, file_path)?;
                            let delta_bytes = delta::delta_size(&delta_data);
                            let file_size = fs::metadata(file_path)?.len();

                            if (delta_bytes as u64) < file_size / 2 {
                                let delta_file_path = backup_file_path.with_extension(
                                    format!("{}.delta", backup_file_path.extension()
                                        .unwrap_or_default().to_string_lossy())
                                );
                                delta::save_delta(&delta_data, &delta_file_path)?;
                                bytes_processed += delta_bytes as u64;
                                info!("Delta backup: {:?} ({} bytes delta vs {} bytes full)",
                                    relative_path, delta_bytes, file_size);
                            } else {
                                fs::copy(file_path, &backup_file_path)?;
                                bytes_processed += file_size;
                            }
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
                    info!("Backup progress: {}% ({}/{} files)", progress, files_backed_up, total_files);
                }
            }

            let hash = Self::calculate_file_hash(file_path)?;
            metadata.file_hashes.insert(
                relative_path.to_string_lossy().to_string(),
                hash,
            );
        }

        let now = Utc::now();
        match backup_type {
            BackupType::Full => metadata.last_full_backup = Some(now),
            BackupType::Incremental => {}
        }
        metadata.last_backup = Some(now);

        let metadata_content = serde_json::to_string_pretty(&metadata)?;
        fs::write(&metadata_path, metadata_content)?;

        Self::cleanup_old_backups(backup_dir, max_backups, backup_mode)?;

        let duration = start_time.elapsed();

        Ok(BackupResult {
            source_dir: source_dir.to_path_buf(),
            backup_dir: backup_dir.to_path_buf(),
            backup_type,
            files_backed_up,
            total_files: files_to_backup.len(),
            bytes_processed,
            duration_ms: duration.as_millis() as u64,
        })
    }

    fn scan_for_changes(
        source_dir: &Path,
        metadata: &SourceMetadata,
        exclude_patterns: &[String],
    ) -> Result<(BackupType, Vec<PathBuf>)> {
        let mut files_to_backup = Vec::new();
        let mut current_hashes = HashMap::new();

        if !source_dir.exists() {
            return Err(anyhow::anyhow!("Source directory does not exist: {:?}", source_dir));
        }

        Self::collect_files(source_dir, source_dir, &mut files_to_backup, &mut current_hashes, exclude_patterns)?;

        let backup_type = if metadata.last_full_backup.is_none() {
            BackupType::Full
        } else {
            BackupType::Incremental
        };

        if matches!(backup_type, BackupType::Full) {
            return Ok((backup_type, files_to_backup));
        }

        let changed_files: Vec<PathBuf> = files_to_backup
            .into_iter()
            .filter(|path| {
                let relative_path = path.strip_prefix(source_dir).unwrap();
                let path_str = relative_path.to_string_lossy();
                
                let current_hash = current_hashes.get(&path_str as &str);
                let stored_hash = metadata.file_hashes.get(&path_str as &str);
                
                current_hash != stored_hash
            })
            .collect();

        Ok((backup_type, changed_files))
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
                let relative_path = path.strip_prefix(base_dir).unwrap();
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
                if parts.len() == 2 {
                    if path_str.starts_with(parts[0]) && path_str.ends_with(parts[1]) {
                        return true;
                    }
                }
            } else if path_str.contains(pattern) {
                return true;
            }
        }
        
        false
    }

    fn calculate_file_hash(path: &Path) -> Result<String> {
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

    fn find_latest_backup_file(backup_dir: &Path, relative_path: &Path) -> Option<PathBuf> {
        let mut backup_dirs: Vec<_> = fs::read_dir(backup_dir).ok()?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                (name.starts_with("full_") || name.starts_with("inc_"))
                    && entry.path().is_dir()
            })
            .collect();

        // Sort by modification time descending (newest first)
        backup_dirs.sort_by(|a, b| {
            let a_time = a.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            let b_time = b.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            b_time.cmp(&a_time)
        });

        // Find the most recent backup that contains this file
        for dir in backup_dirs {
            let candidate = dir.path().join(relative_path);
            if candidate.exists() && candidate.is_file() {
                return Some(candidate);
            }
        }

        None
    }

    fn cleanup_old_backups(backup_dir: &Path, max_backups: usize, backup_mode: &BackupMode) -> Result<()> {
        let mut backups: Vec<_> = fs::read_dir(backup_dir)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                name.starts_with("full_") || name.starts_with("inc_")
            })
            .collect();

        backups.sort_by(|a, b| {
            let a_time = a.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            let b_time = b.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            a_time.cmp(&b_time)
        });

        if backups.len() <= max_backups {
            return Ok(());
        }

        // In delta mode, protect the restore chain: latest full + all inc after it
        let keep_count = if matches!(backup_mode, BackupMode::Delta) {
            let latest_full_idx = backups.iter().rposition(|entry| {
                entry.file_name().to_string_lossy().starts_with("full_")
            });

            let protect_count = match latest_full_idx {
                Some(idx) => backups.len() - idx,
                None => backups.len(), // no full backup exists, keep everything
            };

            max_backups.max(protect_count)
        } else {
            // Copy mode: each backup is independent, no chain protection needed
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

    fn count_inc_since_last_full(backup_dir: &Path) -> usize {
        let mut entries: Vec<_> = match fs::read_dir(backup_dir) {
            Ok(rd) => rd.filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    (name.starts_with("full_") || name.starts_with("inc_")) && e.path().is_dir()
                })
                .collect(),
            Err(_) => return 0,
        };

        // Sort by name (timestamp) descending
        entries.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

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
            Ok(rd) => rd.filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.starts_with("inc_") && e.path().is_dir()
                })
                .collect(),
            Err(_) => return false,
        };

        // Sort by name (timestamp) ascending
        entries.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

        // Find the latest full backup as the chain start
        let full_exists = match fs::read_dir(backup_dir) {
            Ok(rd) => rd.filter_map(|e| e.ok())
                .any(|e| {
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
            if let Err(_) = Self::validate_delta_files_in_dir(&inc_path) {
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
