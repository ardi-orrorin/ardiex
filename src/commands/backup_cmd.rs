use anyhow::Result;
use log::{error, info};

use crate::backup::BackupManager;
use crate::config::ConfigManager;

pub async fn handle_backup() -> Result<()> {
    let config_manager = ConfigManager::load_or_create()?;
    let config = config_manager.get_config().clone();
    let mut backup_manager = BackupManager::new(config);

    info!("Starting manual backup");
    backup_manager.validate_all_sources()?;
    
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
