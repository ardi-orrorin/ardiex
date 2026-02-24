use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

const BLOCK_SIZE: usize = 4096; // 4KB blocks

#[derive(Debug, Serialize, Deserialize)]
pub struct DeltaFile {
    pub original_file_hash: String,
    pub block_size: usize,
    pub total_blocks: usize,
    pub changed_blocks: Vec<DeltaBlock>,
    pub new_file_size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeltaBlock {
    pub index: usize,
    pub hash: String,
    pub data: Vec<u8>,
}

pub fn calculate_block_hashes(file_path: &Path) -> Result<Vec<String>> {
    let file = fs::File::open(file_path)
        .with_context(|| format!("Failed to open file: {:?}", file_path))?;
    let mut reader = BufReader::new(file);
    let mut hashes = Vec::new();
    let mut buffer = vec![0u8; BLOCK_SIZE];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }

        let mut hasher = Sha256::new();
        hasher.update(&buffer[..bytes_read]);
        hashes.push(format!("{:x}", hasher.finalize()));
    }

    Ok(hashes)
}

pub fn create_delta(original_path: &Path, new_path: &Path) -> Result<DeltaFile> {
    let original_hashes = if original_path.exists() {
        calculate_block_hashes(original_path)?
    } else {
        Vec::new()
    };

    let new_file = fs::File::open(new_path)
        .with_context(|| format!("Failed to open new file: {:?}", new_path))?;
    let new_file_size = fs::metadata(new_path)?.len();
    let mut reader = BufReader::new(new_file);
    let mut buffer = vec![0u8; BLOCK_SIZE];
    let mut changed_blocks = Vec::new();
    let mut block_index = 0;

    let mut file_hasher = Sha256::new();
    let original_content = if original_path.exists() {
        fs::read(original_path).unwrap_or_default()
    } else {
        Vec::new()
    };
    file_hasher.update(&original_content);
    let original_file_hash = format!("{:x}", file_hasher.finalize());

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }

        let mut block_hasher = Sha256::new();
        block_hasher.update(&buffer[..bytes_read]);
        let new_hash = format!("{:x}", block_hasher.finalize());

        let is_changed = if block_index < original_hashes.len() {
            original_hashes[block_index] != new_hash
        } else {
            true // new block (file grew)
        };

        if is_changed {
            changed_blocks.push(DeltaBlock {
                index: block_index,
                hash: new_hash,
                data: buffer[..bytes_read].to_vec(),
            });
        }

        block_index += 1;
    }

    let total_blocks = block_index;

    Ok(DeltaFile {
        original_file_hash,
        block_size: BLOCK_SIZE,
        total_blocks,
        changed_blocks,
        new_file_size,
    })
}

pub fn apply_delta(original_path: &Path, delta: &DeltaFile, output_path: &Path) -> Result<()> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut blocks: Vec<Vec<u8>> = Vec::new();

    // Read original file blocks
    if original_path.exists() {
        let file = fs::File::open(original_path)?;
        let mut reader = BufReader::new(file);
        let mut buffer = vec![0u8; delta.block_size];

        loop {
            let bytes_read = reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            blocks.push(buffer[..bytes_read].to_vec());
        }
    }

    // Extend blocks if file grew
    while blocks.len() < delta.total_blocks {
        blocks.push(Vec::new());
    }

    // Apply changed blocks
    for changed_block in &delta.changed_blocks {
        if changed_block.index < blocks.len() {
            blocks[changed_block.index] = changed_block.data.clone();
        }
    }

    // Write output file
    let file = fs::File::create(output_path)?;
    let mut writer = BufWriter::new(file);
    let mut bytes_written: u64 = 0;

    for block in &blocks {
        let remaining = delta.new_file_size - bytes_written;
        let to_write = std::cmp::min(block.len() as u64, remaining) as usize;
        writer.write_all(&block[..to_write])?;
        bytes_written += to_write as u64;
    }

    writer.flush()?;
    Ok(())
}

pub fn save_delta(delta: &DeltaFile, delta_path: &Path) -> Result<()> {
    if let Some(parent) = delta_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_vec(delta)?;
    fs::write(delta_path, content)?;
    Ok(())
}

pub fn load_delta(delta_path: &Path) -> Result<DeltaFile> {
    let content = fs::read(delta_path)?;
    let delta: DeltaFile = serde_json::from_slice(&content)?;
    Ok(delta)
}

pub fn delta_size(delta: &DeltaFile) -> usize {
    delta.changed_blocks.iter().map(|b| b.data.len()).sum()
}

#[cfg(test)]
#[path = "tests/delta_tests.rs"]
mod tests;
