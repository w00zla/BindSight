//! The in-app updater's backend: the channels, the feed per channel, the
//! check, the download + install with progress events. The updater plugin's
//! own IPC commands are not used — only this side can pick the endpoint per
//! check, and the channel is a setting (`Config::update_channel`).
//!
//! Channels: `stable` is built in and reads GitHub's `latest` release (never
//! a pre-release). The other channels are listed in `channels.json` on the
//! repo's main branch, read at every check: each names its newest version's
//! tag (`v0.18.0-joysticks.2`, the channel id in the semver pre-release),
//! whose release carries the feed. CI moves a channel's tag when a channel
//! version is published; a channel is removed by hand once it is merged, its
//! testers then fall back to stable and get the final (`0.18.0` >
//! `0.18.0-joysticks.2`). No channel chosen = the running build's own
//! channel, else stable.
//!
//! Only a newer version is offered — except right after the user switched
//! channels: then any other version is, so a switch back from a channel can
//! reach the lower stable.
//!
//! An install without the updater (bare executable, deb / rpm) only checks:
//! the feed's version against its own, the frontend links the project page.

use std::sync::Mutex;

use log::{info, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

pub const STABLE: &str = "stable";
pub const STABLE_FEED: &str = "https://github.com/w00zla/BindSight/releases/latest/download/latest.json";
const CHANNELS_URL: &str = "https://raw.githubusercontent.com/w00zla/BindSight/main/channels.json";
const GITHUB_REPO: &str = "w00zla/BindSight";

/// A channel the update dialog offers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Channel {
    pub id: String,
    /// The tag of the channel's newest version; `None` for stable.
    #[serde(skip)]
    pub tag: Option<String>,
}

impl Channel {
    fn stable() -> Self {
        Channel { id: STABLE.into(), tag: None }
    }

    /// The `latest.json` this channel reads.
    fn feed(&self) -> String {
        match &self.tag {
            Some(tag) => format!("https://github.com/{GITHUB_REPO}/releases/download/{tag}/latest.json"),
            None => STABLE_FEED.into(),
        }
    }
}

/// A channel id: what a semver pre-release identifier and a branch
/// name `channel/<id>` both take, and nothing that needs escaping in a URL.
pub fn valid_channel_id(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() <= 32
        && id != STABLE
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The channel a version belongs to: `0.18.0-joysticks.2` -> `joysticks`,
/// `None` for a live version or anything that is not `x.y.z-<id>.<n>`.
pub fn channel_of(version: &str) -> Option<String> {
    let v = semver::Version::parse(version.trim().trim_start_matches('v')).ok()?;
    let (id, n) = v.pre.as_str().rsplit_once('.')?;
    (valid_channel_id(id) && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && v.build.is_empty()).then(|| id.to_string())
}

/// The channels of a `channels.json`, stable first. An entry whose id
/// is not a channel id or whose tag is not `v<x.y.z>-<id>.<n>` is dropped
/// (logged); a repeated id keeps its first entry.
pub fn parse_channels(json: &serde_json::Value) -> Vec<Channel> {
    let mut out = vec![Channel::stable()];
    for entry in json.get("channels").and_then(|c| c.as_array()).into_iter().flatten() {
        let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let tag = entry.get("tag").and_then(|v| v.as_str()).unwrap_or_default();
        if !valid_channel_id(id) || !tag.starts_with('v') || channel_of(tag).as_deref() != Some(id) {
            warn!("channels.json: entry {entry} skipped (id or tag invalid)");
            continue;
        }
        if out.iter().any(|c| c.id == id) {
            continue;
        }
        out.push(Channel { id: id.into(), tag: Some(tag.into()) });
    }
    out
}

/// The channel a check reads: the chosen one while `channels.json` lists
/// it, else the running build's own channel if listed, else stable.
pub fn resolve<'a>(chosen: &str, channels: &'a [Channel], build: Option<&str>) -> &'a Channel {
    let find = |id: &str| channels.iter().find(|c| c.id == id);
    find(chosen).or_else(|| build.and_then(find)).or_else(|| find(STABLE)).unwrap_or(&channels[0])
}

