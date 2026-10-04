//! Connections to music the person keeps elsewhere: a server on a device they own or a cloud server they
//! run. Each kind speaks its own protocol behind one trait (`MusicServer`):
//!   subsonic  Navidrome, Airsonic, Gonic, Ampache, Funkwhale, many NAS apps   (remote_subsonic.rs)
//!   plex      Plex Media Server, signed in through plex.tv                     (remote_plex.rs)
//!   jellyfin  Jellyfin and Emby                                                (remote_jellyfin.rs)
//!   webdav    plain folders over WebDAV: Nextcloud, NAS file sharing, rclone   (remote_webdav.rs)
//! A connection is only an address, so it works over the home network, the internet or a VPN alike.
//! Secrets (password or token) never come here to rest: the app keeps them in the system keychain.

use crate::library::Library;
use anyhow::{Result, anyhow, bail};
use rusqlite::params;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

pub const KINDS: &[&str] = &["subsonic", "plex", "jellyfin", "webdav"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub base_url: String,
    pub username: String,
    /// Non-secret details per kind (Plex server id and addresses, Jellyfin user id, inLibrary...)
    pub extra: Value,
}

/// A song, album, artist, playlist or folder on a connected server, in one shape for the interface.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteItem {
    pub id: String,
    /// "song", "album", "artist", "playlist" or "folder"
    pub kind: String,
    pub name: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    /// The server's cover art reference, if any (passed back to `MusicServer::cover`)
    pub cover: Option<String>,
    pub duration: Option<f64>,
    pub track: Option<i64>,
    pub count: Option<i64>,
    /// When the song was added on the server (Unix seconds), if it says
    pub added: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RemoteSearch {
    pub songs: Vec<RemoteItem>,
    pub albums: Vec<RemoteItem>,
    pub artists: Vec<RemoteItem>,
    pub playlists: Vec<RemoteItem>,
    /// Folders (WebDAV)
    pub folders: Vec<RemoteItem>,
}

