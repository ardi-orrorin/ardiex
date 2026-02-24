use super::*;
use crate::delta;
use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_nanos();
    std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), ts))
}

fn make_backup_entry(name: &str, is_full: bool) -> BackupEntry {
    BackupEntry {
        name: name.to_string(),
        path: PathBuf::from(format!("/tmp/{}", name)),
        is_full,
        timestamp: if is_full {
            name.trim_start_matches("full_").to_string()
        } else {
            name.trim_start_matches("inc_").to_string()
        },
    }
}

#[test]
fn list_backups_filters_and_sorts_entries() -> Result<()> {
    let base = unique_temp_dir("ardiex_restore_list");
    fs::create_dir_all(base.join("full_20260224_120000"))?;
    fs::create_dir_all(base.join("inc_20260224_130000"))?;
    fs::create_dir_all(base.join("full_20260224_110000"))?;
    fs::create_dir_all(base.join("ignored_dir"))?;
    fs::write(base.join("inc_20260224_140000"), b"not a dir")?;

    let backups = RestoreManager::list_backups(&base)?;
    let names: Vec<String> = backups.into_iter().map(|b| b.name).collect();
    assert_eq!(
        names,
        vec![
            "full_20260224_110000".to_string(),
            "full_20260224_120000".to_string(),
            "inc_20260224_130000".to_string(),
        ]
    );

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn select_backups_uses_latest_full_before_cutoff() -> Result<()> {
    let backups = vec![
        make_backup_entry("full_20260224_100000", true),
        make_backup_entry("inc_20260224_101000", false),
        make_backup_entry("full_20260224_110000", true),
        make_backup_entry("inc_20260224_111000", false),
        make_backup_entry("inc_20260224_112000", false),
    ];

    let selected = RestoreManager::select_backups(&backups, Some("20260224_111500"))?;
    let names: Vec<String> = selected.into_iter().map(|b| b.name.clone()).collect();
    assert_eq!(
        names,
        vec![
            "full_20260224_110000".to_string(),
            "inc_20260224_111000".to_string(),
        ]
    );
    Ok(())
}

#[test]
fn select_backups_fails_when_no_full_exists_before_cutoff() {
    let backups = vec![
        make_backup_entry("inc_20260224_101000", false),
        make_backup_entry("inc_20260224_102000", false),
    ];

    let err = RestoreManager::select_backups(&backups, Some("20260224_103000"))
        .expect_err("must fail without full backup");
    assert!(err.to_string().contains("No full backup found"));
}

#[test]
fn strip_delta_extension_removes_only_delta_suffix() {
    assert_eq!(
        RestoreManager::strip_delta_extension(Path::new("a.txt.delta")),
        PathBuf::from("a.txt")
    );
    assert_eq!(
        RestoreManager::strip_delta_extension(Path::new("nested/a.bin.delta")),
        PathBuf::from("nested/a.bin")
    );
    assert_eq!(
        RestoreManager::strip_delta_extension(Path::new("a.txt")),
        PathBuf::from("a.txt")
    );
}

#[test]
fn restore_to_point_fails_when_backup_dir_is_empty() -> Result<()> {
    let backup_dir = unique_temp_dir("ardiex_restore_empty_backup");
    let target_dir = unique_temp_dir("ardiex_restore_empty_target");
    fs::create_dir_all(&backup_dir)?;

    let err = RestoreManager::restore_to_point(&backup_dir, &target_dir, None)
        .expect_err("restore must fail when no backups exist");
    assert!(err.to_string().contains("No backups found"));

    fs::remove_dir_all(&backup_dir)?;
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir)?;
    }
    Ok(())
}

