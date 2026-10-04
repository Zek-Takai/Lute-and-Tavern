//! Connections to music the person keeps elsewhere: a server on a device they own or a cloud server they run.
//! The first kind is the Subsonic API (OpenSubsonic), which Navidrome, Airsonic, Gonic, Ampache, Funkwhale and
//! many NAS music apps speak. A connection is only an address, so it works over the home network, the internet
//! or a VPN alike. Passwords never come here to rest: the app keeps them in the system keychain.

use crate::library::Library;
use anyhow::{Result, anyhow, bail};
use md5::{Digest, Md5};
use rusqlite::params;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: i64,
    /// "subsonic"
    pub kind: String,
    pub name: String,
    pub base_url: String,
    pub username: String,
}

/// A song, album, artist or playlist on a connected server, in one shape for the interface.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteItem {
    pub id: String,
    /// "song", "album", "artist" or "playlist"
    pub kind: String,
    pub name: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    /// The server's cover art id, if any
    pub cover: Option<String>,
    pub duration: Option<f64>,
    pub track: Option<i64>,
    pub count: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RemoteSearch {
    pub songs: Vec<RemoteItem>,
    pub albums: Vec<RemoteItem>,
    pub artists: Vec<RemoteItem>,
    pub playlists: Vec<RemoteItem>,
}

