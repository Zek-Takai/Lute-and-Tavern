//! Plain folders over WebDAV: Nextcloud and ownCloud, the file sharing built into most NAS systems, rclone,
//! and cloud drives that offer WebDAV. Folders are browsed as they are; in the Lute & Tavern layout
//! (Albums/Artist/Album, Songs/Title) the artist and album come from the folder names. Search walks the
//! folders once and keeps the list for a while.

use crate::remote::{Connection, MusicServer, RemoteItem, RemoteSearch, Target, http, net};
use anyhow::{Result, anyhow, bail};
use base64::Engine;
use quick_xml::events::Event;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const AUDIO: &[&str] = &["mp3", "m4a", "aac", "flac", "wav", "ogg", "oga", "opus", "aif", "aiff", "alac"];
const COVERS: &[&str] = &["cover.jpg", "cover.png", "folder.jpg", "folder.png", "front.jpg", "albumart.jpg"];
/// Folders walked at most when building the search list (search can live with part of a huge share).
const MAX_FOLDERS: usize = 5000;
/// Folders walked at most when reading every song for My music. A share bigger than this is refused rather
/// than read in part, because a sync takes away songs missing from the list.
const MAX_FOLDERS_FULL: usize = 100_000;
const INDEX_FOR: Duration = Duration::from_secs(15 * 60);

/// Walked folder lists per connection: when, the items, and whether the walk reached every folder.
static INDEX: Mutex<Option<HashMap<i64, (Instant, Vec<RemoteItem>, bool)>>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    /// Path as the server gives it (percent-encoded)
    href: String,
    dir: bool,
    size: Option<u64>,
}

pub struct WebDav<'a> {
    conn: &'a Connection,
    auth: String,
    root: reqwest::Url,
}

impl<'a> WebDav<'a> {
    pub fn new(conn: &'a Connection, password: &'a str) -> Self {
        let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{}:{password}", conn.username)));
        let mut root = reqwest::Url::parse(&conn.base_url).expect("checked when the connection was added");
        if !root.path().ends_with('/') {
            root.set_path(&format!("{}/", root.path()));
        }
        Self { conn, auth, root }
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![("Authorization".to_string(), self.auth.clone())]
    }

    fn url_of(&self, href: &str) -> Result<reqwest::Url> {
        self.root.join(href).map_err(|e| anyhow!("bad path {href}: {e}"))
    }

    fn list(&self, href: &str, depth: &str) -> Result<Vec<Entry>> {
        let url = self.url_of(href)?;
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/></d:prop></d:propfind>"#;
        let res = http()?
            .request(reqwest::Method::from_bytes(b"PROPFIND")?, url.clone())
            .header("Depth", depth)
            .header("Authorization", &self.auth)
            .header("Content-Type", "application/xml")
            .body(body)
            .send()
            .map_err(|e| anyhow!("Could not reach {}: {}", self.conn.name, e.without_url()))?;
        match res.status().as_u16() {
            207 => {}
            401 | 403 => bail!("{} refused the sign-in. Check the username and password.", self.conn.name),
            404 => bail!("{} has no folder at {}", self.conn.name, url.path()),
            s => bail!("{} answered {s} (is this a WebDAV address?)", self.conn.name),
        }
        let entries = parse_multistatus(&res.text().map_err(net)?).map_err(|e| anyhow!("{} sent a folder listing that couldn't be read ({e})", self.conn.name))?;
        // drop the folder itself from its own listing (the server may encode the path differently)
        Ok(entries.into_iter().filter(|e| !same_path(&e.href, url.path())).collect())
    }

