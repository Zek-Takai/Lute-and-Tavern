//! Commands for music from outside the library: internet radio (Radio Browser) and connections to the
//! person's own music servers (Subsonic, Plex, Jellyfin/Emby, WebDAV). Audio and cover art go through the
//! local relay (relay.rs) so sign-in never reaches the page; each connection's password or token lives in
//! the system keychain under "connection-<id>".

use crate::{AppState, relay};
use lt_core::radio::{self, Station};
use lt_core::remote::{self, MusicServer, RemoteItem, RemoteSearch};
use lt_core::remote_plex::{self, PlexServer};
use std::sync::Mutex;
use serde::Serialize;
use tauri::State;

const KEYCHAIN_SERVICE: &str = "Lute & Tavern";

type Res<T> = Result<T, String>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}"))
}

fn key_entry(id: i64) -> Res<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, &format!("connection-{id}")).map_err(|e| format!("The system keychain is not available: {e}"))
}

// ---------- radio ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RadioHome {
    favorites: Vec<Station>,
    recent: Vec<Station>,
    top: Vec<Station>,
    tags: Vec<String>,
}

#[tauri::command]
pub async fn radio_search(state: State<'_, AppState>, name: String, tag: String, country: String, limit: Option<usize>) -> Res<Vec<Station>> {
    let lib = state.lib.clone();
    blocking(move || Ok(lib.mark_favorites(radio::search(&name, &tag, &country, limit.unwrap_or(60))?))).await
}

#[tauri::command]
pub async fn radio_home(state: State<'_, AppState>) -> Res<RadioHome> {
    let lib = state.lib.clone();
    blocking(move || {
        Ok(RadioHome {
            favorites: lib.favorite_stations()?,
            recent: lib.mark_favorites(lib.recent_stations(12)?),
            top: lib.mark_favorites(radio::top("", 30).unwrap_or_default()),
            tags: radio::top_tags(40).unwrap_or_default().into_iter().map(|(t, _)| t).collect(),
        })
    })
    .await
}

#[tauri::command]
pub async fn radio_favorite(state: State<'_, AppState>, station: Station, on: bool) -> Res<()> {
    state.lib.set_station_favorite(&station, on).map_err(|e| e.to_string())
}

/// Records a play (recently played, and the directory's click count) and returns the relay address to play.
#[tauri::command]
pub async fn radio_play(state: State<'_, AppState>, station: Station) -> Res<String> {
    state.lib.station_played(&station).map_err(|e| e.to_string())?;
    radio::count_click(&station.uuid);
    Ok(format!("{}?icy=1", relay::url_for(&station.url)))
}

// ---------- connections ----------

#[tauri::command]
pub async fn connections_list(state: State<'_, AppState>) -> Res<Vec<remote::Connection>> {
    state.lib.connections().map_err(|e| e.to_string())
}

fn save_secret(lib: &lt_core::Library, id: i64, secret: &str) -> anyhow::Result<()> {
    if let Err(e) = key_entry(id).and_then(|k| k.set_password(secret).map_err(|e| e.to_string())) {
        let _ = lib.remove_connection(id);
        anyhow::bail!("Could not save the sign-in in the keychain: {e}");
    }
    Ok(())
}

/// A stable id for this install, which Plex and Jellyfin show in their list of signed-in devices.
fn device_id(lib: &lt_core::Library) -> String {
    if let Some(id) = lib.setting_str("deviceId") {
        return id;
    }
    let mut b = [0u8; 16];
    let _ = getrandom::fill(&mut b);
    let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
    let _ = lib.set_setting("deviceId", &serde_json::Value::String(id.clone()));
    id
}

/// Subsonic, Jellyfin/Emby and WebDAV: checks the server and sign-in first, then saves the connection.
#[tauri::command]
pub async fn connection_add(state: State<'_, AppState>, kind: String, name: String, url: String, username: String, password: String) -> Res<String> {
    let lib = state.lib.clone();
    blocking(move || {
        let base = remote::normalize_url(&url)?;
        let (secret, extra, server) = match kind.as_str() {
            "jellyfin" => lt_core::remote_jellyfin::sign_in(&base, &username, &password, &device_id(&lib))?,
            "subsonic" | "webdav" => {
                let probe = remote::Connection { id: 0, kind: kind.clone(), name: name.clone(), base_url: base.clone(), username: username.clone(), extra: serde_json::Value::Null };
                let server = remote::server(&probe, &password)?.ping()?;
                (password.clone(), serde_json::json!({}), server)
            }
            "plex" => anyhow::bail!("Plex connects with the Sign in with Plex button"),
            k => anyhow::bail!("Unknown kind of connection: {k}"),
        };
        let c = lib.add_connection(&kind, &name, &base, &username, &extra)?;
        save_secret(&lib, c.id, &secret)?;
        Ok(format!("Connected to {server}"))
    })
    .await
}

#[tauri::command]
pub async fn connection_remove(state: State<'_, AppState>, id: i64) -> Res<()> {
    if let Ok(k) = key_entry(id) {
        let _ = k.delete_credential();
    }
    state.lib.remove_connection(id).map_err(|e| e.to_string())
}

// Plex sign-in: start (opens plex.tv in the browser), then the interface checks until the person approved,
// then picks one of the account's servers. The account token is only held in memory between those steps.
static PLEX_ACCOUNT: Mutex<Option<(String, Vec<PlexServer>)>> = Mutex::new(None);

