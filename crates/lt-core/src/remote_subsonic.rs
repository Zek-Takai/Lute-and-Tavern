//! Subsonic API (OpenSubsonic): Navidrome, Airsonic, Gonic, Ampache, Funkwhale, Lyrion, many NAS music apps.
//! Signs in with a salted token, so the password itself is never sent. Servers that refuse tokens (Nextcloud
//! Music, LDAP-backed servers) use the API's plain-password sign-in instead, recorded per connection as
//! `legacyAuth` when the connection is added. That happens only when the server answers that it doesn't
//! accept tokens (error 41 or 42), never after a wrong password or a network error, and never over plain
//! http outside the home network, because the plain-password sign-in can be read by anyone on the way.

use crate::remote::{Connection, MusicServer, RemoteItem, RemoteSearch, Target, http, is_insecure, list, text};
use anyhow::{Result, anyhow, bail};
use md5::{Digest, Md5};
use serde_json::Value;

/// An error the server itself answered with (Subsonic error codes: 40 wrong username or password,
/// 41 token sign-in not supported for this user, 42 this way of signing in not supported, ...).
#[derive(Debug)]
pub struct SubsonicError {
    pub server: String,
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for SubsonicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.server, self.message)
    }
}

impl std::error::Error for SubsonicError {}

/// Whether a failed sign-in was the server saying it doesn't accept the salted-token sign-in at all.
pub fn refuses_tokens(e: &anyhow::Error) -> bool {
    e.downcast_ref::<SubsonicError>().is_some_and(|s| matches!(s.code, 41 | 42))
}

const PLAIN_PASSWORD_OVER_HTTP: &str = "This server only accepts the older sign-in, which sends the password in a form anyone on the way can read. \
     Over plain http:// outside your home network that isn't safe. Use the server's https:// address instead.";

/// Checks the address and sign-in of a new connection. Uses the salted-token sign-in; only when the server
/// answers that it doesn't take tokens does it try the plain-password one (refused over insecure http).
/// Returns the details to keep with the connection and a description of the server.
pub fn connect(conn: &Connection, password: &str) -> Result<(Value, String)> {
    let mut probe = conn.clone();
    probe.extra = serde_json::json!({});
    match (Subsonic { conn: &probe, password }).ping() {
        Ok(server) => Ok((serde_json::json!({}), server)),
        Err(e) if refuses_tokens(&e) => {
            if is_insecure(&conn.base_url) {
                bail!("{PLAIN_PASSWORD_OVER_HTTP}");
            }
            probe.extra = serde_json::json!({"legacyAuth": true});
            let server = (Subsonic { conn: &probe, password }).ping()?;
            Ok((serde_json::json!({"legacyAuth": true}), server))
        }
        Err(e) => Err(e),
    }
}

pub struct Subsonic<'a> {
    pub conn: &'a Connection,
    pub password: &'a str,
}

