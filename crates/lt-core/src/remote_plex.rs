//! Plex Media Server. Sign-in goes through plex.tv with a PIN the person approves in their browser
//! (`start_login` / `check_login`), then `servers` lists the Plex servers on their account with every
//! address each can be reached at. Music is read through the server's own API; audio is the original file.

use crate::remote::{Connection, MusicServer, RemoteItem, RemoteSearch, Target, get_json, http, net, text};
use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::cell::Cell;

const PRODUCT: &str = "Lute & Tavern";

fn plex_headers(client: &str, token: Option<&str>) -> Vec<(String, String)> {
    let mut h = vec![
        ("X-Plex-Product".to_string(), PRODUCT.to_string()),
        ("X-Plex-Client-Identifier".to_string(), client.to_string()),
        ("X-Plex-Version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
    ];
    if let Some(t) = token {
        h.push(("X-Plex-Token".to_string(), t.to_string()));
    }
    h
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Login {
    pub id: i64,
    pub code: String,
    /// Open this in the browser; the person signs in and approves the app
    pub auth_url: String,
}

/// Starts a plex.tv sign-in. `client` is this install's stable identifier.
pub fn start_login(client: &str) -> Result<Login> {
    let mut req = http()?.post("https://plex.tv/api/v2/pins?strong=true").header("Accept", "application/json");
    for (k, v) in plex_headers(client, None) {
        req = req.header(k, v);
    }
    let v: Value = req.send().map_err(net)?.error_for_status().map_err(net)?.json().map_err(net)?;
    let (id, code) = (v["id"].as_i64().ok_or_else(|| anyhow!("plex.tv sent no sign-in id"))?, text(&v, "code"));
    let mut u = reqwest::Url::parse("https://app.plex.tv/auth#")?;
    let frag = reqwest::Url::parse_with_params(
        "https://x/?",
        &[("clientID", client), ("code", code.as_str()), ("context[device][product]", PRODUCT)],
    )?
    .query()
    .unwrap_or("")
    .to_string();
    u.set_fragment(Some(&format!("?{frag}")));
    Ok(Login { id, code, auth_url: u.to_string() })
}

/// The account token once the person approved the sign-in; None while still waiting.
pub fn check_login(client: &str, id: i64) -> Result<Option<String>> {
    let v = get_json(&format!("https://plex.tv/api/v2/pins/{id}"), &plex_headers(client, None), "plex.tv")?;
    Ok(v["authToken"].as_str().filter(|t| !t.is_empty()).map(String::from))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexServer {
    pub name: String,
    pub machine: String,
    /// The token to use with this server (differs from the account token on shared servers)
    #[serde(skip)]
    pub token: String,
    /// Addresses to try, best first: same network, then internet, then Plex's relay
    pub uris: Vec<String>,
    pub owned: bool,
}

/// The Plex servers on the account.
pub fn servers(client: &str, account_token: &str) -> Result<Vec<PlexServer>> {
    let v = get_json("https://plex.tv/api/v2/resources?includeHttps=1&includeRelay=1", &plex_headers(client, Some(account_token)), "plex.tv")?;
    let mut out = Vec::new();
    for r in v.as_array().into_iter().flatten() {
        if !text(r, "provides").contains("server") {
            continue;
        }
        let mut conns: Vec<&Value> = r["connections"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
        // same network first, relay last
        conns.sort_by_key(|c| (c["relay"].as_bool().unwrap_or(false), !c["local"].as_bool().unwrap_or(false)));
        out.push(PlexServer {
            name: text(r, "name"),
            machine: text(r, "clientIdentifier"),
            token: Some(text(r, "accessToken")).filter(|t| !t.is_empty()).unwrap_or_else(|| account_token.to_string()),
            uris: conns.iter().map(|c| text(c, "uri")).filter(|u| !u.is_empty()).collect(),
            owned: r["owned"].as_bool().unwrap_or(false),
        });
    }
    Ok(out)
}

/// The details a Plex connection keeps (not secret): which server, its addresses, this install's id.
pub fn connection_extra(s: &PlexServer, client: &str) -> Value {
    json!({"machine": s.machine, "uris": s.uris, "client": client})
}

pub struct Plex<'a> {
    conn: &'a Connection,
    token: &'a str,
    uris: Vec<String>,
    /// The address that answered last
    current: Cell<usize>,
}

impl<'a> Plex<'a> {
    pub fn new(conn: &'a Connection, token: &'a str) -> Self {
        let mut uris: Vec<String> = conn.extra["uris"].as_array().map(|a| a.iter().filter_map(|u| u.as_str().map(String::from)).collect()).unwrap_or_default();
        if uris.is_empty() {
            uris.push(conn.base_url.clone());
        }
        Self { conn, token, uris, current: Cell::new(0) }
    }

    fn headers(&self) -> Vec<(String, String)> {
        plex_headers(self.conn.extra["client"].as_str().unwrap_or("lute-and-tavern"), Some(self.token))
    }

    fn base(&self) -> &str {
        &self.uris[self.current.get()]
    }

    /// GET a server path, trying each known address until one answers (and remembering it).
    fn get(&self, path: &str) -> Result<Value> {
        let start = self.current.get();
        let mut last = None;
        for k in 0..self.uris.len() {
            let i = (start + k) % self.uris.len();
            match get_json(&format!("{}{path}", self.uris[i]), &self.headers(), &self.conn.name) {
                Ok(v) => {
                    self.current.set(i);
                    return Ok(v["MediaContainer"].clone());
                }
                Err(e) if e.to_string().contains("refused the sign-in") => return Err(e),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("{} has no address to reach", self.conn.name)))
    }

    fn music_sections(&self) -> Result<Vec<String>> {
        Ok(self.get("/library/sections")?["Directory"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["type"] == "artist")
            .map(|d| text(d, "key"))
            .collect())
    }

    fn playlists(&self) -> Result<Vec<RemoteItem>> {
        Ok(items(&self.get("/playlists?playlistType=audio")?["Metadata"]))
    }
}

fn enc(s: &str) -> String {
    reqwest::Url::parse_with_params("https://x/?", &[("v", s)]).ok().and_then(|u| u.query().map(|q| q[2..].to_string())).unwrap_or_default()
}

/// Plex items of any music type, mapped by their "type".
fn item(v: &Value) -> RemoteItem {
    let kind = match v["type"].as_str().unwrap_or("") {
        "track" => "song",
        "album" => "album",
        "artist" => "artist",
        "playlist" => "playlist",
        _ => "",
    };
    let thumb = [&v["thumb"], &v["parentThumb"], &v["grandparentThumb"], &v["composite"]].into_iter().find_map(|t| t.as_str()).map(String::from);
    let (artist, album) = match kind {
        "song" => (Some(text(v, "originalTitle")).filter(|a| !a.is_empty()).unwrap_or_else(|| text(v, "grandparentTitle")), text(v, "parentTitle")),
        "album" => (text(v, "parentTitle"), String::new()),
        _ => (String::new(), String::new()),
    };
    RemoteItem {
        id: text(v, "ratingKey"),
        kind: kind.into(),
        name: text(v, "title"),
        artist,
        album,
        year: Some(text(v, "parentYear")).filter(|y| !y.is_empty()).unwrap_or_else(|| text(v, "year")),
        genre: v["Genre"][0]["tag"].as_str().unwrap_or("").to_string(),
        cover: thumb,
        duration: v["duration"].as_f64().map(|ms| ms / 1000.0),
        track: v["index"].as_i64().filter(|_| kind == "song"),
        count: v["leafCount"].as_i64().or(v["childCount"].as_i64()),
        added: v["addedAt"].as_i64(),
    }
}

fn items(v: &Value) -> Vec<RemoteItem> {
    v.as_array().into_iter().flatten().map(item).filter(|i| !i.kind.is_empty()).collect()
}

impl MusicServer for Plex<'_> {
    fn ping(&self) -> Result<String> {
        let v = self.get("/identity")?;
        let n = self.music_sections()?.len();
        if n == 0 {
            bail!("{} has no music library. Add one in Plex first.", self.conn.name);
        }
        Ok(format!("Plex Media Server {} ({n} music librar{})", text(&v, "version"), if n == 1 { "y" } else { "ies" }))
    }

    fn search(&self, q: &str) -> Result<RemoteSearch> {
        let v = self.get(&format!("/hubs/search?query={}&limit=15", enc(q)))?;
        let mut r = RemoteSearch::default();
        for hub in v["Hub"].as_array().into_iter().flatten() {
            let list = items(&hub["Metadata"]);
            match hub["type"].as_str().unwrap_or("") {
                "track" => r.songs = list,
                "album" => r.albums = list,
                "artist" => r.artists = list,
                "playlist" => {
                    r.playlists = hub["Metadata"].as_array().into_iter().flatten().filter(|p| p["playlistType"] == "audio").map(item).collect()
                }
                _ => {}
            }
        }
        Ok(r)
    }

    fn home(&self) -> Result<RemoteSearch> {
        let mut albums = Vec::new();
        for key in self.music_sections()? {
            albums.extend(items(&self.get(&format!("/library/sections/{key}/all?type=9&sort=addedAt:desc&X-Plex-Container-Start=0&X-Plex-Container-Size=30"))?["Metadata"]));
        }
        albums.truncate(30);
        Ok(RemoteSearch { playlists: self.playlists()?, albums, ..Default::default() })
    }

    fn open(&self, kind: &str, id: &str) -> Result<Vec<RemoteItem>> {
        let path = match kind {
            "album" | "artist" => format!("/library/metadata/{id}/children"),
            "playlist" => format!("/playlists/{id}/items"),
            _ => bail!("Can't open a {kind}"),
        };
        Ok(items(&self.get(&path)?["Metadata"]))
    }

    fn library_stamp(&self) -> Result<Option<String>> {
        // Each music library's last scan and content change.
        let v = self.get("/library/sections")?;
        let stamp: Vec<String> = v["Directory"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["type"] == "artist")
            .map(|d| format!("{}:{}:{}", text(d, "key"), text(d, "contentChangedAt"), text(d, "scannedAt")))
            .collect();
        Ok(Some(stamp.join(",")))
    }

    fn all_songs(&self) -> Result<Vec<RemoteItem>> {
        let mut out = Vec::new();
        for key in self.music_sections()? {
            let mut start = 0usize;
            loop {
                let page = items(&self.get(&format!("/library/sections/{key}/all?type=10&X-Plex-Container-Start={start}&X-Plex-Container-Size=1000"))?["Metadata"]);
                start += page.len();
                let done = page.len() < 1000;
                out.extend(page);
                if done {
                    break;
                }
            }
        }
        Ok(out)
    }

    fn stream(&self, song_id: &str) -> Result<Target> {
        let v = self.get(&format!("/library/metadata/{song_id}"))?;
        let key = v["Metadata"][0]["Media"][0]["Part"][0]["key"].as_str().ok_or_else(|| anyhow!("Plex has no file for that song"))?;
        Ok(Target { url: format!("{}{key}", self.base()), headers: self.headers() })
    }

    fn cover(&self, cover: &str, size: u32) -> Option<Target> {
        Some(Target {
            url: format!("{}/photo/:/transcode?width={size}&height={size}&minSize=1&upscale=1&url={}", self.base(), enc(cover)),
            headers: self.headers(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(routes: Vec<(&'static str, Value)>) -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let has_token = req.headers().iter().any(|h| h.field.equiv("X-Plex-Token") && h.value == "tok");
                let path = req.url().to_string();
                let body = routes.iter().find(|(p, _)| path.starts_with(p)).map(|(_, b)| b.clone());
                let res = match (has_token, body) {
                    (false, _) => tiny_http::Response::from_string("{}").with_status_code(401),
                    (true, Some(b)) => tiny_http::Response::from_string(json!({"MediaContainer": b}).to_string()),
                    (true, None) => tiny_http::Response::from_string("{}").with_status_code(404),
                };
                let _ = req.respond(res);
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[test]
    fn browses_searches_and_streams_a_plex_server() {
        let track = json!({"ratingKey": "11", "type": "track", "title": "Numb", "grandparentTitle": "Linkin Park", "parentTitle": "Meteora", "parentYear": 2003,
                           "duration": 187000, "index": 13, "parentThumb": "/library/metadata/10/thumb/1", "Media": [{"Part": [{"key": "/library/parts/99/file.mp3"}]}]});
        let url = fake(vec![
            ("/identity", json!({"version": "1.42.1"})),
            ("/library/sections/1/all", json!({"Metadata": [{"ratingKey": "10", "type": "album", "title": "Meteora", "parentTitle": "Linkin Park", "leafCount": 13}]})),
            ("/library/sections", json!({"Directory": [{"key": "1", "type": "artist", "title": "Music"}, {"key": "2", "type": "movie", "title": "Movies"}]})),
            ("/playlists/5/items", json!({"Metadata": [track]})),
            ("/playlists", json!({"Metadata": [{"ratingKey": "5", "type": "playlist", "title": "Road", "playlistType": "audio", "leafCount": 1}]})),
            ("/hubs/search", json!({"Hub": [{"type": "track", "Metadata": [track]}, {"type": "movie", "Metadata": [{"ratingKey": "1", "type": "movie", "title": "x"}]}]})),
            ("/library/metadata/10/children", json!({"Metadata": [track]})),
            ("/library/metadata/11", json!({"Metadata": [track]})),
        ]);
        // the first address is dead; the second answers
        let conn = Connection { id: 1, kind: "plex".into(), name: "NAS".into(), base_url: url.clone(), username: String::new(),
                                extra: json!({"uris": ["http://127.0.0.1:9", url], "client": "c1"}) };
        let p = Plex::new(&conn, "tok");
        assert!(p.ping().unwrap().contains("1 music library"));
        let s = p.search("numb").unwrap();
        assert_eq!((s.songs.len(), s.songs[0].artist.as_str(), s.songs[0].duration), (1, "Linkin Park", Some(187.0)));
        let h = p.home().unwrap();
        assert_eq!((h.albums[0].name.as_str(), h.playlists[0].name.as_str()), ("Meteora", "Road"));
        assert_eq!(p.songs_of("album", "10").unwrap()[0].track, Some(13));
        assert_eq!(p.songs_of("playlist", "5").unwrap().len(), 1);
        let t = p.stream("11").unwrap();
        assert_eq!(t.url, format!("{url}/library/parts/99/file.mp3"));
        assert!(t.headers.iter().any(|(k, v)| k == "X-Plex-Token" && v == "tok"), "the token travels as a header, not in the address");
        assert!(p.cover("/library/metadata/10/thumb/1", 300).unwrap().url.contains("url=%2Flibrary%2Fmetadata%2F10%2Fthumb%2F1"));
        let bad = Plex::new(&conn, "wrong");
        assert!(bad.ping().unwrap_err().to_string().contains("refused the sign-in"));
    }
}