#[tauri::command]
pub async fn plex_login_start(app: tauri::AppHandle, state: State<'_, AppState>) -> Res<i64> {
    let lib = state.lib.clone();
    let login = blocking(move || remote_plex::start_login(&device_id(&lib))).await?;
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(&login.auth_url, None::<&str>).map_err(|e| format!("Could not open the browser: {e}"))?;
    Ok(login.id)
}

/// None while waiting; the account's Plex servers once the person approved.
#[tauri::command]
pub async fn plex_login_check(state: State<'_, AppState>, pin: i64) -> Res<Option<Vec<PlexServer>>> {
    let lib = state.lib.clone();
    blocking(move || {
        let client = device_id(&lib);
        let Some(token) = remote_plex::check_login(&client, pin)? else { return Ok(None) };
        let servers = remote_plex::servers(&client, &token)?;
        *PLEX_ACCOUNT.lock().unwrap() = Some((token, servers.clone()));
        Ok(Some(servers))
    })
    .await
}

#[tauri::command]
pub async fn plex_connect(state: State<'_, AppState>, machine: String) -> Res<String> {
    let lib = state.lib.clone();
    blocking(move || {
        let server = PLEX_ACCOUNT
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|(_, list)| list.iter().find(|s| s.machine == machine).cloned())
            .ok_or_else(|| anyhow::anyhow!("Sign in with Plex again"))?;
        let extra = remote_plex::connection_extra(&server, &device_id(&lib));
        let probe = remote::Connection { id: 0, kind: "plex".into(), name: server.name.clone(), base_url: server.uris.first().cloned().unwrap_or_default(), username: String::new(), extra: extra.clone() };
        let about = remote::server(&probe, &server.token)?.ping()?;
        let c = lib.add_connection("plex", &server.name, &probe.base_url, "", &extra)?;
        save_secret(&lib, c.id, &server.token)?;
        *PLEX_ACCOUNT.lock().unwrap() = None;
        Ok(format!("Connected to {} ({about})", server.name))
    })
    .await
}

/// A connected item with its cover ready to show (through the relay).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    #[serde(flatten)]
    item: RemoteItem,
    art: Option<String>,
}

fn with_art(s: &dyn MusicServer, items: Vec<RemoteItem>) -> Vec<Item> {
    items.into_iter().map(|i| Item { art: i.cover.as_deref().and_then(|c| s.cover(c, 300)).map(relay::register), item: i }).collect()
}

/// Runs `f` against a connection with its password or token from the keychain.
fn with_conn<T>(lib: &lt_core::Library, id: i64, f: impl FnOnce(&dyn MusicServer) -> anyhow::Result<T>) -> anyhow::Result<T> {
    let conn = lib.connection(id)?;
    let secret = key_entry(id).and_then(|k| k.get_password().map_err(|e| e.to_string())).map_err(|e| anyhow::anyhow!("No saved sign-in for {}: {e}", conn.name))?;
    let server = remote::server(&conn, &secret)?;
    f(server.as_ref())
}

#[derive(Serialize)]
pub struct RemoteResults {
    songs: Vec<Item>,
    albums: Vec<Item>,
    artists: Vec<Item>,
    playlists: Vec<Item>,
    folders: Vec<Item>,
}

fn results(s: &dyn MusicServer, r: RemoteSearch) -> RemoteResults {
    RemoteResults { songs: with_art(s, r.songs), albums: with_art(s, r.albums), artists: with_art(s, r.artists), playlists: with_art(s, r.playlists), folders: with_art(s, r.folders) }
}

#[tauri::command]
pub async fn remote_search(state: State<'_, AppState>, id: i64, query: String) -> Res<RemoteResults> {
    let lib = state.lib.clone();
    blocking(move || with_conn(&lib, id, |s| Ok(results(s, s.search(&query)?)))).await
}

/// A server's front page: playlists and newest albums, or its top folders.
#[tauri::command]
pub async fn remote_home(state: State<'_, AppState>, id: i64) -> Res<RemoteResults> {
    let lib = state.lib.clone();
    blocking(move || with_conn(&lib, id, |s| Ok(results(s, s.home()?)))).await
}

/// An album's or playlist's songs, an artist's albums, a folder's contents.
#[tauri::command]
pub async fn remote_open(state: State<'_, AppState>, id: i64, kind: String, item: String) -> Res<Vec<Item>> {
    let lib = state.lib.clone();
    blocking(move || with_conn(&lib, id, |s| Ok(with_art(s, s.open(&kind, &item)?)))).await
}

/// The songs to play for an item (an artist plays their albums' songs).
#[tauri::command]
pub async fn remote_songs(state: State<'_, AppState>, id: i64, kind: String, item: String) -> Res<Vec<Item>> {
    let lib = state.lib.clone();
    blocking(move || with_conn(&lib, id, |s| Ok(with_art(s, if kind == "song" { vec![] } else { s.songs_of(&kind, &item)? })))).await
}

/// Relay address for a server song's audio.
#[tauri::command]
pub async fn remote_stream(state: State<'_, AppState>, id: i64, song: String) -> Res<String> {
    let lib = state.lib.clone();
    blocking(move || with_conn(&lib, id, |s| Ok(relay::register(s.stream(&song)?)))).await
}
