//! The in-app assistant (a chat with the person's own AI that can run the app), voice through any Whisper
//! server, and `luteandtavern://` links so outside voice assistants and shortcuts can open the app and ask it
//! for something. The chat and tools live in lt_core::assistant.

use crate::{writer, AppState};
use lt_core::assistant::{self, ChatLine, SpeechConfig};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

const KEYCHAIN_SERVICE: &str = "Lute & Tavern";
const SPEECH_KEY: &str = "speech-api-key";

/// A request that arrived by link before the interface was ready.
#[derive(Default)]
pub struct PendingAsk(Mutex<Option<String>>);

fn speech_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, SPEECH_KEY).map_err(|e| format!("The system keychain is not available: {e}"))
}

fn speech_config(state: &AppState) -> SpeechConfig {
    let l = &state.lib;
    SpeechConfig {
        base_url: l.setting_str("speechBaseUrl").unwrap_or_default(),
        model: l.setting_str("speechModel").unwrap_or_default(),
        api_key: speech_entry().ok().and_then(|e| e.get_password().ok()),
        language: l.setting_str("speechLanguage").unwrap_or_default(),
    }
}

#[tauri::command]
pub async fn assistant_history(state: State<'_, AppState>) -> Result<Vec<ChatLine>, String> {
    state.lib.assistant_history(200).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn assistant_clear(state: State<'_, AppState>) -> Result<(), String> {
    state.lib.clear_assistant().map_err(|e| e.to_string())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Answer {
    ok: bool,
    text: String,
}

/// Sends the person's words to their AI; it may act on the app. The answer arrives as `assistant-answer`.
#[tauri::command]
pub async fn assistant_send(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<(), String> {
    let cfg = writer::config(&state);
    if !cfg.is_ready() {
        return Err("Set up your AI in Settings first: add its address and pick a model.".into());
    }
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let cx = lt_core::mcp::Context {
            lib: &state.lib,
            writer_name: cfg.name.clone(),
            exclude: cfg.exclude,
            full: true,
            version: env!("CARGO_PKG_VERSION"),
        };
        let result = assistant::ask(&cfg, &cx, &text, &mut |fx| {
            if let Some(id) = fx.song {
                let _ = app.emit("song-updated", id);
            } else if fx.library {
                crate::emit_library_changed(&app);
            }
            if let Some(ui) = &fx.ui {
                let _ = app.emit("ai-control", ui.clone());
            }
        });
        let answer = match result {
            Ok(text) => Answer { ok: true, text },
            Err(e) => Answer { ok: false, text: format!("{e:#}") },
        };
        let _ = app.emit("assistant-answer", answer);
    });
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSettings {
    base_url: String,
    model: String,
    language: String,
    has_key: bool,
}

#[tauri::command]
pub fn speech_settings(state: State<'_, AppState>) -> SpeechSettings {
    let c = speech_config(&state);
    SpeechSettings { base_url: c.base_url, model: c.model, language: c.language, has_key: c.api_key.is_some() }
}

/// `api_key`: None keeps the stored key, an empty string removes it.
#[tauri::command]
pub fn set_speech_settings(state: State<'_, AppState>, base_url: String, model: String, language: String, api_key: Option<String>) -> Result<(), String> {
    for (k, v) in [("speechBaseUrl", base_url), ("speechModel", model), ("speechLanguage", language)] {
        state.lib.set_setting(k, &serde_json::Value::String(v.trim().to_string())).map_err(|e| e.to_string())?;
    }
    if let Some(key) = api_key {
        let entry = speech_entry()?;
        if key.trim().is_empty() {
            let _ = entry.delete_credential();
        } else {
            entry.set_password(key.trim()).map_err(|e| format!("Could not save the key in the keychain: {e}{}", crate::secure::KEYRING_HINT))?;
        }
    }
    Ok(())
}

/// Turns a recording into text through the Whisper server set in Settings.
#[tauri::command]
pub async fn transcribe(state: State<'_, AppState>, audio: Vec<u8>, mime: String) -> Result<String, String> {
    let cfg = speech_config(&state);
    let ext = if mime.contains("webm") { "webm" } else if mime.contains("ogg") { "ogg" } else if mime.contains("wav") { "wav" } else { "m4a" };
    tauri::async_runtime::spawn_blocking(move || assistant::transcribe(&cfg, audio, &format!("speech.{ext}"), &mime))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

/// `luteandtavern://ask?text=play%20my%20road%20trip%20mix` asks the assistant; `luteandtavern://open` just
/// brings the app forward. The app opens first if it was closed.
pub fn handle_link(app: &AppHandle, url: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    let Ok(u) = url::Url::parse(url) else { return };
    let action = u.host_str().unwrap_or("").to_string();
    if action != "ask" {
        return;
    }
    let Some(text) = u.query_pairs().find(|(k, _)| k == "text" || k == "q").map(|(_, v)| v.trim().to_string()).filter(|t| !t.is_empty()) else { return };
    crate::log::write("info", "assistant request by link");
    *app.state::<PendingAsk>().0.lock().unwrap() = Some(text.clone());
    let _ = app.emit("assistant-link", text);
}

/// The interface calls this once it is ready, to pick up a link that opened the app.
#[tauri::command]
pub fn take_pending_ask(pending: State<'_, PendingAsk>) -> Option<String> {
    pending.0.lock().unwrap().take()
}