    fn to_items(&self, entries: &[Entry]) -> Vec<RemoteItem> {
        let cover = entries.iter().find(|e| !e.dir && COVERS.contains(&decoded_name(&e.href).to_lowercase().as_str())).map(|e| e.href.clone());
        let mut out: Vec<RemoteItem> = entries
            .iter()
            .filter_map(|e| {
                let name = decoded_name(&e.href);
                if name.starts_with('.') {
                    return None;
                }
                if e.dir {
                    return Some(RemoteItem { id: e.href.clone(), kind: "folder".into(), name, ..Default::default() });
                }
                let ext = name.rsplit_once('.').map(|(_, x)| x.to_lowercase()).unwrap_or_default();
                AUDIO.contains(&ext.as_str()).then(|| song_from_path(&e.href, cover.clone()))
            })
            .collect();
        out.sort_by(|a, b| (a.kind != "folder", a.track, a.name.to_lowercase()).cmp(&(b.kind != "folder", b.track, b.name.to_lowercase())));
        out
    }

    /// Every folder and song, walked once and kept for a while. With `full` the walk must reach every folder
    /// (reading the songs for My music); otherwise it stops after MAX_FOLDERS (search).
    fn index(&self, full: bool) -> Result<Vec<RemoteItem>> {
        if let Some((at, items, complete)) = INDEX.lock().unwrap().as_ref().and_then(|m| m.get(&self.conn.id)) {
            if at.elapsed() < INDEX_FOR && (*complete || !full) {
                return Ok(items.clone());
            }
        }
        let (all, complete) = self.walk(if full { MAX_FOLDERS_FULL } else { MAX_FOLDERS })?;
        INDEX.lock().unwrap().get_or_insert_with(HashMap::new).insert(self.conn.id, (Instant::now(), all.clone(), complete));
        if full && !complete {
            bail!("{} has more than {MAX_FOLDERS_FULL} folders, too many to read for My music. Nothing in My music was changed; connect a smaller folder instead.", self.conn.name);
        }
        Ok(all)
    }

    /// Walks the folders from the top, at most `max` of them, each once. Returns the items and whether it
    /// reached every folder.
    fn walk(&self, max: usize) -> Result<(Vec<RemoteItem>, bool)> {
        let mut all = Vec::new();
        let mut todo = vec![self.root.path().to_string()];
        let mut visited = std::collections::HashSet::new();
        while let Some(dir) = todo.pop() {
            if !visited.insert(norm_path(&dir)) {
                continue;
            }
            if visited.len() > max {
                return Ok((all, false));
            }
            let entries = self.list(&dir, "1")?;
            for it in self.to_items(&entries) {
                if it.kind == "folder" {
                    todo.push(it.id.clone());
                }
                all.push(it);
            }
        }
        Ok((all, true))
    }
}

