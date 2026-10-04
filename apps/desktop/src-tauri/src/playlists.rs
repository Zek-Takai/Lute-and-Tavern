//! Playlists across devices and playlist import. Playlists sync through the backup connection (Settings ›
//! Backup): shortly after any playlist change on this device (from the app, the assistant or an AI app), at
//! launch, and every ten minutes to pick up changes made elsewhere.

use crate::AppState;
use lt_core::playlist_io::ImportResult;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Syncs playlists now if a backup connection is set. Returns false when there is nothing to sync with.
pub fn sync_now(app: &AppHandle) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let Some(conn_id) = state.lib.settings().ok().and_then(|m| m.get("backupConn").and_then(Value::as_i64)) else {
        return Ok(false);
    };
    if state.lib.settings().ok().and_then(|m| m.get("playlistSync").and_then(Value::as_bool)) == Some(false) {
        return Ok(false);
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(true);
    }
    let folder = state.lib.setting_str("backupFolder").unwrap_or_else(|| "Music/Lute & Tavern".into());
    let result = (|| -> anyhow::Result<lt_core::playlist_sync::PlaylistSync> {
        let conn = state.lib.connection(conn_id)?;
        let secret = keyring::Entry::new("Lute & Tavern", &format!("connection-{conn_id}"))?.get_password()?;
        let dav = lt_core::remote_webdav::WebDav::new(&conn, &secret);
        lt_core::playlist_sync::sync(&state.lib, &dav, &folder)
    })();
    RUNNING.store(false, Ordering::SeqCst);
    match result {
        Ok(r) => {
            let _ = state.lib.set_setting("playlistSyncLast", &json!({"at": chrono::Utc::now().timestamp(), "ok": true, "report": r}));
            if r.received + r.deleted_here > 0 {
                crate::emit_library_changed(app);
            }
            Ok(true)
        }
        Err(e) => {
            let msg = format!("Playlist sync failed: {e:#}");
            crate::log::write("error", &msg);
            let _ = state.lib.set_setting("playlistSyncLast", &json!({"at": chrono::Utc::now().timestamp(), "ok": false, "message": msg}));
            Err(msg)
        }
    }
}

/// Watches for playlist changes (a cheap stamp, checked every few seconds) and syncs once they settle.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(10));
        let stamp = |app: &AppHandle| lt_core::playlist_sync::stamp(&app.state::<AppState>().lib).unwrap_or_default();
        let _ = sync_now(&app);
        let mut synced = stamp(&app);
        let mut last_sync = Instant::now();
        let mut seen = synced.clone();
        let mut seen_at = Instant::now();
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let now = stamp(&app);
            if now != seen {
                seen = now.clone();
                seen_at = Instant::now();
            }
            let settled_change = now != synced && seen_at.elapsed() >= Duration::from_secs(15);
            if settled_change || last_sync.elapsed() >= Duration::from_secs(600) {
                let _ = sync_now(&app);
                synced = stamp(&app);
                seen = synced.clone();
                last_sync = Instant::now();
            }
        }
    });
}

#[tauri::command]
pub async fn sync_playlists(app: AppHandle) -> Result<Value, String> {
    let a = app.clone();
    let ran = tauri::async_runtime::spawn_blocking(move || sync_now(&a)).await.map_err(|e| e.to_string())??;
    let state = app.state::<AppState>();
    Ok(json!({"ran": ran, "last": state.lib.settings().ok().and_then(|m| m.get("playlistSyncLast").cloned())}))
}

#[tauri::command]
pub async fn import_playlists(app: AppHandle, state: State<'_, AppState>, files: Vec<String>) -> Result<Vec<ImportResult>, String> {
    let files: Vec<PathBuf> = files.into_iter().map(PathBuf::from).collect();
    let lib = state.lib.clone();
    let r = tauri::async_runtime::spawn_blocking(move || lib.import_playlists(&files)).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}"))?;
    crate::emit_library_changed(&app);
    Ok(r)
}
