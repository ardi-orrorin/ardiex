#[allow(dead_code)]
#[path = "../logger.rs"]
mod logger;

use anyhow::{Context, Result};
use clap::Parser;
use log::{info, warn};
use reqwest::blocking::Client;
use reqwest::header::USER_AGENT;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const DEFAULT_MAX_LOG_FILE_SIZE_MB: u64 = 20;
const UPDATER_LOG_FILE_NAME: &str = "updater.log";
const UPDATE_SKIP_ENV_KEY: &str = "ARDIEX_SKIP_UPDATE_CHECK";
const UPDATE_SKIP_ENV_VALUE: &str = "1";

#[derive(Debug, Parser)]
#[command(name = "updater")]
struct UpdaterArgs {
    #[arg(long)]
    repo: String,
    #[arg(long)]
    asset_url: String,
    #[arg(long)]
    asset_name: String,
    #[arg(long)]
    target_version: String,
    #[arg(long)]
    current_exe: PathBuf,
    #[arg(long)]
    parent_pid: u32,
    #[arg(long = "forward-arg")]
    forward_args: Vec<String>,
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn read_max_log_file_size_mb(exe_dir: &Path) -> u64 {
    let settings_path = exe_dir.join("settings.json");
    let Ok(content) = fs::read_to_string(settings_path) else {
        return DEFAULT_MAX_LOG_FILE_SIZE_MB;
    };

    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return DEFAULT_MAX_LOG_FILE_SIZE_MB;
    };

    let Some(size_mb) = json
        .get("max_log_file_size_mb")
        .and_then(|value| value.as_u64())
    else {
        return DEFAULT_MAX_LOG_FILE_SIZE_MB;
    };

    if size_mb == 0 {
        DEFAULT_MAX_LOG_FILE_SIZE_MB
    } else {
        size_mb
    }
}

fn init_updater_logging() {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()));

    if let Some(exe_dir) = exe_dir {
        let log_dir = exe_dir.join("logs");
        let size_mb = read_max_log_file_size_mb(&exe_dir);
        if let Err(err) =
            logger::init_file_logging_with_size_and_name(&log_dir, size_mb, UPDATER_LOG_FILE_NAME)
        {
            eprintln!("Failed to initialize updater file logging: {}", err);
            logger::init_console_logging();
        }
    } else {
        logger::init_console_logging();
    }
}

fn download_asset(url: &str, destination: &Path) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("Failed to build updater HTTP client")?;

    let mut response = client
        .get(url)
        .header(USER_AGENT, "ardiex-updater")
        .send()
        .with_context(|| format!("Failed to download release asset from {}", url))?
        .error_for_status()
        .with_context(|| format!("Release asset download failed with error status: {}", url))?;

    let mut output =
        File::create(destination).with_context(|| format!("Failed to create {:?}", destination))?;
    io::copy(&mut response, &mut output)
        .with_context(|| format!("Failed to write downloaded asset to {:?}", destination))?;
    Ok(())
}

fn extract_zip_archive(archive_path: &Path, destination_dir: &Path) -> Result<()> {
    let file = File::open(archive_path)
        .with_context(|| format!("Failed to open zip archive {:?}", archive_path))?;
    let mut zip = zip::ZipArchive::new(file)
        .with_context(|| format!("Failed to parse zip archive {:?}", archive_path))?;

    for idx in 0..zip.len() {
        let mut entry = zip.by_index(idx).context("Failed to read zip entry")?;
        let Some(enclosed) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let output_path = destination_dir.join(enclosed);

        if entry.is_dir() {
            fs::create_dir_all(&output_path)
                .with_context(|| format!("Failed to create directory {:?}", output_path))?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {:?}", parent))?;
        }

        let mut output = File::create(&output_path)
            .with_context(|| format!("Failed to create {:?}", output_path))?;
        io::copy(&mut entry, &mut output)
            .with_context(|| format!("Failed to extract zip entry to {:?}", output_path))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                fs::set_permissions(&output_path, fs::Permissions::from_mode(mode))
                    .with_context(|| format!("Failed to set permissions for {:?}", output_path))?;
            }
        }
    }

    Ok(())
}