/// An address to fetch, with the headers it needs (sign-in). Played through the app's relay so the
/// headers never reach the web page.
#[derive(Debug, Clone)]
pub struct Target {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl Target {
    pub fn plain(url: String) -> Self {
        Self { url, headers: vec![] }
    }
}

pub trait MusicServer {
    /// Checks the address and sign-in; returns a short description of the server.
    fn ping(&self) -> Result<String>;
    fn search(&self, q: &str) -> Result<RemoteSearch>;
    /// The server's front page: playlists and newest albums, or top folders.
    fn home(&self) -> Result<RemoteSearch>;
    /// An album's or playlist's songs, an artist's albums, a folder's contents.
    fn open(&self, kind: &str, id: &str) -> Result<Vec<RemoteItem>>;
    /// The songs to play for an item (an artist plays their albums' songs).
    fn songs_of(&self, kind: &str, id: &str) -> Result<Vec<RemoteItem>> {
        match kind {
            "artist" => {
                let mut songs = Vec::new();
                for a in self.open("artist", id)?.into_iter().take(12) {
                    songs.extend(self.open("album", &a.id)?.into_iter().filter(|s| s.kind == "song"));
                    if songs.len() >= 150 {
                        break;
                    }
                }
                Ok(songs)
            }
            _ => Ok(self.open(kind, id)?.into_iter().filter(|s| s.kind == "song").collect()),
        }
    }
    /// Every song on the server (for adding the server's music to My music).
    fn all_songs(&self) -> Result<Vec<RemoteItem>>;
    /// A cheap "what state is the library in" stamp: when it is unchanged since the last sync, nothing needs
    /// re-reading. None when the server can't say (then a sync always re-reads).
    fn library_stamp(&self) -> Result<Option<String>> {
        Ok(None)
    }
    /// The original audio of a song (so seeking works and quality is untouched).
    fn stream(&self, song_id: &str) -> Result<Target>;
    fn cover(&self, cover: &str, size: u32) -> Option<Target>;
}

/// Opens a connection with its secret (password or token) from the keychain.
pub fn server<'a>(conn: &'a Connection, secret: &'a str) -> Result<Box<dyn MusicServer + 'a>> {
    Ok(match conn.kind.as_str() {
        "subsonic" => Box::new(crate::remote_subsonic::Subsonic { conn, password: secret }),
        "plex" => Box::new(crate::remote_plex::Plex::new(conn, secret)),
        "jellyfin" => Box::new(crate::remote_jellyfin::Jellyfin { conn, token: secret }),
        "webdav" => Box::new(crate::remote_webdav::WebDav::new(conn, secret)),
        k => bail!("Unknown kind of connection: {k}"),
    })
}

impl Library {
    pub fn connections(&self) -> Result<Vec<Connection>> {
        let db = self.db();
        let mut st = db.prepare("SELECT id, kind, name, base_url, username, extra FROM connections ORDER BY name COLLATE NOCASE")?;
        let rows = st
            .query_map([], |r| {
                Ok(Connection {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    name: r.get(2)?,
                    base_url: r.get(3)?,
                    username: r.get(4)?,
                    extra: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(Value::Null),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn connection(&self, id: i64) -> Result<Connection> {
        self.connections()?.into_iter().find(|c| c.id == id).ok_or_else(|| anyhow!("No connection with id {id}"))
    }

    pub fn add_connection(&self, kind: &str, name: &str, base_url: &str, username: &str, extra: &Value) -> Result<Connection> {
        if !KINDS.contains(&kind) {
            bail!("Unknown kind of connection: {kind}");
        }
        let url = normalize_url(base_url)?;
        let name = if name.trim().is_empty() { url.clone() } else { name.trim().to_string() };
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let id = {
            let db = self.db();
            db.execute(
                "INSERT INTO connections(kind, name, base_url, username, created_at, extra) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![kind, name, url, username.trim(), t, extra.to_string()],
            )?;
            db.last_insert_rowid()
        };
        self.connection(id)
    }

    pub fn remove_connection(&self, id: i64) -> Result<()> {
        self.db().execute("DELETE FROM connections WHERE id = ?1", [id])?;
        Ok(())
    }
}

/// "nas.local:4533" → "http://nas.local:4533"; trailing slashes and a pasted "/rest" are dropped.
pub fn normalize_url(raw: &str) -> Result<String> {
    let mut u = raw.trim().trim_end_matches('/').to_string();
    if u.is_empty() {
        bail!("Add the server's address");
    }
    if !u.contains("://") {
        u = format!("http://{u}");
    }
    if let Some(stripped) = u.strip_suffix("/rest") {
        u = stripped.to_string();
    }
    reqwest::Url::parse(&u).map_err(|e| anyhow!("That address doesn't look right: {e}"))?;
    Ok(u)
}

/// Downloads a target (a cover image) with its sign-in headers.
pub fn fetch(t: &Target) -> Result<Vec<u8>> {
    let mut req = http()?.get(&t.url);
    for (k, v) in &t.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let res = req.send()?.error_for_status()?;
    Ok(res.bytes()?.to_vec())
}

impl Library {
    /// Changes one non-secret detail of a connection (e.g. `inLibrary`).
    pub fn set_connection_extra(&self, id: i64, key: &str, value: Value) -> Result<Connection> {
        let mut c = self.connection(id)?;
        if !c.extra.is_object() {
            c.extra = serde_json::json!({});
        }
        c.extra[key] = value;
        self.db().execute("UPDATE connections SET extra = ?2 WHERE id = ?1", params![id, c.extra.to_string()])?;
        Ok(c)
    }
}

pub(crate) fn http() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(concat!("LuteAndTavern/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?)
}

/// GET returning JSON, with headers; errors name the server.
pub(crate) fn get_json(url: &str, headers: &[(String, String)], who: &str) -> Result<Value> {
    let mut req = http()?.get(url).header("Accept", "application/json");
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let res = req.send().map_err(|e| anyhow!("Could not reach {who}: {}", e.without_url()))?;
    let status = res.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        bail!("{who} refused the sign-in ({status}). Reconnect it in Settings.");
    }
    if !status.is_success() {
        bail!("{who} answered {status}");
    }
    res.json().map_err(|_| anyhow!("{who} sent something that isn't the expected data"))
}

/// The API returns one object instead of a list when there is a single result (Subsonic), or nothing.
pub(crate) fn list(v: &Value, f: impl Fn(&Value) -> RemoteItem) -> Vec<RemoteItem> {
    match v {
        Value::Array(a) => a.iter().map(f).collect(),
        Value::Object(_) => vec![f(v)],
        _ => vec![],
    }
}

/// "2026-09-29T09:12:33.000Z" (or a date with an offset) → Unix seconds, read as UTC.
pub(crate) fn iso_time(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d, hh, mm, ss) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    // days from civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hh * 3600 + mm * 60 + ss)
}

pub(crate) fn text(v: &Value, k: &str) -> String {
    match &v[k] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_dates_read_as_utc() {
        assert_eq!(iso_time("1970-01-02T00:00:00Z"), Some(86400));
        assert_eq!(iso_time("2026-09-29T09:12:33.000Z"), Some(1790673153));
        assert_eq!(iso_time("nope"), None);
    }

    #[test]
    fn addresses_are_normalized() {
        assert_eq!(normalize_url("nas.local:4533/").unwrap(), "http://nas.local:4533");
        assert_eq!(normalize_url("https://music.example.com/rest").unwrap(), "https://music.example.com");
        assert!(normalize_url("  ").is_err());
    }

    #[test]
    fn connections_keep_their_details() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(d.path()).unwrap();
        let c = l.add_connection("plex", "Home", "http://nas:32400", "", &serde_json::json!({"machine": "abc"})).unwrap();
        assert_eq!((c.kind.as_str(), c.extra["machine"].as_str()), ("plex", Some("abc")));
        assert!(l.add_connection("ftp", "x", "http://x", "", &Value::Null).is_err());
        l.remove_connection(c.id).unwrap();
        assert!(l.connections().unwrap().is_empty());
    }
}
