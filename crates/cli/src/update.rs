#![forbid(unsafe_code)]

use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use cleanr_config::default_state_dir;
use reqwest::blocking::Client;
use semver::Version;
use serde::{Deserialize, Serialize};

const DEFAULT_VERSION_URL: &str =
    "https://github.com/drl990114/cleanr/releases/latest/download/install.json";
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub fn spawn_update_check(
    current_version: &'static str,
) -> Option<Receiver<cleanr_tui::UpdateNotice>> {
    spawn_check(move || check_for_update(current_version))
}

fn spawn_check(
    check: impl FnOnce() -> Option<UpdateAvailable> + Send + 'static,
) -> Option<Receiver<cleanr_tui::UpdateNotice>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("cleanr-update-check".into())
        .spawn(move || {
            if let Some(update) = check() {
                let _ = sender.send(cleanr_tui::UpdateNotice {
                    version: update.version,
                    release_url: update.release_url,
                });
            }
        })
        .ok()?;
    Some(receiver)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAvailable {
    pub version: String,
    pub release_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReleaseInfo {
    version: String,
    #[serde(default)]
    release_url: String,
}

impl ReleaseInfo {
    fn release_url(&self) -> String {
        if self.release_url.is_empty() {
            format!(
                "https://github.com/drl990114/cleanr/releases/tag/v{}",
                self.version.trim_start_matches('v')
            )
        } else {
            self.release_url.clone()
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateCache {
    checked_at: u64,
    release: Option<ReleaseInfo>,
}

pub fn check_for_update(current_version: &str) -> Option<UpdateAvailable> {
    let cache_path = cache_path();
    let now = unix_timestamp();
    let cached = read_cache(&cache_path);
    let ttl = cache_ttl();

    let release = if cached
        .as_ref()
        .is_some_and(|cache| now.saturating_sub(cache.checked_at) < ttl.as_secs())
    {
        cached.and_then(|cache| cache.release)
    } else {
        match fetch_release_info() {
            Ok(release) => {
                write_cache(
                    &cache_path,
                    &UpdateCache {
                        checked_at: now,
                        release: Some(release.clone()),
                    },
                );
                Some(release)
            }
            Err(()) => {
                let release = cached.and_then(|cache| cache.release);
                write_cache(
                    &cache_path,
                    &UpdateCache {
                        checked_at: now,
                        release: release.clone(),
                    },
                );
                release
            }
        }
    }?;

    newer_release(current_version, release)
}

fn newer_release(current_version: &str, release: ReleaseInfo) -> Option<UpdateAvailable> {
    let current = Version::parse(current_version.trim_start_matches('v')).ok()?;
    let latest = Version::parse(release.version.trim_start_matches('v')).ok()?;
    (latest > current).then_some(UpdateAvailable {
        version: latest.to_string(),
        release_url: release.release_url(),
    })
}

fn fetch_release_info() -> Result<ReleaseInfo, ()> {
    let url =
        std::env::var("CLEANR_VERSION_URL").unwrap_or_else(|_| DEFAULT_VERSION_URL.to_string());
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!("cleanr/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| ())?
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::json)
        .map_err(|_| ())
}

fn cache_path() -> PathBuf {
    std::env::var_os("CLEANR_UPDATE_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_state_dir().join("update-check.json"))
}

fn cache_ttl() -> Duration {
    std::env::var("CLEANR_UPDATE_CHECK_TTL_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_CACHE_TTL)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_cache(path: &PathBuf) -> Option<UpdateCache> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cache(path: &PathBuf, cache: &UpdateCache) {
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(raw) = serde_json::to_vec_pretty(cache) else {
        return;
    };
    let _ = fs::write(path, raw);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_check_returns_before_network_work_finishes() {
        let (release, wait) = mpsc::sync_channel(0);
        let receiver = spawn_check(move || {
            wait.recv().expect("release background check");
            Some(UpdateAvailable {
                version: "1.0.0".into(),
                release_url: "https://example.com/release".into(),
            })
        })
        .expect("worker");
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        release.send(()).expect("worker is waiting independently");
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("notice")
                .version,
            "1.0.0"
        );
    }

    #[test]
    fn reports_only_newer_semantic_versions() {
        let release = |version: &str| ReleaseInfo {
            version: version.to_string(),
            release_url: "https://example.com/release".to_string(),
        };

        assert!(newer_release("0.1.0", release("0.2.0")).is_some());
        assert!(newer_release("0.2.0", release("0.2.0")).is_none());
        assert!(newer_release("0.3.0", release("0.2.0")).is_none());
        assert!(newer_release("not-semver", release("0.2.0")).is_none());
    }

    #[test]
    fn accepts_v_prefix_and_builds_default_release_url() {
        let update = newer_release(
            "v0.1.0",
            ReleaseInfo {
                version: "v0.2.0".to_string(),
                release_url: String::new(),
            },
        )
        .expect("newer release");

        assert_eq!(update.version, "0.2.0");
        assert_eq!(
            update.release_url,
            "https://github.com/drl990114/cleanr/releases/tag/v0.2.0"
        );
    }

    #[test]
    fn cache_round_trips_and_invalid_cache_is_ignored() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("nested").join("cache.json");
        write_cache(
            &path,
            &UpdateCache {
                checked_at: 42,
                release: Some(ReleaseInfo {
                    version: "1.2.3".to_string(),
                    release_url: "https://example.com".to_string(),
                }),
            },
        );

        let cache = read_cache(&path).expect("cache");
        assert_eq!(cache.checked_at, 42);
        assert_eq!(cache.release.expect("release").version, "1.2.3");

        fs::write(&path, "invalid").expect("corrupt cache");
        assert!(read_cache(&path).is_none());
    }
}
