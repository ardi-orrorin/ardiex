mod backup;
mod config;
mod delta;
mod logger;
mod restore;
mod watcher;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use log::{error, info, warn};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, sleep};

use backup::BackupManager;
use config::ConfigManager;
use restore::RestoreManager;
use watcher::FileWatcher;

#[derive(Parser)]
#[command(name = "ardiex")]
#[command(about = "Incremental backup system with periodic and event-driven triggers")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Configuration management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Perform a manual backup
    Backup,
    /// Restore from backup
    Restore {
        /// Backup directory to restore from
        backup_dir: PathBuf,
        /// Target directory to restore to
        target_dir: PathBuf,
        /// Restore point timestamp (e.g. 20240221_100000). If omitted, restores to latest.
        #[arg(short, long)]
        point: Option<String>,
        /// List available backups instead of restoring
        #[arg(short, long)]
        list: bool,
    },
    /// Start the backup service (periodic + event-driven)
    Run,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Initialize default configuration
    Init,
    /// Show current configuration
    List,
    /// Add a new source directory
    AddSource {
        /// Source directory path
        path: PathBuf,
        /// Backup directory paths (optional)
        #[arg(short, long)]
        backup: Vec<PathBuf>,
    },
    /// Remove a source directory
    RemoveSource {
        /// Source directory path
        path: PathBuf,
    },
    /// Add a backup directory to a source
    AddBackup {
        /// Source directory path
        source: PathBuf,
        /// Backup directory path
        backup: PathBuf,
    },
    /// Remove a backup directory from a source
    RemoveBackup {
        /// Source directory path
        source: PathBuf,
        /// Backup directory path
        backup: PathBuf,
    },
    /// Set a global configuration value
    Set {
        /// Configuration key
        key: String,
        /// Configuration value
        value: String,
    },
    /// Set a source-specific configuration value (overrides global)
    SetSource {
        /// Source directory path
        source: PathBuf,
        /// Configuration key (exclude_patterns, max_backups, backup_mode, full_backup_interval)
        key: String,
        /// Configuration value (use "reset" to clear and fall back to global)
        value: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    let log_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .map(|p| p.join("logs"));
    
    if let Some(ref log_dir) = log_dir {
        if let Err(e) = logger::init_file_logging(log_dir) {
            eprintln!("Failed to initialize file logging: {}", e);
            // Fallback to console logging
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
        }
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Config { action } => handle_config(action).await?,
        Commands::Backup => handle_backup().await?,
        Commands::Restore { backup_dir, target_dir, point, list } => {
            handle_restore(backup_dir, target_dir, point, list).await?
        }
        Commands::Run => handle_run().await?,
    }

    Ok(())
}

