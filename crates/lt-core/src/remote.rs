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

/// "nas.local:4533" → "https://nas.local:4533"; trailing slashes and a pasted "/rest" are dropped. An address
/// typed without http:// or https:// gets https:// (see `pick_scheme` for the plain-http fallback at home).
pub fn normalize_url(raw: &str) -> Result<String> {
    let mut u = raw.trim().trim_end_matches('/').to_string();
    if u.is_empty() {
        bail!("Add the server's address");
    }
    if !u.contains("://") {
        u = format!("https://{u}");
    }
    if let Some(stripped) = u.strip_suffix("/rest") {
        u = stripped.to_string();
    }
    let parsed = reqwest::Url::parse(&u).map_err(|e| anyhow!("That address doesn't look right: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("The address should start with https:// or http://");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("Put the username and password in their own boxes, not in the address");
    }
    Ok(u)
}

/// The address to use for what the person typed. With http:// or https:// it is used as typed. Without one,
/// https:// is tried first (a quick check with no sign-in in it); plain http:// is used instead only when
/// https doesn't answer and the server is on this computer, the home network or a Tailscale network.
pub fn pick_scheme(raw: &str) -> Result<String> {
    let https = normalize_url(raw)?;
    if raw.trim().contains("://") {
        return Ok(https);
    }
    let reachable = http().ok().and_then(|c| c.get(&https).send().ok()).is_some();
    let url = reqwest::Url::parse(&https)?;
    if !reachable && is_local(&url) {
        return Ok(format!("http://{}", &https["https://".len()..]));
    }
    Ok(https)
}

/// Whether a host is on this computer, a private home network (10/8, 172.16/12, 192.168/16, *.local) or a
/// Tailscale network (100.64/10, *.ts.net), where plain http doesn't cross the internet.
pub fn is_local_host(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local") || h.ends_with(".ts.net") {
        return true;
    }
    match h.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            let o = ip.octets();
            o[0] == 127 || o[0] == 10 || (o[0] == 172 && (16..32).contains(&o[1])) || (o[0] == 192 && o[1] == 168) || (o[0] == 100 && (64..128).contains(&o[1]))
        }
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v4| is_local_host(&v4.to_string())),
        Err(_) => false,
    }
}

pub fn is_local(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(is_local_host)
}

/// True when an address would send sign-in details unencrypted across a network the person doesn't control:
/// plain http:// to a host that isn't local (see `is_local_host`).
pub fn is_insecure(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| u.scheme() == "http" && !is_local(&u))
}

/// Downloads a target (a cover image) with its sign-in headers. A redirect to another server is followed
/// without the sign-in headers.
pub fn fetch(t: &Target) -> Result<Vec<u8>> {
    let mut req = http()?.get(&t.url);
    for (k, v) in &t.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let mut res = req.send().map_err(net)?;
    if let Some(next) = cross_origin_redirect(&res) {
        res = plain_http()?.get(next).send().map_err(net)?;
    }
    let res = res.error_for_status().map_err(net)?;
    Ok(res.bytes().map_err(net)?.to_vec())
}

impl Library {
    /// Changes one non-secret detail of a connection (e.g. `inLibrary`).
    /// Only that one key is written, in one statement, so a background sync saving its stamp can't undo
    /// "In My music" being turned off at the same moment.
    pub fn set_connection_extra(&self, id: i64, key: &str, value: Value) -> Result<Connection> {
        let changed = self.db().execute(
            "UPDATE connections SET extra = json_set(CASE WHEN json_valid(extra) AND json_type(extra) = 'object' THEN extra ELSE '{}' END, ?2, json(?3)) WHERE id = ?1",
            params![id, format!("$.\"{}\"", key.replace('"', "")), value.to_string()],
        )?;
        if changed == 0 {
            bail!("No connection with id {id}");
        }
        self.connection(id)
    }
}

/// The client for talking to a connected server. Redirects are followed only on the same server, so sign-in
/// headers (Authorization, X-Plex-Token, X-Emby-Token, X-Emby-Authorization) never go to another host
/// (reqwest itself only strips the standard ones) and never drop from https to plain http.
pub(crate) fn http() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(concat!("LuteAndTavern/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(same_server_redirects())
        .build()?)
}

/// A client with reqwest's usual redirects, for requests that carry no sign-in.
fn plain_http() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(concat!("LuteAndTavern/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?)
}

/// Whether a redirect from `from` to `to` stays on the same server and doesn't downgrade to plain http.
pub fn same_server(from: &reqwest::Url, to: &reqwest::Url) -> bool {
    if from.host_str() != to.host_str() {
        return false;
    }
    match (from.scheme(), to.scheme()) {
        (a, b) if a == b => from.port_or_known_default() == to.port_or_known_default(),
        // http → https on the same host is an upgrade
        ("http", "https") => true,
        _ => false,
    }
}

/// Follows up to 10 redirects on the same server and stops at one that leaves it (the 3xx answer is returned).
pub fn same_server_redirects() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|a| {
        if a.previous().len() > 10 {
            return a.error("too many redirects");
        }
        match a.previous().first() {
            Some(first) if same_server(first, a.url()) => a.follow(),
            _ => a.stop(),
        }
    })
}

/// The address a response redirects to when that is another server (see `same_server_redirects`).
pub fn cross_origin_redirect(res: &reqwest::blocking::Response) -> Option<reqwest::Url> {
    if !res.status().is_redirection() {
        return None;
    }
    let loc = res.headers().get(reqwest::header::LOCATION)?.to_str().ok()?;
    let next = res.url().join(loc).ok()?;
    (!same_server(res.url(), &next) && matches!(next.scheme(), "http" | "https")).then_some(next)
}

