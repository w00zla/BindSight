//! The in-app updater's backend: the feed per channel, the check, the
//! download + install with progress events. The updater plugin's own IPC
//! commands are not used — only this side can pick the endpoint per check,
//! and the channel is a setting (`Config::update_channel`).
//!
//! Feeds: the stable channel reads GitHub's `latest` release (never a
//! pre-release); the prerelease channel reads the rolling
//! `prerelease-version` release, whose `latest.json` CI points at the newest
//! published release, pre-release or stable (`.github/workflows/release.yml`).
//! A pre-release is the same binary and version number as the stable it may
//! become: publishing it as a full release promotes it, no rebuild.
//!
//! An install without the updater (bare executable, deb / rpm) only checks:
//! the feed's version against its own, the frontend links the project page.

use std::sync::Mutex;

use log::{info, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::config::UpdateChannel;

pub const STABLE_FEED: &str = "https://github.com/w00zla/BindSight/releases/latest/download/latest.json";
pub const PRERELEASE_FEED: &str = "https://github.com/w00zla/BindSight/releases/download/prerelease-version/latest.json";
const GITHUB_REPO: &str = "w00zla/BindSight";

/// The feed a channel reads.
pub fn feed_url(channel: UpdateChannel) -> &'static str {
    match channel {
        UpdateChannel::Stable => STABLE_FEED,
        UpdateChannel::Prerelease => PRERELEASE_FEED,
    }
}

/// The GitHub release body (the changelog) for a version's `v<version>` tag,
/// as plain text. Best-effort: any failure yields an empty string, because
/// release notes are cosmetic and must never fail the update check. A
/// pre-release lives under its real version tag too, so this works on both
/// channels.
async fn fetch_release_notes(version: &str) -> String {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/tags/v{version}");
    match release_body(&url).await {
        Ok(body) => body,
        Err(e) => {
            warn!("release notes fetch ({url}) failed: {e}");
            String::new()
        }
    }
}

async fn release_body(url: &str) -> Result<String, String> {
    let json = get_json(url).await?;
    Ok(json.get("body").and_then(|b| b.as_str()).unwrap_or_default().trim().to_string())
}

/// GET a JSON document from GitHub.
async fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        // GitHub rejects requests without a User-Agent.
        .user_agent(concat!("BindSight/", env!("CARGO_PKG_VERSION")))
        // Never let a slow server stall the update check.
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Whether the feed's version is newer than the running one.
fn is_newer(feed: &str, current: &str) -> Result<bool, String> {
    let parse = |v: &str| semver::Version::parse(v.trim().trim_start_matches('v')).map_err(|e| format!("version {v:?}: {e}"));
    Ok(parse(feed)? > parse(current)?)
}

/// An install without the updater (bare executable, deb / rpm): read the
/// channel's feed by hand and compare versions; the frontend links the
/// project page, nothing is downloaded.
async fn check_feed(url: &str) -> Result<Option<UpdateInfo>, String> {
    let feed = get_json(url).await?;
    let version = feed.get("version").and_then(|v| v.as_str()).ok_or("feed without a version")?;
    if !is_newer(version, env!("CARGO_PKG_VERSION"))? {
        return Ok(None);
    }
    let version = version.trim().trim_start_matches('v').to_string();
    let gh = fetch_release_notes(&version).await;
    let str_of = |k: &str| feed.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    Ok(Some(UpdateInfo {
        date: str_of("pub_date").get(..10).unwrap_or_default().to_string(),
        notes: if gh.is_empty() { str_of("notes") } else { gh },
        version,
    }))
}

/// The update the last check found, kept for `install_update`.
#[derive(Default)]
pub struct UpdateState(Mutex<Option<Update>>);

/// What the frontend shows about a found update.
#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    /// `YYYY-MM-DD`, empty when the feed has none.
    pub date: String,
    pub notes: String,
}

/// `update-progress` event payload.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Progress {
    Download { downloaded: u64, total: Option<u64> },
    Installing,
}

