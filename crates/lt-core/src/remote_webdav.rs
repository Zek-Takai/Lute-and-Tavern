//! Plain folders over WebDAV: Nextcloud and ownCloud, the file sharing built into most NAS systems, rclone,
//! and cloud drives that offer WebDAV. Folders are browsed as they are; in the Lute & Tavern layout
//! (Albums/Artist/Album, Songs/Title) the artist and album come from the folder names. Search walks the
//! folders once and keeps the list for a while.

use crate::remote::{Connection, MusicServer, RemoteItem, RemoteSearch, Target, http};
use anyhow::{Result, anyhow, bail};
use base64::Engine;
use quick_xml::events::Event;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const AUDIO: &[&str] = &["mp3", "m4a", "aac", "flac", "wav", "ogg", "oga", "opus", "aif", "aiff", "alac"];
const COVERS: &[&str] = &["cover.jpg", "cover.png", "folder.jpg", "folder.png", "front.jpg", "albumart.jpg"];
/// Folders walked at most when building the search list.
const MAX_FOLDERS: usize = 5000;
const INDEX_FOR: Duration = Duration::from_secs(15 * 60);

static INDEX: Mutex<Option<HashMap<i64, (Instant, Vec<RemoteItem>)>>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    /// Path as the server gives it (percent-encoded)
    href: String,
    dir: bool,
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
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>"#;
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
        let entries = parse_multistatus(&res.text()?);
        // drop the folder itself from its own listing
        let me = url.path().trim_end_matches('/').to_string();
        Ok(entries.into_iter().filter(|e| e.href.trim_end_matches('/') != me).collect())
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

    /// Every folder and song, walked once and kept for a while (search).
    fn index(&self) -> Result<Vec<RemoteItem>> {
        if let Some((at, items)) = INDEX.lock().unwrap().as_ref().and_then(|m| m.get(&self.conn.id)) {
            if at.elapsed() < INDEX_FOR {
                return Ok(items.clone());
            }
        }
        let mut all = Vec::new();
        let mut todo = vec![self.root.path().to_string()];
        let mut seen = 0;
        while let Some(dir) = todo.pop() {
            seen += 1;
            if seen > MAX_FOLDERS {
                break;
            }
            let entries = self.list(&dir, "1")?;
            for it in self.to_items(&entries) {
                if it.kind == "folder" {
                    todo.push(it.id.clone());
                }
                all.push(it);
            }
        }
        INDEX.lock().unwrap().get_or_insert_with(HashMap::new).insert(self.conn.id, (Instant::now(), all.clone()));
        Ok(all)
    }
}

/// Last path segment, decoded ("Lute%20%26%20Tavern/" → "Lute & Tavern").
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

/// Reads a PROPFIND answer: each response's href and whether it is a folder.
fn parse_multistatus(xml: &str) -> Vec<Entry> {
    let mut r = quick_xml::Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let (mut out, mut href, mut dir, mut in_href) = (Vec::new(), String::new(), false, false);
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                "response" => (href, dir) = (String::new(), false),
                "href" => in_href = true,
                "collection" => dir = true,
                _ => {}
            },
            Ok(Event::Empty(e)) if e.local_name().as_ref() == "collection" => dir = true,
            Ok(Event::Text(t)) if in_href => href.push_str(&t.xml10_content()),
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
                "response" if !href.is_empty() => {
                    // some servers send full URLs; keep only the path
                    let path = reqwest::Url::parse(&href).map(|u| u.path().to_string()).unwrap_or_else(|_| href.clone());
                    out.push(Entry { href: path, dir });
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
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
        let all = self.index()?;
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
        let e = parse_multistatus(xml);
        assert_eq!(e, vec![
            Entry { href: "/dav/Music/".into(), dir: true },
            Entry { href: "/dav/Music/Lute%20%26%20Tavern/".into(), dir: true },
            Entry { href: "/dav/Music/a&b.mp3".into(), dir: false },
        ]);
        assert_eq!(decoded_name("/dav/Music/Lute%20%26%20Tavern/"), "Lute & Tavern");
        let s = song_from_path("/dav/Albums/Linkin%20Park/Meteora/13%20-%20Numb.mp3", None);
        assert_eq!((s.name.as_str(), s.artist.as_str(), s.album.as_str(), s.track), ("Numb", "Linkin Park", "Meteora", Some(13)));
        let s = song_from_path("/dav/Songs/Porch%20Light/Porch%20Light.m4a", None);
        assert_eq!((s.name.as_str(), s.album.as_str()), ("Porch Light", ""));
    }
}
