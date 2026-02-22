use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum BackupMode {
    #[default]
    #[serde(rename = "delta")]
    Delta,
    #[serde(rename = "copy")]
    Copy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupConfig {
    pub sources: Vec<SourceConfig>,
    pub enable_periodic: bool,
    pub enable_event_driven: bool,
    pub exclude_patterns: Vec<String>,
    pub max_backups: usize,
    #[serde(default)]
    pub backup_mode: BackupMode,
    #[serde(default = "default_cron_schedule")]
    pub cron_schedule: String,
    #[serde(default = "default_true")]
    pub enable_min_interval_by_size: bool,
    #[serde(default = "default_max_log_file_size_mb")]
    pub max_log_file_size_mb: u64,
    pub metadata: HashMap<String, SourceMetadata>,
}

fn default_cron_schedule() -> String {
    "0 0 * * * *".to_string() // every hour
}

fn default_true() -> bool {
    true
}

fn default_max_log_file_size_mb() -> u64 {
    20
}

pub fn auto_full_backup_interval(max_backups: usize) -> usize {
    if max_backups <= 1 { 1 } else { max_backups - 1 }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    pub source_dir: PathBuf,
    pub backup_dirs: Vec<PathBuf>,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_patterns: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_backups: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_mode: Option<BackupMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_schedule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_event_driven: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_periodic: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ResolvedSourceConfig {
    pub exclude_patterns: Vec<String>,
    pub max_backups: usize,
    pub backup_mode: BackupMode,
    pub full_backup_interval: usize,
    pub cron_schedule: String,
    pub enable_event_driven: bool,
    pub enable_periodic: bool,
}

impl SourceConfig {
    pub fn effective_backup_dirs(&self) -> Vec<PathBuf> {
        if self.backup_dirs.is_empty() {
            vec![self.source_dir.join(".backup")]
        } else {
            self.backup_dirs.clone()
        }
    }

    pub fn resolve(&self, global: &BackupConfig) -> ResolvedSourceConfig {
        let resolved_max_backups = self.max_backups.unwrap_or(global.max_backups);

        ResolvedSourceConfig {
            exclude_patterns: self
                .exclude_patterns
                .clone()
                .unwrap_or_else(|| global.exclude_patterns.clone()),
            max_backups: resolved_max_backups,
            backup_mode: self
                .backup_mode
                .clone()
                .unwrap_or_else(|| global.backup_mode.clone()),
            // Full backup interval is always derived from max_backups.
            full_backup_interval: auto_full_backup_interval(resolved_max_backups),
            cron_schedule: self
                .cron_schedule
                .clone()
                .unwrap_or_else(|| global.cron_schedule.clone()),
            enable_event_driven: self
                .enable_event_driven
                .unwrap_or(global.enable_event_driven),
            enable_periodic: self.enable_periodic.unwrap_or(global.enable_periodic),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceMetadata {
    pub last_full_backup: Option<DateTime<Utc>>,
    pub last_backup: Option<DateTime<Utc>>,
    pub file_hashes: HashMap<String, String>,
    #[serde(default)]
    pub backup_history: Vec<BackupHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackupHistoryType {
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "inc")]
    Incremental,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupHistoryEntry {
    pub backup_name: String,
    pub backup_type: BackupHistoryType,
    pub created_at: DateTime<Utc>,
    pub files_backed_up: usize,
    pub bytes_processed: u64,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            sources: vec![],
            enable_periodic: true,
            enable_event_driven: true,
            exclude_patterns: vec![
                "*.tmp".to_string(),
                "*.log".to_string(),
                ".git/*".to_string(),
                ".DS_Store".to_string(),
            ],
            max_backups: 10,
            backup_mode: BackupMode::Delta,
            cron_schedule: "0 0 * * * *".to_string(),
            enable_min_interval_by_size: true,
            max_log_file_size_mb: default_max_log_file_size_mb(),
            metadata: HashMap::new(),
        }
    }
}

pub struct ConfigManager {
    pub config_path: PathBuf,
    config: BackupConfig,
}

impl ConfigManager {
    pub fn load_or_create() -> Result<Self> {
        let config_path = get_config_path()?;

        let config = if config_path.exists() {
            let content =
                fs::read_to_string(&config_path).context("Failed to read settings.json")?;
            serde_json::from_str(&content).context("Failed to parse settings.json")?
        } else {
            let config = BackupConfig::default();
            let content = serde_json::to_string_pretty(&config)
                .context("Failed to serialize default config")?;
            fs::write(&config_path, content).context("Failed to create default settings.json")?;
            config
        };

        Ok(Self {
            config_path,
            config,
        })
    }

    pub fn save(&mut self) -> Result<()> {
        let content =
            serde_json::to_string_pretty(&self.config).context("Failed to serialize config")?;
        fs::write(&self.config_path, content).context("Failed to save settings.json")?;
        Ok(())
    }

    pub fn get_config(&self) -> &BackupConfig {
        &self.config
    }

    pub fn get_config_mut(&mut self) -> &mut BackupConfig {
        &mut self.config
    }

    pub fn add_source(&mut self, source_dir: PathBuf, backup_dirs: Vec<PathBuf>) -> Result<()> {
        if !source_dir.exists() {
            return Err(anyhow::anyhow!(
                "Source directory does not exist: {:?}",
                source_dir
            ));
        }

        let source_config = SourceConfig {
            source_dir: source_dir.clone(),
            backup_dirs,
            enabled: true,
            exclude_patterns: None,
            max_backups: None,
            backup_mode: None,
            cron_schedule: None,
            enable_event_driven: None,
            enable_periodic: None,
        };

        if let Some(existing) = self
            .config
            .sources
            .iter_mut()
            .find(|s| s.source_dir == source_dir)
        {
            *existing = source_config;
        } else {
            self.config.sources.push(source_config);
        }

        self.save()?;
        Ok(())
    }

    pub fn remove_source(&mut self, source_dir: &Path) -> Result<()> {
        self.config.sources.retain(|s| s.source_dir != source_dir);
        self.config
            .metadata
            .remove(&source_dir.to_string_lossy().into_owned());
        self.save()?;
        Ok(())
    }

    pub fn add_backup_dir(&mut self, source_dir: &Path, backup_dir: PathBuf) -> Result<()> {
        if let Some(source) = self
            .config
            .sources
            .iter_mut()
            .find(|s| s.source_dir == source_dir)
            && !source.backup_dirs.contains(&backup_dir)
        {
            source.backup_dirs.push(backup_dir);
            self.save()?;
        }
        Ok(())
    }

    pub fn remove_backup_dir(&mut self, source_dir: &Path, backup_dir: &Path) -> Result<()> {
        if let Some(source) = self
            .config
            .sources
            .iter_mut()
            .find(|s| s.source_dir == source_dir)
        {
            source.backup_dirs.retain(|d| d != backup_dir);
            self.save()?;
        }
        Ok(())
    }
}

fn get_config_path() -> Result<PathBuf> {
    let mut exe_path = std::env::current_exe().context("Failed to get executable path")?;
    exe_path.pop();
    exe_path.push("settings.json");
    Ok(exe_path)
}
