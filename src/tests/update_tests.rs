use super::*;
use std::cmp::Ordering;

#[test]
fn normalize_version_strips_prefix_and_prerelease() {
    assert_eq!(normalize_version("v1.2.3"), "1.2.3");
    assert_eq!(normalize_version("V2.0.1"), "2.0.1");
    assert_eq!(normalize_version("1.2.3-beta.1"), "1.2.3");
    assert_eq!(normalize_version("1.2.3+build.5"), "1.2.3");
}

#[test]
fn compare_versions_orders_semver_triplets() {
    assert_eq!(compare_versions("1.2.3", "1.2.3"), Ordering::Equal);
    assert_eq!(compare_versions("1.2.3", "1.2.4"), Ordering::Less);
    assert_eq!(compare_versions("1.10.0", "1.2.0"), Ordering::Greater);
}

#[test]
fn is_newer_version_detects_candidate_newer() {
    assert!(is_newer_version("0.1.0", "0.2.0"));
    assert!(!is_newer_version("0.2.0", "0.2.0"));
    assert!(!is_newer_version("0.3.0", "0.2.9"));
}

#[test]
fn find_release_asset_download_url_returns_error_for_missing_asset() {
    let release = GitHubRelease {
        tag_name: "v1.0.0".to_string(),
        assets: vec![GitHubReleaseAsset {
            name: "ardiex-linux-amd64.tar.gz".to_string(),
            browser_download_url: "https://example.invalid/asset".to_string(),
        }],
    };

    let err = find_release_asset_download_url(&release, "ardiex-windows-amd64.zip")
        .expect_err("missing asset must return error");
    assert!(err.to_string().contains("Release asset"));
}
