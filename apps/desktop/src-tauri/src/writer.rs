//! Songwriter commands. The endpoint and model live in settings; the API key lives in the system keychain.
//! Writing runs in the background and reports back with `song-updated` or `song-failed` events.

use crate::AppState;
use lt_core::Song;
use lt_core::songwriter::{self, WriterConfig};
use serde::Serialize;
use tauri::{Emitter, Manager, State};

const KEYCHAIN_SERVICE: &str = "Lute & Tavern";
const KEYCHAIN_USER: &str = "songwriter-api-key";
pub const DEFAULT_NAME: &str = "Bard";

fn api_key() -> Option<String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER).ok()?.get_password().ok()
}

fn config(state: &AppState) -> WriterConfig {
    let l = &state.lib;
    WriterConfig {
        base_url: l.setting_str("writerBaseUrl").unwrap_or_default(),
        model: l.setting_str("writerModel").unwrap_or_default(),
        name: l.setting_str("writerName").unwrap_or_else(|| DEFAULT_NAME.into()),
        api_key: api_key(),
    }
}

fn tags(state: &AppState) -> Vec<String> {
    state.lib.genres().map(|g| g.into_iter().map(|g| g.name).collect()).unwrap_or_default()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterSettings {
    base_url: String,
    model: String,
    name: String,
    has_key: bool,
}

#[tauri::command]
pub async fn writer_config(state: State<'_, AppState>) -> Result<WriterSettings, String> {
    let c = config(&state);
    Ok(WriterSettings { has_key: c.api_key.is_some(), base_url: c.base_url, model: c.model, name: c.name })
}

/// `api_key`: None keeps the stored key, an empty string removes it.
#[tauri::command]
pub async fn set_writer_config(
    state: State<'_, AppState>,
    base_url: String,
    model: String,
    name: String,
    api_key: Option<String>,
) -> Result<(), String> {
    let l = &state.lib;
    let name = if name.trim().is_empty() { DEFAULT_NAME.to_string() } else { name.trim().to_string() };
    let old_name = config(&state).name;
    for (k, v) in [("writerBaseUrl", base_url.trim().to_string()), ("writerModel", model.trim().to_string()), ("writerName", name.clone())] {
        l.set_setting(k, &serde_json::Value::String(v)).map_err(|e| e.to_string())?;
    }
    l.rename_writer(&old_name, &name).map_err(|e| e.to_string())?;
    if let Some(key) = api_key {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER).map_err(|e| format!("The system keychain is not available: {e}"))?;
        if key.trim().is_empty() {
            let _ = entry.delete_credential();
        } else {
            entry.set_password(key.trim()).map_err(|e| format!("Could not save the key in the keychain: {e}"))?;
        }
    }
    Ok(())
}

/// Checks the connection and returns the models the server offers.
#[tauri::command]
pub async fn writer_models(state: State<'_, AppState>, base_url: String, api_key: Option<String>) -> Result<Vec<String>, String> {
    let mut cfg = config(&state);
    cfg.base_url = base_url;
    if let Some(k) = api_key.filter(|k| !k.trim().is_empty()) {
        cfg.api_key = Some(k);
    }
    tauri::async_runtime::spawn_blocking(move || songwriter::list_models(&cfg)).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}"))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Failed {
    id: i64,
    message: String,
}

/// Creates the song right away (status "writing") and fills it in when the songwriter answers.
#[tauri::command]
pub async fn request_song(app: tauri::AppHandle, state: State<'_, AppState>, request: String, genre: Option<String>) -> Result<Song, String> {
    let cfg = config(&state);
    if !cfg.is_ready() {
        return Err("Set up the songwriter in Settings first: add its address and pick a model.".into());
    }
    let song = state.lib.create_request(&request, genre.as_deref(), &cfg.name).map_err(|e| e.to_string())?;
    let id = song.id;
    let keep_genre = genre.as_deref().is_some_and(|g| !g.trim().is_empty());
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let result = songwriter::write_song(&cfg, &request, genre.as_deref(), &tags(&state))
            .and_then(|w| state.lib.apply_draft(id, &w.draft, "First draft", keep_genre).map(|_| w.reply));
        finish(&app, &state, id, result);
    });
    Ok(song)
}

/// Sends feedback to the songwriter; the rewrite is saved as a new version.
#[tauri::command]
pub async fn rewrite_song(app: tauri::AppHandle, state: State<'_, AppState>, id: i64, instruction: String) -> Result<(), String> {
    let cfg = config(&state);
    if !cfg.is_ready() {
        return Err("Set up the songwriter in Settings first.".into());
    }
    let lib = &state.lib;
    let history = lib.messages(id).map_err(|e| e.to_string())?;
    lib.add_message(id, "user", &instruction).map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let result = state.lib.song(id).and_then(|song| {
            let w = songwriter::rewrite(&cfg, &song, &history, &instruction, &tags(&state))?;
            let note: String = instruction.chars().take(80).collect();
            state.lib.apply_draft(id, &w.draft, &note, true).map(|_| w.reply)
        });
        finish(&app, &state, id, result);
    });
    Ok(())
}

fn finish(app: &tauri::AppHandle, state: &AppState, id: i64, result: anyhow::Result<String>) {
    match result {
        Ok(reply) => {
            let reply = if reply.is_empty() { "Done. The new version is on the Song tab.".to_string() } else { reply };
            let _ = state.lib.add_message(id, "writer", &reply);
            let _ = app.emit("song-updated", id);
        }
        Err(e) => {
            let message = format!("{e:#}");
            let _ = state.lib.add_message(id, "writer", &format!("I couldn't finish that: {message}"));
            let _ = app.emit("song-failed", Failed { id, message });
        }
    }
}