impl Subsonic<'_> {
    /// Address of a call, with the salted-token sign-in (or the plain-password one for servers that need it).
    pub fn url(&self, method: &str, extra: &[(&str, &str)]) -> String {
        let mut u = reqwest::Url::parse(&format!("{}/rest/{method}", self.conn.base_url)).expect("checked when the connection was added");
        {
            let mut q = u.query_pairs_mut();
            q.append_pair("u", &self.conn.username);
            if self.legacy() {
                let hex: String = self.password.bytes().map(|b| format!("{b:02x}")).collect();
                q.append_pair("p", &format!("enc:{hex}"));
            } else {
                let salt = format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
                let token = format!("{:x}", Md5::digest(format!("{}{salt}", self.password).as_bytes()));
                q.append_pair("t", &token).append_pair("s", &salt);
            }
            q.append_pair("v", "1.16.1").append_pair("c", "LuteAndTavern").append_pair("f", "json");
            for (k, v) in extra {
                q.append_pair(k, v);
            }
        }
        u.to_string()
    }

    fn legacy(&self) -> bool {
        self.conn.extra["legacyAuth"].as_bool().unwrap_or(false)
    }

    /// A signed address, refused when it would carry the plain password over insecure http.
    fn signed(&self, method: &str, extra: &[(&str, &str)]) -> Result<String> {
        if self.legacy() && is_insecure(&self.conn.base_url) {
            bail!("{PLAIN_PASSWORD_OVER_HTTP}");
        }
        Ok(self.url(method, extra))
    }

    fn call(&self, method: &str, extra: &[(&str, &str)]) -> Result<Value> {
        let res = http()?
            .get(self.signed(method, extra)?)
            .send()
            .map_err(|e| anyhow!("Could not reach {}: {}", self.conn.base_url, e.without_url()))?;
        let status = res.status();
        let v: Value = res.json().map_err(|_| anyhow!("{} answered {status} but not as a Subsonic music server", self.conn.base_url))?;
        let r = &v["subsonic-response"];
        if r["status"] != "ok" {
            return Err(SubsonicError {
                server: self.conn.name.clone(),
                code: r["error"]["code"].as_i64().unwrap_or(0),
                message: r["error"]["message"].as_str().unwrap_or("unknown error").to_string(),
            }
            .into());
        }
        Ok(r.clone())
    }

    fn playlists(&self) -> Result<Vec<RemoteItem>> {
        Ok(list(&self.call("getPlaylists.view", &[])?["playlists"]["playlist"], playlist))
    }
}