/// `channels.json` from the repo; no file yet = stable only.
async fn fetch_channels() -> Result<Vec<Channel>, String> {
    match get_json(CHANNELS_URL).await {
        Ok(json) => Ok(parse_channels(&json)),
        Err(e) if e.starts_with("HTTP 404") => Ok(vec![Channel::stable()]),
        Err(e) => Err(format!("channel list: {e}")),
    }
}

/// The channels the dialog offers and the one in effect.
#[derive(Debug, Serialize)]
pub struct ChannelList {
    pub channels: Vec<Channel>,
    pub current: String,
}

/// The channels to pick from, with the one a check would read now. Without
/// the list (offline) only stable and the chosen id come back.
#[tauri::command]
pub(crate) async fn update_channels(data: State<'_, Mutex<crate::AppData>>) -> Result<ChannelList, String> {
    let chosen = data.lock().unwrap().config.update_channel.clone();
    let channels = fetch_channels().await?;
    let current = resolve(&chosen, &channels, channel_of(env!("CARGO_PKG_VERSION")).as_deref()).id.clone();
    Ok(ChannelList { channels, current })
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

/// Whether the feed's version is offered: a newer one, or right after a
/// channel switch any other one.
fn is_offered(feed: &str, current: &str, switched: bool) -> Result<bool, String> {
    let parse = |v: &str| semver::Version::parse(v.trim().trim_start_matches('v')).map_err(|e| format!("version {v:?}: {e}"));
    let (feed, current) = (parse(feed)?, parse(current)?);
    Ok(if switched { feed != current } else { feed > current })
}

/// An install without the updater (bare executable, deb / rpm): read the
/// channel's feed by hand and compare versions; the frontend links the
/// project page, nothing is downloaded.
async fn check_feed(url: &str, switched: bool) -> Result<Option<UpdateInfo>, String> {
    let feed = get_json(url).await?;
    let version = feed.get("version").and_then(|v| v.as_str()).ok_or("feed without a version")?;
    if !is_offered(version, env!("CARGO_PKG_VERSION"), switched)? {
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

/// Check the feed of the channel in effect (see [`resolve`]); `switched`:
/// the user just picked another channel, so any other version is offered.
/// The found update (if any) is kept for `install_update`.
#[tauri::command]
pub(crate) async fn check_update(
    switched: bool,
    app: AppHandle,
    state: State<'_, UpdateState>,
    data: State<'_, Mutex<crate::AppData>>,
) -> Result<Option<UpdateInfo>, String> {
    let chosen = data.lock().unwrap().config.update_channel.clone();
    let build = channel_of(env!("CARGO_PKG_VERSION"));
    // Only a channel needs the list; stable works without it (offline, no file).
    let channels = if chosen == STABLE || (chosen.is_empty() && build.is_none()) {
        vec![Channel::stable()]
    } else {
        fetch_channels().await.map_err(|e| {
            warn!("update check failed: {e}");
            e
        })?
    };
    let channel = resolve(&chosen, &channels, build.as_deref());
    let id = &channel.id;
    let url = channel.feed();
    if !crate::updater_available() {
        let info = check_feed(&url, switched).await.map_err(|e| {
            warn!("update check ({id}) failed: {e}");
            e
        })?;
        match &info {
            Some(i) => info!("update check ({id}): v{} available", i.version),
            None => info!("update check ({id}): up to date"),
        }
        return Ok(info);
    }
    let updater = app
        .updater_builder()
        .endpoints(vec![url.parse().map_err(|e| format!("{url}: {e}"))?])
        .map_err(|e| e.to_string())?
        .version_comparator(move |current, remote| if switched { remote.version != current } else { remote.version > current })
        .build()
        .map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| {
        warn!("update check ({id}) failed: {e}");
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
        Some(i) => info!("update check ({id}): v{} available", i.version),
        None => info!("update check ({id}): up to date"),
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
    fn channel_ids_and_versions() {
        assert!(valid_channel_id("joysticks"));
        assert!(valid_channel_id("identical-joysticks2"));
        for bad in ["", "stable", "Beta", "1st", "a/b", "a.b", "a_b", &"x".repeat(33)] {
            assert!(!valid_channel_id(bad), "{bad}");
        }
        assert_eq!(channel_of("0.18.0-joysticks.2").as_deref(), Some("joysticks"));
        assert_eq!(channel_of("v0.18.0-identical-joysticks.12").as_deref(), Some("identical-joysticks"));
        for none in ["0.17.0", "0.18.0-joysticks", "0.18.0-beta.x", "0.18.0-Beta.1", "0.18.0-joysticks.1+b", "nope"] {
            assert_eq!(channel_of(none), None, "{none}");
        }
    }

    #[test]
    fn channels_json_is_validated() {
        let json = serde_json::json!({ "channels": [
            { "id": "joysticks", "tag": "v0.18.0-joysticks.2" },
            { "id": "pads", "tag": "v0.18.0-pads.1" },
            { "id": "joysticks", "tag": "v0.18.0-joysticks.9" },
            { "id": "stable", "tag": "v0.18.0-stable.1" },
            { "id": "evil", "tag": "v0.18.0-other.1" },
            { "id": "evil", "tag": "../../x" },
            { "id": "notag" },
            "junk"
        ]});
        let got = parse_channels(&json);
        let ids: Vec<&str> = got.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["stable", "joysticks", "pads"]);
        assert_eq!(got[1].feed(), "https://github.com/w00zla/BindSight/releases/download/v0.18.0-joysticks.2/latest.json");
        assert_eq!(got[0].feed(), STABLE_FEED);
        // Anything else: stable only.
        assert_eq!(parse_channels(&serde_json::json!([1, 2])), [Channel::stable()]);
    }

    #[test]
    fn resolve_prefers_choice_then_build_then_stable() {
        let list = parse_channels(&serde_json::json!({ "channels": [{ "id": "joysticks", "tag": "v0.18.0-joysticks.2" }] }));
        assert_eq!(resolve("joysticks", &list, None).id, "joysticks");
        assert_eq!(resolve("stable", &list, Some("joysticks")).id, "stable");
        // Nothing chosen, or a channel that is gone: the build's own, else stable.
        assert_eq!(resolve("", &list, Some("joysticks")).id, "joysticks");
        assert_eq!(resolve("gone", &list, Some("joysticks")).id, "joysticks");
        assert_eq!(resolve("gone", &list, Some("pads")).id, "stable");
        assert_eq!(resolve("", &list, None).id, "stable");
    }

    #[test]
    fn offered_versions() {
        assert_eq!(is_offered("0.17.0", "0.16.0", false), Ok(true));
        assert_eq!(is_offered("v1.0.0", "0.16.9", false), Ok(true));
        assert_eq!(is_offered("0.16.0", "0.16.0", false), Ok(false));
        assert_eq!(is_offered("0.15.2", "0.16.0", false), Ok(false));
        // A channel version is below its final, and above the live line it builds on.
        assert_eq!(is_offered("0.18.0", "0.18.0-joysticks.2", false), Ok(true));
        assert_eq!(is_offered("0.18.0-joysticks.3", "0.18.0-joysticks.2", false), Ok(true));
        assert_eq!(is_offered("0.17.4", "0.18.0-joysticks.2", false), Ok(false));
        // Right after a switch any other version, never the same one.
        assert_eq!(is_offered("0.17.4", "0.18.0-joysticks.2", true), Ok(true));
        assert_eq!(is_offered("0.17.4", "0.17.4", true), Ok(false));
        assert!(is_offered("latest", "0.16.0", false).is_err());
        assert!(is_offered("", "0.16.0", false).is_err());
    }

    #[test]
    fn progress_serializes_with_a_kind_tag() {
        let json = serde_json::to_value(Progress::Download { downloaded: 5, total: Some(10) }).unwrap();
        assert_eq!(json["kind"], "download");
        assert_eq!(json["downloaded"], 5);
        assert_eq!(serde_json::to_value(Progress::Installing).unwrap()["kind"], "installing");
    }
}
