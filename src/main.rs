mod backup;
mod cli;
mod commands;
mod config;
mod delta;
mod logger;
mod restore;
mod update;
mod watcher;

use anyhow::{Context, Result};
use clap::Parser;
use log::{info, warn};
use mimalloc::MiMalloc;
use std::process::{Command, Stdio};

use cli::{Cli, Commands};
use commands::backup_cmd::handle_backup;
use commands::config_cmd::handle_config;
use commands::restore_cmd::handle_restore;
use commands::run_cmd::handle_run;
use config::ConfigManager;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const UPDATE_REPO: &str = "ardi-orrorin/ardiex";
const UPDATE_SKIP_ENV_KEY: &str = "ARDIEX_SKIP_UPDATE_CHECK";
const UPDATE_SKIP_ENV_VALUE: &str = "1";

fn updater_binary_name() -> &'static str {
    if cfg!(windows) {
        "updater.exe"
    } else {
        "updater"
    }
}

fn should_skip_update_check(args: &[String]) -> bool {
    if std::env::var(UPDATE_SKIP_ENV_KEY)
        .ok()
        .map(|v| v == UPDATE_SKIP_ENV_VALUE || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return true;
    }

    args.iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h" | "--version" | "-V"))
}

async fn maybe_delegate_to_updater(forward_args: &[String]) -> Result<bool> {
    if should_skip_update_check(forward_args) {
        return Ok(false);
    }

    let current_version = env!("CARGO_PKG_VERSION");
    let latest_release =
        match tokio::task::spawn_blocking(|| update::fetch_latest_release(UPDATE_REPO)).await {
            Ok(Ok(release)) => release,
            Ok(Err(e)) => {
                warn!("[UPDATE] Latest release check failed: {}", e);
                return Ok(false);
            }
            Err(e) => {
                warn!("[UPDATE] Latest release check task failed: {}", e);
                return Ok(false);
            }
        };

    let latest_version = update::normalize_version(&latest_release.tag_name);
    if !update::is_newer_version(current_version, &latest_version) {
        info!(
            "[UPDATE] Current version {} is up-to-date (latest: {})",
            current_version, latest_version
        );
        return Ok(false);
    }

    let asset_name = match update::expected_release_asset_name_for_current_target() {
        Ok(name) => name,
        Err(e) => {
            warn!("[UPDATE] Unsupported target for auto-update: {}", e);
            return Ok(false);
        }
    };
    let asset_url = match update::find_release_asset_download_url(&latest_release, asset_name) {
        Ok(url) => url,
        Err(e) => {
            warn!("[UPDATE] Latest release asset lookup failed: {}", e);
            return Ok(false);
        }
    };

    let current_exe = std::env::current_exe().context("Failed to resolve current executable")?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Current executable has no parent directory"))?;
    let updater_path = exe_dir.join(updater_binary_name());

    if !updater_path.exists() {
        warn!(
            "[UPDATE] New version {} exists but updater binary is missing: {:?}",
            latest_version, updater_path
        );
        return Ok(false);
    }

    let mut cmd = Command::new(&updater_path);
    cmd.arg("--repo")
        .arg(UPDATE_REPO)
        .arg("--asset-url")
        .arg(asset_url)
        .arg("--asset-name")
        .arg(asset_name)
        .arg("--target-version")
        .arg(latest_version)
        .arg("--current-exe")
        .arg(&current_exe)
        .arg("--parent-pid")
        .arg(std::process::id().to_string());

    for arg in forward_args {
        cmd.arg("--forward-arg").arg(arg);
    }

    cmd.env(UPDATE_SKIP_ENV_KEY, UPDATE_SKIP_ENV_VALUE)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = cmd.spawn().with_context(|| {
        format!(
            "Failed to spawn updater process: {:?} (target version: {})",
            updater_path, latest_release.tag_name
        )
    })?;

    info!(
        "[UPDATE] Delegated update to {:?} (pid: {}, target: {})",
        updater_path,
        child.id(),
        latest_release.tag_name
    );
    Ok(true)
}

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

    let forward_args: Vec<String> = std::env::args().skip(1).collect();
    match maybe_delegate_to_updater(&forward_args).await {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(e) => warn!(
            "[UPDATE] Failed to delegate updater, continue current process: {}",
            e
        ),
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