impl MusicServer for Subsonic<'_> {
    fn ping(&self) -> Result<String> {
        let r = self.call("ping.view", &[])?;
        let server = r["type"].as_str().unwrap_or("Subsonic server");
        Ok(format!("{server} (API {})", r["version"].as_str().unwrap_or("?")))
    }

    fn search(&self, q: &str) -> Result<RemoteSearch> {
        let r = self.call("search3.view", &[("query", q), ("songCount", "25"), ("albumCount", "12"), ("artistCount", "8")])?;
        let s = &r["searchResult3"];
        let ql = q.to_lowercase();
        Ok(RemoteSearch {
            songs: list(&s["song"], song),
            albums: list(&s["album"], album),
            artists: list(&s["artist"], artist),
            playlists: self.playlists()?.into_iter().filter(|p| p.name.to_lowercase().contains(&ql)).collect(),
            folders: vec![],
        })
    }

    fn home(&self) -> Result<RemoteSearch> {
        let albums = list(&self.call("getAlbumList2.view", &[("type", "newest"), ("size", "30")])?["albumList2"]["album"], album);
        Ok(RemoteSearch { playlists: self.playlists()?, albums, ..Default::default() })
    }

    fn open(&self, kind: &str, id: &str) -> Result<Vec<RemoteItem>> {
        Ok(match kind {
            "album" => list(&self.call("getAlbum.view", &[("id", id)])?["album"]["song"], song),
            "playlist" => list(&self.call("getPlaylist.view", &[("id", id)])?["playlist"]["entry"], song),
            "artist" => list(&self.call("getArtist.view", &[("id", id)])?["artist"]["album"], album),
            _ => bail!("Can't open a {kind}"),
        })
    }

    fn all_songs(&self) -> Result<Vec<RemoteItem>> {
        // Album by album (the one listing every Subsonic server supports), six albums at a time.
        let mut albums = Vec::new();
        let mut offset = 0usize;
        loop {
            let off = offset.to_string();
            let page = list(&self.call("getAlbumList2.view", &[("type", "alphabeticalByName"), ("size", "500"), ("offset", &off)])?["albumList2"]["album"], album);
            offset += page.len();
            let done = page.len() < 500;
            albums.extend(page);
            if done {
                break;
            }
        }
        let chunks: Vec<&[RemoteItem]> = albums.chunks(albums.len().div_ceil(6).max(1)).collect();
        let results: Vec<Result<Vec<RemoteItem>>> = std::thread::scope(|sc| {
            let handles: Vec<_> = chunks
                .iter()
                .map(|chunk| sc.spawn(move || -> Result<Vec<RemoteItem>> {
                    let mut out = Vec::new();
                    for a in *chunk {
                        out.extend(self.open("album", &a.id)?);
                    }
                    Ok(out)
                }))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(anyhow::anyhow!("a reader stopped")))).collect()
        });
        let mut out = Vec::new();
        for r in results {
            out.extend(r?);
        }
        Ok(out)
    }

    fn library_stamp(&self) -> Result<Option<String>> {
        // getIndexes carries the library's last-modified time (milliseconds).
        let r = self.call("getIndexes.view", &[])?;
        Ok(r["indexes"]["lastModified"].as_i64().map(|t| t.to_string()))
    }

    fn stream(&self, song_id: &str) -> Result<Target> {
        Ok(Target::plain(self.signed("stream.view", &[("id", song_id), ("format", "raw")])?))
    }

    fn cover(&self, cover: &str, size: u32) -> Option<Target> {
        Some(Target::plain(self.signed("getCoverArt.view", &[("id", cover), ("size", &size.to_string())]).ok()?))
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
        added: v["created"].as_str().and_then(crate::remote::iso_time),
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
    fn signed_urls_hide_the_password() {
        let c = Connection { id: 1, kind: "subsonic".into(), name: "Home".into(), base_url: "http://nas.local:4533".into(), username: "listener".into(), extra: Value::Null };
        let s = Subsonic { conn: &c, password: "hunter2" };
        let u = s.stream("abc").unwrap().url;
        assert!(u.starts_with("http://nas.local:4533/rest/stream.view?u=listener&t="));
        assert!(!u.contains("hunter2"), "only the salted token goes on the wire");
    }

    #[test]
    fn servers_that_refuse_tokens_get_the_plain_sign_in() {
        let c = Connection { id: 1, kind: "subsonic".into(), name: "NC".into(), base_url: "https://nas:8090/apps/music/subsonic".into(), username: "listener".into(), extra: serde_json::json!({"legacyAuth": true}) };
        let u = Subsonic { conn: &c, password: "ab" }.url("ping.view", &[]);
        assert!(u.contains("u=listener&p=enc%3A6162") && !u.contains("&t="), "{u}");
    }

    /// A Subsonic server that accepts only `password`, and (if `tokens` is false) only the plain sign-in.
    fn fake(password: &'static str, tokens: bool) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let url = req.url().to_string();
                log.lock().unwrap().push(url.clone());
                let q: std::collections::HashMap<String, String> = reqwest::Url::parse(&format!("http://x{url}")).unwrap().query_pairs().into_owned().collect();
                let r = if let Some(p) = q.get("p") {
                    let hex: String = password.bytes().map(|b| format!("{b:02x}")).collect();
                    if *p == format!("enc:{hex}") { json!({"status": "ok", "type": "fake", "version": "1.16.1"}) } else { json!({"status": "failed", "error": {"code": 40, "message": "Wrong username or password"}}) }
                } else if !tokens {
                    json!({"status": "failed", "error": {"code": 41, "message": "Token authentication not supported"}})
                } else {
                    let ok = q.get("t").map(String::as_str) == Some(&format!("{:x}", Md5::digest(format!("{password}{}", q["s"]).as_bytes())));
                    if ok { json!({"status": "ok", "type": "fake", "version": "1.16.1"}) } else { json!({"status": "failed", "error": {"code": 40, "message": "Wrong username or password"}}) }
                };
                let _ = req.respond(tiny_http::Response::from_string(json!({"subsonic-response": r}).to_string()));
            }
        });
        (format!("http://127.0.0.1:{port}"), seen)
    }

    fn conn(base: &str) -> Connection {
        Connection { id: 0, kind: "subsonic".into(), name: "Home".into(), base_url: base.into(), username: "listener".into(), extra: json!({}) }
    }

    #[test]
    fn plain_password_sign_in_only_when_the_server_refuses_tokens() {
        // tokens work: no fallback
        let (base, seen) = fake("hunter2", true);
        let (extra, _) = connect(&conn(&base), "hunter2").unwrap();
        assert!(extra["legacyAuth"].is_null());
        // a wrong password never retries with the plain one
        let err = connect(&conn(&base), "wrong").unwrap_err();
        assert!(!refuses_tokens(&err) && err.to_string().contains("Wrong username"), "{err}");
        assert!(seen.lock().unwrap().iter().all(|u| !u.contains("p=")), "the password was never sent");
        // a server that answers 41 gets the plain sign-in (local http is fine)
        let (base, _) = fake("hunter2", false);
        let (extra, _) = connect(&conn(&base), "hunter2").unwrap();
        assert_eq!(extra["legacyAuth"], json!(true));
        // a network error never falls back
        let err = connect(&conn("http://127.0.0.1:9"), "hunter2").unwrap_err();
        assert!(!refuses_tokens(&err));
    }

    #[test]
    fn plain_password_never_goes_over_insecure_http() {
        let refused = anyhow::Error::from(SubsonicError { server: "x".into(), code: 41, message: "no tokens".into() });
        assert!(refuses_tokens(&refused));
        assert!(!refuses_tokens(&anyhow::Error::from(SubsonicError { server: "x".into(), code: 40, message: "wrong".into() })));
        let c = Connection { extra: json!({"legacyAuth": true}), ..conn("http://music.example.com:4533") };
        let s = Subsonic { conn: &c, password: "hunter2" };
        assert!(s.ping().unwrap_err().to_string().contains("https://"));
        assert!(s.stream("1").is_err() && s.cover("1", 300).is_none());
    }

    #[test]
    fn single_results_and_lists_both_parse() {
        let items = list(&json!({"id": 7, "title": "Song", "artist": "A", "duration": 200, "coverArt": "al-1"}), song);
        assert_eq!((items[0].id.as_str(), items[0].name.as_str(), items[0].cover.as_deref()), ("7", "Song", Some("al-1")));
        assert_eq!(list(&json!([{"id": "1", "name": "Al"}, {"id": "2", "name": "B"}]), album).len(), 2);
        assert!(list(&Value::Null, song).is_empty());
    }

    #[test]
    fn reads_every_song_album_by_album_and_reports_a_stamp() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let url = req.url().to_string();
                let body = if url.contains("getAlbumList2") {
                    let albums: Vec<Value> = (0..13).map(|i| json!({"id": format!("al{i}"), "name": format!("Album {i}")})).collect();
                    json!({"albumList2": {"album": albums}})
                } else if url.contains("getAlbum.view") {
                    let id = url.split("id=").nth(1).unwrap().split('&').next().unwrap().to_string();
                    json!({"album": {"song": [{"id": format!("{id}-1"), "title": "A"}, {"id": format!("{id}-2"), "title": "B"}]}})
                } else if url.contains("getIndexes") {
                    json!({"indexes": {"lastModified": 1790000000000i64}})
                } else {
                    json!({})
                };
                let mut r = body;
                r["status"] = json!("ok");
                let _ = req.respond(tiny_http::Response::from_string(json!({"subsonic-response": r}).to_string()));
            }
        });
        let c = Connection { id: 1, kind: "subsonic".into(), name: "NC".into(), base_url: format!("http://127.0.0.1:{port}"), username: "listener".into(), extra: json!({"legacyAuth": true}) };
        let s = Subsonic { conn: &c, password: "x" };
        let songs = s.all_songs().unwrap();
        assert_eq!(songs.len(), 26, "13 albums x 2 songs, read in parallel");
        let mut ids: Vec<String> = songs.iter().map(|x| x.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 26);
        assert_eq!(s.library_stamp().unwrap().as_deref(), Some("1790000000000"));
    }
}
