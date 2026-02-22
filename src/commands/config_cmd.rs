use anyhow::{Context, Result};
use cron::Schedule;
use log::warn;
use std::str::FromStr;

use crate::cli::ConfigAction;
use crate::config::{self, ConfigManager};

pub fn ensure_absolute(path: &std::path::Path, label: &str) -> Result<()> {
    if !path.is_absolute() {
        return Err(anyhow::anyhow!(
            "{} must be an absolute path: {:?}",
            label,
            path
        ));
    }
    Ok(())
}

pub async fn handle_config(action: ConfigAction) -> Result<()> {
    let mut config_manager =
        ConfigManager::load_or_create().context("Failed to load configuration")?;

    match action {
        ConfigAction::Init => {
            println!(
                "Configuration initialized at: {:?}",
                config_manager.config_path
            );
        }
        ConfigAction::List => {
            let config = config_manager.get_config();
            let global_auto_full_interval = config::auto_full_backup_interval(config.max_backups);
            println!("Configuration:");
            println!("  Enable periodic: {}", config.enable_periodic);
            println!("  Enable event-driven: {}", config.enable_event_driven);
            println!("  Max backups: {}", config.max_backups);
            println!("  Backup mode: {:?}", config.backup_mode);
            println!(
                "  Full backup interval (auto): {} (derived from max_backups)",
                global_auto_full_interval
            );
            println!("  Cron schedule: {}", config.cron_schedule);
            println!(
                "  Min interval by size: {}",
                config.enable_min_interval_by_size
            );
            println!("  Max log file size (MB): {}", config.max_log_file_size_mb);
            println!("  Exclude patterns: {:?}", config.exclude_patterns);
            println!("\nSources:");
            for source in &config.sources {
                let effective_max_backups = source.max_backups.unwrap_or(config.max_backups);
                println!("  Source: {:?}", source.source_dir);
                println!("    Enabled: {}", source.enabled);
                println!("    Backup dirs: {:?}", source.backup_dirs);
                println!(
                    "    Full backup interval (auto/effective): {}",
                    config::auto_full_backup_interval(effective_max_backups)
                );
                if let Some(ref ep) = source.exclude_patterns {
                    println!("    Exclude patterns (local): {:?}", ep);
                }
                if let Some(mb) = source.max_backups {
                    println!("    Max backups (local): {}", mb);
                }
                if let Some(ref bm) = source.backup_mode {
                    println!("    Backup mode (local): {:?}", bm);
                }
                if let Some(ref cs) = source.cron_schedule {
                    println!("    Cron schedule (local): {}", cs);
                }
                if let Some(eed) = source.enable_event_driven {
                    println!("    Enable event-driven (local): {}", eed);
                }
                if let Some(ep) = source.enable_periodic {
                    println!("    Enable periodic (local): {}", ep);
                }
            }
        }
        ConfigAction::AddSource { path, backup } => {
            ensure_absolute(&path, "Source path")?;
            for b in &backup {
                ensure_absolute(b, "Backup path")?;
            }
            if !path.exists() {
                return Err(anyhow::anyhow!(
                    "Source directory does not exist: {:?}",
                    path
                ));
            }

            // Show file list for confirmation
            let mut file_count = 0;
            let mut total_size: u64 = 0;
            println!("Files in {:?}:", path);
            println!("{:-<60}", "");
            for entry in walkdir::WalkDir::new(&path)
                .max_depth(3)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                let rel = entry.path().strip_prefix(&path).unwrap_or(entry.path());
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                total_size += size;
                file_count += 1;
                if file_count <= 20 {
                    println!("  {} ({:.1} KB)", rel.display(), size as f64 / 1024.0);
                }
            }
            if file_count > 20 {
                println!("  ... and {} more files", file_count - 20);
            }
            println!("{:-<60}", "");
            println!(
                "Total: {} files ({:.2} MB)",
                file_count,
                total_size as f64 / 1024.0 / 1024.0
            );
            println!();

            print!("Add this source? [y/N] ");
            std::io::Write::flush(&mut std::io::stdout())?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if input.trim().to_lowercase() != "y" {
                println!("Cancelled.");
                return Ok(());
            }

            config_manager.add_source(path, backup)?;
            println!("Source added successfully");
        }
        ConfigAction::RemoveSource { path } => {
            ensure_absolute(&path, "Source path")?;
            config_manager.remove_source(&path)?;
            println!("Source removed successfully");
        }
        ConfigAction::AddBackup { source, backup } => {
            ensure_absolute(&source, "Source path")?;
            ensure_absolute(&backup, "Backup path")?;
            config_manager.add_backup_dir(&source, backup)?;
            println!("Backup directory added successfully");
        }
        ConfigAction::RemoveBackup { source, backup } => {
            ensure_absolute(&source, "Source path")?;
            ensure_absolute(&backup, "Backup path")?;
            config_manager.remove_backup_dir(&source, &backup)?;
            println!("Backup directory removed successfully");
        }
        ConfigAction::Set { key, value } => {
            let config = config_manager.get_config_mut();
            match key.as_str() {
                "enable_periodic" => {
                    config.enable_periodic =
                        value.parse().context("Invalid value for enable_periodic")?;
                }
                "enable_event_driven" => {
                    config.enable_event_driven = value
                        .parse()
                        .context("Invalid value for enable_event_driven")?;
                }
                "max_backups" => {
                    let v: usize = value.parse().context("Invalid value for max_backups")?;
                    if v == 0 {
                        return Err(anyhow::anyhow!("max_backups must be > 0"));
                    }
                    config.max_backups = v;
                }
                "backup_mode" => {
                    let new_mode = match value.as_str() {
                        "delta" => config::BackupMode::Delta,
                        "copy" => config::BackupMode::Copy,
                        _ => {
                            return Err(anyhow::anyhow!(
                                "Invalid backup_mode: '{}'. Use 'delta' or 'copy'",
                                value
                            ));
                        }
                    };
                    config.backup_mode = new_mode;
                }
                "cron_schedule" => {
                    Schedule::from_str(&value)
                        .map_err(|e| anyhow::anyhow!("Invalid cron expression: '{}'. Error: {}\nFormat: sec min hour day-of-month month day-of-week year", value, e))?;
                    config.cron_schedule = value;
                }
                "enable_min_interval_by_size" => {
                    config.enable_min_interval_by_size = value
                        .parse()
                        .context("Invalid value for enable_min_interval_by_size")?;
                }
                "max_log_file_size_mb" => {
                    let v: u64 = value
                        .parse()
                        .context("Invalid value for max_log_file_size_mb")?;
                    if v == 0 {
                        return Err(anyhow::anyhow!("max_log_file_size_mb must be > 0"));
                    }
                    config.max_log_file_size_mb = v;
                }
                _ => {
                    warn!("Unknown configuration key: {}", key);
                    return Ok(());
                }
            }
            config_manager.save()?;
            println!("Configuration updated successfully");
        }
        ConfigAction::SetSource { source, key, value } => {
            ensure_absolute(&source, "Source path")?;
            let config = config_manager.get_config_mut();
            let src = config.sources.iter_mut().find(|s| s.source_dir == source);

            let src = match src {
                Some(s) => s,
                None => return Err(anyhow::anyhow!("Source not found: {:?}", source)),
            };

            let is_reset = value == "reset";

            match key.as_str() {
                "exclude_patterns" => {
                    src.exclude_patterns = if is_reset {
                        None
                    } else {
                        Some(value.split(',').map(|s| s.trim().to_string()).collect())
                    };
                }
                "max_backups" => {
                    src.max_backups = if is_reset {
                        None
                    } else {
                        let parsed: usize =
                            value.parse().context("Invalid value for max_backups")?;
                        if parsed == 0 {
                            return Err(anyhow::anyhow!("max_backups must be > 0"));
                        }
                        Some(parsed)
                    };
                }
                "backup_mode" => {
                    src.backup_mode = if is_reset {
                        None
                    } else {
                        Some(match value.as_str() {
                            "delta" => config::BackupMode::Delta,
                            "copy" => config::BackupMode::Copy,
                            _ => {
                                return Err(anyhow::anyhow!(
                                    "Invalid backup_mode: '{}'. Use 'delta' or 'copy'",
                                    value
                                ));
                            }
                        })
                    };
                }
                "cron_schedule" => {
                    src.cron_schedule = if is_reset {
                        None
                    } else {
                        Schedule::from_str(&value)
                            .map_err(|e| anyhow::anyhow!("Invalid cron expression: '{}'. Error: {}\nFormat: sec min hour day-of-month month day-of-week year", value, e))?;
                        Some(value)
                    };
                }
                "enable_event_driven" => {
                    src.enable_event_driven = if is_reset {
                        None
                    } else {
                        Some(
                            value
                                .parse()
                                .context("Invalid value for enable_event_driven (true/false)")?,
                        )
                    };
                }
                "enable_periodic" => {
                    src.enable_periodic = if is_reset {
                        None
                    } else {
                        Some(
                            value
                                .parse()
                                .context("Invalid value for enable_periodic (true/false)")?,
                        )
                    };
                }
                _ => {
                    warn!("Unknown source configuration key: {}", key);
                    return Ok(());
                }
            }
            config_manager.save()?;
            if is_reset {
                println!("Source config '{}' reset to global default", key);
            } else {
                println!("Source config updated successfully");
            }
        }
    }

    Ok(())
}
