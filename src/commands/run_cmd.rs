use anyhow::{Context, Result};
use cron::Schedule;
use log::{error, info};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::backup::BackupManager;
use crate::config::{self, ConfigManager};
use crate::watcher::FileWatcher;

pub async fn handle_run() -> Result<()> {
    let config_manager = ConfigManager::load_or_create()
        .context("Failed to load configuration")?;
    
    let config = config_manager.get_config().clone();
    let (backup_tx, mut backup_rx) = mpsc::channel::<()>(100);

    let mut backup_manager = BackupManager::new(config.clone());
    backup_manager.validate_all_sources()?;

    // Cron-based scheduler: spawn one task per source with its own schedule
    let mut cron_tasks = Vec::new();
    if config.enable_periodic {
        for source in &config.sources {
            if !source.enabled {
                continue;
            }
            let resolved = source.resolve(&config);
            if !resolved.enable_periodic {
                info!("Source {:?}: periodic backup disabled, skipping cron task", source.source_dir);
                continue;
            }
            let cron_expr = resolved.cron_schedule.clone();
            let source_dir = source.source_dir.clone();
            let backup_tx = backup_tx.clone();
            let enable_min_interval = config.enable_min_interval_by_size;

            let schedule = Schedule::from_str(&cron_expr)
                .map_err(|e| anyhow::anyhow!("Invalid cron for {:?}: {}", source_dir, e))?;

            let task = tokio::spawn(async move {
                // Calculate min interval based on source size
                let min_interval = if enable_min_interval {
                    let interval = BackupManager::calculate_min_interval_by_size(&source_dir);
                    info!(
                        "Source {:?}: min interval by size = {}s",
                        source_dir,
                        interval.as_secs()
                    );
                    interval
                } else {
                    Duration::from_secs(0)
                };

                let mut last_backup_time: Option<std::time::Instant> = None;

                loop {
                    let now = chrono::Utc::now();
                    if let Some(next) = schedule.upcoming(chrono::Utc).next() {
                        let wait_duration = (next - now).to_std().unwrap_or(Duration::from_secs(60));
                        sleep(wait_duration).await;

                        // Enforce minimum interval
                        if let Some(last) = last_backup_time {
                            let elapsed = last.elapsed();
                            if elapsed < min_interval {
                                let remaining = min_interval - elapsed;
                                info!(
                                    "Source {:?}: min interval not reached, waiting {}s more",
                                    source_dir,
                                    remaining.as_secs()
                                );
                                sleep(remaining).await;
                            }
                        }

                        info!("Cron triggered backup for source: {:?}", source_dir);
                        if let Err(e) = backup_tx.send(()).await {
                            error!("Failed to send cron backup trigger: {}", e);
                            break;
                        }
                        last_backup_time = Some(std::time::Instant::now());
                    } else {
                        sleep(Duration::from_secs(60)).await;
                    }
                }
            });
            cron_tasks.push(task);
        }
    }

    let watcher_task = if config.enable_event_driven {
        let watch_paths: Vec<PathBuf> = config.sources
            .iter()
            .filter(|s| {
                if !s.enabled { return false; }
                let resolved = s.resolve(&config);
                resolved.enable_event_driven && matches!(resolved.backup_mode, config::BackupMode::Delta)
            })
            .map(|s| s.source_dir.clone())
            .collect();

        Some(tokio::task::spawn_blocking(move || {
            match FileWatcher::new(watch_paths, backup_tx.clone(), Duration::from_millis(300)) {
                Ok(_watcher) => {
                    info!("File watcher started");
                    // Keep _watcher alive — dropping it stops file watching
                    loop {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
                Err(e) => {
                    error!("Failed to start file watcher: {}", e);
                }
            }
        }))
    } else {
        None
    };

    if matches!(config.backup_mode, config::BackupMode::Copy) {
        info!("Running in copy mode - event-driven backup is disabled, periodic backup only");
    }

    info!(
        "Ardiex backup service started (mode: {:?}, cron: {}, min_interval_by_size: {})",
        config.backup_mode, config.cron_schedule, config.enable_min_interval_by_size
    );

    loop {
        tokio::select! {
            _ = backup_rx.recv() => {
                info!("Backup triggered");
                match backup_manager.backup_all_sources().await {
                    Ok(results) => {
                        for result in results {
                            info!(
                                "Backup completed: {} files to {:?} ({:.2} MB)",
                                result.files_backed_up,
                                result.backup_dir,
                                result.bytes_processed as f64 / 1024.0 / 1024.0
                            );
                        }
                    }
                    Err(e) => {
                        error!("Backup failed: {}", e);
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Received Ctrl+C, shutting down");
                break;
            }
        }
    }

    for task in &cron_tasks {
        task.abort();
    }
    if let Some(task) = watcher_task {
        task.abort();
    }

    info!("Ardiex backup service stopped");
    Ok(())
}
