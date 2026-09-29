//! Lute & Tavern desktop app: Tauri commands over lt-core, the `lt://` file protocol and the music-folder watcher.

mod log;
mod mcp;
mod musickit;
mod protocol;
mod secure;
mod updates;
mod watch;
mod writer;

use lt_core::library::{Folder, GenreTag, Playlist, ScanReport, Sheet, Take};
use lt_core::{Library, Song, SongDetail, SongPatch};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};

pub struct AppState {
    pub lib: Arc<Library>,
    pub watcher: watch::Watcher,
}

type Res<T> = Result<T, String>;

/// Runs library work off the UI thread and turns errors into messages the interface can show.
async fn blocking<T, F>(state: &State<'_, AppState>, f: F) -> Res<T>
where
    T: Send + 'static,
    F: FnOnce(&Library) -> anyhow::Result<T> + Send + 'static,
{
    let lib = state.lib.clone();
    tauri::async_runtime::spawn_blocking(move || f(&lib)).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    songs: Vec<Song>,
    playlists: Vec<Playlist>,
    genres: Vec<GenreTag>,
    folders: Vec<Folder>,
    settings: serde_json::Map<String, serde_json::Value>,
}

#[tauri::command]
async fn library(state: State<'_, AppState>) -> Res<Snapshot> {
    blocking(&state, |l| {
        Ok(Snapshot { songs: l.songs()?, playlists: l.playlists()?, genres: l.genres()?, folders: l.folders()?, settings: l.settings()? })
    })
    .await
}

#[tauri::command]
async fn song_detail(state: State<'_, AppState>, id: i64) -> Res<SongDetail> {
    blocking(&state, move |l| l.song_detail(id)).await
}

#[tauri::command]
async fn update_song(state: State<'_, AppState>, id: i64, patch: SongPatch) -> Res<Song> {
    blocking(&state, move |l| l.update_song(id, &patch)).await
}

#[tauri::command]
async fn set_art(state: State<'_, AppState>, id: i64, path: String) -> Res<Song> {
    blocking(&state, move |l| l.set_art_from_file(id, &PathBuf::from(path))).await
}

#[tauri::command]
async fn clear_art(state: State<'_, AppState>, id: i64) -> Res<Song> {
    blocking(&state, move |l| l.clear_art(id)).await
}

#[tauri::command]
async fn save_sheet_music(state: State<'_, AppState>, id: i64, notes: serde_json::Value, chords: Vec<String>) -> Res<()> {
    blocking(&state, move |l| l.save_sheet_music(id, &notes, &chords)).await
}

#[tauri::command]
async fn delete_song(state: State<'_, AppState>, id: i64) -> Res<()> {
    blocking(&state, move |l| l.delete_song(id)).await
}

// ---------- music folders ----------

#[tauri::command]
async fn add_folder(app: tauri::AppHandle, state: State<'_, AppState>, path: String) -> Res<ScanReport> {
    let report = blocking(&state, move |l| {
        let f = l.add_folder(&PathBuf::from(path))?;
        l.scan_folder(&f)
    })
    .await?;
    state.watcher.refresh(&app);
    Ok(report)
}

#[tauri::command]
async fn remove_folder(app: tauri::AppHandle, state: State<'_, AppState>, id: i64) -> Res<()> {
    blocking(&state, move |l| l.remove_folder(id)).await?;
    state.watcher.refresh(&app);
    Ok(())
}

#[tauri::command]
async fn rescan(state: State<'_, AppState>) -> Res<ScanReport> {
    blocking(&state, |l| {
        let mut total = ScanReport::default();
        for f in l.folders()? {
            let r = l.scan_folder(&f)?;
            total.added += r.added;
            total.updated += r.updated;
            total.removed += r.removed;
            total.untagged += r.untagged;
        }
        Ok(total)
    })
    .await
}

/// Files and folders dropped on the window or picked by hand. Folders become watched music folders.
#[tauri::command]
async fn import_paths(app: tauri::AppHandle, state: State<'_, AppState>, paths: Vec<String>) -> Res<ScanReport> {
    let (report, added_folder) = blocking(&state, move |l| {
        let mut total = ScanReport::default();
        let mut added_folder = false;
        for p in paths.iter().map(PathBuf::from) {
            if p.is_dir() {
                let f = l.add_folder(&p)?;
                let r = l.scan_folder(&f)?;
                total.added += r.added;
                total.updated += r.updated;
                total.untagged += r.untagged;
                added_folder = true;
            } else if lt_core::tags::is_audio(&p) {
                let folder = l.folder_for(&p)?;
                match l.import_file(&p, folder.as_ref().map(|f| f.id), folder.as_ref().map(|f| std::path::Path::new(&f.path)))? {
                    lt_core::library::Imported::Added { untagged } => {
                        total.added += 1;
                        total.untagged += untagged as usize;
                    }
                    lt_core::library::Imported::Updated => total.updated += 1,
                    lt_core::library::Imported::Unchanged => {}
                }
            }
        }
        Ok((total, added_folder))
    })
    .await?;
    if added_folder {
        state.watcher.refresh(&app);
    }
    Ok(report)
}

/// Makes a folder the library: the app organizes it into song folders from now on.
#[tauri::command]
async fn set_library_folder(app: tauri::AppHandle, state: State<'_, AppState>, path: String) -> Res<ScanReport> {
    let report = blocking(&state, move |l| {
        let f = l.set_library_folder(&PathBuf::from(path))?;
        l.scan_folder(&f)
    })
    .await?;
    state.watcher.refresh(&app);
    Ok(report)
}

/// Songs dropped on the window or picked with "Add songs to my library": audio files move into the
/// library folder and get organized; folders are watched where they are.
#[tauri::command]
async fn add_to_library(app: tauri::AppHandle, state: State<'_, AppState>, paths: Vec<String>) -> Res<ScanReport> {
    let (report, watched) = blocking(&state, move |l| {
        let Some(lib) = l.library_folder()? else { anyhow::bail!("Choose a library folder in Settings first.") };
        let root = PathBuf::from(&lib.path);
        let mut watched = false;
        let mut total = ScanReport::default();
        for p in paths.iter().map(PathBuf::from) {
            if p.is_dir() {
                if !p.starts_with(&root) {
                    let f = l.add_folder(&p)?;
                    let r = l.scan_folder(&f)?;
                    total.added += r.added;
                    total.untagged += r.untagged;
                    watched = true;
                }
            } else if lt_core::tags::is_audio(&p) && !p.starts_with(&root) {
                let name = p.file_name().map(|n| n.to_os_string()).unwrap_or_default();
                lt_core::organize::move_file(&p, &root.join(name))?;
            }
        }
        // Freshly moved files count as settled: they were complete before the move.
        let r = l.scan_folder_now(&lib)?;
        total.added += r.added;
        total.updated += r.updated;
        total.untagged += r.untagged;
        Ok((total, watched))
    })
    .await?;
    if watched {
        state.watcher.refresh(&app);
    }
    Ok(report)
}

// ---------- playlists ----------

#[tauri::command]
async fn create_playlist(state: State<'_, AppState>, name: String, songs: Vec<i64>) -> Res<Playlist> {
    blocking(&state, move |l| l.create_playlist(&name, &songs)).await
}

#[tauri::command]
async fn rename_playlist(state: State<'_, AppState>, id: i64, name: String) -> Res<()> {
    blocking(&state, move |l| l.rename_playlist(id, &name)).await
}

#[tauri::command]
async fn delete_playlist(state: State<'_, AppState>, id: i64) -> Res<()> {
    blocking(&state, move |l| l.delete_playlist(id)).await
}

#[tauri::command]
async fn set_playlist_songs(state: State<'_, AppState>, id: i64, songs: Vec<i64>) -> Res<()> {
    blocking(&state, move |l| l.set_playlist_songs(id, &songs)).await
}

#[tauri::command]
async fn toggle_in_playlist(state: State<'_, AppState>, id: i64, song: i64, on: bool) -> Res<()> {
    blocking(&state, move |l| l.toggle_in_playlist(id, song, on)).await
}

// ---------- genre tags ----------

#[tauri::command]
async fn create_genre(state: State<'_, AppState>, name: String) -> Res<GenreTag> {
    blocking(&state, move |l| l.create_genre(&name)).await
}

#[tauri::command]
async fn rename_genre(state: State<'_, AppState>, id: i64, name: String) -> Res<()> {
    blocking(&state, move |l| l.rename_genre(id, &name)).await
}

#[tauri::command]
async fn delete_genre(state: State<'_, AppState>, id: i64) -> Res<()> {
    blocking(&state, move |l| l.delete_genre(id)).await
}

// ---------- settings ----------

#[tauri::command]
async fn set_setting(state: State<'_, AppState>, key: String, value: serde_json::Value) -> Res<()> {
    blocking(&state, move |l| l.set_setting(&key, &value)).await
}

// ---------- sheets and takes ----------

#[tauri::command]
async fn add_sheet(state: State<'_, AppState>, song: i64, path: String) -> Res<Sheet> {
    blocking(&state, move |l| l.add_sheet(song, &PathBuf::from(path))).await
}

#[tauri::command]
async fn remove_sheet(state: State<'_, AppState>, id: i64) -> Res<()> {
    blocking(&state, move |l| l.remove_sheet(id)).await
}

#[tauri::command]
async fn read_sheet_text(state: State<'_, AppState>, id: i64) -> Res<String> {
    blocking(&state, move |l| {
        let p = l.sheet_path(id)?.ok_or_else(|| anyhow::anyhow!("sheet not found"))?;
        Ok(std::fs::read_to_string(p)?)
    })
    .await
}

/// Writes text the person chose to save, such as a MusicXML export, to the path they picked.
#[tauri::command]
async fn export_text(path: String, contents: String) -> Res<()> {
    std::fs::write(&path, contents).map_err(|e| format!("Could not save {path}: {e}"))
}

#[tauri::command]
async fn add_take(state: State<'_, AppState>, song: i64, path: String, origin: String) -> Res<Take> {
    blocking(&state, move |l| l.add_take(song, &PathBuf::from(path), &origin)).await
}

#[tauri::command]
async fn set_master_take(state: State<'_, AppState>, song: i64, take: i64) -> Res<()> {
    blocking(&state, move |l| l.set_master_take(song, take)).await
}

#[tauri::command]
async fn set_status(state: State<'_, AppState>, song: i64, status: String) -> Res<()> {
    blocking(&state, move |l| l.set_status(song, &status)).await
}

pub fn emit_library_changed(app: &tauri::AppHandle) {
    let _ = app.emit("library-changed", ());
}

/// Port the installed app serves its interface on (only reachable from this computer).
const LOCALHOST_PORT: u16 = 17654;

/// The main window. Made in code so Apple's sign-in popup (opened by MusicKit) can open as a real
/// window that talks back to the app; any other popup is refused.
fn main_window(app: &tauri::App) -> tauri::Result<()> {
    let handle = app.handle().clone();
    // Apple's sign-in only answers pages served over http(s). Development already runs on the Vite server;
    // installed builds serve the interface from localhost (see LOCALHOST_PORT) instead of tauri://.
    let url = if cfg!(debug_assertions) {
        tauri::WebviewUrl::App("index.html".into())
    } else {
        tauri::WebviewUrl::External(format!("http://localhost:{LOCALHOST_PORT}").parse().expect("static url"))
    };
    tauri::WebviewWindowBuilder::new(app, "main", url)
        .title("Lute & Tavern")
        .inner_size(1320.0, 860.0)
        .min_inner_size(900.0, 600.0)
        .on_new_window(move |url, features| {
            let apple = url.host_str().is_some_and(|h| h == "apple.com" || h.ends_with(".apple.com"));
            if !apple {
                return tauri::webview::NewWindowResponse::Deny;
            }
            let label = format!("apple-signin-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
            match tauri::WebviewWindowBuilder::new(&handle, label, tauri::WebviewUrl::External("about:blank".parse().expect("static url")))
                .window_features(features)
                .title("Sign in to Apple Music")
                .build()
            {
                Ok(window) => tauri::webview::NewWindowResponse::Create { window },
                Err(_) => tauri::webview::NewWindowResponse::Deny,
            }
        })
        .build()?;
    Ok(())
}

/// Closes Apple's sign-in window once sign-in has finished (the page's own close request is not honored).
#[tauri::command]
fn close_signin_windows(app: tauri::AppHandle) {
    for (label, w) in app.webview_windows() {
        if label.starts_with("apple-signin-") {
            let _ = w.close();
        }
    }
}

/// Saves a song from a streaming service (Apple Music) into the library, or returns it if it is there.
#[tauri::command]
async fn save_external_song(state: State<'_, AppState>, song: lt_core::library::ExternalSong) -> Res<Song> {
    blocking(&state, move |l| l.save_external_song(&song)).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_localhost::Builder::new(LOCALHOST_PORT).build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::Pending::default())
        .manage(musickit::TokenCache::default())
        .manage(mcp::Running::default())
        .register_asynchronous_uri_scheme_protocol("lt", protocol::handle)
        .setup(|app| {
            main_window(app)?;
            let dir = app.path().app_data_dir()?;
            log::init(&dir);
            let lib = Arc::new(Library::open(&dir)?);
            app.manage(AppState { lib, watcher: watch::Watcher::default() });
            if app.state::<AppState>().lib.setting_bool("mcpEnabled") {
                if let Err(e) = mcp::start(app.handle()) {
                    log::write("error", &format!("MCP server: {e}"));
                }
            }
            let handle = app.handle().clone();
            // Catch up on anything that changed while the app was closed, then start watching.
            tauri::async_runtime::spawn_blocking(move || {
                let state = handle.state::<AppState>();
                let mut changed = false;
                for f in state.lib.folders().unwrap_or_default() {
                    if let Ok(r) = state.lib.scan_folder(&f) {
                        changed |= r.added + r.updated + r.removed > 0;
                    }
                }
                state.watcher.refresh(&handle);
                if changed {
                    emit_library_changed(&handle);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            library,
            song_detail,
            update_song,
            set_art,
            clear_art,
            save_sheet_music,
            delete_song,
            add_folder,
            remove_folder,
            rescan,
            import_paths,
            set_library_folder,
            add_to_library,
            create_playlist,
            rename_playlist,
            delete_playlist,
            set_playlist_songs,
            toggle_in_playlist,
            create_genre,
            rename_genre,
            delete_genre,
            set_setting,
            add_sheet,
            remove_sheet,
            read_sheet_text,
            export_text,
            add_take,
            set_master_take,
            set_status,
            writer::writer_config,
            writer::set_writer_config,
            writer::writer_models,
            writer::request_song,
            writer::rewrite_song,
            log::log_event,
            secure::secure_get,
            secure::secure_set,
            secure::secure_delete,
            secure::clear_web_data,
            musickit::musickit_status,
            musickit::musickit_set_key,
            musickit::musickit_forget_key,
            musickit::musickit_token,
            save_external_song,
            close_signin_windows,
            updates::update_info,
            updates::check_update,
            updates::install_update,
            mcp::mcp_info,
            mcp::mcp_set_enabled,
            mcp::mcp_token,
            mcp::mcp_new_token,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Lute & Tavern");
}