impl Library {
    pub fn connections(&self) -> Result<Vec<Connection>> {
        let db = self.db();
        let mut st = db.prepare("SELECT id, kind, name, base_url, username FROM connections ORDER BY name COLLATE NOCASE")?;
        let rows = st
            .query_map([], |r| Ok(Connection { id: r.get(0)?, kind: r.get(1)?, name: r.get(2)?, base_url: r.get(3)?, username: r.get(4)? }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn connection(&self, id: i64) -> Result<Connection> {
        self.connections()?.into_iter().find(|c| c.id == id).ok_or_else(|| anyhow!("No connection with id {id}"))
    }

    pub fn add_connection(&self, kind: &str, name: &str, base_url: &str, username: &str) -> Result<Connection> {
        if kind != "subsonic" {
            bail!("Unknown kind of connection: {kind}");
        }
        let url = normalize_url(base_url)?;
        let name = if name.trim().is_empty() { url.clone() } else { name.trim().to_string() };
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let id = {
            let db = self.db();
            db.execute(
                "INSERT INTO connections(kind, name, base_url, username, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![kind, name, url, username.trim(), t],
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

/// Talks to one Subsonic-compatible server.
pub struct Subsonic<'a> {
    pub conn: &'a Connection,
    pub password: &'a str,
}

impl Subsonic<'_> {
    /// Address of a call, with the salted-token sign-in the API uses (the password itself is never sent).
    pub fn url(&self, method: &str, extra: &[(&str, &str)]) -> String {
        let salt = format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let token = format!("{:x}", Md5::digest(format!("{}{salt}", self.password).as_bytes()));
        let mut u = reqwest::Url::parse(&format!("{}/rest/{method}", self.conn.base_url)).expect("checked when the connection was added");
        {
            let mut q = u.query_pairs_mut();
            q.append_pair("u", &self.conn.username).append_pair("t", &token).append_pair("s", &salt);
            q.append_pair("v", "1.16.1").append_pair("c", "LuteAndTavern").append_pair("f", "json");
            for (k, v) in extra {
                q.append_pair(k, v);
            }
        }
        u.to_string()
    }

    fn call(&self, method: &str, extra: &[(&str, &str)]) -> Result<Value> {
        let c = reqwest::blocking::Client::builder().timeout(Duration::from_secs(20)).build()?;
        let res = c
            .get(self.url(method, extra))
            .send()
            .map_err(|e| anyhow!("Could not reach {}: {}", self.conn.base_url, e.without_url()))?;
        let status = res.status();
        let v: Value = res.json().map_err(|_| anyhow!("{} answered {status} but not as a Subsonic music server", self.conn.base_url))?;
        let r = &v["subsonic-response"];
        if r["status"] != "ok" {
            let msg = r["error"]["message"].as_str().unwrap_or("unknown error");
            bail!("{}: {msg}", self.conn.name);
        }
        Ok(r.clone())
    }

    /// Checks the address and sign-in.
    pub fn ping(&self) -> Result<String> {
        let r = self.call("ping.view", &[])?;
        let server = r["type"].as_str().or(r["serverVersion"].as_str()).unwrap_or("Subsonic server");
        Ok(format!("{server} (API {})", r["version"].as_str().unwrap_or("?")))
    }

    pub fn search(&self, q: &str) -> Result<RemoteSearch> {
        let r = self.call("search3.view", &[("query", q), ("songCount", "25"), ("albumCount", "12"), ("artistCount", "8")])?;
        let s = &r["searchResult3"];
        let mut out = RemoteSearch {
            songs: list(&s["song"], song),
            albums: list(&s["album"], album),
            artists: list(&s["artist"], artist),
            playlists: vec![],
        };
        let ql = q.to_lowercase();
        out.playlists = self.playlists()?.into_iter().filter(|p| p.name.to_lowercase().contains(&ql)).collect();
        Ok(out)
    }

    pub fn playlists(&self) -> Result<Vec<RemoteItem>> {
        Ok(list(&self.call("getPlaylists.view", &[])?["playlists"]["playlist"], playlist))
    }

    /// Newest albums, for the server's home page.
    pub fn newest_albums(&self, size: usize) -> Result<Vec<RemoteItem>> {
        let n = size.to_string();
        Ok(list(&self.call("getAlbumList2.view", &[("type", "newest"), ("size", &n)])?["albumList2"]["album"], album))
    }

    /// Songs of an album or playlist; for an artist, their albums.
    pub fn open(&self, kind: &str, id: &str) -> Result<Vec<RemoteItem>> {
        Ok(match kind {
            "album" => list(&self.call("getAlbum.view", &[("id", id)])?["album"]["song"], song),
            "playlist" => list(&self.call("getPlaylist.view", &[("id", id)])?["playlist"]["entry"], song),
            "artist" => list(&self.call("getArtist.view", &[("id", id)])?["artist"]["album"], album),
            _ => bail!("Can't open a {kind}"),
        })
    }

    /// Songs to play for an item: the song itself, an album's or playlist's songs, or an artist's songs.
    pub fn songs_of(&self, kind: &str, id: &str) -> Result<Vec<RemoteItem>> {
        match kind {
            "artist" => {
                let mut songs = Vec::new();
                for a in self.open("artist", id)?.into_iter().take(12) {
                    songs.extend(self.open("album", &a.id)?);
                    if songs.len() >= 150 {
                        break;
                    }
                }
                Ok(songs)
            }
            _ => self.open(kind, id),
        }
    }

    /// Where the audio of a song is (original file, so seeking works and quality is untouched).
    pub fn stream_url(&self, song_id: &str) -> String {
        self.url("stream.view", &[("id", song_id), ("format", "raw")])
    }

    pub fn cover_url(&self, cover_id: &str, size: u32) -> String {
        let s = size.to_string();
        self.url("getCoverArt.view", &[("id", cover_id), ("size", &s)])
    }
}

/// The API returns one object instead of a list when there is a single result.
fn list(v: &Value, f: fn(&Value) -> RemoteItem) -> Vec<RemoteItem> {
    match v {
        Value::Array(a) => a.iter().map(f).collect(),
        Value::Object(_) => vec![f(v)],
        _ => vec![],
    }
}

fn text(v: &Value, k: &str) -> String {
    match &v[k] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn base(v: &Value, kind: &str) -> RemoteItem {
    RemoteItem {
        id: text(v, "id"),
        kind: kind.into(),
        name: Some(text(v, "title")).filter(|t| !t.is_empty()).unwrap_or_else(|| text(v, "name")),
        artist: text(v, "artist"),
        album: text(v, "album"),
        year: text(v, "year"),
        genre: text(v, "genre"),
        cover: Some(text(v, "coverArt")).filter(|c| !c.is_empty()),
        duration: v["duration"].as_f64(),
        track: v["track"].as_i64(),
        count: v["songCount"].as_i64(),
    }
}

fn song(v: &Value) -> RemoteItem {
    base(v, "song")
}
fn album(v: &Value) -> RemoteItem {
    base(v, "album")
}
fn artist(v: &Value) -> RemoteItem {
    RemoteItem { count: v["albumCount"].as_i64(), ..base(v, "artist") }
}
fn playlist(v: &Value) -> RemoteItem {
    RemoteItem { artist: text(v, "owner"), ..base(v, "playlist") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn addresses_are_normalized() {
        assert_eq!(normalize_url("nas.local:4533/").unwrap(), "http://nas.local:4533");
        assert_eq!(normalize_url("https://music.example.com/rest").unwrap(), "https://music.example.com");
        assert!(normalize_url("  ").is_err());
    }

    #[test]
    fn connections_are_saved_and_signed_urls_hide_the_password() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(d.path()).unwrap();
        let c = l.add_connection("subsonic", "Home", "nas.local:4533", "listener").unwrap();
        assert_eq!(l.connections().unwrap().len(), 1);
        let s = Subsonic { conn: &c, password: "hunter2" };
        let u = s.stream_url("abc");
        assert!(u.starts_with("http://nas.local:4533/rest/stream.view?u=listener&t="));
        assert!(!u.contains("hunter2"), "only the salted token goes on the wire");
        l.remove_connection(c.id).unwrap();
        assert!(l.connections().unwrap().is_empty());
    }

    #[test]
    fn single_results_and_lists_both_parse() {
        let one = json!({"id": 7, "title": "Song", "artist": "A", "duration": 200, "coverArt": "al-1"});
        let items = list(&one, song);
        assert_eq!((items[0].id.as_str(), items[0].name.as_str(), items[0].cover.as_deref()), ("7", "Song", Some("al-1")));
        assert_eq!(list(&json!([{"id": "1", "name": "Al"}, {"id": "2", "name": "B"}]), album).len(), 2);
        assert!(list(&Value::Null, song).is_empty());
    }
}
