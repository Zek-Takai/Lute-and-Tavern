//! Playlist import from any common format, and matching its entries to songs in the library (local files or
//! songs streamed from a connected server). Formats: M3U/M3U8, PLS, XSPF, WPL, iTunes / Apple Music XML (a
//! whole library or one exported playlist), Apple Music "Export Playlist" text (UTF-16, tab separated) and
//! CSV (e.g. Exportify). Entries are matched by file first, then by title, artist, album and length, with
//! forgiving comparison so renamed or retagged songs still match.

use crate::Song;
use crate::library::Library;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlEntry {
    pub path: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedPlaylist {
    pub name: String,
    pub entries: Vec<PlEntry>,
}

// ---------- reading ----------

/// Reads a playlist file; an iTunes library file gives all of its playlists.
pub fn parse_file(path: &Path) -> Result<Vec<ParsedPlaylist>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Playlist").to_string();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let text = decode(&bytes);
    let head = text.trim_start().get(..400).unwrap_or(text.trim_start()).to_lowercase();
    let lists = if head.contains("<plist") || ext == "plist" {
        parse_itunes(&bytes)?
    } else if head.contains("<playlist") && head.contains("xspf") || ext == "xspf" {
        vec![parse_xspf(&text, &name)]
    } else if head.contains("<smil") || ext == "wpl" || ext == "zpl" {
        vec![parse_wpl(&text, &name)]
    } else if head.starts_with("[playlist]") || ext == "pls" {
        vec![parse_pls(&text, &name)]
    } else if ext == "m3u" || ext == "m3u8" || head.starts_with("#extm3u") {
        vec![parse_m3u(&text, &name)]
    } else if ext == "txt" || ext == "csv" || ext == "tsv" {
        vec![parse_table(&text, &name)?]
    } else {
        bail!("{} isn't a playlist format I know (M3U, PLS, XSPF, WPL, iTunes XML, TXT or CSV)", path.display())
    };
    Ok(lists.into_iter().filter(|l| !l.entries.is_empty()).collect())
}

/// UTF-8 or UTF-16 (iTunes/Apple Music text exports) by BOM; falls back to Windows-1252 for old M3U files.
fn decode(b: &[u8]) -> String {
    if let Some((enc, _)) = encoding_rs::Encoding::for_bom(b) {
        return enc.decode(b).0.into_owned();
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(b).0.into_owned(),
    }
}

/// "file://localhost/C:/Users/x/Music/a%20b.mp3" or "C:\\Music\\a b.mp3" → a plain path with forward slashes.
fn clean_path(p: &str) -> String {
    let p = p.trim();
    let p = p.strip_prefix("file://localhost").or_else(|| p.strip_prefix("file://")).unwrap_or(p);
    let decoded = percent_decode(p);
    decoded.replace('\\', "/")
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

/// Title (and track artist/album from folders) guessed from a path like ".../Artist/Album/03 - Title.mp3".
fn from_path(path: &str) -> PlEntry {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let file = parts.last().copied().unwrap_or("");
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
    let title = strip_track_number(stem);
    let n = parts.len();
    PlEntry {
        path: Some(path.to_string()),
        title,
        album: if n >= 2 { parts[n - 2].to_string() } else { String::new() },
        artist: if n >= 3 { parts[n - 3].to_string() } else { String::new() },
        duration: None,
    }
}

fn strip_track_number(stem: &str) -> String {
    let t = stem.trim();
    let digits = t.chars().take_while(|c| c.is_ascii_digit() || *c == '-').count();
    if digits > 0 && digits <= 5 {
        let rest = t[digits..].trim_start_matches([' ', '-', '.', '_']).trim();
        if !rest.is_empty() {
            return rest.to_string();
        }
    }
    t.to_string()
}

pub fn parse_m3u(text: &str, name: &str) -> ParsedPlaylist {
    let mut out = ParsedPlaylist { name: name.into(), entries: vec![] };
    let mut pending: Option<PlEntry> = None;
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("#PLAYLIST:") {
            out.name = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (secs, label) = rest.split_once(',').unwrap_or((rest, ""));
            let (artist, title) = label.split_once(" - ").map(|(a, t)| (a.trim(), t.trim())).unwrap_or(("", label.trim()));
            pending = Some(PlEntry { duration: secs.trim().parse::<f64>().ok().filter(|d| *d > 0.0), artist: artist.into(), title: title.into(), ..Default::default() });
        } else if let Some(rest) = line.strip_prefix("#EXTALB:") {
            if let Some(p) = pending.as_mut() {
                p.album = rest.trim().into();
            }
        } else if let Some(rest) = line.strip_prefix("#LT:") {
            // our own export: exact tags for other devices
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(rest) {
                let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
                pending = Some(PlEntry { title: s("title"), artist: s("artist"), album: s("album"), duration: v["duration"].as_f64(), path: None });
            }
        } else if !line.is_empty() && !line.starts_with('#') {
            let path = clean_path(line);
            let guessed = from_path(&path);
            let mut e = pending.take().unwrap_or_default();
            if e.title.is_empty() {
                e.title = guessed.title;
            }
            if e.album.is_empty() {
                e.album = guessed.album;
            }
            if e.artist.is_empty() {
                e.artist = guessed.artist;
            }
            e.path = Some(path);
            out.entries.push(e);
        }
    }
    out
}

