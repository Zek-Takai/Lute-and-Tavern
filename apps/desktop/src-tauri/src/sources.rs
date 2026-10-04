//! Commands for music from outside the library: internet radio (Radio Browser) and connections to the
//! person's own music servers (Subsonic API). Audio and cover art go through the local relay (relay.rs);
//! server passwords live in the system keychain under "connection-<id>".

use crate::{AppState, relay};
use lt_core::radio::{self, Station};
use lt_core::remote::{RemoteItem, RemoteSearch, Subsonic};
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
pub async fn connections_list(state: State<'_, AppState>) -> Res<Vec<lt_core::remote::Connection>> {
    state.lib.connections().map_err(|e| e.to_string())
}

/// Checks the server and sign-in first, then saves the connection (password into the keychain).
#[tauri::command]
pub async fn connection_add(state: State<'_, AppState>, kind: String, name: String, url: String, username: String, password: String) -> Res<String> {
    let lib = state.lib.clone();
    blocking(move || {
        let probe = lt_core::remote::Connection { id: 0, kind: kind.clone(), name: name.clone(), base_url: lt_core::remote::normalize_url(&url)?, username: username.clone() };
        let server = Subsonic { conn: &probe, password: &password }.ping()?;
        let c = lib.add_connection(&kind, &name, &url, &username)?;
        if let Err(e) = key_entry(c.id).and_then(|k| k.set_password(&password).map_err(|e| e.to_string())) {
            let _ = lib.remove_connection(c.id);
            anyhow::bail!("Could not save the password in the keychain: {e}");
        }
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

/// A connected item with its cover ready to show (through the relay).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    #[serde(flatten)]
    item: RemoteItem,
    art: Option<String>,
}

fn with_art(s: &Subsonic, items: Vec<RemoteItem>) -> Vec<Item> {
    items.into_iter().map(|i| Item { art: i.cover.as_deref().map(|c| relay::url_for(&s.cover_url(c, 300))), item: i }).collect()
}

/// Runs `f` against a connection with its password from the keychain.
fn with_conn<T>(lib: &lt_core::Library, id: i64, f: impl FnOnce(&Subsonic) -> anyhow::Result<T>) -> anyhow::Result<T> {
    let conn = lib.connection(id)?;
    let password = key_entry(id).and_then(|k| k.get_password().map_err(|e| e.to_string())).map_err(|e| anyhow::anyhow!("No saved password for {}: {e}", conn.name))?;
    f(&Subsonic { conn: &conn, password: &password })
}

#[derive(Serialize)]
pub struct RemoteResults {
    songs: Vec<Item>,
    albums: Vec<Item>,
    artists: Vec<Item>,
    playlists: Vec<Item>,
}

fn results(s: &Subsonic, r: RemoteSearch) -> RemoteResults {
    RemoteResults { songs: with_art(s, r.songs), albums: with_art(s, r.albums), artists: with_art(s, r.artists), playlists: with_art(s, r.playlists) }
}

#[tauri::command]
pub async fn remote_search(state: State<'_, AppState>, id: i64, query: String) -> Res<RemoteResults> {
    let lib = state.lib.clone();
    blocking(move || with_conn(&lib, id, |s| Ok(results(s, s.search(&query)?)))).await
}

/// A server's front page: its playlists and newest albums.
#[tauri::command]
pub async fn remote_home(state: State<'_, AppState>, id: i64) -> Res<RemoteResults> {
    let lib = state.lib.clone();
    blocking(move || {
        with_conn(&lib, id, |s| {
            Ok(results(s, RemoteSearch { playlists: s.playlists()?, albums: s.newest_albums(30)?, ..Default::default() }))
        })
    })
    .await
}

/// An album's or playlist's songs, or an artist's albums.
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
    blocking(move || with_conn(&lib, id, |s| Ok(relay::url_for(&s.stream_url(&song))))).await
}
