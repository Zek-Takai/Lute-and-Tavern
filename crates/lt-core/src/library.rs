//! The library: songs, watched folders, playlists, sheets, takes and settings, all in one SQLite file.
//! Files the app keeps (cover art, sheet uploads, takes) are copied into the data folder:
//!
//! ```text
//! <data>/library.db
//! <data>/art/<sha1>.<ext>
//! <data>/sheets/<song id>/<file>
//! <data>/takes/<song id>/<file>
//! ```

use crate::{db, genre, tags};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct DataDirs {
    pub root: PathBuf,
    pub art: PathBuf,
    pub sheets: PathBuf,
    pub takes: PathBuf,
}

impl DataDirs {
    pub fn new(root: &Path) -> Result<Self> {
        let d = Self { root: root.into(), art: root.join("art"), sheets: root.join("sheets"), takes: root.join("takes") };
        for p in [&d.root, &d.art, &d.sheets, &d.takes] {
            std::fs::create_dir_all(p).with_context(|| format!("creating {}", p.display()))?;
        }
        Ok(d)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    pub id: i64,
    pub source: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    pub track: Option<i64>,
    pub duration: Option<f64>,
    pub has_audio: bool,
    pub art: Option<String>,
    pub added_at: i64,
    pub updated_at: i64,
    pub request: Option<String>,
    pub status: Option<String>,
    pub sub_genre: String,
    pub key: String,
    pub bpm: i64,
    pub style: String,
    pub exclude: String,
    pub lyrics: String,
    pub master_take: Option<i64>,
    pub notes: serde_json::Value,
    pub chords: Vec<String>,
}

const SONG_COLS: &str = "s.id, s.source, s.title, s.artist, s.album, s.year, s.genre, s.track, s.duration, \
    (s.path IS NOT NULL OR s.master_take IS NOT NULL OR EXISTS (SELECT 1 FROM takes t WHERE t.song_id = s.id)), \
    s.art, s.added_at, s.updated_at, s.request, s.status, s.sub_genre, s.song_key, s.bpm, s.style, s.exclude, s.lyrics, \
    s.master_take, s.notes_json, s.chords_json";

fn song_from_row(r: &Row) -> rusqlite::Result<Song> {
    let notes: String = r.get(22)?;
    let chords: String = r.get(23)?;
    Ok(Song {
        id: r.get(0)?,
        source: r.get(1)?,
        title: r.get(2)?,
        artist: r.get(3)?,
        album: r.get(4)?,
        year: r.get(5)?,
        genre: r.get(6)?,
        track: r.get(7)?,
        duration: r.get(8)?,
        has_audio: r.get(9)?,
        art: r.get(10)?,
        added_at: r.get(11)?,
        updated_at: r.get(12)?,
        request: r.get(13)?,
        status: r.get(14)?,
        sub_genre: r.get(15)?,
        key: r.get(16)?,
        bpm: r.get(17)?,
        style: r.get(18)?,
        exclude: r.get(19)?,
        lyrics: r.get(20)?,
        master_take: r.get(21)?,
        notes: serde_json::from_str(&notes).unwrap_or(serde_json::Value::Array(vec![])),
        chords: serde_json::from_str(&chords).unwrap_or_default(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Take {
    pub id: i64,
    pub name: String,
    pub origin: String,
    pub added_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub id: i64,
    pub label: String,
    pub note: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub role: String,
    pub text: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sheet {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub added_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongDetail {
    #[serde(flatten)]
    pub song: Song,
    pub takes: Vec<Take>,
    pub versions: Vec<Version>,
    pub messages: Vec<Message>,
    pub sheets: Vec<Sheet>,
    pub playlists: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub songs: Vec<i64>,
}

/// A genre tag. `origin` says who made it: a default, the person, the songwriter, or a music file's own tag.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenreTag {
    pub id: i64,
    pub name: String,
    pub origin: String,
    pub songs: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Folder {
    pub id: i64,
    pub path: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub untagged: usize,
}

/// Fields a person can edit on any song.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SongPatch {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<String>,
    pub genre: Option<String>,
}

/// What the songwriter hands back for a song.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Draft {
    pub title: String,
    pub genre: String,
    pub sub_genre: String,
    pub key: String,
    pub bpm: i64,
    pub style: String,
    pub exclude: String,
    pub lyrics: String,
    /// Optional hook melody in the app's sheet format
    pub notes: Option<serde_json::Value>,
    pub chords: Option<Vec<String>>,
}

pub struct Library {
    conn: Mutex<Connection>,
    pub dirs: DataDirs,
}

impl Library {
    pub fn open(data_dir: &Path) -> Result<Self> {
        let dirs = DataDirs::new(data_dir)?;
        let conn = db::open(&dirs.root.join("library.db"))?;
        Ok(Self { conn: Mutex::new(conn), dirs })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---------- songs ----------

    pub fn songs(&self) -> Result<Vec<Song>> {
        let db = self.db();
        let mut st = db.prepare(&format!("SELECT {SONG_COLS} FROM songs s ORDER BY s.added_at DESC, s.id DESC"))?;
        let rows = st.query_map([], song_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn song(&self, id: i64) -> Result<Song> {
        let db = self.db();
        db.query_row(&format!("SELECT {SONG_COLS} FROM songs s WHERE s.id = ?1"), [id], song_from_row)
            .optional()?
            .ok_or_else(|| anyhow!("song {id} not found"))
    }

    pub fn song_detail(&self, id: i64) -> Result<SongDetail> {
        let song = self.song(id)?;
        let db = self.db();
        let takes = db
            .prepare("SELECT id, name, origin, added_at FROM takes WHERE song_id = ?1 ORDER BY id")?
            .query_map([id], |r| Ok(Take { id: r.get(0)?, name: r.get(1)?, origin: r.get(2)?, added_at: r.get(3)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let versions = db
            .prepare("SELECT id, label, note, created_at FROM versions WHERE song_id = ?1 ORDER BY id DESC")?
            .query_map([id], |r| Ok(Version { id: r.get(0)?, label: r.get(1)?, note: r.get(2)?, created_at: r.get(3)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let messages = db
            .prepare("SELECT role, text, created_at FROM messages WHERE song_id = ?1 ORDER BY id")?
            .query_map([id], |r| Ok(Message { role: r.get(0)?, text: r.get(1)?, created_at: r.get(2)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let sheets = db
            .prepare("SELECT id, name, kind, added_at FROM sheets WHERE song_id = ?1 ORDER BY id")?
            .query_map([id], |r| Ok(Sheet { id: r.get(0)?, name: r.get(1)?, kind: r.get(2)?, added_at: r.get(3)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let playlists = db
            .prepare("SELECT playlist_id FROM playlist_songs WHERE song_id = ?1")?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(SongDetail { song, takes, versions, messages, sheets, playlists })
    }

    pub fn update_song(&self, id: i64, p: &SongPatch) -> Result<Song> {
        {
            let db = self.db();
            let t = now();
            if let Some(v) = &p.title {
                let v = v.trim();
                db.execute("UPDATE songs SET title = ?2, updated_at = ?3 WHERE id = ?1", params![id, if v.is_empty() { "Untitled" } else { v }, t])?;
            }
            for (col, val) in [("artist", &p.artist), ("album", &p.album), ("year", &p.year)] {
                if let Some(v) = val {
                    db.execute(&format!("UPDATE songs SET {col} = ?2, updated_at = ?3 WHERE id = ?1"), params![id, v.trim(), t])?;
                }
            }
        }
        if let Some(g) = &p.genre {
            let name = self.ensure_genre(g, "user")?;
            self.db().execute("UPDATE songs SET genre = ?2, updated_at = ?3 WHERE id = ?1", params![id, name, now()])?;
        }
        self.song(id)
    }

    /// Copies an image into the art folder and attaches it to the song.
    pub fn set_art_from_file(&self, id: i64, src: &Path) -> Result<Song> {
        let bytes = std::fs::read(src).with_context(|| format!("reading {}", src.display()))?;
        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("jpg").to_ascii_lowercase();
        let name = self.store_art(&bytes, &ext)?;
        self.db().execute("UPDATE songs SET art = ?2, updated_at = ?3 WHERE id = ?1", params![id, name, now()])?;
        self.song(id)
    }

    pub fn clear_art(&self, id: i64) -> Result<Song> {
        self.db().execute("UPDATE songs SET art = NULL, updated_at = ?2 WHERE id = ?1", params![id, now()])?;
        self.song(id)
    }

    fn store_art(&self, bytes: &[u8], ext: &str) -> Result<String> {
        let name = format!("{}.{}", sha1_smol::Sha1::from(bytes).digest(), ext);
        let path = self.dirs.art.join(&name);
        if !path.exists() {
            std::fs::write(&path, bytes)?;
        }
        Ok(name)
    }

    pub fn save_sheet_music(&self, id: i64, notes: &serde_json::Value, chords: &[String]) -> Result<()> {
        self.db().execute(
            "UPDATE songs SET notes_json = ?2, chords_json = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, notes.to_string(), serde_json::to_string(chords)?, now()],
        )?;
        Ok(())
    }

    /// Removes a song from the library. Music files on disk are never touched;
    /// files the app copied for the song (takes, sheets) are removed with it.
    pub fn delete_song(&self, id: i64) -> Result<()> {
        self.db().execute("DELETE FROM songs WHERE id = ?1", [id])?;
        for dir in [self.dirs.takes.join(id.to_string()), self.dirs.sheets.join(id.to_string())] {
            let _ = std::fs::remove_dir_all(dir);
        }
        Ok(())
    }

    // ---------- music folders ----------

    pub fn folders(&self) -> Result<Vec<Folder>> {
        let db = self.db();
        let rows = db
            .prepare("SELECT id, path FROM folders ORDER BY path")?
            .query_map([], |r| Ok(Folder { id: r.get(0)?, path: r.get(1)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn add_folder(&self, path: &Path) -> Result<Folder> {
        let path = path.canonicalize().with_context(|| format!("opening {}", path.display()))?;
        if !path.is_dir() {
            bail!("{} is not a folder", path.display());
        }
        let p = path.to_string_lossy().to_string();
        let db = self.db();
        db.execute("INSERT OR IGNORE INTO folders(path) VALUES (?1)", [&p])?;
        let id = db.query_row("SELECT id FROM folders WHERE path = ?1", [&p], |r| r.get(0))?;
        Ok(Folder { id, path: p })
    }

    /// Stops watching a folder and removes its songs from the library (files stay on disk).
    pub fn remove_folder(&self, id: i64) -> Result<()> {
        self.db().execute("DELETE FROM folders WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Brings the library in line with a folder: new and changed files are read, deleted files drop out.
    pub fn scan_folder(&self, folder: &Folder) -> Result<ScanReport> {
        let root = PathBuf::from(&folder.path);
        let mut report = ScanReport::default();
        let mut seen = std::collections::HashSet::new();
        for entry in walkdir::WalkDir::new(&root).follow_links(true).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !entry.file_type().is_file() || !tags::is_audio(path) || is_hidden(path.strip_prefix(&root).unwrap_or(path)) {
                continue;
            }
            seen.insert(path.to_string_lossy().to_string());
            match self.import_file(path, Some(folder.id), Some(&root))? {
                Imported::Added { untagged } => {
                    report.added += 1;
                    report.untagged += untagged as usize;
                }
                Imported::Updated => report.updated += 1,
                Imported::Unchanged => {}
            }
        }
        let known: Vec<(i64, String)> = {
            let db = self.db();
            let mut st = db.prepare("SELECT id, path FROM songs WHERE folder_id = ?1")?;
            
            st.query_map([folder.id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, path) in known {
            if !seen.contains(&path) {
                self.db().execute("DELETE FROM songs WHERE id = ?1", [id])?;
                report.removed += 1;
            }
        }
        Ok(report)
    }

    /// Adds or refreshes one audio file. Unchanged files (same modification time) are skipped.
    pub fn import_file(&self, path: &Path, folder_id: Option<i64>, root: Option<&Path>) -> Result<Imported> {
        let p = path.to_string_lossy().to_string();
        let mtime = std::fs::metadata(path)?.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
        let existing: Option<(i64, Option<i64>)> =
            self.db().query_row("SELECT id, file_mtime FROM songs WHERE path = ?1", [&p], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        if let Some((_, old)) = existing
            && old == mtime {
                return Ok(Imported::Unchanged);
            }
        let mut info = tags::read_track(path, root);
        info.genre = self.ensure_genre(&info.genre, "file")?;
        let art = match &info.picture {
            Some((bytes, ext)) => Some(self.store_art(bytes, ext)?),
            None => None,
        };
        let t = now();
        let db = self.db();
        match existing {
            Some((id, _)) => {
                db.execute(
                    "UPDATE songs SET title=?2, artist=?3, album=?4, year=?5, genre=?6, track=?7, duration=?8, file_mtime=?9, \
                     art=COALESCE(?10, art), updated_at=?11 WHERE id=?1",
                    params![id, info.title, info.artist, info.album, info.year, info.genre, info.track, info.duration, mtime, art, t],
                )?;
                Ok(Imported::Updated)
            }
            None => {
                db.execute(
                    "INSERT INTO songs(source, title, artist, album, year, genre, track, duration, path, file_mtime, folder_id, art, added_at, updated_at) \
                     VALUES ('library', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                    params![info.title, info.artist, info.album, info.year, info.genre, info.track, info.duration, p, mtime, folder_id, art, t],
                )?;
                Ok(Imported::Added { untagged: info.genre == genre::UNSORTED })
            }
        }
    }

    /// Drops a single file's song when the file disappears from a watched folder.
    pub fn forget_path(&self, path: &Path) -> Result<bool> {
        let n = self.db().execute("DELETE FROM songs WHERE path = ?1", [path.to_string_lossy()])?;
        Ok(n > 0)
    }

    /// The watched folder a path lives in, if any.
    pub fn folder_for(&self, path: &Path) -> Result<Option<Folder>> {
        Ok(self.folders()?.into_iter().filter(|f| path.starts_with(&f.path)).max_by_key(|f| f.path.len()))
    }

    // ---------- genre tags ----------

    /// All tags with how many songs use each. "Unsorted" is not a tag; it is where untagged songs wait.
    pub fn genres(&self) -> Result<Vec<GenreTag>> {
        let db = self.db();
        let rows = db
            .prepare(
                "SELECT g.id, g.name, g.origin, (SELECT COUNT(*) FROM songs s WHERE s.genre = g.name COLLATE NOCASE) \
                 FROM genres g ORDER BY g.name COLLATE NOCASE",
            )?
            .query_map([], |r| Ok(GenreTag { id: r.get(0)?, name: r.get(1)?, origin: r.get(2)?, songs: r.get(3)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Returns the tag's stored spelling, creating the tag when it does not exist yet.
    pub fn ensure_genre(&self, raw: &str, origin: &str) -> Result<String> {
        let name = genre::normalize(raw);
        if name == genre::UNSORTED {
            return Ok(name);
        }
        let db = self.db();
        db.execute("INSERT OR IGNORE INTO genres(name, origin, created_at) VALUES (?1, ?2, ?3)", params![name, origin, now()])?;
        Ok(db.query_row("SELECT name FROM genres WHERE name = ?1 COLLATE NOCASE", [&name], |r| r.get(0))?)
    }

    pub fn create_genre(&self, raw: &str) -> Result<GenreTag> {
        let name = genre::normalize(raw);
        if name == genre::UNSORTED {
            bail!("Give the tag a name");
        }
        let exists: bool = self.db().query_row("SELECT EXISTS(SELECT 1 FROM genres WHERE name = ?1 COLLATE NOCASE)", [&name], |r| r.get(0))?;
        if exists {
            bail!("There is already a tag called {name}");
        }
        self.ensure_genre(&name, "user")?;
        self.genres()?.into_iter().find(|g| g.name.eq_ignore_ascii_case(&name)).ok_or_else(|| anyhow!("tag not saved"))
    }

    /// Renames a tag and every song that carries it. Renaming onto an existing tag merges the two.
    pub fn rename_genre(&self, id: i64, raw: &str) -> Result<()> {
        let name = genre::normalize(raw);
        if name == genre::UNSORTED {
            bail!("Give the tag a name");
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        let old: String = tx.query_row("SELECT name FROM genres WHERE id = ?1", [id], |r| r.get(0))?;
        let other: Option<i64> = tx.query_row("SELECT id FROM genres WHERE name = ?1 COLLATE NOCASE AND id <> ?2", params![name, id], |r| r.get(0)).optional()?;
        match other {
            Some(_) => {
                tx.execute("DELETE FROM genres WHERE id = ?1", [id])?;
            }
            None => {
                tx.execute("UPDATE genres SET name = ?2 WHERE id = ?1", params![id, name])?;
            }
        }
        tx.execute("UPDATE songs SET genre = ?2, updated_at = ?3 WHERE genre = ?1 COLLATE NOCASE", params![old, name, now()])?;
        tx.commit()?;
        Ok(())
    }

    /// Deletes a tag; its songs go back to Unsorted.
    pub fn delete_genre(&self, id: i64) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let old: String = tx.query_row("SELECT name FROM genres WHERE id = ?1", [id], |r| r.get(0))?;
        tx.execute("DELETE FROM genres WHERE id = ?1", [id])?;
        tx.execute("UPDATE songs SET genre = ?2, updated_at = ?3 WHERE genre = ?1 COLLATE NOCASE", params![old, genre::UNSORTED, now()])?;
        tx.commit()?;
        Ok(())
    }

    // ---------- playlists ----------

    pub fn playlists(&self) -> Result<Vec<Playlist>> {
        let db = self.db();
        let mut lists = db
            .prepare("SELECT id, name FROM playlists ORDER BY created_at, id")?
            .query_map([], |r| Ok(Playlist { id: r.get(0)?, name: r.get(1)?, songs: vec![] }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut st = db.prepare("SELECT song_id FROM playlist_songs WHERE playlist_id = ?1 ORDER BY pos")?;
        for p in &mut lists {
            p.songs = st.query_map([p.id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        }
        Ok(lists)
    }

    pub fn create_playlist(&self, name: &str, songs: &[i64]) -> Result<Playlist> {
        let id = {
            let db = self.db();
            db.execute("INSERT INTO playlists(name, created_at) VALUES (?1, ?2)", params![name.trim(), now()])?;
            db.last_insert_rowid()
        };
        self.set_playlist_songs(id, songs)?;
        Ok(Playlist { id, name: name.trim().into(), songs: songs.to_vec() })
    }

    pub fn rename_playlist(&self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            bail!("A playlist needs a name");
        }
        self.db().execute("UPDATE playlists SET name = ?2 WHERE id = ?1", params![id, name])?;
        Ok(())
    }

    pub fn delete_playlist(&self, id: i64) -> Result<()> {
        self.db().execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_playlist_songs(&self, id: i64, songs: &[i64]) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute("DELETE FROM playlist_songs WHERE playlist_id = ?1", [id])?;
        let mut seen = std::collections::HashSet::new();
        for (pos, s) in songs.iter().filter(|s| seen.insert(**s)).enumerate() {
            tx.execute("INSERT INTO playlist_songs(playlist_id, song_id, pos) VALUES (?1, ?2, ?3)", params![id, s, pos as i64])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn toggle_in_playlist(&self, id: i64, song: i64, on: bool) -> Result<()> {
        let db = self.db();
        if on {
            let pos: i64 = db.query_row("SELECT COALESCE(MAX(pos) + 1, 0) FROM playlist_songs WHERE playlist_id = ?1", [id], |r| r.get(0))?;
            db.execute("INSERT OR IGNORE INTO playlist_songs(playlist_id, song_id, pos) VALUES (?1, ?2, ?3)", params![id, song, pos])?;
        } else {
            db.execute("DELETE FROM playlist_songs WHERE playlist_id = ?1 AND song_id = ?2", params![id, song])?;
        }
        Ok(())
    }

    // ---------- settings ----------

    pub fn settings(&self) -> Result<serde_json::Map<String, serde_json::Value>> {
        let db = self.db();
        let mut st = db.prepare("SELECT key, value FROM settings")?;
        let map = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|r| r.ok())
            .map(|(k, v)| (k, serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v))))
            .collect();
        Ok(map)
    }

    pub fn set_setting(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        self.db().execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value.to_string()],
        )?;
        Ok(())
    }

    pub fn setting_str(&self, key: &str) -> Option<String> {
        self.settings().ok()?.get(key).and_then(|v| v.as_str().map(String::from)).filter(|s| !s.is_empty())
    }

    // ---------- sheets (uploaded sheet music) ----------

    pub fn add_sheet(&self, song: i64, src: &Path) -> Result<Sheet> {
        let name = src.file_name().and_then(|n| n.to_str()).ok_or_else(|| anyhow!("bad file name"))?.to_string();
        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let kind = match ext.as_str() {
            "pdf" => "pdf",
            "musicxml" | "xml" => "musicxml",
            "png" | "jpg" | "jpeg" | "gif" | "webp" => "image",
            _ => bail!("Sheet music can be a PDF, an image, or a MusicXML file"),
        };
        let dir = self.dirs.sheets.join(song.to_string());
        std::fs::create_dir_all(&dir)?;
        let dest = unique_path(&dir, &name);
        std::fs::copy(src, &dest).with_context(|| format!("copying {}", src.display()))?;
        let t = now();
        let db = self.db();
        db.execute(
            "INSERT INTO sheets(song_id, name, kind, path, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![song, name, kind, dest.to_string_lossy(), t],
        )?;
        Ok(Sheet { id: db.last_insert_rowid(), name, kind: kind.into(), added_at: t })
    }

    pub fn remove_sheet(&self, id: i64) -> Result<()> {
        if let Some(p) = self.sheet_path(id)? {
            let _ = std::fs::remove_file(p);
        }
        self.db().execute("DELETE FROM sheets WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---------- takes (audio made for a songwriter song) ----------

    pub fn add_take(&self, song: i64, src: &Path, origin: &str) -> Result<Take> {
        let name = src.file_name().and_then(|n| n.to_str()).ok_or_else(|| anyhow!("bad file name"))?.to_string();
        if !tags::is_audio(src) {
            bail!("A take has to be an audio file (MP3, WAV, FLAC, M4A or OGG)");
        }
        let dir = self.dirs.takes.join(song.to_string());
        std::fs::create_dir_all(&dir)?;
        let dest = unique_path(&dir, &name);
        std::fs::copy(src, &dest).with_context(|| format!("copying {}", src.display()))?;
        let t = now();
        let db = self.db();
        db.execute(
            "INSERT INTO takes(song_id, name, path, origin, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![song, name, dest.to_string_lossy(), origin, t],
        )?;
        let id = db.last_insert_rowid();
        db.execute(
            "UPDATE songs SET master_take = COALESCE(master_take, ?2), \
             status = CASE WHEN status IN ('writing','ready') THEN 'takes' ELSE status END, updated_at = ?3 WHERE id = ?1",
            params![song, id, t],
        )?;
        Ok(Take { id, name, origin: origin.into(), added_at: t })
    }

    pub fn set_master_take(&self, song: i64, take: i64) -> Result<()> {
        self.db().execute("UPDATE songs SET master_take = ?2, updated_at = ?3 WHERE id = ?1", params![song, take, now()])?;
        Ok(())
    }

    pub fn set_status(&self, song: i64, status: &str) -> Result<()> {
        if !["writing", "ready", "takes", "done"].contains(&status) {
            bail!("unknown status {status}");
        }
        self.db().execute("UPDATE songs SET status = ?2, updated_at = ?3 WHERE id = ?1", params![song, status, now()])?;
        Ok(())
    }

    // ---------- songwriter songs ----------

    /// Creates a song the songwriter is about to write.
    pub fn create_request(&self, request: &str, genre_hint: Option<&str>, writer: &str) -> Result<Song> {
        let t = now();
        let genre = self.ensure_genre(genre_hint.unwrap_or(""), "user")?;
        let id = {
            let db = self.db();
            db.execute(
                "INSERT INTO songs(source, title, artist, genre, request, status, added_at, updated_at) VALUES ('ai', 'New song', ?1, ?2, ?3, 'writing', ?4, ?4)",
                params![writer, genre, request.trim(), t],
            )?;
            db.last_insert_rowid()
        };
        self.add_message(id, "user", request)?;
        self.song(id)
    }

    /// Stores a draft or rewrite from the songwriter and records it as a new version.
    pub fn apply_draft(&self, id: i64, d: &Draft, note: &str, keep_genre: bool) -> Result<Song> {
        let t = now();
        let genre = if keep_genre { genre::UNSORTED.to_string() } else { self.ensure_genre(&d.genre, "writer")? };
        {
            let db = self.db();
            let genre_sql = if keep_genre { "genre" } else { "?11" };
            db.execute(
                &format!(
                    "UPDATE songs SET title = ?2, sub_genre = ?3, song_key = ?4, bpm = ?5, style = ?6, exclude = ?7, lyrics = ?8, \
                     status = CASE WHEN status = 'writing' THEN 'ready' ELSE status END, updated_at = ?9, \
                     notes_json = COALESCE(?10, notes_json), chords_json = COALESCE(?12, chords_json), genre = CASE WHEN ?11 = 'Unsorted' THEN genre ELSE {genre_sql} END \
                     WHERE id = ?1"
                ),
                params![
                    id,
                    if d.title.trim().is_empty() { "Untitled" } else { d.title.trim() },
                    d.sub_genre,
                    d.key,
                    d.bpm,
                    d.style,
                    d.exclude,
                    d.lyrics,
                    t,
                    d.notes.as_ref().filter(|n| n.as_array().is_some_and(|a| !a.is_empty())).map(|n| n.to_string()),
                    genre,
                    d.chords.as_ref().filter(|c| !c.is_empty()).map(|c| serde_json::to_string(c).unwrap_or_default()),
                ],
            )?;
            let n: i64 = db.query_row("SELECT COUNT(*) FROM versions WHERE song_id = ?1", [id], |r| r.get(0))?;
            db.execute(
                "INSERT INTO versions(song_id, label, note, title, lyrics, style, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, format!("v{}", n + 1), note, d.title, d.lyrics, d.style, t],
            )?;
        }
        self.song(id)
    }

    pub fn add_message(&self, id: i64, role: &str, text: &str) -> Result<()> {
        self.db().execute("INSERT INTO messages(song_id, role, text, created_at) VALUES (?1, ?2, ?3, ?4)", params![id, role, text, now()])?;
        Ok(())
    }

    pub fn messages(&self, id: i64) -> Result<Vec<Message>> {
        Ok(self.song_detail(id)?.messages)
    }

    // ---------- file lookups for the audio/image protocol ----------

    /// The file to play for a song: its own file, or the master take for songwriter songs.
    pub fn audio_path(&self, id: i64) -> Result<Option<PathBuf>> {
        let db = self.db();
        let own: Option<Option<String>> = db.query_row("SELECT path FROM songs WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        if let Some(Some(p)) = own {
            return Ok(Some(p.into()));
        }
        let take: Option<String> = db
            .query_row(
                "SELECT t.path FROM takes t JOIN songs s ON s.id = t.song_id WHERE t.song_id = ?1 \
                 ORDER BY (t.id = s.master_take) DESC, t.id LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(take.map(PathBuf::from))
    }

    pub fn take_path(&self, id: i64) -> Result<Option<PathBuf>> {
        Ok(self.db().query_row("SELECT path FROM takes WHERE id = ?1", [id], |r| r.get::<_, String>(0)).optional()?.map(PathBuf::from))
    }

    pub fn sheet_path(&self, id: i64) -> Result<Option<PathBuf>> {
        Ok(self.db().query_row("SELECT path FROM sheets WHERE id = ?1", [id], |r| r.get::<_, String>(0)).optional()?.map(PathBuf::from))
    }

    /// Art files are addressed by name; only plain names inside the art folder resolve.
    pub fn art_path(&self, name: &str) -> Option<PathBuf> {
        if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
            return None;
        }
        let p = self.dirs.art.join(name);
        p.is_file().then_some(p)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Imported {
    Added { untagged: bool },
    Updated,
    Unchanged,
}

fn is_hidden(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str().to_str().is_some_and(|s| s.starts_with('.') && s.len() > 1 && s != ".."))
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = p.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    (2..).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|c| !c.exists()).expect("unbounded range")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let l = Library::open(dir.path()).unwrap();
        (dir, l)
    }

    #[test]
    fn scans_folder_and_tracks_removals() {
        let (dir, l) = lib();
        let music = dir.path().join("music/Band/Album");
        std::fs::create_dir_all(&music).unwrap();
        std::fs::write(music.join("01 First.mp3"), b"not really audio").unwrap();
        std::fs::write(music.join("cover.txt"), b"ignored").unwrap();
        let f = l.add_folder(&dir.path().join("music")).unwrap();
        let r = l.scan_folder(&f).unwrap();
        assert_eq!((r.added, r.untagged), (1, 1));
        let songs = l.songs().unwrap();
        assert_eq!(songs[0].title, "First");
        assert_eq!(songs[0].artist, "Band");
        assert_eq!(songs[0].album, "Album");
        assert!(songs[0].has_audio);
        assert_eq!(l.scan_folder(&f).unwrap().added, 0, "unchanged files are skipped");
        std::fs::remove_file(music.join("01 First.mp3")).unwrap();
        assert_eq!(l.scan_folder(&f).unwrap().removed, 1);
        assert!(l.songs().unwrap().is_empty());
    }

    #[test]
    fn playlists_keep_order_and_membership() {
        let (_d, l) = lib();
        let a = l.create_request("a folk song", Some("folk"), "Bard").unwrap();
        let b = l.create_request("a rock song", None, "Bard").unwrap();
        let p = l.create_playlist("Road trip", &[b.id, a.id]).unwrap();
        assert_eq!(l.playlists().unwrap()[0].songs, vec![b.id, a.id]);
        l.toggle_in_playlist(p.id, a.id, false).unwrap();
        assert_eq!(l.playlists().unwrap()[0].songs, vec![b.id]);
        assert_eq!(l.song_detail(b.id).unwrap().playlists, vec![p.id]);
        assert_eq!(a.genre, "Folk");
        assert_eq!(b.genre, "Unsorted");
    }

    #[test]
    fn drafts_become_versions() {
        let (_d, l) = lib();
        let s = l.create_request("sad country song", Some("Country"), "Bard").unwrap();
        let d = Draft { title: "Porch Light".into(), genre: "Country".into(), lyrics: "[Verse]\nHello".into(), bpm: 80, ..Default::default() };
        let s2 = l.apply_draft(s.id, &d, "First draft", false).unwrap();
        assert_eq!(s2.title, "Porch Light");
        assert_eq!(s2.status.as_deref(), Some("ready"));
        let det = l.song_detail(s.id).unwrap();
        assert_eq!(det.versions.len(), 1);
        assert_eq!(det.versions[0].label, "v1");
    }

    #[test]
    fn genre_tags_are_created_renamed_and_deleted() {
        let (_d, l) = lib();
        assert!(l.genres().unwrap().iter().any(|g| g.name == "Rock" && g.origin == "default"));
        let s = l.create_request("a song", Some("sea shanty"), "Bard").unwrap();
        assert_eq!(s.genre, "Sea Shanty");
        let tag = l.genres().unwrap().into_iter().find(|g| g.name == "Sea Shanty").unwrap();
        assert_eq!((tag.origin.as_str(), tag.songs), ("user", 1));
        assert!(l.create_genre("sea SHANTY").is_err(), "duplicates are refused");
        l.rename_genre(tag.id, "Shanty").unwrap();
        assert_eq!(l.song(s.id).unwrap().genre, "Shanty");
        l.delete_genre(tag.id).unwrap();
        assert_eq!(l.song(s.id).unwrap().genre, "Unsorted");
        let rock = l.ensure_genre("rock", "writer").unwrap();
        assert_eq!(rock, "Rock", "existing tags keep their spelling");
    }

    #[test]
    fn settings_round_trip() {
        let (_d, l) = lib();
        l.set_setting("sidebarDefault", &serde_json::json!("genres")).unwrap();
        assert_eq!(l.setting_str("sidebarDefault").as_deref(), Some("genres"));
    }
}
