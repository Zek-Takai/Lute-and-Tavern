//! Visualizer shows: one per song, named by its song ID, kept in the Shows folder inside the backup folder on
//! the server (made once by tools/show-analyzer). A song's show is fetched the first time it plays and kept in
//! the app's own data folder after that. A song without one plays with live listening instead.

use crate::AppState;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::State;

/// Songs asked about recently that have no show on the server, so they aren't asked about on every play.
static MISSES: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);

fn valid(uid: &str) -> bool {
    uid.len() == 42 && uid.starts_with("LT-") && uid[3..].chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
}

/// Where song IDs and shows come from: the WebDAV connection chosen for them (Settings › Sync & backup),
/// else the backup connection, else the first WebDAV connection; and the folder holding `Shows/`.
pub fn source(lib: &lt_core::Library) -> anyhow::Result<Option<(lt_core::remote::Connection, String, Vec<String>)>> {
    let settings = lib.settings()?;
    let chosen = ["showsConn", "backupConn"].iter().find_map(|k| settings.get(*k).and_then(Value::as_i64));
    let conn = match chosen {
        Some(id) => lib.connection(id).ok().filter(|c| c.kind == "webdav"),
        None => None,
    }
    .or_else(|| lib.connections().ok()?.into_iter().find(|c| c.kind == "webdav"));
    let Some(conn) = conn else { return Ok(None) };
    let folder = lib.setting_str("showsFolder").or_else(|| lib.setting_str("backupFolder")).unwrap_or_else(|| "Music/Lute & Tavern".into());
    let secret = keyring::Entry::new("Lute & Tavern", &format!("connection-{}", conn.id))?.get_password()?;
    let parts = folder.split('/').map(str::trim).filter(|p| !p.is_empty()).map(String::from).collect();
    Ok(Some((conn, secret, parts)))
}

/// A file from the server's Shows folder, or nothing if it isn't there (or there's nowhere to look).
pub fn shows_file(lib: &lt_core::Library, name: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let Some((conn, secret, folder)) = source(lib)? else { return Ok(None) };
    let dav = lt_core::remote_webdav::WebDav::new(&conn, &secret);
    let mut parts: Vec<&str> = folder.iter().map(String::as_str).collect();
    parts.extend(["Shows", name]);
    dav.get_bytes(&dav.url_for(&parts, false))
}

fn fetch(lib: &lt_core::Library, uid: &str) -> anyhow::Result<Option<Vec<u8>>> {
    shows_file(lib, &format!("{uid}.json"))
}

/// Gives server songs their song IDs from the server's list now (also done after every server sync).
pub fn map_song_ids(lib: &lt_core::Library) -> anyhow::Result<usize> {
    let Some(bytes) = shows_file(lib, "song-ids.tsv")? else { return Ok(0) };
    lib.map_remote_uids(&String::from_utf8_lossy(&bytes))
}

#[tauri::command]
pub async fn refresh_song_ids(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<usize, String> {
    let lib = state.lib.clone();
    let n = tauri::async_runtime::spawn_blocking(move || map_song_ids(&lib)).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}"))?;
    // a song that just got its ID can now find its show
    MISSES.lock().unwrap().take();
    if n > 0 {
        crate::emit_library_changed(&app);
    }
    Ok(n)
}

/// The show for a song (by song ID) as JSON text, or nothing if it has none (yet).
#[tauri::command]
pub async fn song_show(state: State<'_, AppState>, uid: String) -> Result<Option<String>, String> {
    if !valid(&uid) {
        return Ok(None);
    }
    let lib = state.lib.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dir = lib.dirs.root.join("shows");
        let path = dir.join(format!("{uid}.json"));
        if let Ok(text) = std::fs::read_to_string(&path) {
            return Ok(Some(text));
        }
        {
            let mut m = MISSES.lock().unwrap();
            if m.get_or_insert_with(HashMap::new).get(&uid).is_some_and(|t| t.elapsed() < RETRY_AFTER) {
                return Ok(None);
            }
        }
        match fetch(&lib, &uid) {
            Ok(Some(bytes)) if serde_json::from_slice::<Value>(&bytes).is_ok() => {
                let _ = std::fs::create_dir_all(&dir);
                let tmp = path.with_extension("part");
                if std::fs::write(&tmp, &bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
                Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
            }
            Ok(_) => {
                MISSES.lock().unwrap().get_or_insert_with(HashMap::new).insert(uid, Instant::now());
                Ok(None)
            }
            // offline or the server is busy: live listening this time, ask again next play
            Err(e) => {
                crate::log::write("warn", &format!("show for {uid}: {e:#}"));
                Ok(None)
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