/// A network error as a message without its address, which can carry sign-in details (Subsonic tokens).
pub fn net(e: reqwest::Error) -> anyhow::Error {
    anyhow::Error::new(e.without_url())
}

/// GET returning JSON, with headers; errors name the server.
pub(crate) fn get_json(url: &str, headers: &[(String, String)], who: &str) -> Result<Value> {
    let mut req = http()?.get(url).header("Accept", "application/json");
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let res = req.send().map_err(|e| anyhow!("Could not reach {who}: {}", e.without_url()))?;
    let status = res.status();
    if status.is_redirection() {
        bail!("{who} sent the request on to another server, so the sign-in wasn't passed along");
    }
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
        assert_eq!(normalize_url("nas.local:4533/").unwrap(), "https://nas.local:4533");
        assert_eq!(normalize_url("http://nas.local:4533/").unwrap(), "http://nas.local:4533");
        assert_eq!(normalize_url("https://music.example.com/rest").unwrap(), "https://music.example.com");
        assert!(normalize_url("  ").is_err());
        assert!(normalize_url("https://listener:hunter2@music.example.com").is_err(), "no passwords inside the address");
        assert!(normalize_url("ftp://nas").is_err());
    }

    #[test]
    fn local_hosts_are_recognized() {
        for h in ["localhost", "127.0.0.1", "127.8.9.1", "::1", "[::1]", "10.1.2.3", "172.16.0.1", "172.31.255.1", "192.168.1.20", "nas.local", "100.64.0.1", "100.127.1.1", "box.tail1234.ts.net"] {
            assert!(is_local_host(h), "{h}");
        }
        for h in ["music.example.com", "8.8.8.8", "172.32.0.1", "172.15.0.1", "100.128.0.1", "192.169.1.1", "local.example.com", "11.0.0.1"] {
            assert!(!is_local_host(h), "{h}");
        }
        assert!(is_insecure("http://music.example.com:4533"));
        assert!(!is_insecure("https://music.example.com"));
        assert!(!is_insecure("http://192.168.1.20:4533"));
    }

    #[test]
    fn no_scheme_tries_https_then_plain_http_only_at_home() {
        // a local server answering only plain http
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let _ = req.respond(tiny_http::Response::from_string("hi"));
            }
        });
        assert_eq!(pick_scheme(&format!("127.0.0.1:{port}")).unwrap(), format!("http://127.0.0.1:{port}"));
        assert_eq!(pick_scheme(&format!("http://127.0.0.1:{port}/")).unwrap(), format!("http://127.0.0.1:{port}"));
        // a server out on the internet never falls back to plain http (.invalid never resolves)
        assert_eq!(pick_scheme("music.example.invalid:4533").unwrap(), "https://music.example.invalid:4533");
    }

    #[test]
    fn redirects_never_carry_sign_in_to_another_server() {
        let u = |s: &str| reqwest::Url::parse(s).unwrap();
        assert!(same_server(&u("http://nas:8096/a"), &u("http://nas:8096/b")));
        assert!(same_server(&u("http://nas/a"), &u("https://nas/a")), "an upgrade to https is fine");
        assert!(!same_server(&u("https://nas/a"), &u("http://nas/a")), "never down to plain http");
        assert!(!same_server(&u("https://nas/a"), &u("https://cdn.example.com/a")));
        assert!(!same_server(&u("http://nas:8096/a"), &u("http://nas:9000/a")));

        // a "server" that sends covers on to another host, which records the headers it gets
        let other = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let other_port = other.server_addr().to_ip().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for req in other.incoming_requests() {
                let _ = tx.send(req.headers().iter().map(|h| h.field.to_string().to_ascii_lowercase()).collect::<Vec<_>>());
                let _ = req.respond(tiny_http::Response::from_data(vec![7u8; 200]));
            }
        });
        let first = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = first.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            for req in first.incoming_requests() {
                // "localhost" is another host name than 127.0.0.1
                let to = format!("http://localhost:{other_port}/cover.jpg");
                let _ = req.respond(tiny_http::Response::empty(302).with_header(tiny_http::Header::from_bytes("Location", to.as_bytes()).unwrap()));
            }
        });
        let t = Target {
            url: format!("http://127.0.0.1:{port}/Items/1/Images/Primary"),
            headers: vec![("X-Emby-Token".into(), "secret".into()), ("X-Plex-Token".into(), "secret".into()), ("Authorization".into(), "Basic x".into())],
        };
        assert_eq!(fetch(&t).unwrap().len(), 200, "the cover still arrives");
        let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        for h in ["x-emby-token", "x-plex-token", "authorization", "x-emby-authorization"] {
            assert!(!got.contains(&h.to_string()), "{h} reached the other server: {got:?}");
        }
        // an API call with sign-in stops instead
        let err = get_json(&t.url, &t.headers, "Home").unwrap_err().to_string();
        assert!(err.contains("another server"), "{err}");
    }

    #[test]
    fn connections_keep_their_details() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(d.path()).unwrap();
        let c = l.add_connection("plex", "Home", "http://nas:32400", "", &serde_json::json!({"machine": "abc"})).unwrap();
        assert_eq!((c.kind.as_str(), c.extra["machine"].as_str()), ("plex", Some("abc")));
        assert!(l.add_connection("ftp", "x", "http://x", "", &Value::Null).is_err());
        let c = l.set_connection_extra(c.id, "inLibrary", Value::Bool(true)).unwrap();
        assert_eq!((c.extra["machine"].as_str(), c.extra["inLibrary"].as_bool()), (Some("abc"), Some(true)), "other details are kept");
        l.remove_connection(c.id).unwrap();
        assert!(l.connections().unwrap().is_empty());
        assert!(l.set_connection_extra(c.id, "inLibrary", Value::Bool(true)).is_err());
    }
}