fn parse_pls(text: &str, name: &str) -> ParsedPlaylist {
    let mut by: HashMap<u32, PlEntry> = HashMap::new();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let k = k.trim().to_lowercase();
        let num = |p: &str| k.strip_prefix(p).and_then(|n| n.parse::<u32>().ok());
        if let Some(n) = num("file") {
            let path = clean_path(v);
            let g = from_path(&path);
            let e = by.entry(n).or_default();
            e.path = Some(path);
            if e.title.is_empty() {
                e.title = g.title;
            }
        } else if let Some(n) = num("title") {
            let (a, t) = v.split_once(" - ").map(|(a, t)| (a.trim(), t.trim())).unwrap_or(("", v.trim()));
            let e = by.entry(n).or_default();
            e.title = t.into();
            e.artist = a.into();
        } else if let Some(n) = num("length") {
            by.entry(n).or_default().duration = v.trim().parse::<f64>().ok().filter(|d| *d > 0.0);
        }
    }
    let mut keys: Vec<u32> = by.keys().copied().collect();
    keys.sort();
    ParsedPlaylist { name: name.into(), entries: keys.into_iter().filter_map(|k| by.remove(&k)).collect() }
}

/// Text between <tag> and </tag> (first match), unescaped.
fn xml_tag(block: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    block.find(&open).and_then(|i| {
        let rest = &block[i + open.len()..];
        rest.find(&close).map(|j| unescape(&rest[..j]))
    }).unwrap_or_default()
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&").trim().to_string()
}

fn parse_xspf(text: &str, name: &str) -> ParsedPlaylist {
    let title = xml_tag(text.split("<trackList").next().unwrap_or(""), "title");
    let entries = text
        .split("<track>")
        .skip(1)
        .map(|t| {
            let t = t.split("</track>").next().unwrap_or(t);
            let loc = xml_tag(t, "location");
            let mut e = PlEntry {
                title: xml_tag(t, "title"),
                artist: xml_tag(t, "creator"),
                album: xml_tag(t, "album"),
                duration: xml_tag(t, "duration").parse::<f64>().ok().map(|ms| ms / 1000.0),
                path: None,
            };
            if !loc.is_empty() {
                let path = clean_path(&loc);
                if e.title.is_empty() {
                    e.title = from_path(&path).title;
                }
                e.path = Some(path);
            }
            e
        })
        .collect();
    ParsedPlaylist { name: if title.is_empty() { name.into() } else { title }, entries }
}

fn parse_wpl(text: &str, name: &str) -> ParsedPlaylist {
    let title = xml_tag(text, "title");
    let entries = text
        .split("<media ")
        .skip(1)
        .filter_map(|m| {
            let src = m.split("src=\"").nth(1)?.split('"').next()?;
            Some(from_path(&clean_path(&unescape(src))))
        })
        .collect();
    ParsedPlaylist { name: if title.is_empty() { name.into() } else { title }, entries }
}

