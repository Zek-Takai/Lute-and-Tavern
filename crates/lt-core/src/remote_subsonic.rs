//! Subsonic API (OpenSubsonic): Navidrome, Airsonic, Gonic, Ampache, Funkwhale, Lyrion, many NAS music apps.
//! Signs in with a salted token, so the password itself is never sent. Servers that refuse tokens (Nextcloud
//! Music, LDAP-backed servers) use the API's plain-password sign-in instead, recorded per connection as
//! `legacyAuth` when the connection is added (only sensible over HTTPS).

use crate::remote::{Connection, MusicServer, RemoteItem, RemoteSearch, Target, http, list, text};
use anyhow::{Result, anyhow, bail};
use md5::{Digest, Md5};
use serde_json::Value;

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
            if self.conn.extra["legacyAuth"].as_bool().unwrap_or(false) {
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

    fn call(&self, method: &str, extra: &[(&str, &str)]) -> Result<Value> {
        let res = http()?
            .get(self.url(method, extra))
            .send()
            .map_err(|e| anyhow!("Could not reach {}: {}", self.conn.base_url, e.without_url()))?;
        let status = res.status();
        let v: Value = res.json().map_err(|_| anyhow!("{} answered {status} but not as a Subsonic music server", self.conn.base_url))?;
        let r = &v["subsonic-response"];
        if r["status"] != "ok" {
            bail!("{}: {}", self.conn.name, r["error"]["message"].as_str().unwrap_or("unknown error"));
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

    fn stream(&self, song_id: &str) -> Result<Target> {
        Ok(Target::plain(self.url("stream.view", &[("id", song_id), ("format", "raw")])))
    }

    fn cover(&self, cover: &str, size: u32) -> Option<Target> {
        Some(Target::plain(self.url("getCoverArt.view", &[("id", cover), ("size", &size.to_string())])))
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

    #[test]
    fn single_results_and_lists_both_parse() {
        let items = list(&json!({"id": 7, "title": "Song", "artist": "A", "duration": 200, "coverArt": "al-1"}), song);
        assert_eq!((items[0].id.as_str(), items[0].name.as_str(), items[0].cover.as_deref()), ("7", "Song", Some("al-1")));
        assert_eq!(list(&json!([{"id": "1", "name": "Al"}, {"id": "2", "name": "B"}]), album).len(), 2);
        assert!(list(&Value::Null, song).is_empty());
    }
}