/// Last path segment, decoded ("Lute%20%26%20Tavern/" → "Lute & Tavern").
/// Writing, for backups: addresses are built from path pieces under the connection's address, so names with
/// spaces, "&" or "#" are encoded correctly.
impl WebDav<'_> {
    /// The address of `pieces` (folder names and a file name) under the connection's address.
    pub fn url_for(&self, pieces: &[&str], dir: bool) -> reqwest::Url {
        let mut u = self.root.clone();
        {
            let mut seg = u.path_segments_mut().expect("http address");
            seg.pop_if_empty();
            for p in pieces.iter().filter(|p| !p.is_empty()) {
                seg.push(p);
            }
            if dir {
                seg.push("");
            }
        }
        u
    }

    fn send(&self, method: &str, url: &reqwest::Url) -> Result<reqwest::blocking::RequestBuilder> {
        Ok(http()?.request(reqwest::Method::from_bytes(method.as_bytes())?, url.clone()).header("Authorization", &self.auth))
    }

    /// Size of a file on the server, `Some(None)` for a folder, `None` when nothing is there.
    pub fn stat(&self, url: &reqwest::Url) -> Result<Option<Option<u64>>> {
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/></d:prop></d:propfind>"#;
        let res = self.send("PROPFIND", url)?.header("Depth", "0").header("Content-Type", "application/xml").body(body).send().map_err(net)?;
        match res.status().as_u16() {
            404 => Ok(None),
            207 => Ok(parse_multistatus(&res.text().map_err(net)?)
                .map_err(|e| anyhow!("{} sent a folder listing that couldn't be read ({e})", self.conn.name))?
                .into_iter()
                .next()
                .map(|e| if e.dir { None } else { e.size })),
            401 | 403 => bail!("{} refused the sign-in", self.conn.name),
            s => bail!("{} answered {s} while checking {}", self.conn.name, url.path()),
        }
    }

    /// Creates a folder and any missing folders above it.
    pub fn make_folders(&self, pieces: &[&str]) -> Result<()> {
        for i in 1..=pieces.len() {
            let u = self.url_for(&pieces[..i], true);
            if self.stat(&u)?.is_some() {
                continue;
            }
            let res = self.send("MKCOL", &u)?.send().map_err(net)?;
            if !(res.status().is_success() || res.status().as_u16() == 405) {
                bail!("{} could not create the folder {} ({})", self.conn.name, u.path(), res.status());
            }
        }
        Ok(())
    }

    /// Uploads a local file (streamed, no size limit) to `url`.
    pub fn upload(&self, url: &reqwest::Url, file: &std::path::Path) -> Result<()> {
        let f = std::fs::File::open(file)?;
        let len = f.metadata()?.len();
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(None)
            .redirect(crate::remote::same_server_redirects())
            .build()?;
        let res = client
            .put(url.clone())
            .header("Authorization", &self.auth)
            .header("Content-Length", len)
            .body(reqwest::blocking::Body::sized(f, len))
            .send()
            .map_err(net)?;
        if !res.status().is_success() {
            bail!("{} refused the upload of {} ({})", self.conn.name, file.display(), res.status());
        }
        Ok(())
    }

    /// A small file's contents, or None when it isn't there.
    pub fn get_bytes(&self, url: &reqwest::Url) -> Result<Option<Vec<u8>>> {
        let res = self.send("GET", url)?.send().map_err(net)?;
        match res.status().as_u16() {
            404 => Ok(None),
            s if (200..300).contains(&s) => Ok(Some(res.bytes().map_err(net)?.to_vec())),
            s => bail!("{} answered {s} for {}", self.conn.name, url.path()),
        }
    }

    pub fn put_bytes(&self, url: &reqwest::Url, bytes: Vec<u8>) -> Result<()> {
        let res = self.send("PUT", url)?.body(bytes).send().map_err(net)?;
        if !res.status().is_success() {
            bail!("{} refused to save {} ({})", self.conn.name, url.path(), res.status());
        }
        Ok(())
    }

    /// Deletes one file (used only for playlist files the person deleted).
    pub fn delete(&self, url: &reqwest::Url) -> Result<()> {
        let res = self.send("DELETE", url)?.send().map_err(net)?;
        if !(res.status().is_success() || res.status().as_u16() == 404) {
            bail!("{} could not delete {} ({})", self.conn.name, url.path(), res.status());
        }
        Ok(())
    }

    /// Moves a file on the server (never overwrites).
    pub fn move_to(&self, from: &reqwest::Url, to: &reqwest::Url) -> Result<()> {
        let res = self.send("MOVE", from)?.header("Destination", to.as_str()).header("Overwrite", "F").send().map_err(net)?;
        if !res.status().is_success() {
            bail!("{} could not move {} ({})", self.conn.name, from.path(), res.status());
        }
        Ok(())
    }
}

/// A path decoded and without a trailing slash, for comparing paths the server and this app encode differently.
fn norm_path(p: &str) -> String {
    percent_decode(p.trim_end_matches('/'))
}

fn same_path(a: &str, b: &str) -> bool {
    norm_path(a) == norm_path(b)
}