#[test]
fn restore_to_point_applies_full_and_incremental_copy_chain() -> Result<()> {
    let base = unique_temp_dir("ardiex_restore_copy_chain");
    let backup_dir = base.join("backup");
    let target_dir = base.join("target");
    let full_dir = backup_dir.join("full_20260224_120000");
    let inc_dir = backup_dir.join("inc_20260224_121000");
    fs::create_dir_all(&full_dir)?;
    fs::create_dir_all(&inc_dir)?;

    fs::write(full_dir.join("a.txt"), b"v1")?;
    fs::write(inc_dir.join("a.txt"), b"v2")?;

    let restored = RestoreManager::restore_to_point(&backup_dir, &target_dir, None)?;
    assert_eq!(restored, 2);
    assert_eq!(fs::read(target_dir.join("a.txt"))?, b"v2");

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn restore_to_point_applies_delta_incremental() -> Result<()> {
    let base = unique_temp_dir("ardiex_restore_delta_chain");
    let backup_dir = base.join("backup");
    let target_dir = base.join("target");
    let full_dir = backup_dir.join("full_20260224_120000");
    let inc_dir = backup_dir.join("inc_20260224_121000");
    fs::create_dir_all(&full_dir)?;
    fs::create_dir_all(&inc_dir)?;

    let full_file = full_dir.join("a.txt");
    fs::write(&full_file, b"hello-old")?;
    let tmp_new = base.join("tmp_new.txt");
    fs::write(&tmp_new, b"hello-new")?;

    let delta_data = delta::create_delta(&full_file, &tmp_new)?;
    delta::save_delta(&delta_data, &inc_dir.join("a.txt.delta"))?;

    let restored = RestoreManager::restore_to_point(&backup_dir, &target_dir, None)?;
    assert_eq!(restored, 2);
    assert_eq!(fs::read(target_dir.join("a.txt"))?, b"hello-new");

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn restore_to_point_respects_cutoff_and_skips_later_incrementals() -> Result<()> {
    let base = unique_temp_dir("ardiex_restore_cutoff");
    let backup_dir = base.join("backup");
    let target_dir = base.join("target");
    let full_dir = backup_dir.join("full_20260224_120000");
    let inc1_dir = backup_dir.join("inc_20260224_121000");
    let inc2_dir = backup_dir.join("inc_20260224_122000");
    fs::create_dir_all(&full_dir)?;
    fs::create_dir_all(&inc1_dir)?;
    fs::create_dir_all(&inc2_dir)?;

    fs::write(full_dir.join("a.txt"), b"v1")?;
    fs::write(inc1_dir.join("a.txt"), b"v2")?;
    fs::write(inc2_dir.join("a.txt"), b"v3")?;

    let restored =
        RestoreManager::restore_to_point(&backup_dir, &target_dir, Some("20260224_121000"))?;
    assert_eq!(restored, 2);
    assert_eq!(fs::read(target_dir.join("a.txt"))?, b"v2");

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn list_backups_fails_when_backup_dir_does_not_exist() {
    let missing = PathBuf::from("/tmp/ardiex_restore_missing_backup_dir");
    let err = RestoreManager::list_backups(&missing)
        .expect_err("listing backups on missing dir must fail");
    assert!(!err.to_string().is_empty());
}

#[test]
fn restore_to_point_fails_when_cutoff_is_before_any_full() -> Result<()> {
    let base = unique_temp_dir("ardiex_restore_cutoff_before_full");
    let backup_dir = base.join("backup");
    let target_dir = base.join("target");
    fs::create_dir_all(backup_dir.join("full_20260224_120000"))?;

    let err = RestoreManager::restore_to_point(&backup_dir, &target_dir, Some("20260224_110000"))
        .expect_err("cutoff before first full must fail");
    assert!(
        err.to_string()
            .contains("No full backup found before restore point")
    );

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn restore_to_point_fails_on_invalid_delta_file() -> Result<()> {
    let base = unique_temp_dir("ardiex_restore_invalid_delta");
    let backup_dir = base.join("backup");
    let target_dir = base.join("target");
    let full_dir = backup_dir.join("full_20260224_120000");
    let inc_dir = backup_dir.join("inc_20260224_121000");
    fs::create_dir_all(&full_dir)?;
    fs::create_dir_all(&inc_dir)?;
    fs::write(full_dir.join("a.txt"), b"v1")?;
    fs::write(inc_dir.join("a.txt.delta"), b"{invalid delta")?;

    let err = RestoreManager::restore_to_point(&backup_dir, &target_dir, None)
        .expect_err("invalid delta content must fail restore");
    assert!(!err.to_string().is_empty());

    fs::remove_dir_all(&base)?;
    Ok(())
}
