//! Hooks to the person's own sync app (lt_core::sync). After the library folder changes, the app tells
//! the sync app: Syncthing through its local API, or any tool through the person's command. Changes are
//! gathered for a few seconds so a burst of writes makes one call, and changes right after a command ran
//! are ignored so a sync tool's own downloads don't start another run.

use crate::AppState;
use lt_core::sync::{self, CommandRun, Conflict, SyncStatus, Syncthing};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};

const KEYCHAIN_SERVICE: &str = "Lute & Tavern";
const KEY_USER: &str = "syncthing-api-key";

#[derive(Default)]
pub struct Hook {
    pending: Mutex<(Vec<PathBuf>, bool)>,
    /// The person's sync command is running; changes meanwhile wait for one follow-up run.
    running: std::sync::atomic::AtomicBool,
    queued: Mutex<Option<Vec<PathBuf>>>,
    last_run: Mutex<Option<CommandRun>>,
    quiet_until: Mutex<Option<Instant>>,
}

fn key() -> Option<String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEY_USER).ok()?.get_password().ok()
}

fn kind(state: &AppState) -> String {
    state.lib.setting_str("syncKind").unwrap_or_else(|| "none".into())
}

fn library_root(state: &AppState) -> Option<PathBuf> {
    state.lib.library_folder().ok().flatten().map(|f| PathBuf::from(f.path))
}

fn syncthing_address(state: &AppState) -> String {
    state.lib.setting_str("syncthingAddress").unwrap_or_else(|| "http://127.0.0.1:8384".into())
}

/// Runs the hook for these changes now. Returns a message for the interface.
fn fire(app: &AppHandle, changed: Vec<PathBuf>) -> Result<String, String> {
    let state = app.state::<AppState>();
    let root = library_root(&state).ok_or("Choose a library folder first (Settings › Library folder).")?;
    match kind(&state).as_str() {
        "syncthing" => {
            let k = key().ok_or("Add Syncthing's API key first.")?;
            Syncthing { address: &syncthing_address(&state), api_key: &k }.nudge(&root, &changed).map_err(|e| format!("{e:#}"))?;
            Ok("Asked Syncthing to sync now".into())
        }
        "command" => {
            use std::sync::atomic::Ordering;
            let cmd = state.lib.setting_str("syncCommand").ok_or("Add the command to run first.")?;
            let hook = app.state::<Hook>();
            if hook.running.swap(true, Ordering::SeqCst) {
                hook.queued.lock().unwrap().get_or_insert_with(Vec::new).extend(changed);
                return Ok("A sync is already running; these changes go with the next one".into());
            }
            let mut changed = changed;
            let msg = loop {
                let run = sync::run_command(&cmd, &root, &changed, Duration::from_secs(30 * 60));
                let msg = if run.ok { "Sync command finished".to_string() } else { format!("Sync command failed: {}", run.output.lines().last().unwrap_or("")) };
                *hook.quiet_until.lock().unwrap() = Some(Instant::now() + Duration::from_secs(15));
                *hook.last_run.lock().unwrap() = Some(run);
                match hook.queued.lock().unwrap().take() {
                    Some(mut next) => {
                        next.sort();
                        next.dedup();
                        changed = next;
                    }
                    None => break msg,
                }
            };
            hook.running.store(false, Ordering::SeqCst);
            // changes that arrived as the last run ended
            let late = hook.queued.lock().unwrap().take();
            if let Some(next) = late {
                return fire(app, next);
            }
            Ok(msg)
        }
        "folder" => Ok("Your sync app picks up changes in this folder by itself".into()),
        _ => Err("Sync isn't set up".into()),
    }
}

/// Called by the folder watcher for changes inside the library folder.
pub fn changed(app: &AppHandle, paths: Vec<PathBuf>) {
    let state = app.state::<AppState>();
    let delay = match kind(&state).as_str() {
        "syncthing" => Duration::from_secs(5),
        "command" => Duration::from_secs(30),
        _ => return,
    };
    let hook = app.state::<Hook>();
    if hook.quiet_until.lock().unwrap().is_some_and(|t| Instant::now() < t) {
        return;
    }
    let mut p = hook.pending.lock().unwrap();
    p.0.extend(paths);
    if p.1 {
        return; // a run is already scheduled; these changes ride along
    }
    p.1 = true;
    drop(p);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let hook = app.state::<Hook>();
        let batch = {
            let mut p = hook.pending.lock().unwrap();
            p.1 = false;
            std::mem::take(&mut p.0)
        };
        let mut batch = batch;
        batch.sort();
        batch.dedup();
        if let Err(e) = fire(&app, batch) {
            crate::log::write("error", &format!("sync hook: {e}"));
        }
    });
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSettings {
    kind: String,
    address: String,
    command: String,
    has_key: bool,
}

#[tauri::command]
pub fn sync_settings(state: State<'_, AppState>) -> SyncSettings {
    SyncSettings { kind: kind(&state), address: syncthing_address(&state), command: state.lib.setting_str("syncCommand").unwrap_or_default(), has_key: key().is_some() }
}

/// `api_key`: None keeps the stored key, an empty string removes it.
#[tauri::command]
pub fn set_sync_settings(state: State<'_, AppState>, kind: String, address: String, command: String, api_key: Option<String>) -> Result<(), String> {
    if !["none", "folder", "syncthing", "command"].contains(&kind.as_str()) {
        return Err(format!("unknown sync kind {kind}"));
    }
    for (k, v) in [("syncKind", kind), ("syncthingAddress", address.trim().to_string()), ("syncCommand", command.trim().to_string())] {
        state.lib.set_setting(k, &serde_json::Value::String(v)).map_err(|e| e.to_string())?;
    }
    if let Some(k) = api_key {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEY_USER).map_err(|e| format!("The system keychain is not available: {e}"))?;
        if k.trim().is_empty() {
            let _ = entry.delete_credential();
        } else {
            entry.set_password(k.trim()).map_err(|e| format!("Could not save the key in the keychain: {e}{}", crate::secure::KEYRING_HINT))?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    status: Option<SyncStatus>,
    error: Option<String>,
    last_run: Option<CommandRun>,
    conflicts: Vec<Conflict>,
}

#[tauri::command]
pub async fn sync_status(app: AppHandle) -> Result<SyncReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let root = library_root(&state);
        let conflicts = root.as_deref().map(sync::conflicts).unwrap_or_default();
        let (mut status, mut error) = (None, None);
        if kind(&state) == "syncthing" {
            match (key(), root.as_deref()) {
                (Some(k), Some(r)) => match (Syncthing { address: &syncthing_address(&state), api_key: &k }).status(r) {
                    Ok(s) => status = Some(s),
                    Err(e) => error = Some(format!("{e:#}")),
                },
                (None, _) => error = Some("Add Syncthing's API key.".into()),
                (_, None) => error = Some("Choose a library folder first.".into()),
            }
        }
        SyncReport { status, error, last_run: app.state::<Hook>().last_run.lock().unwrap().clone(), conflicts }
    })
    .await
    .map_err(|e| e.to_string())
}

/// "Sync now": runs the hook right away for the whole library.
#[tauri::command]
pub async fn sync_now(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || fire(&app, vec![])).await.map_err(|e| e.to_string())?
}

/// Shows a conflict copy in Finder / Explorer so the person can decide which one to keep.
#[tauri::command]
pub fn sync_reveal(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().reveal_item_in_dir(&path).map_err(|e| e.to_string())
}