fn decoded_name(href: &str) -> String {
    let last = href.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    percent_decode(last)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A song from its path: "Albums/Artist/Album/03 - Title.flac" or "Songs/Title/Title.m4a" or anything else.
fn song_from_path(href: &str, cover: Option<String>) -> RemoteItem {
    let parts: Vec<String> = href.trim_end_matches('/').split('/').filter(|p| !p.is_empty()).map(percent_decode).collect();
    let file = parts.last().cloned().unwrap_or_default();
    let stem = file.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(file);
    let (track, title) = match stem.split_once(" - ") {
        Some((n, t)) if n.trim().parse::<i64>().is_ok() => (n.trim().parse().ok(), t.to_string()),
        _ => (None, stem),
    };
    let n = parts.len();
    let at = |k: usize| if n > k { parts[n - 1 - k].clone() } else { String::new() };
    let (artist, album) = if n >= 4 && parts[n - 4] == "Albums" { (at(2), at(1)) } else if n >= 3 && parts[n - 3] == "Songs" { (String::new(), String::new()) } else { (String::new(), at(1)) };
    RemoteItem { id: href.to_string(), kind: "song".into(), name: title, artist, album, track, cover, ..Default::default() }
}

/// Reads a PROPFIND answer: each response's href and whether it is a folder. An answer that isn't readable
/// XML is an error, never a shorter list (a sync would take the missing songs away).
fn parse_multistatus(xml: &str) -> std::result::Result<Vec<Entry>, quick_xml::Error> {
    let mut r = quick_xml::Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let (mut out, mut href, mut dir, mut in_href) = (Vec::new(), String::new(), false, false);
    let (mut size, mut in_len) = (None, false);
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                "response" => (href, dir, size) = (String::new(), false, None),
                "href" => in_href = true,
                "getcontentlength" => in_len = true,
                "collection" => dir = true,
                _ => {}
            },
            Ok(Event::Empty(e)) if e.local_name().as_ref() == "collection" => dir = true,
            Ok(Event::Text(t)) if in_href => href.push_str(&t.xml10_content()),
            Ok(Event::Text(t)) if in_len => size = t.xml10_content().trim().parse().ok(),
            Ok(Event::GeneralRef(e)) if in_href => href.push_str(match &*e {
                "amp" => "&",
                "lt" => "<",
                "gt" => ">",
                "quot" => "\"",
                "apos" => "'",
                _ => "",
            }),
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                "href" => in_href = false,
                "getcontentlength" => in_len = false,
                "response" if !href.is_empty() => {
                    // some servers send full URLs; keep only the path
                    let path = reqwest::Url::parse(&href).map(|u| u.path().to_string()).unwrap_or_else(|_| href.clone());
                    out.push(Entry { href: path, dir, size });
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(e),
            _ => {}
        }
    }
    Ok(out)
}