/// iTunes / Apple Music XML: a whole library (Tracks + Playlists) or one exported playlist (same shape).
fn parse_itunes(bytes: &[u8]) -> Result<Vec<ParsedPlaylist>> {
    let v: plist::Value = plist::from_bytes(bytes).context("reading the iTunes XML")?;
    let root = v.as_dictionary().context("not an iTunes playlist file")?;
    let tracks = root.get("Tracks").and_then(|t| t.as_dictionary());
    let track = |id: i64| -> Option<PlEntry> {
        let t = tracks?.get(&id.to_string())?.as_dictionary()?;
        let s = |k: &str| t.get(k).and_then(|v| v.as_string()).unwrap_or("").to_string();
        let ms = t.get("Total Time").and_then(|v| v.as_signed_integer());
        let loc = s("Location");
        Some(PlEntry {
            title: s("Name"),
            artist: Some(s("Artist")).filter(|a| !a.is_empty()).unwrap_or_else(|| s("Album Artist")),
            album: s("Album"),
            duration: ms.map(|m| m as f64 / 1000.0),
            path: (!loc.is_empty()).then(|| clean_path(&loc)),
        })
    };
    let mut out = Vec::new();
    for p in root.get("Playlists").and_then(|p| p.as_array()).into_iter().flatten() {
        let Some(p) = p.as_dictionary() else { continue };
        let flag = |k: &str| p.get(k).and_then(|v| v.as_boolean()).unwrap_or(false);
        // skip the whole-library list, iTunes' built-in lists (Music, Movies, Podcasts...) and folders
        if flag("Master") || flag("Folder") || p.contains_key("Distinguished Kind") {
            continue;
        }
        let name = p.get("Name").and_then(|v| v.as_string()).unwrap_or("Playlist").to_string();
        let entries = p
            .get("Playlist Items")
            .and_then(|i| i.as_array())
            .into_iter()
            .flatten()
            .filter_map(|it| it.as_dictionary()?.get("Track ID")?.as_signed_integer())
            .filter_map(|id| track(id))
            .collect();
        out.push(ParsedPlaylist { name, entries });
    }
    Ok(out)
}

/// Apple Music / iTunes "Export Playlist" text (tab separated) and CSV exports (Exportify and similar).
fn parse_table(text: &str, name: &str) -> Result<ParsedPlaylist> {
    let first = text.lines().next().unwrap_or("");
    let delim = if first.matches('\t').count() >= first.matches(',').count() && first.contains('\t') { '\t' } else { ',' };
    let rows: Vec<Vec<String>> = text.lines().filter(|l| !l.trim().is_empty()).map(|l| split_row(l, delim)).collect();
    let Some(header) = rows.first() else { bail!("the file is empty") };
    let col = |names: &[&str]| header.iter().position(|h| names.iter().any(|n| h.trim().eq_ignore_ascii_case(n)));
    let title = col(&["Name", "Title", "Track Name", "Song", "Track"]).context("no title column (Name, Title or Track Name)")?;
    let artist = col(&["Artist", "Artist Name(s)", "Artists", "Artist Name"]);
    let album = col(&["Album", "Album Name"]);
    let time = col(&["Time", "Duration", "Duration (ms)", "Length", "Track Duration (ms)"]);
    let ms = header.get(time.unwrap_or(usize::MAX)).is_some_and(|h| h.to_lowercase().contains("ms"));
    let loc = col(&["Location", "Path", "File"]);
    let get = |r: &Vec<String>, c: Option<usize>| c.and_then(|c| r.get(c)).map(|s| s.trim().to_string()).unwrap_or_default();
    let entries = rows[1..]
        .iter()
        .map(|r| {
            let d = get(r, time);
            let duration = if let Some((m, s)) = d.split_once(':') { Some(m.parse::<f64>().unwrap_or(0.0) * 60.0 + s.parse::<f64>().unwrap_or(0.0)) } else { d.parse::<f64>().ok().map(|x| if ms { x / 1000.0 } else { x }) };
            let l = get(r, loc);
            PlEntry {
                title: get(r, Some(title)),
                artist: get(r, artist).split([',', ';']).next().unwrap_or("").trim().to_string(),
                album: get(r, album),
                duration: duration.filter(|d| *d > 0.0),
                path: (!l.is_empty()).then(|| clean_path(&l)),
            }
        })
        .filter(|e| !e.title.is_empty())
        .collect();
    Ok(ParsedPlaylist { name: name.into(), entries })
}

fn split_row(line: &str, delim: char) -> Vec<String> {
    if delim == '\t' {
        return line.split('\t').map(String::from).collect();
    }
    let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == delim && !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

// ---------- matching ----------

/// Lowercase words without punctuation, "the", featured artists, and bracketed extras ("(Radio)", "[Live]").
fn words(s: &str) -> Vec<String> {
    let mut t = s.to_lowercase();
    for cut in [" feat.", " feat ", " ft.", " featuring "] {
        if let Some(i) = t.find(cut) {
            t.truncate(i);
        }
    }
    let mut clean = String::new();
    let mut depth = 0;
    for c in t.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            c if depth == 0 => clean.push(if c.is_alphanumeric() { c } else { ' ' }),
            _ => {}
        }
    }
    let w: Vec<String> = clean.split_whitespace().filter(|w| *w != "the" && *w != "a").map(String::from).collect();
    if w.is_empty() { s.to_lowercase().split_whitespace().map(String::from).collect() } else { w }
}

