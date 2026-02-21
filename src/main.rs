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

use cli::{Cli, Commands};
use commands::config_cmd::handle_config;
use commands::backup_cmd::handle_backup;
use commands::run_cmd::handle_run;
use commands::restore_cmd::handle_restore;

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
