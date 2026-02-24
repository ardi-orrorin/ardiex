use super::*;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_nanos();
    std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), ts))
}

#[test]
fn calculate_block_hashes_is_deterministic() -> Result<()> {
    let base = unique_temp_dir("ardiex_delta_hashes");
    fs::create_dir_all(&base)?;
    let file = base.join("a.bin");
    fs::write(&file, b"abcdefg")?;

    let h1 = calculate_block_hashes(&file)?;
    let h2 = calculate_block_hashes(&file)?;
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 1);

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn create_delta_has_no_changed_blocks_when_files_are_equal() -> Result<()> {
    let base = unique_temp_dir("ardiex_delta_equal");
    fs::create_dir_all(&base)?;
    let original = base.join("old.bin");
    let new = base.join("new.bin");
    fs::write(&original, b"same-content")?;
    fs::write(&new, b"same-content")?;

    let delta = create_delta(&original, &new)?;
    assert_eq!(delta.changed_blocks.len(), 0);
    assert_eq!(delta.total_blocks, 1);
    assert_eq!(delta.new_file_size, b"same-content".len() as u64);

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn create_delta_marks_changed_block_when_content_differs() -> Result<()> {
    let base = unique_temp_dir("ardiex_delta_diff");
    fs::create_dir_all(&base)?;
    let original = base.join("old.bin");
    let new = base.join("new.bin");
    fs::write(&original, b"aaaa")?;
    fs::write(&new, b"bbbb")?;

    let delta = create_delta(&original, &new)?;
    assert_eq!(delta.changed_blocks.len(), 1);
    assert_eq!(delta.changed_blocks[0].index, 0);
    assert_eq!(delta.new_file_size, 4);

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn create_delta_with_missing_original_includes_all_new_blocks() -> Result<()> {
    let base = unique_temp_dir("ardiex_delta_missing_original");
    fs::create_dir_all(&base)?;
    let original = base.join("missing.bin");
    let new = base.join("new.bin");
    fs::write(&new, vec![1u8; 5000])?;

    let delta = create_delta(&original, &new)?;
    assert_eq!(delta.total_blocks, 2);
    assert_eq!(delta.changed_blocks.len(), 2);
    assert_eq!(delta.new_file_size, 5000);

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn apply_delta_reconstructs_new_file_from_original() -> Result<()> {
    let base = unique_temp_dir("ardiex_delta_apply");
    fs::create_dir_all(&base)?;
    let original = base.join("old.bin");
    let new = base.join("new.bin");
    let restored = base.join("restored.bin");

    fs::write(&original, b"old-content-1234")?;
    fs::write(&new, b"new-content-9876")?;

    let delta = create_delta(&original, &new)?;
    apply_delta(&original, &delta, &restored)?;

    assert_eq!(fs::read(&restored)?, fs::read(&new)?);

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn apply_delta_without_original_creates_file() -> Result<()> {
    let base = unique_temp_dir("ardiex_delta_apply_missing_original");
    fs::create_dir_all(&base)?;
    let original = base.join("missing.bin");
    let new = base.join("new.bin");
    let restored = base.join("restored.bin");

    fs::write(&new, b"brand-new-content")?;
    let delta = create_delta(&original, &new)?;
    apply_delta(&original, &delta, &restored)?;

    assert_eq!(fs::read(&restored)?, b"brand-new-content");

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn save_and_load_delta_roundtrip() -> Result<()> {
    let base = unique_temp_dir("ardiex_delta_save_load");
    fs::create_dir_all(&base)?;
    let original = base.join("old.bin");
    let new = base.join("new.bin");
    let delta_path = base.join("changes.delta");

    fs::write(&original, b"abc")?;
    fs::write(&new, b"abd")?;
    let delta = create_delta(&original, &new)?;
    save_delta(&delta, &delta_path)?;
    let loaded = load_delta(&delta_path)?;

    assert_eq!(loaded.block_size, delta.block_size);
    assert_eq!(loaded.total_blocks, delta.total_blocks);
    assert_eq!(loaded.new_file_size, delta.new_file_size);
    assert_eq!(loaded.changed_blocks.len(), delta.changed_blocks.len());

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn delta_size_returns_sum_of_changed_block_bytes() -> Result<()> {
    let base = unique_temp_dir("ardiex_delta_size");
    fs::create_dir_all(&base)?;
    let original = base.join("old.bin");
    let new = base.join("new.bin");

    fs::write(&original, vec![1u8; 4096])?;
    fs::write(&new, vec![2u8; 5000])?;
    let delta = create_delta(&original, &new)?;
    let size = delta_size(&delta);
    let expected: usize = delta.changed_blocks.iter().map(|b| b.data.len()).sum();
    assert_eq!(size, expected);

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn calculate_block_hashes_fails_for_missing_file() {
    let missing = PathBuf::from("/tmp/ardiex_missing_hash_target.bin");
    let err =
        calculate_block_hashes(&missing).expect_err("missing file hash calculation must fail");
    assert!(err.to_string().contains("Failed to open file"));
}

#[test]
fn create_delta_fails_when_new_file_is_missing() {
    let base = unique_temp_dir("ardiex_delta_new_missing");
    fs::create_dir_all(&base).expect("base dir create must succeed");
    let original = base.join("old.bin");
    let new = base.join("missing_new.bin");
    fs::write(&original, b"old").expect("old file write must succeed");

    let err = create_delta(&original, &new).expect_err("missing new file must fail");
    assert!(err.to_string().contains("Failed to open new file"));

    fs::remove_dir_all(&base).expect("base cleanup must succeed");
}

#[test]
fn load_delta_fails_on_invalid_delta_json() -> Result<()> {
    let base = unique_temp_dir("ardiex_delta_invalid_json");
    fs::create_dir_all(&base)?;
    let delta_path = base.join("broken.delta");
    fs::write(&delta_path, b"{invalid json")?;

    let err = load_delta(&delta_path).expect_err("invalid delta json must fail");
    assert!(!err.to_string().is_empty());

    fs::remove_dir_all(&base)?;
    Ok(())
}

#[test]
fn save_delta_fails_when_target_path_is_directory() -> Result<()> {
    let base = unique_temp_dir("ardiex_delta_save_to_dir");
    fs::create_dir_all(&base)?;
    let original = base.join("old.bin");
    let new = base.join("new.bin");
    fs::write(&original, b"abc")?;
    fs::write(&new, b"abd")?;
    let delta = create_delta(&original, &new)?;

    let err = save_delta(&delta, &base).expect_err("saving delta to directory path must fail");
    assert!(!err.to_string().is_empty());

    fs::remove_dir_all(&base)?;
    Ok(())
}