/// How alike two names are, 0..1 (word overlap; exact match = 1).
fn likeness(a: &str, b: &str) -> f64 {
    let (a, b) = (words(a), words(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let common = a.iter().filter(|w| b.contains(w)).count() as f64;
    let total = a.len().max(b.len()) as f64;
    // one title fully inside the other ("Be Like That" / "If I Could Be Like That"): a strong hint
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    if short.len() >= 2 && long.windows(short.len()).any(|w| w == short.as_slice()) {
        return 0.8_f64.max(common / total);
    }
    // also count near-identical spellings ("heros" / "heroes")
    let fuzzy = a.iter().filter(|w| !b.contains(w) && b.iter().any(|x| close(w, x))).count() as f64 * 0.8;
    ((common + fuzzy) / total).min(1.0)
}

fn close(a: &str, b: &str) -> bool {
    if a.len() < 4 || b.len() < 4 {
        return false;
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
        prev = cur;
    }
    prev[b.len()] <= 1 + a.len().max(b.len()) / 8
}

/// The best library song for an entry, if it's a confident match.
pub fn best_match(e: &PlEntry, songs: &[Song]) -> Option<i64> {
    // 1. by file: a path inside one of the library's song folders
    if let Some(p) = &e.path {
        let p = p.to_lowercase();
        if let Some(s) = songs.iter().find(|s| s.song_dir.as_deref().is_some_and(|d| p.starts_with(&d.to_lowercase()))) {
            return Some(s.id);
        }
    }
    // 2. by tags: title must agree; artist, album and length decide between candidates
    let mut best: Option<(f64, i64)> = None;
    for s in songs {
        let mut t = likeness(&e.title, &s.title);
        let same_len = matches!((e.duration, s.duration), (Some(a), Some(b)) if (a - b).abs() <= 1.5);
        if t < 0.6 {
            // A retitled song ("Let the bodies floor" → "Bodies") still matches when the artist is the same,
            // the length agrees to the second, and the titles share a word.
            if t > 0.0 && same_len && !e.artist.is_empty() && likeness(&e.artist, &s.artist) >= 0.9 {
                t = 0.6;
            } else {
                continue;
            }
        }
        let mut score = t * 0.55;
        if t >= 0.8 && same_len {
            score += 0.08; // title nearly the same and the very same length: trust it even if the artist was retagged
        }
        if !e.artist.is_empty() && !s.artist.is_empty() {
            score += likeness(&e.artist, &s.artist) * 0.25;
        } else {
            score += 0.1;
        }
        if !e.album.is_empty() && !s.album.is_empty() {
            score += likeness(&e.album, &s.album) * 0.08;
        }
        if let (Some(a), Some(b)) = (e.duration, s.duration) {
            let d = (a - b).abs();
            score += if d <= 2.0 { 0.12 } else if d <= 6.0 { 0.06 } else if d > 30.0 { -0.2 } else { 0.0 };
        }
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, s.id));
        }
    }
    best.filter(|(score, _)| *score >= 0.62).map(|(_, id)| id)
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub playlist: String,
    pub id: i64,
    pub matched: usize,
    pub total: usize,
    /// Entries that were the same song again (kept once)
    pub duplicates: usize,
    pub missing: Vec<String>,
    /// The missing entries themselves, each with the song it came after (so a later find keeps the order).
    pub missing_entries: Vec<MissingEntry>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingEntry {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Option<f64>,
    pub after: Option<i64>,
}

