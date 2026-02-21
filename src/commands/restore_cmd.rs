use anyhow::Result;
use log::{error, info};
use std::path::PathBuf;

use crate::restore::RestoreManager;

pub async fn handle_restore(
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
