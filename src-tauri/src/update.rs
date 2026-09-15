//! The in-app updater's backend: the feed per channel, the check, the
//! download + install with progress events. The updater plugin's own IPC
//! commands are not used — only this side can pick the endpoint per check,
//! and the channel is a setting (`Config::update_channel`).
//!
//! Feeds: the stable channel reads GitHub's `latest` release (never a
//! pre-release); the beta channel reads the rolling `beta-version` release,
//! whose `latest.json` CI points at the newest published release, beta or
//! stable (`.github/workflows/release.yml`). A beta is the same binary and
//! version number as the stable it may become: publishing it as a full
//! release promotes it, no rebuild.

use std::sync::Mutex;

use log::{info, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::config::UpdateChannel;

pub const STABLE_FEED: &str = "https://github.com/w00zla/BindSight/releases/latest/download/latest.json";
pub const BETA_FEED: &str = "https://github.com/w00zla/BindSight/releases/download/beta-version/latest.json";

/// The feed a channel reads.
pub fn feed_url(channel: UpdateChannel) -> &'static str {
    match channel {
        UpdateChannel::Stable => STABLE_FEED,
        UpdateChannel::Beta => BETA_FEED,
    }
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
    let info = found.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        date: u.date.map(|d| d.date().to_string()).unwrap_or_default(),
        notes: u.body.clone().unwrap_or_default(),
    });
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
        assert!(feed_url(UpdateChannel::Beta).ends_with("/releases/download/beta-version/latest.json"));
        assert_ne!(feed_url(UpdateChannel::Stable), feed_url(UpdateChannel::Beta));
        for url in [STABLE_FEED, BETA_FEED] {
            assert!(url.starts_with("https://"), "{url}");
        }
    }

    #[test]
    fn progress_serializes_with_a_kind_tag() {
        let json = serde_json::to_value(Progress::Download { downloaded: 5, total: Some(10) }).unwrap();
        assert_eq!(json["kind"], "download");
        assert_eq!(json["downloaded"], 5);
        assert_eq!(serde_json::to_value(Progress::Installing).unwrap()["kind"], "installing");
    }
}
