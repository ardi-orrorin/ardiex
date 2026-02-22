use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ardiex")]
#[command(about = "Incremental backup system with periodic and event-driven triggers")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
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
pub enum ConfigAction {
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
    ///
    /// Available keys:
    ///   enable_periodic        (true/false)
    ///   enable_event_driven    (true/false)
    ///   max_backups            (number)
    ///   backup_mode            (delta/copy)
    ///   cron_schedule          ("sec min hour day month dow")
    ///   enable_min_interval_by_size  (true/false)
    ///   max_log_file_size_mb   (number, > 0)
    Set {
        /// Key: enable_periodic, enable_event_driven, max_backups, backup_mode, cron_schedule, enable_min_interval_by_size, max_log_file_size_mb
        key: String,
        /// Configuration value
        value: String,
    },
    /// Set a source-specific configuration value (overrides global)
    ///
    /// Available keys:
    ///   exclude_patterns       (comma-separated, e.g. "*.cache,*.tmp")
    ///   max_backups            (number)
    ///   backup_mode            (delta/copy)
    ///   cron_schedule          ("sec min hour day month dow")
    ///   enable_event_driven    (true/false)
    ///   enable_periodic        (true/false)
    /// Use "reset" as value to clear and fall back to global
    SetSource {
        /// Source directory path
        source: PathBuf,
        /// Key: exclude_patterns, max_backups, backup_mode, cron_schedule, enable_event_driven, enable_periodic (use "reset" as value to clear)
        key: String,
        /// Configuration value (use "reset" to clear override)
        value: String,
    },
}