/// Check the channel's feed; the found update (if any) is kept for
/// `install_update`.
#[tauri::command]
pub async fn check_update(
    channel: UpdateChannel,
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<Option<UpdateInfo>, String> {
    let url = feed_url(channel);
    if !crate::updater_available() {
        let info = check_feed(url).await.map_err(|e| {
            warn!("update check ({channel:?}) failed: {e}");
            e
        })?;
        match &info {
            Some(i) => info!("update check ({channel:?}): v{} available", i.version),
            None => info!("update check ({channel:?}): up to date"),
        }
        return Ok(info);
    }
    let updater = app
        .updater_builder()
        .endpoints(vec![url.parse().map_err(|e| format!("{url}: {e}"))?])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| {
        warn!("update check ({channel:?}) failed: {e}");
        e.to_string()
    })?;
    let info = if let Some(u) = found.as_ref() {
        // Prefer the GitHub release's changelog; fall back to the feed's own
        // `notes` when it cannot be fetched.
        let gh = fetch_release_notes(&u.version).await;
        Some(UpdateInfo {
            version: u.version.clone(),
            date: u.date.map(|d| d.date().to_string()).unwrap_or_default(),
            notes: if gh.is_empty() { u.body.clone().unwrap_or_default() } else { gh },
        })
    } else {
        None
    };
    match &info {
        Some(i) => info!("update check ({channel:?}): v{} available", i.version),
        None => info!("update check ({channel:?}): up to date"),
    }
    *state.0.lock().unwrap() = found;
    Ok(info)
}

/// Download and install the update the last check found, reporting
/// `update-progress`, then restart. On Windows the installer takes over and
/// ends the process itself.
#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, UpdateState>) -> Result<(), String> {
    if !crate::updater_available() {
        return Err("no updater in this install".into());
    }
    let update = state.0.lock().unwrap().clone().ok_or("no update checked")?;
    info!("update v{} downloading", update.version);
    let mut downloaded: u64 = 0;
    let on_chunk = app.clone();
    let on_done = app.clone();
    update
        .download_and_install(
            |chunk, total| {
                downloaded += chunk as u64;
                let _ = on_chunk.emit("update-progress", Progress::Download { downloaded, total });
            },
            || {
                let _ = on_done.emit("update-progress", Progress::Installing);
            },
        )
        .await
        .map_err(|e| {
            warn!("update install failed: {e}");
            e.to_string()
        })?;
    info!("update v{} installed, restarting", update.version);
    app.restart()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feeds_per_channel() {
        assert!(feed_url(UpdateChannel::Stable).ends_with("/releases/latest/download/latest.json"));
        assert!(feed_url(UpdateChannel::Prerelease).ends_with("/releases/download/prerelease-version/latest.json"));
        assert_ne!(feed_url(UpdateChannel::Stable), feed_url(UpdateChannel::Prerelease));
        for url in [STABLE_FEED, PRERELEASE_FEED] {
            assert!(url.starts_with("https://"), "{url}");
        }
    }

    #[test]
    fn newer_versions() {
        assert_eq!(is_newer("0.17.0", "0.16.0"), Ok(true));
        assert_eq!(is_newer("v1.0.0", "0.16.9"), Ok(true));
        assert_eq!(is_newer("0.16.0", "0.16.0"), Ok(false));
        assert_eq!(is_newer("0.15.2", "0.16.0"), Ok(false));
        assert!(is_newer("latest", "0.16.0").is_err());
        assert!(is_newer("", "0.16.0").is_err());
    }

    #[test]
    fn progress_serializes_with_a_kind_tag() {
        let json = serde_json::to_value(Progress::Download { downloaded: 5, total: Some(10) }).unwrap();
        assert_eq!(json["kind"], "download");
        assert_eq!(json["downloaded"], 5);
        assert_eq!(serde_json::to_value(Progress::Installing).unwrap()["kind"], "installing");
    }
}
