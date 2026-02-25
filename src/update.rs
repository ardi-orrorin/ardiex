use anyhow::{Context, Result};
use serde::Deserialize;
use std::cmp::Ordering;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct GitHubReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<GitHubReleaseAsset>,
}

pub fn fetch_latest_release(repo: &str) -> Result<GitHubRelease> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("Failed to build HTTP client")?;

    let release = client
        .get(url)
        .header(reqwest::header::USER_AGENT, "ardiex-updater")
        .send()
        .context("Failed to request latest GitHub release")?
        .error_for_status()
        .context("GitHub release endpoint returned error status")?
        .json::<GitHubRelease>()
        .context("Failed to parse latest GitHub release response")?;

    Ok(release)
}

pub fn normalize_version(input: &str) -> String {
    let without_prefix = input.trim().trim_start_matches(['v', 'V']);
    let core = without_prefix
        .split('-')
        .next()
        .unwrap_or(without_prefix)
        .split('+')
        .next()
        .unwrap_or(without_prefix);
    core.to_string()
}

fn parse_semver_triplet(input: &str) -> Option<(u64, u64, u64)> {
    let normalized = normalize_version(input);
    let mut parts = normalized.split('.');

    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;

    // Ignore extra components beyond MAJOR.MINOR.PATCH for strictness.
    if parts.next().is_some() {
        return None;
    }

    Some((major, minor, patch))
}

pub fn compare_versions(current: &str, candidate: &str) -> Ordering {
    match (
        parse_semver_triplet(current),
        parse_semver_triplet(candidate),
    ) {
        (Some(c), Some(n)) => c.cmp(&n),
        _ => normalize_version(current).cmp(&normalize_version(candidate)),
    }
}

pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    compare_versions(current, candidate) == Ordering::Less
}

pub fn expected_release_asset_name_for_current_target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("ardiex-linux-amd64.tar.gz"),
        ("linux", "aarch64") => Ok("ardiex-linux-arm64.tar.gz"),
        ("macos", "x86_64") => Ok("ardiex-macos-amd64.tar.gz"),
        ("macos", "aarch64") => Ok("ardiex-macos-arm64.tar.gz"),
        ("windows", "x86_64") => Ok("ardiex-windows-amd64.zip"),
        _ => Err(anyhow::anyhow!(
            "Unsupported target for auto-update: os={}, arch={}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )),
    }
}

pub fn find_release_asset_download_url(
    release: &GitHubRelease,
    asset_name: &str,
) -> Result<String> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Release asset '{}' not found in tag '{}' (assets={})",
                asset_name,
                release.tag_name,
                release.assets.len()
            )
        })
}

#[cfg(test)]
#[path = "tests/update_tests.rs"]
mod tests;
