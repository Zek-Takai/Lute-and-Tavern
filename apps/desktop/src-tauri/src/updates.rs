//! In-app updates, off unless the person turns them on in Settings.
//!
//! Only official release builds can update themselves: `scripts/build-mac.sh --release` compiles in the
//! update-signing public key (LT_UPDATER_PUBKEY) and the default update address (LT_UPDATE_URL).
//! Everyday local builds carry neither, so the Updates switch explains that instead of failing.

use crate::AppState;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

const PUBKEY: Option<&str> = option_env!("LT_UPDATER_PUBKEY");
const DEFAULT_URL: Option<&str> = option_env!("LT_UPDATE_URL");

#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// This build was made to update itself (official release).
    can_update: bool,
    current_version: String,
    default_url: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    version: String,
    notes: String,
}

#[tauri::command]
pub fn update_info(app: tauri::AppHandle) -> UpdateInfo {
    UpdateInfo {
        can_update: PUBKEY.is_some_and(|k| !k.is_empty()),
        current_version: app.package_info().version.to_string(),
        default_url: DEFAULT_URL.unwrap_or_default().to_string(),
    }
}

fn update_url(state: &AppState) -> Option<String> {
    state.lib.setting_str("updateUrl").or_else(|| DEFAULT_URL.filter(|u| !u.is_empty()).map(String::from))
}

/// Asks the update address whether a newer version exists. Returns nothing when this is the newest.
#[tauri::command]
pub async fn check_update(app: tauri::AppHandle, state: State<'_, AppState>, pending: State<'_, Pending>) -> Result<Option<Available>, String> {
    let pubkey = PUBKEY.filter(|k| !k.is_empty()).ok_or("This copy was built on this Mac for everyday use and can't update itself. Official releases can.")?;
    let url = update_url(&state).ok_or("Add an update address in Settings first.")?;
    let endpoint = url.parse().map_err(|_| format!("\"{url}\" is not a web address"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| e.to_string())?
        .pubkey(pubkey)
        .build()
        .map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| format!("Could not check for updates: {e}"))?;
    let available = update.as_ref().map(|u| Available { version: u.version.clone(), notes: u.body.clone().unwrap_or_default() });
    *pending.0.lock().unwrap_or_else(|e| e.into_inner()) = update;
    Ok(available)
}

/// Downloads and installs the update found by `check_update`, then restarts the app.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle, pending: State<'_, Pending>) -> Result<(), String> {
    let update = pending.0.lock().unwrap_or_else(|e| e.into_inner()).take().ok_or("Check for updates first.")?;
    let mut done: u64 = 0;
    let progress = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                done += chunk as u64;
                let _ = progress.emit("update-progress", (done, total));
            },
            || {},
        )
        .await
        .map_err(|e| format!("The update could not be installed: {e}"))?;
    app.restart();
}
