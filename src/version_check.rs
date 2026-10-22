// RaftCLI: tell the user when a newer RaftCLI has been published
// Rob Dobson 2026
//
// The latest version on crates.io is looked up at most once a day (using curl, so no HTTP/TLS crate is
// needed) and remembered in the user's cache folder. Any failure (offline, no curl) is silent.
// Set RAFT_NO_UPDATE_CHECK=1 to turn the check off.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
const CRATES_IO_URL: &str = "https://crates.io/api/v1/crates/raftcli";

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct VersionCache {
    checked_at: u64,
    latest: Option<String>,
}

fn cache_file() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("raftcli").join("latest_version.json"))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// Ask crates.io for the latest stable version
fn query_latest() -> Option<String> {
    let user_agent = format!("raftcli/{} ({})", env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_REPOSITORY"));
    let output = Command::new("curl")
        .args(["-s", "-f", "-L", "--max-time", "3", "-A", &user_agent, CRATES_IO_URL])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    json["crate"]["max_stable_version"].as_str().map(|s| s.to_string())
}

// The latest version, from the cache if it was checked recently
fn latest_version() -> Option<String> {
    let path = cache_file()?;
    let cache: VersionCache = std::fs::read_to_string(&path).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let now = now_secs();
    if now.saturating_sub(cache.checked_at) < CHECK_INTERVAL_SECS {
        return cache.latest;
    }
    // A failed check keeps the previous answer and isn't retried until the next interval
    let latest = query_latest().or(cache.latest);
    let new_cache = VersionCache { checked_at: now, latest: latest.clone() };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string(&new_cache) {
        let _ = std::fs::write(&path, s);
    }
    latest
}

fn parse_version(version: &str) -> Vec<u64> {
    version.trim().trim_start_matches('v')
        .split(|c| c == '-' || c == '+').next().unwrap_or("")
        .split('.').map(|part| part.parse().unwrap_or(0)).collect()
}

fn is_newer(latest: &str, current: &str) -> bool {
    parse_version(latest) > parse_version(current)
}

/// Print a note if a newer RaftCLI is available
pub fn notify_if_newer() {
    if std::env::var("RAFT_NO_UPDATE_CHECK").map(|v| !v.is_empty() && v != "0" && v != "false").unwrap_or(false) {
        return;
    }
    let current = env!("CARGO_PKG_VERSION");
    if let Some(latest) = latest_version() {
        if is_newer(&latest, current) {
            println!("Note: RaftCLI {} is available (this is {}) - update with: cargo install raftcli", latest, current);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        assert!(is_newer("1.25.1", "1.24.1"));
        assert!(is_newer("1.25.10", "1.25.9"));
        assert!(is_newer("2.0.0", "1.99.99"));
        assert!(!is_newer("1.25.1", "1.25.1"));
        assert!(!is_newer("1.24.1", "1.25.1"));
        assert!(!is_newer("1.25.1-beta.1", "1.25.1"));
    }
}
