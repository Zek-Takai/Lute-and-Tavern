//! Visualizer shows: one per song, named by its song ID (made once by tools/show-analyzer). A song's show is
//! looked for in `_Software/Shows` in the library folder (made on this computer), then
//! in the Shows folder on the server (fetched the first time the song plays and kept in the app's data folder).
//! A song without one plays with live listening instead. The analyzer itself is not part of the app: Settings ›
//! Shows explains how to install what it needs (from its makers' own sites) and run it.

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
        for folder in [local_folder(&lib), dir.clone()] {
            if let Ok(text) = std::fs::read_to_string(folder.join(format!("{uid}.json"))) {
                return Ok(Some(text));
            }
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

// ---------- shows made on this computer ----------

/// Where everything that isn't a song lives: `_Software` in the library folder (left alone by the organizer,
/// song scans and backups), or the app's data folder when there's no library folder yet.
fn software_dir(lib: &lt_core::Library) -> std::path::PathBuf {
    match lib.library_folder() {
        Ok(Some(f)) => std::path::Path::new(&f.path).join(lt_core::organize::SOFTWARE),
        _ => lib.dirs.root.join(lt_core::organize::SOFTWARE),
    }
}

/// Where shows made on this computer go: `_Software/Shows`.
fn local_folder(lib: &lt_core::Library) -> std::path::PathBuf {
    software_dir(lib).join("Shows")
}

/// Where the show analyzer and its own Python environment go: `_Software/Show analyzer`.
fn analyzer_dir(lib: &lt_core::Library) -> std::path::PathBuf {
    software_dir(lib).join("Show analyzer")
}

fn has_show(lib: &lt_core::Library, uid: &str) -> bool {
    let name = format!("{uid}.json");
    [local_folder(lib), lib.dirs.root.join("shows")].iter().any(|d| d.join(&name).is_file())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowsInfo {
    folder: String,
    analyzer: String,
    analyzer_saved: bool,
    python_ready: bool,
    list_waiting: bool,
    shows: usize,
    songs: usize,
    without: usize,
}

/// The Shows and analyzer folders, how far the setup has got, and how many of this computer's songs have a show.
#[tauri::command]
pub async fn shows_info(state: State<'_, AppState>) -> Result<ShowsInfo, String> {
    let lib = state.lib.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<ShowsInfo> {
        let folder = local_folder(&lib);
        let analyzer = analyzer_dir(&lib);
        let shows = std::fs::read_dir(&folder)
            .map(|d| d.filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().strip_suffix(".json").is_some_and(valid)).count())
            .unwrap_or(0);
        let songs = lib.songs_with_audio()?;
        let without = songs.iter().filter(|(uid, _)| !has_show(&lib, uid)).count();
        let python = analyzer.join("python");
        Ok(ShowsInfo {
            folder: folder.to_string_lossy().into_owned(),
            analyzer: analyzer.to_string_lossy().into_owned(),
            analyzer_saved: analyzer.join("run.py").is_file(),
            python_ready: python.join("bin/python").is_file() || python.join("Scripts/python.exe").is_file(),
            list_waiting: std::fs::metadata(analyzer.join("worklist.tsv")).is_ok_and(|m| m.len() > 0),
            shows,
            songs: songs.len(),
            without,
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

/// Writes the analyzer's song list (`worklist.tsv`: song ID, tab, full path) into the analyzer folder: every
/// song on this computer that has no show yet. Returns how many songs are on it.
#[tauri::command]
pub async fn make_show_list(state: State<'_, AppState>) -> Result<usize, String> {
    let lib = state.lib.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<usize> {
        let mut text = String::new();
        let mut n = 0;
        for (uid, path) in lib.songs_with_audio()? {
            let p = path.to_string_lossy();
            if has_show(&lib, &uid) || p.contains(['\t', '\n', '\r']) {
                continue;
            }
            text.push_str(&format!("{uid}\t{p}\n"));
            n += 1;
        }
        let dir = analyzer_dir(&lib);
        std::fs::create_dir_all(&dir)?;
        std::fs::create_dir_all(local_folder(&lib))?;
        std::fs::write(dir.join("worklist.tsv"), text)?;
        Ok(n)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

/// The show analyzer's own files (our code; its open-source parts are installed from their makers' sites).
const ANALYZER_FILES: [(&str, &str); 6] = [
    ("README.md", include_str!("../../../../tools/show-analyzer/README.md")),
    ("analyze.py", include_str!("../../../../tools/show-analyzer/analyze.py")),
    ("run.py", include_str!("../../../../tools/show-analyzer/run.py")),
    ("requirements.txt", include_str!("../../../../tools/show-analyzer/requirements.txt")),
    ("setup.sh", include_str!("../../../../tools/show-analyzer/setup.sh")),
    ("setup.ps1", include_str!("../../../../tools/show-analyzer/setup.ps1")),
];

/// Saves (or refreshes) the analyzer's files in `_Software/Show analyzer` and makes `_Software/Shows`.
#[tauri::command]
pub async fn save_show_analyzer(state: State<'_, AppState>) -> Result<String, String> {
    let lib = state.lib.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
        let dir = analyzer_dir(&lib);
        std::fs::create_dir_all(&dir)?;
        std::fs::create_dir_all(local_folder(&lib))?;
        for (name, body) in ANALYZER_FILES {
            std::fs::write(dir.join(name), body)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("setup.sh"), std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(dir.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

/// Opens the Shows folder or the analyzer folder (made if missing) in Finder / Explorer / the file manager.
#[tauri::command]
pub async fn open_shows_folder(app: tauri::AppHandle, state: State<'_, AppState>, which: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let folder = if which == "analyzer" { analyzer_dir(&state.lib) } else { local_folder(&state.lib) };
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    app.opener().open_path(folder.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

/// The web pages Settings › Shows links to: each maker's own download page (always their newest release) and
/// project page. Only these can be opened from there.
const LINKS: [(&str, &str); 10] = [
    ("python-download", "https://www.python.org/downloads/"),
    ("python-site", "https://www.python.org/"),
    ("pytorch-download", "https://pytorch.org/get-started/locally/"),
    ("pytorch-site", "https://pytorch.org/"),
    ("demucs-download", "https://pypi.org/project/demucs/"),
    ("demucs-site", "https://github.com/facebookresearch/demucs"),
    ("librosa-download", "https://pypi.org/project/librosa/"),
    ("librosa-site", "https://librosa.org/"),
    ("ffmpeg-download", "https://pypi.org/project/imageio-ffmpeg/"),
    ("ffmpeg-site", "https://ffmpeg.org/"),
];

#[tauri::command]
pub async fn open_show_link(app: tauri::AppHandle, key: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let url = LINKS.iter().find(|(k, _)| *k == key).map(|(_, u)| *u).ok_or("Unknown link.")?;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}
