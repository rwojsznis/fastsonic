//! Whether a newer release exists, from GitHub's releases.
//!
//! Most people never look at a releases page, so a build they downloaded
//! once is the one they keep, bugs included. Asking once a day and pointing
//! at the release page is cheap, and the only thing that leaves the machine
//! is the request itself.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/crmne/fastpotify/releases/latest";

/// How often a running app asks again.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// The version number, without a leading `v`.
    pub version: String,
    /// The release page, with every download.
    pub url: String,
}

#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
    html_url: String,
}

/// The newest release, when it is newer than this build.
pub async fn newer_release(http: &reqwest::Client) -> Result<Option<Release>> {
    let latest: LatestRelease = http
        .get(LATEST_RELEASE_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("unexpected release listing")?;
    let version = latest.tag_name.trim_start_matches('v').to_string();
    Ok(
        is_newer(&version, env!("CARGO_PKG_VERSION")).then_some(Release {
            version,
            url: latest.html_url,
        }),
    )
}

/// `major.minor.patch`; anything else (a pre-release tag, say) is `None`.
fn parse(version: &str) -> Option<[u64; 3]> {
    let mut parts = version
        .trim()
        .split('.')
        .map(|part| part.parse::<u64>().ok());
    Some([parts.next()??, parts.next()??, parts.next()??])
}

/// Whether `candidate` is a later version than `current`. Unparseable
/// versions are never newer, so a pre-release is never announced.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse(candidate), parse(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.1.4", "0.1.3"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("0.1.3", "0.1.3"));
        assert!(!is_newer("0.1.2", "0.1.3"));
        assert!(
            !is_newer("0.2.0-rc1", "0.1.3"),
            "pre-releases are not announced"
        );
        assert!(!is_newer("nightly", "0.1.3"));
    }
}