fn extract_archive(archive_path: &Path, archive_name: &str, destination_dir: &Path) -> Result<()> {
    if archive_name.ends_with(".tar.gz") {
        let file = File::open(archive_path)
            .with_context(|| format!("Failed to open tar.gz archive {:?}", archive_path))?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(destination_dir)
            .with_context(|| format!("Failed to unpack tar.gz archive to {:?}", destination_dir))?;
        return Ok(());
    }

    if archive_name.ends_with(".zip") {
        return extract_zip_archive(archive_path, destination_dir);
    }

    Err(anyhow::anyhow!(
        "Unsupported release asset format for updater: {}",
        archive_name
    ))
}

fn find_file_by_name(root: &Path, file_name: &str) -> Option<PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .find_map(|entry| {
            if entry.file_type().is_file() && entry.file_name().to_string_lossy() == file_name {
                Some(entry.path().to_path_buf())
            } else {
                None
            }
        })
}

#[cfg(unix)]
fn is_process_running(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_process_running(pid: u32) -> bool {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid)])
        .output();

    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .any(|line| line.contains(&pid.to_string()) && !line.contains("No tasks are running"))
}

#[cfg(not(any(unix, windows)))]
fn is_process_running(_pid: u32) -> bool {
    false
}

fn wait_for_parent_exit(parent_pid: u32, timeout: Duration) {
    let started = Instant::now();
    while is_process_running(parent_pid) {
        if started.elapsed() >= timeout {
            warn!(
                "[UPDATER] Parent process {} did not exit within {}s; continuing with retry copy",
                parent_pid,
                timeout.as_secs()
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn replace_binary_with_retry(
    new_binary_path: &Path,
    target_binary_path: &Path,
    timeout: Duration,
    interval: Duration,
) -> Result<()> {
    let started = Instant::now();

    loop {
        match fs::copy(new_binary_path, target_binary_path) {
            Ok(_) => {
                info!(
                    "[UPDATER] Replaced binary: {:?} -> {:?}",
                    new_binary_path, target_binary_path
                );
                return Ok(());
            }
            Err(err) => {
                if started.elapsed() >= timeout {
                    return Err(anyhow::anyhow!(
                        "Failed to replace target binary within {}s: {}",
                        timeout.as_secs(),
                        err
                    ));
                }
                std::thread::sleep(interval);
            }
        }
    }
}

fn restart_target_binary(target_exe: &Path, args: &[String]) -> Result<()> {
    let mut cmd = Command::new(target_exe);
    cmd.args(args)
        .env(UPDATE_SKIP_ENV_KEY, UPDATE_SKIP_ENV_VALUE)
        .spawn()
        .with_context(|| format!("Failed to restart updated binary {:?}", target_exe))?;

    info!("[UPDATER] Restarted updated binary: {:?}", target_exe);
    Ok(())
}

fn updater_main() -> Result<()> {
    let args = UpdaterArgs::parse();
    info!(
        "[UPDATER] Start repo={}, target_version={}, asset_name={}",
        args.repo, args.target_version, args.asset_name
    );

    wait_for_parent_exit(args.parent_pid, Duration::from_secs(30));

    let work_dir = std::env::temp_dir().join(format!(
        "ardiex-updater-{}-{}",
        std::process::id(),
        now_millis()
    ));
    let archive_path = work_dir.join(&args.asset_name);
    let extract_dir = work_dir.join("extract");
    fs::create_dir_all(&extract_dir)
        .with_context(|| format!("Failed to create updater work dir {:?}", extract_dir))?;

    download_asset(&args.asset_url, &archive_path)?;
    info!("[UPDATER] Download completed: {:?}", archive_path);

    extract_archive(&archive_path, &args.asset_name, &extract_dir)?;
    info!("[UPDATER] Extract completed: {:?}", extract_dir);

    let target_file_name = args
        .current_exe
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid current_exe path: {:?}", args.current_exe))?
        .to_string_lossy()
        .to_string();
    let new_binary_path = find_file_by_name(&extract_dir, &target_file_name).ok_or_else(|| {
        anyhow::anyhow!(
            "Updated binary '{}' not found in extracted asset",
            target_file_name
        )
    })?;

    replace_binary_with_retry(
        &new_binary_path,
        &args.current_exe,
        Duration::from_secs(60),
        Duration::from_millis(500),
    )?;

    restart_target_binary(&args.current_exe, &args.forward_args)?;

    if let Err(err) = fs::remove_dir_all(&work_dir) {
        warn!(
            "[UPDATER] Failed to cleanup temporary updater directory {:?}: {}",
            work_dir, err
        );
    }

    info!(
        "[UPDATER] Update completed successfully: {}",
        args.target_version
    );
    Ok(())
}

fn main() -> Result<()> {
    init_updater_logging();
    updater_main()
}