impl Library {
    /// Imports playlist files; each becomes a playlist of the songs that matched (a name already in use gets " (2)").
    pub fn import_playlists(&self, files: &[std::path::PathBuf]) -> Result<Vec<ImportResult>> {
        // The person's own files and servers first; their saved Apple Music songs when those don't have it.
        let (apple, songs): (Vec<Song>, Vec<Song>) = self.songs()?.into_iter().partition(|s| s.provider == "apple");
        let mut names: Vec<String> = self.playlists()?.into_iter().map(|p| p.name.to_lowercase()).collect();
        let mut out = Vec::new();
        for f in files {
            for pl in parse_file(f)? {
                let mut ids = Vec::new();
                let mut missing = Vec::new();
                let mut missing_entries = Vec::new();
                let mut duplicates = 0;
                for e in &pl.entries {
                    match best_match(e, &songs).or_else(|| best_match(e, &apple)) {
                        Some(id) if !ids.contains(&id) => ids.push(id),
                        Some(_) => duplicates += 1,
                        None => {
                            missing.push(if e.artist.is_empty() { e.title.clone() } else { format!("{} – {}", e.artist, e.title) });
                            missing_entries.push(MissingEntry { title: e.title.clone(), artist: e.artist.clone(), album: e.album.clone(), duration: e.duration, after: ids.last().copied() });
                        }
                    }
                }
                let mut name = pl.name.trim().to_string();
                let base = name.clone();
                let mut n = 2;
                while names.contains(&name.to_lowercase()) {
                    name = format!("{base} ({n})");
                    n += 1;
                }
                names.push(name.to_lowercase());
                let p = self.create_playlist(&name, &ids)?;
                out.push(ImportResult { playlist: name, id: p.id, matched: ids.len(), total: pl.entries.len(), duplicates, missing, missing_entries });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_common_formats() {
        let d = tempfile::tempdir().unwrap();
        let w = |n: &str, s: &str| {
            let p = d.path().join(n);
            std::fs::write(&p, s).unwrap();
            parse_file(&p).unwrap()
        };
        let m = w("Road.m3u8", "#EXTM3U\n#EXTINF:187,Linkin Park - Numb\n../Albums/Linkin Park/Meteora/13 - Numb.mp3\n");
        assert_eq!((m[0].name.as_str(), m[0].entries[0].title.as_str(), m[0].entries[0].artist.as_str(), m[0].entries[0].duration), ("Road", "Numb", "Linkin Park", Some(187.0)));
        let p = w("x.pls", "[playlist]\nFile1=C:\\Music\\Aqua\\Aquarium\\02 Cartoon Heroes.mp3\nTitle1=Aqua - Cartoon Heroes\nLength1=216\nNumberOfEntries=1\n");
        assert_eq!((p[0].entries[0].title.as_str(), p[0].entries[0].artist.as_str()), ("Cartoon Heroes", "Aqua"));
        let x = w("a.xspf", "<?xml version=\"1.0\"?><playlist version=\"1\" xmlns=\"http://xspf.org/ns/0/\"><title>Mix</title><trackList><track><title>Faint</title><creator>Linkin Park</creator><duration>162000</duration></track></trackList></playlist>");
        assert_eq!((x[0].name.as_str(), x[0].entries[0].duration), ("Mix", Some(162.0)));
        let c = w("e.csv", "Track Name,Artist Name(s),Album Name,Duration (ms)\n\"Numb\",\"Linkin Park, Jay-Z\",Meteora,187000\n");
        assert_eq!((c[0].entries[0].artist.as_str(), c[0].entries[0].duration), ("Linkin Park", Some(187.0)));
    }

    #[test]
    fn forgiving_matching() {
        assert!(likeness("Cartoon Heros", "Cartoon Heroes (Radio)") >= 0.9);
        assert!(likeness("Stomach Vs. Heart", "Stomach vs. Heart") == 1.0);
        assert!(likeness("Numb", "Faint") == 0.0);
        assert!(likeness("If I Could Be Like That", "Be Like That") >= 0.8);
        let mk = |id, t: &str, a: &str, d| Song { id, title: t.into(), artist: a.into(), duration: Some(d), ..Default::default() };
        let songs = vec![mk(1, "Numb", "Linkin Park", 187.0), mk(2, "Numb", "U2", 260.0), mk(3, "Cartoon Heroes (Radio)", "Aqua", 216.0)];
        let e = |t: &str, a: &str, d| PlEntry { title: t.into(), artist: a.into(), duration: Some(d), ..Default::default() };
        assert_eq!(best_match(&e("Numb", "Linkin Park", 188.0), &songs), Some(1));
        assert_eq!(best_match(&e("Cartoon Heros", "Aqua", 216.0), &songs), Some(3));
        assert_eq!(best_match(&e("In The End", "Linkin Park", 216.0), &songs), None);
        let more = vec![mk(4, "Bodies", "Drowning Pool", 201.0), mk(5, "Rock Lee's Theme", "Toshio Masuda", 130.0)];
        assert_eq!(best_match(&e("Let the bodies floor", "Drowning Pool", 201.0), &more), Some(4), "retitled, same artist and length");
        assert_eq!(best_match(&e("Let the bodies floor", "Drowning Pool", 260.0), &more), None, "different length: not the same song");
        assert_eq!(best_match(&e("Rock Lee", "Naruto", 130.0), &more), Some(5), "retagged artist, same title words and length");
    }
}