impl MusicServer for WebDav<'_> {
    fn ping(&self) -> Result<String> {
        let n = self.list(self.root.path(), "1")?.len();
        Ok(format!("WebDAV folder ({n} items at the top)"))
    }

    fn search(&self, q: &str) -> Result<RemoteSearch> {
        let words: Vec<String> = q.to_lowercase().split_whitespace().map(String::from).collect();
        let hit = |i: &RemoteItem| {
            let hay = format!("{} {} {}", i.name, i.artist, i.album).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        };
        let all = self.index(false)?;
        Ok(RemoteSearch {
            songs: all.iter().filter(|i| i.kind == "song" && hit(i)).take(60).cloned().collect(),
            folders: all.iter().filter(|i| i.kind == "folder" && hit(i)).take(30).cloned().collect(),
            ..Default::default()
        })
    }

    fn home(&self) -> Result<RemoteSearch> {
        let items = self.to_items(&self.list(self.root.path(), "1")?);
        Ok(RemoteSearch {
            songs: items.iter().filter(|i| i.kind == "song").cloned().collect(),
            folders: items.into_iter().filter(|i| i.kind == "folder").collect(),
            ..Default::default()
        })
    }

    fn open(&self, kind: &str, id: &str) -> Result<Vec<RemoteItem>> {
        if kind != "folder" {
            bail!("Can't open a {kind}");
        }
        Ok(self.to_items(&self.list(id, "1")?))
    }

    fn songs_of(&self, kind: &str, id: &str) -> Result<Vec<RemoteItem>> {
        Ok(self.open(kind, id)?.into_iter().filter(|i| i.kind == "song").collect())
    }

    fn all_songs(&self) -> Result<Vec<RemoteItem>> {
        Ok(self.index(true)?.into_iter().filter(|i| i.kind == "song").collect())
    }

    fn stream(&self, song_id: &str) -> Result<Target> {
        Ok(Target { url: self.url_of(song_id)?.to_string(), headers: self.headers() })
    }

    fn cover(&self, cover: &str, _size: u32) -> Option<Target> {
        Some(Target { url: self.url_of(cover).ok()?.to_string(), headers: self.headers() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_listings_and_song_layouts() {
        let xml = r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:">
          <D:response><D:href>/dav/Music/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response>
          <D:response><D:href>/dav/Music/Lute%20%26%20Tavern/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response>
          <D:response><D:href>https://nas/dav/Music/a&amp;b.mp3</D:href><D:propstat><D:prop><D:resourcetype/></D:prop></D:propstat></D:response>
        </D:multistatus>"#;
        let e = parse_multistatus(xml).unwrap();
        assert_eq!(e, vec![
            Entry { href: "/dav/Music/".into(), dir: true, size: None },
            Entry { href: "/dav/Music/Lute%20%26%20Tavern/".into(), dir: true, size: None },
            Entry { href: "/dav/Music/a&b.mp3".into(), dir: false, size: None },
        ]);
        assert_eq!(decoded_name("/dav/Music/Lute%20%26%20Tavern/"), "Lute & Tavern");
        let s = song_from_path("/dav/Albums/Linkin%20Park/Meteora/13%20-%20Numb.mp3", None);
        assert_eq!((s.name.as_str(), s.artist.as_str(), s.album.as_str(), s.track), ("Numb", "Linkin Park", "Meteora", Some(13)));
        let s = song_from_path("/dav/Songs/Porch%20Light/Porch%20Light.m4a", None);
        assert_eq!((s.name.as_str(), s.album.as_str()), ("Porch Light", ""));
    }

    #[test]
    fn broken_listings_are_errors_not_short_lists() {
        let xml = r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:">
          <D:response><D:href>/dav/Music/a.mp3</D:href></D:response>
          <D:response><D:href>/dav/Music/b.mp3</D:wrong></D:response>"#;
        assert!(parse_multistatus(xml).is_err());
    }

    #[test]
    fn a_folder_is_not_listed_inside_itself_however_it_is_encoded() {
        // reqwest keeps "(" and ")" as they are; Nextcloud encodes them
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let walks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let w = walks.clone();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                w.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let body = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">
                  <d:response><d:href>/dav/Music%20%28old%29/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
                  <d:response><d:href>/dav/Music%20%28old%29/Numb.mp3</d:href><d:propstat><d:prop><d:resourcetype/></d:prop></d:propstat></d:response>
                </d:multistatus>"#;
                let _ = req.respond(tiny_http::Response::from_string(body).with_status_code(207));
            }
        });
        let c = Connection { id: 991, kind: "webdav".into(), name: "NAS".into(), base_url: format!("http://127.0.0.1:{port}/dav/Music (old)"), username: "u".into(), extra: serde_json::json!({}) };
        let d = WebDav::new(&c, "p");
        let home = d.home().unwrap();
        assert!(home.folders.is_empty(), "{:?}", home.folders);
        assert_eq!(d.all_songs().unwrap().len(), 1);
        assert_eq!(walks.load(std::sync::atomic::Ordering::SeqCst), 2, "home once, the walk once");
    }

    #[test]
    fn a_walk_that_hits_the_folder_limit_says_so() {
        // every folder holds two more folders, forever
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let me = req.url().trim_end_matches('/').to_string();
                let body = format!(r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">
                  <d:response><d:href>{me}/a/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
                  <d:response><d:href>{me}/b/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
                  <d:response><d:href>{me}/s.mp3</d:href><d:propstat><d:prop><d:resourcetype/></d:prop></d:propstat></d:response>
                </d:multistatus>"#);
                let _ = req.respond(tiny_http::Response::from_string(body).with_status_code(207));
            }
        });
        let c = Connection { id: 992, kind: "webdav".into(), name: "NAS".into(), base_url: format!("http://127.0.0.1:{port}/m"), username: "u".into(), extra: serde_json::json!({}) };
        let d = WebDav::new(&c, "p");
        let (items, complete) = d.walk(20).unwrap();
        assert!(!complete && !items.is_empty());
    }
}