async fn handle_config(action: ConfigAction) -> Result<()> {
    let mut config_manager = ConfigManager::load_or_create()
        .context("Failed to load configuration")?;

    match action {
        ConfigAction::Init => {
            println!("Configuration initialized at: {:?}", 
                config_manager.config_path);
        }
        ConfigAction::List => {
            let config = config_manager.get_config();
            println!("Configuration:");
            println!("  Periodic interval: {} minutes", config.periodic_interval_minutes);
            println!("  Enable periodic: {}", config.enable_periodic);
            println!("  Enable event-driven: {}", config.enable_event_driven);
            println!("  Max backups: {}", config.max_backups);
            println!("  Backup mode: {:?}", config.backup_mode);
            println!("  Full backup interval: {} (inc backups before forced full)", config.full_backup_interval);
            println!("  Exclude patterns: {:?}", config.exclude_patterns);
            println!("\nSources:");
            for source in &config.sources {
                println!("  Source: {:?}", source.source_dir);
                println!("    Enabled: {}", source.enabled);
                println!("    Backup dirs: {:?}", source.backup_dirs);
                if let Some(ref ep) = source.exclude_patterns {
                    println!("    Exclude patterns (local): {:?}", ep);
                }
                if let Some(mb) = source.max_backups {
                    println!("    Max backups (local): {}", mb);
                }
                if let Some(ref bm) = source.backup_mode {
                    println!("    Backup mode (local): {:?}", bm);
                }
                if let Some(fbi) = source.full_backup_interval {
                    println!("    Full backup interval (local): {}", fbi);
                }
            }
        }
        ConfigAction::AddSource { path, backup } => {
            if !path.exists() {
                return Err(anyhow::anyhow!("Source directory does not exist: {:?}", path));
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
            println!("Total: {} files ({:.2} MB)", file_count, total_size as f64 / 1024.0 / 1024.0);
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
            config_manager.remove_source(&path)?;
            println!("Source removed successfully");
        }
        ConfigAction::AddBackup { source, backup } => {
            config_manager.add_backup_dir(&source, backup)?;
            println!("Backup directory added successfully");
        }
        ConfigAction::RemoveBackup { source, backup } => {
            config_manager.remove_backup_dir(&source, &backup)?;
            println!("Backup directory removed successfully");
        }
        ConfigAction::Set { key, value } => {
            let config = config_manager.get_config_mut();
            match key.as_str() {
                "periodic_interval_minutes" => {
                    config.periodic_interval_minutes = value.parse()
                        .context("Invalid value for periodic_interval_minutes")?;
                }
                "enable_periodic" => {
                    config.enable_periodic = value.parse()
                        .context("Invalid value for enable_periodic")?;
                }
                "enable_event_driven" => {
                    config.enable_event_driven = value.parse()
                        .context("Invalid value for enable_event_driven")?;
                }
                "max_backups" => {
                    config.max_backups = value.parse()
                        .context("Invalid value for max_backups")?;
                }
                "backup_mode" => {
                    config.backup_mode = match value.as_str() {
                        "delta" => config::BackupMode::Delta,
                        "copy" => config::BackupMode::Copy,
                        _ => return Err(anyhow::anyhow!("Invalid backup_mode: '{}'. Use 'delta' or 'copy'", value)),
                    };
                }
                "full_backup_interval" => {
                    config.full_backup_interval = value.parse()
                        .context("Invalid value for full_backup_interval")?;
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
            let config = config_manager.get_config_mut();
            let src = config.sources.iter_mut()
                .find(|s| s.source_dir == source);
            
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
                        Some(value.parse().context("Invalid value for max_backups")?)
                    };
                }
                "backup_mode" => {
                    src.backup_mode = if is_reset {
                        None
                    } else {
                        Some(match value.as_str() {
                            "delta" => config::BackupMode::Delta,
                            "copy" => config::BackupMode::Copy,
                            _ => return Err(anyhow::anyhow!("Invalid backup_mode: '{}'. Use 'delta' or 'copy'", value)),
                        })
                    };
                }
                "full_backup_interval" => {
                    src.full_backup_interval = if is_reset {
                        None
                    } else {
                        Some(value.parse().context("Invalid value for full_backup_interval")?)
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

async fn handle_backup() -> Result<()> {
    let config_manager = ConfigManager::load_or_create()?;
    let config = config_manager.get_config().clone();
    let mut backup_manager = BackupManager::new(config);

    info!("Starting manual backup");
    
    match backup_manager.backup_all_sources().await {
        Ok(results) => {
            for result in results {
                println!(
                    "Backup completed: {} files to {:?} ({:.2} MB in {} ms)",
                    result.files_backed_up,
                    result.backup_dir,
                    result.bytes_processed as f64 / 1024.0 / 1024.0,
                    result.duration_ms
                );
            }
        }
        Err(e) => {
            error!("Backup failed: {}", e);
            return Err(e);
        }
    }

    Ok(())
}

async fn handle_run() -> Result<()> {
    let config_manager = ConfigManager::load_or_create()
        .context("Failed to load configuration")?;
    
    let config = config_manager.get_config().clone();
    let (backup_tx, mut backup_rx) = mpsc::channel::<()>(100);

    let mut backup_manager = BackupManager::new(config.clone());

    let periodic_task = if config.enable_periodic {
        let interval_minutes = config.periodic_interval_minutes;
        let backup_tx = backup_tx.clone();
        
        Some(tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(interval_minutes * 60));
            loop {
                interval.tick().await;
                if let Err(e) = backup_tx.send(()).await {
                    error!("Failed to send periodic backup trigger: {}", e);
                    break;
                }
                info!("Periodic backup triggered");
            }
        }))
    } else {
        None
    };

    let watcher_task = if config.enable_event_driven && matches!(config.backup_mode, config::BackupMode::Delta) {
        let watch_paths: Vec<PathBuf> = config.sources
            .iter()
            .filter(|s| s.enabled)
            .map(|s| s.source_dir.clone())
            .collect();

        Some(tokio::task::spawn_blocking(move || {
            match FileWatcher::new(watch_paths, backup_tx.clone(), Duration::from_millis(300)) {
                Ok(_) => {
                    info!("File watcher started");
                    loop {
                        let _ = sleep(Duration::from_secs(1));
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

    info!("Ardiex backup service started (mode: {:?})", config.backup_mode);

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

    if let Some(task) = periodic_task {
        task.abort();
    }
    if let Some(task) = watcher_task {
        task.abort();
    }

    info!("Ardiex backup service stopped");
    Ok(())
}

async fn handle_restore(
    backup_dir: PathBuf,
    target_dir: PathBuf,
    point: Option<String>,
    list: bool,
) -> Result<()> {
    if list {
        let backups = RestoreManager::list_backups(&backup_dir)?;
        if backups.is_empty() {
            println!("No backups found in {:?}", backup_dir);
            return Ok(());
        }
        println!("Available backups in {:?}:", backup_dir);
        for backup in &backups {
            let backup_type = if backup.is_full { "FULL" } else { "INC " };
            println!("  [{}] {} ({})", backup_type, backup.timestamp, backup.name);
        }
        return Ok(());
    }

    info!("Starting restore from {:?} to {:?}", backup_dir, target_dir);

    let point_ref = point.as_deref();
    match RestoreManager::restore_to_point(&backup_dir, &target_dir, point_ref) {
        Ok(files_restored) => {
            println!(
                "Restore completed: {} files restored to {:?}",
                files_restored, target_dir
            );
        }
        Err(e) => {
            error!("Restore failed: {}", e);
            return Err(e);
        }
    }

    Ok(())
}
