mod backup;
mod cli;
mod commands;
mod config;
mod delta;
mod logger;
mod restore;
mod watcher;

use anyhow::Result;
use clap::Parser;
use mimalloc::MiMalloc;

use cli::{Cli, Commands};
use commands::backup_cmd::handle_backup;
use commands::config_cmd::handle_config;
use commands::restore_cmd::handle_restore;
use commands::run_cmd::handle_run;
use config::ConfigManager;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    let log_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .map(|p| p.join("logs"));

    let max_log_file_size_mb = match ConfigManager::load_or_create() {
        Ok(cm) => cm.get_config().max_log_file_size_mb,
        Err(e) => {
            eprintln!(
                "Failed to read settings for max_log_file_size_mb, using default: {}",
                e
            );
            20
        }
    };

    if let Some(ref log_dir) = log_dir {
        if let Err(e) = logger::init_file_logging_with_size(log_dir, max_log_file_size_mb) {
            eprintln!("Failed to initialize file logging: {}", e);
            logger::init_console_logging();
        }
    } else {
        logger::init_console_logging();
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Config { action } => handle_config(action).await?,
        Commands::Backup => handle_backup().await?,
        Commands::Restore {
            backup_dir,
            target_dir,
            point,
            list,
        } => handle_restore(backup_dir, target_dir, point, list).await?,
        Commands::Run => handle_run().await?,
    }

    Ok(())
}
