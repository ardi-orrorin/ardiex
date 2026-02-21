use anyhow::{Context, Result};
use cron::Schedule;
use log::{error, info, warn};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, MissedTickBehavior};

use crate::backup::BackupManager;
use crate::config::{self, ConfigManager};
use crate::watcher::FileWatcher;

struct RuntimeHandles {
    cron_tasks: Vec<JoinHandle<()>>,
    watcher_task: Option<JoinHandle<()>>,
}

impl RuntimeHandles {
    fn abort_all(&mut self) {
        for task in &self.cron_tasks {
            task.abort();
        }
        if let Some(task) = &self.watcher_task {
            task.abort();
        }
        self.cron_tasks.clear();
        self.watcher_task = None;
    }
}

fn config_fingerprint(config: &config::BackupConfig) -> Result<String> {
    serde_json::to_string(config).context("Failed to serialize config fingerprint")
}

fn config_snapshot_pretty_json(config: &config::BackupConfig, phase: &str) -> String {
    let snapshot = serde_json::json!({
        "phase": phase,
        "config": config
    });

    serde_json::to_string_pretty(&snapshot)
        .unwrap_or_else(|_| "{\"error\":\"failed to serialize config snapshot\"}".to_string())
}

fn log_config_snapshot(config: &config::BackupConfig, phase: &str) {
    let pretty = config_snapshot_pretty_json(config, phase);
    info!("[CONFIG] {}", pretty);
}

fn print_config_snapshot(config: &config::BackupConfig, phase: &str) {
    let pretty = config_snapshot_pretty_json(config, phase);
    println!("[CONFIG] {}", pretty);
}

fn spawn_runtime_handles(
    config: &config::BackupConfig,
    backup_tx: mpsc::Sender<()>,
) -> Result<RuntimeHandles> {
    // Cron-based scheduler: spawn one task per source with its own schedule
    let mut cron_tasks = Vec::new();
    if config.enable_periodic {
        for source in &config.sources {
            if !source.enabled {
                continue;
            }
            let resolved = source.resolve(config);
            if !resolved.enable_periodic {
                info!(
                    "Source {:?}: periodic backup disabled, skipping cron task",
                    source.source_dir
                );
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
                        let wait_duration =
                            (next - now).to_std().unwrap_or(Duration::from_secs(60));
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
        let watch_paths: Vec<PathBuf> = config
            .sources
            .iter()
            .filter(|s| {
                if !s.enabled {
                    return false;
                }
                let resolved = s.resolve(config);
                resolved.enable_event_driven && matches!(resolved.backup_mode, config::BackupMode::Delta)
            })
            .map(|s| s.source_dir.clone())
            .collect();

        if watch_paths.is_empty() {
            info!("No eligible sources for event-driven watcher");
            None
        } else {
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
        }
    } else {
        None
    };

    if matches!(config.backup_mode, config::BackupMode::Copy) {
        info!("Running in copy mode - event-driven backup is disabled, periodic backup only");
    }

    Ok(RuntimeHandles {
        cron_tasks,
        watcher_task,
    })
}

pub async fn handle_run() -> Result<()> {
    let config_manager = ConfigManager::load_or_create().context("Failed to load configuration")?;
    let mut active_config = config_manager.get_config().clone();
    let mut active_fingerprint = config_fingerprint(&active_config)?;
    let mut failed_reload_fingerprint: Option<String> = None;

    let (backup_tx, mut backup_rx) = mpsc::channel::<()>(100);

    let mut backup_manager = BackupManager::new(active_config.clone());
    backup_manager.validate_all_sources()?;
    log_config_snapshot(&active_config, "startup");
    print_config_snapshot(&active_config, "startup");
    let mut runtime_handles = spawn_runtime_handles(&active_config, backup_tx.clone())?;

    info!(
        "Ardiex backup service started (mode: {:?}, cron: {}, min_interval_by_size: {})",
        active_config.backup_mode, active_config.cron_schedule, active_config.enable_min_interval_by_size
    );

    let mut reload_tick = tokio::time::interval(Duration::from_secs(2));
    reload_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            maybe_trigger = backup_rx.recv() => {
                if maybe_trigger.is_none() {
                    warn!("Backup trigger channel closed, shutting down");
                    break;
                }

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
            _ = reload_tick.tick() => {
                let latest = match ConfigManager::load_or_create() {
                    Ok(manager) => manager.get_config().clone(),
                    Err(e) => {
                        error!("Failed to reload configuration file: {}", e);
                        continue;
                    }
                };

                let latest_fingerprint = match config_fingerprint(&latest) {
                    Ok(fp) => fp,
                    Err(e) => {
                        error!("Failed to fingerprint latest config: {}", e);
                        continue;
                    }
                };

                if latest_fingerprint == active_fingerprint {
                    continue;
                }

                if failed_reload_fingerprint.as_deref() == Some(latest_fingerprint.as_str()) {
                    continue;
                }

                info!(
                    "[HOT-RELOAD] Detected settings.json change, attempting to apply new configuration"
                );

                let mut new_backup_manager = BackupManager::new(latest.clone());
                if let Err(e) = new_backup_manager.validate_all_sources() {
                    error!("[HOT-RELOAD] Rejected invalid configuration: {}", e);
                    failed_reload_fingerprint = Some(latest_fingerprint);
                    continue;
                }

                let new_runtime_handles = match spawn_runtime_handles(&latest, backup_tx.clone()) {
                    Ok(handles) => handles,
                    Err(e) => {
                        error!(
                            "[HOT-RELOAD] Failed while creating runtime tasks for new configuration: {}",
                            e
                        );
                        failed_reload_fingerprint = Some(latest_fingerprint);
                        continue;
                    }
                };

                runtime_handles.abort_all();
                runtime_handles = new_runtime_handles;
                backup_manager = new_backup_manager;
                active_config = latest;
                active_fingerprint = latest_fingerprint;
                failed_reload_fingerprint = None;
                log_config_snapshot(&active_config, "hot-reload");
                print_config_snapshot(&active_config, "hot-reload");

                info!(
                    "[HOT-RELOAD] Applied successfully (mode: {:?}, cron: {}, min_interval_by_size: {})",
                    active_config.backup_mode, active_config.cron_schedule, active_config.enable_min_interval_by_size
                );
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Received Ctrl+C, shutting down");
                break;
            }
        }
    }

    runtime_handles.abort_all();
    info!("Ardiex backup service stopped");
    Ok(())
}
