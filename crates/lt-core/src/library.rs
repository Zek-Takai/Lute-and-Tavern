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
use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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
        let d = Self {
            root: root.into(),
            art: root.join("art"),
            sheets: root.join("sheets"),
            takes: root.join("takes"),
        };
        for p in [&d.root, &d.art, &d.sheets, &d.takes] {
            std::fs::create_dir_all(p).with_context(|| format!("creating {}", p.display()))?;
        }
        Ok(d)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
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
    /// The song has its own audio file or a take (not only stems).
    pub has_file: bool,
    /// Number of stem files (vocals, drums...) attached to the song.
    pub stems: i64,
    /// The song's own folder in the library, when it lives in the library folder.
    pub song_dir: Option<String>,
    /// Where the song plays from: "local" (a file) or a service such as "apple".
    pub provider: String,
    /// The service's id for the song (Apple Music catalog id).
    pub provider_id: Option<String>,
    /// Artwork from the service, as a URL template.
    pub art_url: Option<String>,
}

/// A song from a streaming service, as the interface sends it when saving it to the library.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSong {
    pub provider: String,
    pub provider_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub year: String,
    pub track: Option<i64>,
    pub duration: Option<f64>,
    pub art_url: Option<String>,
}

const SONG_COLS: &str = "s.id, s.source, s.title, s.artist, s.album, s.year, s.genre, s.track, s.duration, \
    (s.path IS NOT NULL OR EXISTS (SELECT 1 FROM takes t WHERE t.song_id = s.id) OR EXISTS (SELECT 1 FROM stems st WHERE st.song_id = s.id)), \
    s.art, s.added_at, s.updated_at, s.request, s.status, s.sub_genre, s.song_key, s.bpm, s.style, s.exclude, s.lyrics, \
    s.master_take, s.notes_json, s.chords_json, \
    (s.path IS NOT NULL OR EXISTS (SELECT 1 FROM takes t WHERE t.song_id = s.id)), (SELECT COUNT(*) FROM stems st WHERE st.song_id = s.id), \
    (SELECT s.song_dir FROM folders f WHERE f.id = s.folder_id AND f.managed = 1), s.provider, s.provider_id, s.art_url";

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
        has_file: r.get(24)?,
        stems: r.get(25)?,
        song_dir: r.get(26)?,
        provider: r.get(27)?,
        provider_id: r.get(28)?,
        art_url: r.get(29)?,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Take {
    pub id: i64,
    pub name: String,
    pub origin: String,
    pub added_at: i64,
    /// The lyrics version the take was made from
    pub version_id: Option<i64>,
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
pub struct Stem {
    pub id: i64,
    pub name: String,
    /// File extension, e.g. "wav" or "mp3"; one part can arrive in both.
    pub format: String,
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
    pub stem_files: Vec<Stem>,
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
    /// The library folder: the app keeps it organized. Other folders are only watched.
    pub managed: bool,
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

/// What attaching stems did; `kept_wav` says which WAVs could not become FLAC, and why.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attached {
    pub count: usize,
    pub flac: usize,
    pub kept_wav: Vec<String>,
}

/// What converting the library's WAV stems did.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlacReport {
    pub converted: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub kept_wav: Vec<String>,
}

pub struct Library {
    conn: Mutex<Connection>,
    pub dirs: DataDirs,
}

impl Library {
    pub fn open(data_dir: &Path) -> Result<Self> {
        let dirs = DataDirs::new(data_dir)?;
        let conn = db::open(&dirs.root.join("library.db"))?;
        Ok(Self {
            conn: Mutex::new(conn),
            dirs,
        })
    }

    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---------- songs ----------

    pub fn songs(&self) -> Result<Vec<Song>> {
        let db = self.db();
        let mut st = db.prepare(&format!(
            "SELECT {SONG_COLS} FROM songs s ORDER BY s.added_at DESC, s.id DESC"
        ))?;
        let rows = st
            .query_map([], song_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn song(&self, id: i64) -> Result<Song> {
        let db = self.db();
        db.query_row(
            &format!("SELECT {SONG_COLS} FROM songs s WHERE s.id = ?1"),
            [id],
            song_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow!("song {id} not found"))
    }

    pub fn song_detail(&self, id: i64) -> Result<SongDetail> {
        let song = self.song(id)?;
        let db = self.db();
        let takes = db
            .prepare("SELECT id, name, origin, added_at, version_id FROM takes WHERE song_id = ?1 ORDER BY id")?
            .query_map([id], |r| {
                Ok(Take {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    origin: r.get(2)?,
                    added_at: r.get(3)?,
                    version_id: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let versions = db
            .prepare("SELECT id, label, note, created_at FROM versions WHERE song_id = ?1 ORDER BY id DESC")?
            .query_map([id], |r| Ok(Version { id: r.get(0)?, label: r.get(1)?, note: r.get(2)?, created_at: r.get(3)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let messages = db
            .prepare("SELECT role, text, created_at FROM messages WHERE song_id = ?1 ORDER BY id")?
            .query_map([id], |r| {
                Ok(Message {
                    role: r.get(0)?,
                    text: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let sheets = db
            .prepare("SELECT id, name, kind, added_at FROM sheets WHERE song_id = ?1 ORDER BY id")?
            .query_map([id], |r| {
                Ok(Sheet {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    added_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let playlists = db
            .prepare("SELECT playlist_id FROM playlist_songs WHERE song_id = ?1")?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let stem_files = db
            .prepare("SELECT id, name, path FROM stems WHERE song_id = ?1 ORDER BY name COLLATE NOCASE, path")?
            .query_map([id], |r| {
                let path: String = r.get(2)?;
                let format = Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                Ok(Stem { id: r.get(0)?, name: r.get(1)?, format })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(SongDetail {
            song,
            takes,
            versions,
            messages,
            sheets,
            playlists,
            stem_files,
        })
    }

    pub fn update_song(&self, id: i64, p: &SongPatch) -> Result<Song> {
        {
            let db = self.db();
            let t = now();
            if let Some(v) = &p.title {
                let v = v.trim();
                db.execute(
                    "UPDATE songs SET title = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id, if v.is_empty() { "Untitled" } else { v }, t],
                )?;
            }
            for (col, val) in [
                ("artist", &p.artist),
                ("album", &p.album),
                ("year", &p.year),
            ] {
                if let Some(v) = val {
                    db.execute(
                        &format!("UPDATE songs SET {col} = ?2, updated_at = ?3 WHERE id = ?1"),
                        params![id, v.trim(), t],
                    )?;
                }
            }
        }
        if let Some(g) = &p.genre {
            let name = self.ensure_genre(g, "user")?;
            self.db().execute(
                "UPDATE songs SET genre = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, name, now()],
            )?;
        }
        if p.title.is_some() || p.artist.is_some() {
            self.relocate(id)?;
        } else {
            self.write_sidecar(id)?;
        }
        self.song(id)
    }

    /// Copies an image into the art folder and attaches it to the song.
    /// Sets artwork from an image, or from the cover embedded in an audio file (e.g. Suno's MP3 download).
    pub fn set_art_from_file(&self, id: i64, src: &Path) -> Result<Song> {
        let name = if tags::is_audio(src) {
            let (bytes, ext) = tags::read_track(src, None)
                .picture
                .ok_or_else(|| anyhow!("{} has no cover art inside it. Suno puts covers in its MP3 downloads.", src.display()))?;
            self.store_art(&bytes, ext)?
        } else {
            let bytes = std::fs::read(src).with_context(|| format!("reading {}", src.display()))?;
            let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("jpg").to_ascii_lowercase();
            self.store_art(&bytes, &ext)?
        };
        self.db().execute(
            "UPDATE songs SET art = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, name, now()],
        )?;
        self.write_cover(id)?;
        self.embed_art(id)?;
        self.song(id)
    }

    /// Writes the song's artwork into its own audio: its takes, and its main file when that lives in the
    /// library folder (files in watched folders are never changed). Returns how many files got the cover.
    pub fn embed_art(&self, id: i64) -> Result<usize> {
        let (art, path, folder_id): (Option<String>, Option<String>, Option<i64>) = self.db().query_row(
            "SELECT art, path, folder_id FROM songs WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let Some(image) = art.and_then(|a| self.art_path(&a)) else { return Ok(0) };
        let image = std::fs::read(image)?;
        let mut files: Vec<PathBuf> = {
            let db = self.db();
            let mut st = db.prepare("SELECT path FROM takes WHERE song_id = ?1")?;
            st.query_map([id], |r| r.get::<_, String>(0))?.flatten().map(PathBuf::from).collect()
        };
        if let (Some(p), Some(root)) = (path.map(PathBuf::from), self.managed_root(folder_id)) {
            if p.starts_with(&root) {
                files.push(p);
            }
        }
        let mut n = 0;
        for f in files {
            let already = tags::read_track(&f, None).picture.is_some_and(|(b, _)| b == image);
            if already || tags::embed_cover(&f, &image).is_ok() {
                n += 1;
            }
        }
        Ok(n)
    }

    pub fn clear_art(&self, id: i64) -> Result<Song> {
        self.db().execute(
            "UPDATE songs SET art = NULL, updated_at = ?2 WHERE id = ?1",
            params![id, now()],
        )?;
        self.song(id)
    }

    pub(crate) fn store_art(&self, bytes: &[u8], ext: &str) -> Result<String> {
        let name = format!("{}.{}", sha1_smol::Sha1::from(bytes).digest(), ext);
        let path = self.dirs.art.join(&name);
        if !path.exists() {
            std::fs::write(&path, bytes)?;
        }
        Ok(name)
    }

    pub fn save_sheet_music(
        &self,
        id: i64,
        notes: &serde_json::Value,
        chords: &[String],
    ) -> Result<()> {
        self.db().execute(
            "UPDATE songs SET notes_json = ?2, chords_json = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, notes.to_string(), serde_json::to_string(chords)?, now()],
        )?;
        self.write_sidecar(id)
    }

    /// Removes a song from the library. Music files on disk are never touched;
    /// files the app copied for the song (takes, sheets) are removed with it.
    pub fn delete_song(&self, id: i64) -> Result<()> {
        let (dir, folder_id): (Option<String>, Option<i64>) = self.db().query_row(
            "SELECT song_dir, folder_id FROM songs WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if let (Some(dir), Some(_)) = (dir, self.managed_root(folder_id)) {
            bail!(
                "This song lives in your library folder. To remove it, move its folder out of the library or to the Trash:\n{dir}"
            );
        }
        self.db().execute("DELETE FROM songs WHERE id = ?1", [id])?;
        for dir in [
            self.dirs.takes.join(id.to_string()),
            self.dirs.sheets.join(id.to_string()),
        ] {
            let _ = std::fs::remove_dir_all(dir);
        }
        Ok(())
    }

    /// Adds a streaming song to the library; saving the same song twice returns the saved one.
    pub fn save_external_song(&self, e: &ExternalSong) -> Result<Song> {
        if !["apple"].contains(&e.provider.as_str()) {
            bail!("unknown music service {}", e.provider);
        }
        let genre = self.ensure_genre(&e.genre, "file")?;
        let t = now();
        let id: i64 = {
            let db = self.db();
            db.execute(
                "INSERT INTO songs(source, provider, provider_id, title, artist, album, year, genre, track, duration, art_url, added_at, updated_at) \
                 VALUES ('library', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11) ON CONFLICT(provider, provider_id) WHERE provider_id IS NOT NULL DO NOTHING",
                params![e.provider, e.provider_id, e.title, e.artist, e.album, e.year, genre, e.track, e.duration, e.art_url, t],
            )?;
            db.query_row(
                "SELECT id FROM songs WHERE provider = ?1 AND provider_id = ?2",
                params![e.provider, e.provider_id],
                |r| r.get(0),
            )?
        };
        self.song(id)
    }

    // ---------- music folders ----------

    pub fn folders(&self) -> Result<Vec<Folder>> {
        let db = self.db();
        let rows = db
            .prepare("SELECT id, path, managed FROM folders ORDER BY managed DESC, path")?
            .query_map([], |r| {
                Ok(Folder {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    managed: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn add_folder(&self, path: &Path) -> Result<Folder> {
        let path = path
            .canonicalize()
            .with_context(|| format!("opening {}", path.display()))?;
        if !path.is_dir() {
            bail!("{} is not a folder", path.display());
        }
        let p = path.to_string_lossy().to_string();
        let db = self.db();
        db.execute("INSERT OR IGNORE INTO folders(path) VALUES (?1)", [&p])?;
        let (id, managed) = db.query_row(
            "SELECT id, managed FROM folders WHERE path = ?1",
            [&p],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(Folder {
            id,
            path: p,
            managed,
        })
    }

    /// Stops watching a folder and removes its songs from the library (files stay on disk).
    pub fn remove_folder(&self, id: i64) -> Result<()> {
        self.db()
            .execute("DELETE FROM folders WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Brings the library in line with a folder: new and changed files are read, deleted files drop out.
    pub fn scan_folder(&self, folder: &Folder) -> Result<ScanReport> {
        let root = PathBuf::from(&folder.path);
        self.organize(folder)?;
        let mut report = ScanReport::default();
        let mut seen = std::collections::HashSet::new();
        let mut files: Vec<PathBuf> = walkdir::WalkDir::new(&root)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_file()
                    && tags::is_audio(e.path())
                    && !is_hidden(e.path().strip_prefix(&root).unwrap_or(e.path()))
                    && crate::sync::conflict_of(e.path()).is_none()
            })
            .filter(|e| {
                !folder.managed
                    || crate::organize::place_of(&root, e.path()) != crate::organize::Place::Extra
            })
            .map(|e| e.into_path())
            .collect();
        // Whole songs first, so stems find the song they belong to.
        files.sort_by_key(|p| (tags::stem_of(p).is_some(), p.clone()));
        for path in &files {
            let path = path.as_path();
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
            let mut st =
                db.prepare("SELECT id, path FROM songs WHERE folder_id = ?1 AND path IS NOT NULL")?;

            st.query_map([folder.id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, path) in known {
            if !seen.contains(&path) {
                self.db().execute("DELETE FROM songs WHERE id = ?1", [id])?;
                report.removed += 1;
            }
        }
        let known_stems: Vec<(i64, String)> = {
            let db = self.db();
            let mut st = db.prepare("SELECT id, path FROM stems WHERE folder_id = ?1")?;
            st.query_map([folder.id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, path) in known_stems {
            if !seen.contains(&path) {
                self.db().execute("DELETE FROM stems WHERE id = ?1", [id])?;
            }
        }
        report.removed += self.drop_empty_stem_songs()?;
        Ok(report)
    }

    /// Adds or refreshes one audio file. Unchanged files (same modification time) are skipped.
    pub fn import_file(
        &self,
        path: &Path,
        folder_id: Option<i64>,
        root: Option<&Path>,
    ) -> Result<Imported> {
        use crate::organize::{Place, place_of};
        let managed = self.managed_root(folder_id);
        let place = managed.as_deref().map(|r| place_of(r, path));
        match place {
            Some(Place::Extra) => return Ok(Imported::Unchanged),
            Some(Place::Stem(dir, name)) => {
                let (song_id, created) = self.song_for_dir(&dir, folder_id)?;
                return self.attach_stem(path, song_id, &name, folder_id, created);
            }
            _ => {}
        }
        if let Some((base, name)) = tags::stem_of(path) {
            return self.import_stem(path, &base, &name, folder_id, root);
        }
        let song_dir = match &place {
            Some(Place::Main(dir)) => Some(dir.clone()),
            _ => None,
        };
        let p = path.to_string_lossy().to_string();
        let mtime = std::fs::metadata(path)?
            .modified()
            .ok()
            .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        let existing: Option<(i64, Option<i64>)> = self
            .db()
            .query_row(
                "SELECT id, file_mtime FROM songs WHERE path = ?1",
                [&p],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, old)) = existing
            && old == mtime
        {
            if let Some(d) = &song_dir {
                self.db().execute(
                    "UPDATE songs SET song_dir = ?2 WHERE id = ?1 AND song_dir IS NULL",
                    params![id, d.to_string_lossy()],
                )?;
            }
            return Ok(Imported::Unchanged);
        }
        let mut info = tags::read_track(path, if song_dir.is_some() { None } else { root });
        if existing.is_none()
            && let Some(d) = &song_dir
            && let Some(id) = self.adopt_main_file(d, path, mtime, info.duration)?
        {
            self.write_sidecar(id)?;
            return Ok(Imported::Updated);
        }
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
                let id = db.last_insert_rowid();
                if let Some(d) = &song_dir {
                    db.execute(
                        "UPDATE songs SET song_dir = ?2 WHERE id = ?1",
                        params![id, d.to_string_lossy()],
                    )?;
                }
                drop(db);
                self.adopt_stems(id, path)?;
                if song_dir.is_some() {
                    self.write_cover(id)?;
                    self.write_sidecar(id)?;
                }
                Ok(Imported::Added {
                    untagged: info.genre == genre::UNSORTED,
                })
            }
        }
    }

    /// Drops a single file's song when the file disappears from a watched folder.
    pub fn forget_path(&self, path: &Path) -> Result<bool> {
        let p = path.to_string_lossy();
        let n = self
            .db()
            .execute("DELETE FROM songs WHERE path = ?1", [&p])?;
        let m = self
            .db()
            .execute("DELETE FROM stems WHERE path = ?1", [&p])?;
        self.drop_empty_stem_songs()?;
        Ok(n + m > 0)
    }

    // ---------- stems ----------

    /// Key for a song made only of stems: the folder the stems sit in plus the song title.
    fn stem_group(dir: &Path, base: &str) -> String {
        format!("{}\u{1f}{}", dir.to_string_lossy(), base.to_lowercase())
    }

    /// The whole-song file a set of stems belongs to: "Song.mp3" beside the stems or one folder up
    /// (Suno puts stems in a "Song Stems" folder).
    fn host_song_for(&self, dir: &Path, base: &str) -> Result<Option<i64>> {
        let db = self.db();
        for d in [Some(dir), dir.parent()].into_iter().flatten() {
            for ext in tags::AUDIO_EXTENSIONS {
                let candidate = d.join(format!("{base}.{ext}"));
                let id: Option<i64> = db
                    .query_row(
                        "SELECT id FROM songs WHERE path = ?1 COLLATE NOCASE",
                        [candidate.to_string_lossy()],
                        |r| r.get(0),
                    )
                    .optional()?;
                if id.is_some() {
                    return Ok(id);
                }
            }
        }
        Ok(None)
    }

    fn import_stem(
        &self,
        path: &Path,
        base: &str,
        name: &str,
        folder_id: Option<i64>,
        root: Option<&Path>,
    ) -> Result<Imported> {
        let p = path.to_string_lossy().to_string();
        let mtime = std::fs::metadata(path)?
            .modified()
            .ok()
            .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        let existing: Option<Option<i64>> = self
            .db()
            .query_row("SELECT file_mtime FROM stems WHERE path = ?1", [&p], |r| {
                r.get(0)
            })
            .optional()?;
        if existing == Some(mtime) {
            return Ok(Imported::Unchanged);
        }
        let dir = path.parent().unwrap_or(Path::new(""));
        let group = Self::stem_group(dir, base);
        let info = tags::read_track(path, root);
        let mut created = false;
        let song_id = match self.host_song_for(dir, base)? {
            Some(id) => id,
            None => {
                let found: Option<i64> = self
                    .db()
                    .query_row(
                        "SELECT id FROM songs WHERE stem_group = ?1",
                        [&group],
                        |r| r.get(0),
                    )
                    .optional()?;
                match found {
                    Some(id) => id,
                    None => {
                        let genre = self.ensure_genre(&info.genre, "file")?;
                        let album = if info.album.to_lowercase().ends_with(" stems") {
                            String::new()
                        } else {
                            info.album.clone()
                        };
                        let t = now();
                        let db = self.db();
                        db.execute(
                            "INSERT INTO songs(source, title, artist, album, year, genre, duration, folder_id, stem_group, added_at, updated_at) \
                             VALUES ('library', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                            params![base, info.artist, album, info.year, genre, info.duration, folder_id, group, t],
                        )?;
                        created = true;
                        db.last_insert_rowid()
                    }
                }
            }
        };
        self.attach_stem(path, song_id, name, folder_id, created)
    }

    /// Records one stem file for a song. Stems with better tags (Suno's MP3s carry the artist and
    /// cover art) fill in what the song is missing.
    pub(crate) fn attach_stem(
        &self,
        path: &Path,
        song_id: i64,
        name: &str,
        folder_id: Option<i64>,
        created: bool,
    ) -> Result<Imported> {
        let p = path.to_string_lossy().to_string();
        let mtime = std::fs::metadata(path)?
            .modified()
            .ok()
            .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        let existing: Option<Option<i64>> = self
            .db()
            .query_row("SELECT file_mtime FROM stems WHERE path = ?1", [&p], |r| {
                r.get(0)
            })
            .optional()?;
        if existing == Some(mtime) && !created {
            return Ok(Imported::Unchanged);
        }
        let info = tags::read_track(path, None);
        let art = match &info.picture {
            Some((bytes, ext)) => Some(self.store_art(bytes, ext)?),
            None => None,
        };
        {
            let db = self.db();
            db.execute(
                "UPDATE songs SET artist = CASE WHEN artist IN ('', 'Unknown artist') AND ?2 <> 'Unknown artist' THEN ?2 ELSE artist END, \
                 art = COALESCE(art, ?3), year = CASE WHEN year = '' THEN ?4 ELSE year END, duration = COALESCE(duration, ?5) WHERE id = ?1",
                params![song_id, info.artist, art, info.year, info.duration],
            )?;
            // A stem imported before stems were understood was a song of its own; it becomes a part again.
            db.execute("DELETE FROM songs WHERE path = ?1", [&p])?;
            db.execute(
                "INSERT INTO stems(song_id, name, path, file_mtime, folder_id) VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(path) DO UPDATE SET song_id = excluded.song_id, name = excluded.name, file_mtime = excluded.file_mtime",
                params![song_id, name, p, mtime, folder_id],
            )?;
        }
        if created || art.is_some() {
            self.write_cover(song_id)?;
            self.write_sidecar(song_id)?;
        }
        Ok(if created {
            Imported::Added {
                untagged: info.genre == genre::UNSORTED,
            }
        } else if existing.is_some() {
            Imported::Updated
        } else {
            Imported::Unchanged
        })
    }

    /// When a whole-song file arrives after its stems, the stems move over to it.
    fn adopt_stems(&self, song_id: i64, path: &Path) -> Result<()> {
        let (Some(dir), Some(base)) = (path.parent(), path.file_stem().and_then(|s| s.to_str()))
        else {
            return Ok(());
        };
        let groups = [
            Self::stem_group(dir, base),
            Self::stem_group(&dir.join(format!("{base} Stems")), base),
        ];
        let db = self.db();
        for g in groups {
            let orphan: Option<i64> = db
                .query_row("SELECT id FROM songs WHERE stem_group = ?1", [&g], |r| {
                    r.get(0)
                })
                .optional()?;
            if let Some(old) = orphan {
                db.execute(
                    "UPDATE stems SET song_id = ?1 WHERE song_id = ?2",
                    params![song_id, old],
                )?;
                db.execute("UPDATE songs SET art = COALESCE(art, (SELECT art FROM songs WHERE id = ?2)) WHERE id = ?1", params![song_id, old])?;
                db.execute("DELETE FROM songs WHERE id = ?1", [old])?;
            }
        }
        Ok(())
    }

    /// Removes stems-only songs whose stems are all gone.
    fn drop_empty_stem_songs(&self) -> Result<usize> {
        Ok(self.db().execute(
            "DELETE FROM songs WHERE stem_group IS NOT NULL AND path IS NULL AND NOT EXISTS (SELECT 1 FROM stems st WHERE st.song_id = songs.id)",
            [],
        )?)
    }

    pub fn stem_path(&self, id: i64) -> Result<Option<PathBuf>> {
        Ok(self
            .db()
            .query_row("SELECT path FROM stems WHERE id = ?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(PathBuf::from))
    }

    /// The watched folder a path lives in, if any.
    pub fn folder_for(&self, path: &Path) -> Result<Option<Folder>> {
        Ok(self
            .folders()?
            .into_iter()
            .filter(|f| path.starts_with(&f.path))
            .max_by_key(|f| f.path.len()))
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
        db.execute(
            "INSERT OR IGNORE INTO genres(name, origin, created_at) VALUES (?1, ?2, ?3)",
            params![name, origin, now()],
        )?;
        Ok(db.query_row(
            "SELECT name FROM genres WHERE name = ?1 COLLATE NOCASE",
            [&name],
            |r| r.get(0),
        )?)
    }

    pub fn create_genre(&self, raw: &str) -> Result<GenreTag> {
        let name = genre::normalize(raw);
        if name == genre::UNSORTED {
            bail!("Give the tag a name");
        }
        let exists: bool = self.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM genres WHERE name = ?1 COLLATE NOCASE)",
            [&name],
            |r| r.get(0),
        )?;
        if exists {
            bail!("There is already a tag called {name}");
        }
        self.ensure_genre(&name, "user")?;
        self.genres()?
            .into_iter()
            .find(|g| g.name.eq_ignore_ascii_case(&name))
            .ok_or_else(|| anyhow!("tag not saved"))
    }

    /// Renames a tag and every song that carries it. Renaming onto an existing tag merges the two.
    pub fn rename_genre(&self, id: i64, raw: &str) -> Result<()> {
        let name = genre::normalize(raw);
        if name == genre::UNSORTED {
            bail!("Give the tag a name");
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        let old: String =
            tx.query_row("SELECT name FROM genres WHERE id = ?1", [id], |r| r.get(0))?;
        let other: Option<i64> = tx
            .query_row(
                "SELECT id FROM genres WHERE name = ?1 COLLATE NOCASE AND id <> ?2",
                params![name, id],
                |r| r.get(0),
            )
            .optional()?;
        match other {
            Some(_) => {
                tx.execute("DELETE FROM genres WHERE id = ?1", [id])?;
            }
            None => {
                tx.execute(
                    "UPDATE genres SET name = ?2 WHERE id = ?1",
                    params![id, name],
                )?;
            }
        }
        tx.execute(
            "UPDATE songs SET genre = ?2, updated_at = ?3 WHERE genre = ?1 COLLATE NOCASE",
            params![old, name, now()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Deletes a tag; its songs go back to Unsorted.
    pub fn delete_genre(&self, id: i64) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let old: String =
            tx.query_row("SELECT name FROM genres WHERE id = ?1", [id], |r| r.get(0))?;
        tx.execute("DELETE FROM genres WHERE id = ?1", [id])?;
        tx.execute(
            "UPDATE songs SET genre = ?2, updated_at = ?3 WHERE genre = ?1 COLLATE NOCASE",
            params![old, genre::UNSORTED, now()],
        )?;
        tx.commit()?;
        Ok(())
    }

    // ---------- playlists ----------

    pub fn playlists(&self) -> Result<Vec<Playlist>> {
        let db = self.db();
        let mut lists = db
            .prepare("SELECT id, name FROM playlists ORDER BY created_at, id")?
            .query_map([], |r| {
                Ok(Playlist {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    songs: vec![],
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut st =
            db.prepare("SELECT song_id FROM playlist_songs WHERE playlist_id = ?1 ORDER BY pos")?;
        for p in &mut lists {
            p.songs = st
                .query_map([p.id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }
        Ok(lists)
    }

    pub fn create_playlist(&self, name: &str, songs: &[i64]) -> Result<Playlist> {
        let id = {
            let db = self.db();
            db.execute(
                "INSERT INTO playlists(name, created_at) VALUES (?1, ?2)",
                params![name.trim(), now()],
            )?;
            db.last_insert_rowid()
        };
        self.set_playlist_songs(id, songs)?;
        Ok(Playlist {
            id,
            name: name.trim().into(),
            songs: songs.to_vec(),
        })
    }

    pub fn rename_playlist(&self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            bail!("A playlist needs a name");
        }
        self.db().execute(
            "UPDATE playlists SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        Ok(())
    }

    pub fn delete_playlist(&self, id: i64) -> Result<()> {
        self.db()
            .execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_playlist_songs(&self, id: i64, songs: &[i64]) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute("DELETE FROM playlist_songs WHERE playlist_id = ?1", [id])?;
        let mut seen = std::collections::HashSet::new();
        for (pos, s) in songs.iter().filter(|s| seen.insert(**s)).enumerate() {
            tx.execute(
                "INSERT INTO playlist_songs(playlist_id, song_id, pos) VALUES (?1, ?2, ?3)",
                params![id, s, pos as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn toggle_in_playlist(&self, id: i64, song: i64, on: bool) -> Result<()> {
        let db = self.db();
        if on {
            let pos: i64 = db.query_row(
                "SELECT COALESCE(MAX(pos) + 1, 0) FROM playlist_songs WHERE playlist_id = ?1",
                [id],
                |r| r.get(0),
            )?;
            db.execute("INSERT OR IGNORE INTO playlist_songs(playlist_id, song_id, pos) VALUES (?1, ?2, ?3)", params![id, song, pos])?;
        } else {
            db.execute(
                "DELETE FROM playlist_songs WHERE playlist_id = ?1 AND song_id = ?2",
                params![id, song],
            )?;
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
            .map(|(k, v)| {
                (
                    k,
                    serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v)),
                )
            })
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

    pub fn setting_bool(&self, key: &str) -> bool {
        self.settings()
            .ok()
            .and_then(|m| m.get(key).and_then(|v| v.as_bool()))
            .unwrap_or(false)
    }

    pub fn setting_str(&self, key: &str) -> Option<String> {
        self.settings()
            .ok()?
            .get(key)
            .and_then(|v| v.as_str().map(String::from))
            .filter(|s| !s.is_empty())
    }

    // ---------- sheets (uploaded sheet music) ----------

    pub fn add_sheet(&self, song: i64, src: &Path) -> Result<Sheet> {
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow!("bad file name"))?
            .to_string();
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let kind = match ext.as_str() {
            "pdf" => "pdf",
            "musicxml" | "xml" => "musicxml",
            "png" | "jpg" | "jpeg" | "gif" | "webp" => "image",
            _ => bail!("Sheet music can be a PDF, an image, or a MusicXML file"),
        };
        let dir = self.extras_dir(song, "sheets")?;
        std::fs::create_dir_all(&dir)?;
        let dest = unique_path(&dir, &name);
        std::fs::copy(src, &dest).with_context(|| format!("copying {}", src.display()))?;
        let t = now();
        let db = self.db();
        db.execute(
            "INSERT INTO sheets(song_id, name, kind, path, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![song, name, kind, dest.to_string_lossy(), t],
        )?;
        Ok(Sheet {
            id: db.last_insert_rowid(),
            name,
            kind: kind.into(),
            added_at: t,
        })
    }

    pub fn remove_sheet(&self, id: i64) -> Result<()> {
        if let Some(p) = self.sheet_path(id)? {
            let _ = std::fs::remove_file(p);
        }
        self.db()
            .execute("DELETE FROM sheets WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---------- takes (audio made for a songwriter song) ----------

    pub fn add_take(&self, song: i64, src: &Path, origin: &str) -> Result<Take> {
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow!("bad file name"))?
            .to_string();
        if !tags::is_audio(src) {
            bail!("A take has to be an audio file (MP3, WAV, FLAC, M4A or OGG)");
        }
        let dir = self.extras_dir(song, "takes")?;
        std::fs::create_dir_all(&dir)?;
        let dest = unique_path(&dir, &name);
        std::fs::copy(src, &dest).with_context(|| format!("copying {}", src.display()))?;
        let t = now();
        let db = self.db();
        let version_id: Option<i64> = db
            .query_row("SELECT id FROM versions WHERE song_id = ?1 ORDER BY id DESC LIMIT 1", [song], |r| r.get(0))
            .optional()?;
        db.execute(
            "INSERT INTO takes(song_id, name, path, origin, added_at, version_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![song, name, dest.to_string_lossy(), origin, t, version_id],
        )?;
        let id = db.last_insert_rowid();
        db.execute(
            "UPDATE songs SET master_take = COALESCE(master_take, ?2), updated_at = ?3 WHERE id = ?1",
            params![song, id, t],
        )?;
        drop(db);
        if !self.art_from_takes(song)? {
            self.embed_art(song)?;
        }
        self.write_sidecar(song)?;
        Ok(Take {
            id,
            name,
            origin: origin.into(),
            added_at: t,
            version_id,
        })
    }

    /// Deletes a take and its file. When it was the master, the newest remaining take becomes the master.
    pub fn delete_take(&self, take: i64) -> Result<()> {
        let (song, path, master): (i64, String, Option<i64>) = self.db().query_row(
            "SELECT t.song_id, t.path, s.master_take FROM takes t JOIN songs s ON s.id = t.song_id WHERE t.id = ?1",
            [take],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        {
            let db = self.db();
            db.execute("DELETE FROM takes WHERE id = ?1", [take])?;
            if master == Some(take) {
                db.execute(
                    "UPDATE songs SET master_take = (SELECT id FROM takes WHERE song_id = ?1 ORDER BY id DESC LIMIT 1) WHERE id = ?1",
                    [song],
                )?;
            }
            db.execute("UPDATE songs SET updated_at = ?2 WHERE id = ?1", params![song, now()])?;
        }
        let _ = std::fs::remove_file(&path);
        self.write_sidecar(song)
    }

    /// Attaches stem files to a song; a folder brings every audio file inside it. Files already in the
    /// library folder move over, and any song they had become on their own is removed; others are copied.
    /// `to_flac`: store WAV files as FLAC (same audio, about half the size).
    pub fn attach_stems(&self, song: i64, paths: &[PathBuf], to_flac: bool) -> Result<Attached> {
        let title = self.song(song)?.title;
        let files = audio_files_in(paths)?;
        if files.is_empty() {
            bail!(
                "No audio files there. Stems have to be audio files (WAV, MP3, FLAC, M4A or OGG)."
            );
        }
        let dir = self.extras_dir(song, "stems")?;
        std::fs::create_dir_all(&dir)?;
        let lib_root = self.library_folder()?.map(|f| PathBuf::from(f.path));
        let folder_id: Option<i64> =
            self.db()
                .query_row("SELECT folder_id FROM songs WHERE id = ?1", [song], |r| {
                    r.get(0)
                })?;
        let mut emptied = Vec::new();
        let mut report = Attached { count: files.len(), ..Default::default() };
        for src in &files {
            let part = stem_part_name(src, &title);
            let ext = src
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("wav")
                .to_ascii_lowercase();
            let sp = src.to_string_lossy().to_string();
            let in_lib = lib_root.as_ref().is_some_and(|r| src.starts_with(r));
            let flac = (to_flac && ext == "wav")
                .then(|| {
                    unique_path(
                        &dir,
                        &format!("{}.flac", crate::organize::clean_name(&part)),
                    )
                })
                .filter(|f| match crate::audio::try_wav_to_flac(src, f) {
                    Ok(()) => {
                        report.flac += 1;
                        true
                    }
                    Err(why) => {
                        let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        report.kept_wav.push(format!("{name}: {why}"));
                        false
                    }
                });
            let dest = match flac {
                Some(f) => f,
                None => unique_path(
                    &dir,
                    &format!("{}.{ext}", crate::organize::clean_name(&part)),
                ),
            };
            let converted = dest.extension().is_some_and(|e| e == "flac") && ext == "wav";
            if converted {
                if in_lib {
                    std::fs::remove_file(src)?;
                }
            } else if in_lib {
                if std::fs::rename(src, &dest).is_err() {
                    std::fs::copy(src, &dest)
                        .with_context(|| format!("copying {}", src.display()))?;
                    std::fs::remove_file(src)?;
                }
            } else {
                std::fs::copy(src, &dest).with_context(|| format!("copying {}", src.display()))?;
            }
            if !in_lib {
                // Dropped into the app earlier, the same file was copied into the library as its own song or stem.
                if let Some(root) = &lib_root {
                    for (kind, id, path) in self.library_copies_of(src, root, song)? {
                        std::fs::remove_file(&path)?;
                        let db = self.db();
                        if kind == "song" {
                            db.execute("UPDATE songs SET path = NULL WHERE id = ?1", [id])?;
                            emptied.push(id);
                        } else {
                            let owner: i64 = db.query_row(
                                "SELECT song_id FROM stems WHERE id = ?1",
                                [id],
                                |r| r.get(0),
                            )?;
                            db.execute("DELETE FROM stems WHERE id = ?1", [id])?;
                            emptied.push(owner);
                        }
                    }
                }
            }
            let mtime = std::fs::metadata(&dest)?
                .modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            let dp = dest.to_string_lossy().to_string();
            let db = self.db();
            // A file that already was a stem of another song moves its row here.
            let was_stem: Option<(i64, i64)> = db
                .query_row(
                    "SELECT id, song_id FROM stems WHERE path = ?1",
                    [&sp],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            match (was_stem, in_lib) {
                (Some((sid, owner)), true) => {
                    db.execute("UPDATE stems SET song_id = ?2, name = ?3, path = ?4, file_mtime = ?5, folder_id = ?6 WHERE id = ?1", params![sid, song, part, dp, mtime, folder_id])?;
                    if owner != song {
                        emptied.push(owner);
                    }
                }
                _ => {
                    db.execute("INSERT INTO stems(song_id, name, path, file_mtime, folder_id) VALUES (?1, ?2, ?3, ?4, ?5)", params![song, part, dp, mtime, folder_id])?;
                }
            }
            // A file that had been imported as its own song stops being one.
            if in_lib {
                let was_song: Option<i64> = db
                    .query_row(
                        "SELECT id FROM songs WHERE path = ?1 AND id != ?2",
                        params![sp, song],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(other) = was_song {
                    db.execute("UPDATE songs SET path = NULL WHERE id = ?1", [other])?;
                    emptied.push(other);
                }
            }
        }
        for other in emptied {
            self.drop_if_empty(other)?;
        }
        self.db().execute(
            "UPDATE songs SET updated_at = ?2 WHERE id = ?1",
            params![song, now()],
        )?;
        self.art_from_takes(song)?;
        self.write_sidecar(song)?;
        Ok(report)
    }

    /// Converts WAV stems already in the library to FLAC (all songs, or one). The WAV is deleted only after
    /// its FLAC copy decodes to exactly the same samples. WAVs that can't convert are listed in `kept_wav`.
    pub fn convert_stems_to_flac(&self, song: Option<i64>) -> Result<FlacReport> {
        let rows: Vec<(i64, String)> = {
            let db = self.db();
            let mut st = db.prepare("SELECT id, path FROM stems WHERE (?1 IS NULL OR song_id = ?1) AND lower(path) LIKE '%.wav'")?;
            st.query_map([song], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut report = FlacReport::default();
        let mut songs = std::collections::BTreeSet::new();
        for (id, path) in rows {
            let wav = PathBuf::from(&path);
            let Some(dir) = wav.parent() else { continue };
            let name = wav.file_stem().and_then(|s| s.to_str()).unwrap_or("Stem");
            let flac = unique_path(dir, &format!("{name}.flac"));
            if let Err(why) = crate::audio::try_wav_to_flac(&wav, &flac) {
                report.kept_wav.push(format!("{}: {why}", wav.display()));
                continue;
            }
            let (b, a) = (
                std::fs::metadata(&wav)?.len(),
                std::fs::metadata(&flac)?.len(),
            );
            let mtime = std::fs::metadata(&flac)?
                .modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            let owner: i64 = {
                let db = self.db();
                db.execute(
                    "UPDATE stems SET path = ?2, file_mtime = ?3 WHERE id = ?1",
                    params![id, flac.to_string_lossy(), mtime],
                )?;
                db.query_row("SELECT song_id FROM stems WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?
            };
            std::fs::remove_file(&wav)?;
            songs.insert(owner);
            report.converted += 1;
            report.bytes_before += b;
            report.bytes_after += a;
        }
        for id in songs {
            self.write_sidecar(id)?;
        }
        Ok(report)
    }

    /// Songs and stems inside the library folder whose file is a byte-for-byte copy of `src`.
    fn library_copies_of(
        &self,
        src: &Path,
        root: &Path,
        except_song: i64,
    ) -> Result<Vec<(&'static str, i64, PathBuf)>> {
        let len = std::fs::metadata(src)?.len();
        let candidates: Vec<(&'static str, i64, String)> = {
            let db = self.db();
            let mut out = Vec::new();
            let mut st = db.prepare("SELECT id, path FROM songs WHERE path IS NOT NULL AND id != ?1 AND source = 'library'")?;
            out.extend(
                st.query_map([except_song], |r| Ok(("song", r.get(0)?, r.get(1)?)))?
                    .flatten(),
            );
            let mut st = db.prepare("SELECT id, path FROM stems WHERE song_id != ?1")?;
            out.extend(
                st.query_map([except_song], |r| Ok(("stem", r.get(0)?, r.get(1)?)))?
                    .flatten(),
            );
            out
        };
        let mut found = Vec::new();
        let mut want: Option<Vec<u8>> = None;
        for (kind, id, p) in candidates {
            let p = PathBuf::from(p);
            if !p.starts_with(root) || std::fs::metadata(&p).map(|m| m.len()).ok() != Some(len) {
                continue;
            }
            let a = match &want {
                Some(a) => a,
                None => want.insert(std::fs::read(src)?),
            };
            if std::fs::read(&p).is_ok_and(|b| &b == a) {
                found.push((kind, id, p));
            }
        }
        Ok(found)
    }

    /// Removes a library song left with no audio at all (its file and stems moved elsewhere), and its empty folder.
    fn drop_if_empty(&self, id: i64) -> Result<()> {
        let (dir, empty): (Option<String>, bool) = self.db().query_row(
            "SELECT song_dir, path IS NULL AND provider = 'local' AND source = 'library' \
             AND NOT EXISTS (SELECT 1 FROM stems WHERE song_id = ?1) AND NOT EXISTS (SELECT 1 FROM takes WHERE song_id = ?1) \
             FROM songs WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if !empty {
            return Ok(());
        }
        self.db().execute("DELETE FROM songs WHERE id = ?1", [id])?;
        if let Some(dir) = dir.map(PathBuf::from) {
            for d in [dir.join("stems"), dir.clone()] {
                let junk_only = std::fs::read_dir(&d)
                    .map(|r| {
                        r.flatten().all(|e| {
                            let n = e.file_name().to_string_lossy().to_lowercase();
                            n == "song.json"
                                || n == ".ds_store"
                                || n == "desktop.ini"
                                || n == "thumbs.db"
                                || n.starts_with("cover.")
                        })
                    })
                    .unwrap_or(false);
                if junk_only {
                    let _ = std::fs::remove_dir_all(&d);
                }
            }
        }
        Ok(())
    }

    pub fn set_master_take(&self, song: i64, take: i64) -> Result<()> {
        self.db().execute(
            "UPDATE songs SET master_take = ?2, updated_at = ?3 WHERE id = ?1",
            params![song, take, now()],
        )?;
        self.art_from_takes(song)?;
        Ok(())
    }

    /// A song without artwork takes the cover embedded in its audio: the master take first, then other takes,
    /// then its stems (Suno embeds the cover in MP3 downloads, not in M4A or WAV). Returns true when art was added.
    pub fn art_from_takes(&self, song: i64) -> Result<bool> {
        let paths: Vec<String> = {
            let db = self.db();
            let has_art: bool = db.query_row("SELECT art IS NOT NULL FROM songs WHERE id = ?1", [song], |r| r.get(0))?;
            if has_art {
                return Ok(false);
            }
            let mut st = db.prepare(
                "SELECT t.path FROM takes t JOIN songs s ON s.id = t.song_id WHERE t.song_id = ?1 \
                 ORDER BY t.id = s.master_take DESC, t.id DESC",
            )?;
            let mut paths: Vec<String> = st.query_map([song], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            let mut st = db.prepare("SELECT path FROM stems WHERE song_id = ?1 ORDER BY lower(path) LIKE '%.mp3' DESC, id")?;
            paths.extend(st.query_map([song], |r| r.get::<_, String>(0))?.flatten());
            paths
        };
        for p in paths {
            if let Some((bytes, ext)) = tags::read_track(Path::new(&p), None).picture {
                let name = self.store_art(&bytes, ext)?;
                self.db().execute("UPDATE songs SET art = ?2, updated_at = ?3 WHERE id = ?1", params![song, name, now()])?;
                self.write_cover(song)?;
                self.embed_art(song)?;
                self.write_sidecar(song)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Fills in artwork from takes or stems for songs that have none, and writes each song's artwork into its
    /// takes (songs from before this existed).
    pub fn backfill_take_art(&self) -> Result<usize> {
        let ids: Vec<i64> = {
            let db = self.db();
            let mut st = db.prepare(
                "SELECT id FROM songs WHERE art IS NULL AND (EXISTS (SELECT 1 FROM takes t WHERE t.song_id = songs.id) \
                 OR EXISTS (SELECT 1 FROM stems st WHERE st.song_id = songs.id))",
            )?;
            st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        let mut n = 0;
        for id in ids {
            n += self.art_from_takes(id)? as usize;
        }
        let with_art: Vec<i64> = {
            let db = self.db();
            let mut st = db.prepare("SELECT id FROM songs WHERE art IS NOT NULL AND EXISTS (SELECT 1 FROM takes t WHERE t.song_id = songs.id)")?;
            st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        for id in with_art {
            self.embed_art(id)?;
        }
        Ok(n)
    }

    pub fn set_status(&self, song: i64, status: &str) -> Result<()> {
        if !["writing", "ready"].contains(&status) {
            bail!("unknown status {status}");
        }
        self.db().execute(
            "UPDATE songs SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![song, status, now()],
        )?;
        Ok(())
    }

    // ---------- songwriter songs ----------

    /// Creates a song the songwriter is about to write.
    pub fn create_request(
        &self,
        request: &str,
        genre_hint: Option<&str>,
        writer: &str,
    ) -> Result<Song> {
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

    /// Moves the songwriter's credit to a new name on every song still credited to the old one.
    /// Songs whose artist the person changed by hand are left alone. Returns how many changed.
    pub fn rename_writer(&self, old: &str, new: &str) -> Result<usize> {
        if old == new {
            return Ok(0);
        }
        let ids: Vec<i64> = {
            let db = self.db();
            let mut st = db.prepare("SELECT id FROM songs WHERE source = 'ai' AND artist = ?1")?;
            let ids = st
                .query_map([old], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            db.execute(
                "UPDATE songs SET artist = ?2, updated_at = ?3 WHERE source = 'ai' AND artist = ?1",
                params![old, new, now()],
            )?;
            ids
        };
        for &id in &ids {
            self.write_sidecar(id)?;
        }
        Ok(ids.len())
    }

    /// Stores a draft or rewrite from the songwriter and records it as a new version.
    pub fn apply_draft(&self, id: i64, d: &Draft, note: &str, keep_genre: bool) -> Result<Song> {
        let t = now();
        let genre = if keep_genre {
            genre::UNSORTED.to_string()
        } else {
            self.ensure_genre(&d.genre, "writer")?
        };
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
            let n: i64 = db.query_row(
                "SELECT COUNT(*) FROM versions WHERE song_id = ?1",
                [id],
                |r| r.get(0),
            )?;
            db.execute(
                "INSERT INTO versions(song_id, label, note, title, lyrics, style, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, format!("v{}", n + 1), note, d.title, d.lyrics, d.style, t],
            )?;
        }
        if self.ensure_song_dir(id)?.is_some() {
            self.relocate(id)?;
        }
        self.song(id)
    }

    pub fn add_message(&self, id: i64, role: &str, text: &str) -> Result<()> {
        self.db().execute(
            "INSERT INTO messages(song_id, role, text, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, role, text, now()],
        )?;
        self.write_sidecar(id)
    }

    pub fn messages(&self, id: i64) -> Result<Vec<Message>> {
        Ok(self.song_detail(id)?.messages)
    }

    // ---------- file lookups for the audio/image protocol ----------

    /// The file to play for a song: its own file, or the master take for songwriter songs.
    pub fn audio_path(&self, id: i64) -> Result<Option<PathBuf>> {
        let db = self.db();
        let own: Option<Option<String>> = db
            .query_row("SELECT path FROM songs WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
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
        Ok(self
            .db()
            .query_row("SELECT path FROM takes WHERE id = ?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(PathBuf::from))
    }

    pub fn sheet_path(&self, id: i64) -> Result<Option<PathBuf>> {
        Ok(self
            .db()
            .query_row("SELECT path FROM sheets WHERE id = ?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(PathBuf::from))
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
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| s.starts_with('.') && s.len() > 1 && s != "..")
    })
}

/// Audio files among `paths`; folders bring every audio file inside them (hidden files skipped), sorted.
pub fn audio_files_in(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for p in paths {
        if p.is_dir() {
            let mut stack = vec![p.clone()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d)?.flatten() {
                    let path = e.path();
                    let hidden = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with('.'));
                    if hidden {
                        continue;
                    }
                    if path.is_dir() {
                        stack.push(path);
                    } else if tags::is_audio(&path) {
                        files.push(path);
                    }
                }
            }
        } else if tags::is_audio(p) {
            files.push(p.clone());
        }
    }
    files.sort();
    Ok(files)
}

/// A stem's part name: "Song (Vocals).wav" gives "Vocals"; "Song - Drums.wav" or "Drums.wav" gives "Drums".
fn stem_part_name(path: &Path, title: &str) -> String {
    if let Some((_, name)) = tags::stem_of(path) {
        return name;
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Stem")
        .trim();
    let rest = if !title.is_empty()
        && stem.len() > title.len()
        && stem[..title.len()].eq_ignore_ascii_case(title)
    {
        &stem[title.len()..]
    } else {
        stem
    };
    let rest = rest.trim_matches(|c: char| c.is_whitespace() || "-_()[].".contains(c));
    if rest.is_empty() {
        stem.to_string()
    } else {
        rest.to_string()
    }
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    (2..)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|c| !c.exists())
        .expect("unbounded range")
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
        assert_eq!(
            l.scan_folder(&f).unwrap().added,
            0,
            "unchanged files are skipped"
        );
        std::fs::remove_file(music.join("01 First.mp3")).unwrap();
        assert_eq!(l.scan_folder(&f).unwrap().removed, 1);
        assert!(l.songs().unwrap().is_empty());
    }

    #[test]
    fn stems_become_one_song_and_move_to_the_full_mix() {
        let (dir, l) = lib();
        let stems = dir.path().join("music/Every Leaf Stems");
        std::fs::create_dir_all(&stems).unwrap();
        for f in [
            "Every Leaf (Vocals).wav",
            "Every Leaf (Vocals).mp3",
            "Every Leaf (Drums).mp3",
            "Every Leaf (Backing Vocals).wav",
        ] {
            std::fs::write(stems.join(f), b"x").unwrap();
        }
        let f = l.add_folder(&dir.path().join("music")).unwrap();
        let r = l.scan_folder(&f).unwrap();
        assert_eq!(r.added, 1, "four stem files are one song");
        let songs = l.songs().unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!(
            (
                songs[0].title.as_str(),
                songs[0].stems,
                songs[0].has_file,
                songs[0].has_audio
            ),
            ("Every Leaf", 4, false, true)
        );
        assert_eq!(songs[0].album, "", "the 'X Stems' folder is not an album");
        assert_eq!(l.scan_folder(&f).unwrap().added, 0);

        // The full song arrives one folder up: the stems move to it and the stems-only song goes away.
        std::fs::write(dir.path().join("music/Every Leaf.mp3"), b"x").unwrap();
        l.scan_folder(&f).unwrap();
        let songs = l.songs().unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!((songs[0].stems, songs[0].has_file), (4, true));
        let detail = l.song_detail(songs[0].id).unwrap();
        assert_eq!(
            detail
                .stem_files
                .iter()
                .filter(|s| s.name == "Vocals")
                .count(),
            2,
            "wav and mp3 of one part"
        );

        // Deleting the stems leaves the full song.
        std::fs::remove_dir_all(&stems).unwrap();
        l.scan_folder(&f).unwrap();
        assert_eq!(l.songs().unwrap()[0].stems, 0);
    }

    #[test]
    fn stems_imported_as_songs_before_are_regrouped() {
        let (dir, l) = lib();
        let music = dir.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        for f in ["Tune (Vocals).mp3", "Tune (Bass).mp3"] {
            std::fs::write(music.join(f), b"x").unwrap();
        }
        let f = l.add_folder(&music).unwrap();
        // What 0.1.0 did: every stem file became its own song.
        let t = now();
        for f in ["Tune (Vocals).mp3", "Tune (Bass).mp3"] {
            l.db().execute(
                "INSERT INTO songs(source, title, path, folder_id, added_at, updated_at) VALUES ('library', ?1, ?2, 1, ?3, ?3)",
                params![f, music.canonicalize().unwrap().join(f).to_string_lossy(), t],
            ).unwrap();
        }
        assert_eq!(l.songs().unwrap().len(), 2);
        l.scan_folder(&f).unwrap();
        let songs = l.songs().unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!((songs[0].title.as_str(), songs[0].stems), ("Tune", 2));
    }

    #[test]
    fn playlists_keep_order_and_membership() {
        let (_d, l) = lib();
        let a = l
            .create_request("a folk song", Some("folk"), "Bard")
            .unwrap();
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
    fn renaming_the_writer_moves_its_credit() {
        let (_d, l) = lib();
        let a = l.create_request("a folk song", None, "Bard").unwrap();
        let b = l.create_request("a rock song", None, "Bard").unwrap();
        l.update_song(
            b.id,
            &SongPatch {
                artist: Some("Me".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(l.rename_writer("Bard", "Ari").unwrap(), 1);
        assert_eq!(l.song(a.id).unwrap().artist, "Ari");
        assert_eq!(
            l.song(b.id).unwrap().artist,
            "Me",
            "a hand-set artist stays"
        );
    }

    #[test]
    fn stems_attach_to_a_song_from_a_folder() {
        let (d, l) = lib();
        let s = l.create_request("a song", None, "Bard").unwrap();
        l.apply_draft(
            s.id,
            &Draft {
                title: "Porch Light".into(),
                lyrics: "x".into(),
                ..Default::default()
            },
            "v",
            false,
        )
        .unwrap();
        let src = d.path().join("Porch Light Stems");
        std::fs::create_dir_all(&src).unwrap();
        for f in [
            "Porch Light (Vocals).wav",
            "Porch Light - Drums.mp3",
            "Bass.flac",
            "notes.txt",
        ] {
            std::fs::write(src.join(f), b"x").unwrap();
        }
        assert_eq!(l.attach_stems(s.id, &[src.clone()], false).unwrap().count, 3);
        let mut names: Vec<String> = l
            .song_detail(s.id)
            .unwrap()
            .stem_files
            .into_iter()
            .map(|f| f.name)
            .collect();
        names.sort();
        assert_eq!(names, ["Bass", "Drums", "Vocals"]);
        assert!(
            src.join("Bass.flac").exists(),
            "files outside the library are copied, not moved"
        );
        assert!(
            l.attach_stems(s.id, &[d.path().join("nothing-here")], false)
                .is_err()
        );
        let bad = d.path().join("Broken (Synth).wav");
        std::fs::write(&bad, b"not really a wav").unwrap();
        let r = l.attach_stems(s.id, &[bad], true).unwrap();
        assert_eq!((r.count, r.flac, r.kept_wav.len()), (1, 0, 1), "a WAV that can't convert is kept and reported");
        assert!(r.kept_wav[0].starts_with("Broken (Synth).wav: "), "{:?}", r.kept_wav);
    }

    #[test]
    fn attaching_stems_that_became_songs_removes_those_songs() {
        let (d, l) = lib();
        let root = d.path().join("Library");
        let f = l.set_library_folder(&root).unwrap();
        let stray = root.join("Songs/Guitar/Guitar.mp3");
        std::fs::create_dir_all(stray.parent().unwrap()).unwrap();
        std::fs::write(&stray, b"x").unwrap();
        l.scan_folder(&f).unwrap();
        assert_eq!(l.songs().unwrap().len(), 1);
        let s = l.create_request("a song", None, "Bard").unwrap();
        l.apply_draft(
            s.id,
            &Draft {
                title: "Porch Light".into(),
                lyrics: "x".into(),
                ..Default::default()
            },
            "v",
            false,
        )
        .unwrap();
        assert_eq!(l.attach_stems(s.id, &[stray.clone()], false).unwrap().count, 1);
        let songs = l.songs().unwrap();
        assert_eq!(songs.len(), 1, "the stray song is gone");
        assert_eq!(songs[0].stems, 1);
        assert!(
            !stray.exists() && !root.join("Songs/Guitar").exists(),
            "moved, and its emptied folder removed"
        );
        l.scan_folder(&f).unwrap();
        assert_eq!(
            l.songs().unwrap().len(),
            1,
            "a rescan does not bring it back"
        );
        assert_eq!(l.song(s.id).unwrap().stems, 1);
    }

    #[test]
    fn attaching_the_original_folder_folds_in_library_copies() {
        let (d, l) = lib();
        let root = d.path().join("Library");
        let f = l.set_library_folder(&root).unwrap();
        let outside = d.path().join("Downloads/Porch Light Stems");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("Porch Light (Bass).mp3"), b"bass bytes").unwrap();
        let copy = root.join("Songs/Bass/Bass.mp3");
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::write(&copy, b"bass bytes").unwrap();
        l.scan_folder(&f).unwrap();
        let s = l.create_request("a song", None, "Bard").unwrap();
        l.apply_draft(
            s.id,
            &Draft {
                title: "Porch Light".into(),
                lyrics: "x".into(),
                ..Default::default()
            },
            "v",
            false,
        )
        .unwrap();
        l.attach_stems(s.id, &[outside.clone()], false).unwrap();
        assert_eq!(
            l.songs().unwrap().len(),
            1,
            "the library copy's song is gone"
        );
        assert!(!copy.exists() && outside.join("Porch Light (Bass).mp3").exists());
    }

    #[test]
    fn takes_join_their_version_and_can_be_deleted() {
        let (d, l) = lib();
        let s = l.create_request("a song", None, "Bard").unwrap();
        l.apply_draft(s.id, &Draft { title: "Porch Light".into(), lyrics: "x".into(), ..Default::default() }, "v", false).unwrap();
        let v1 = l.song_detail(s.id).unwrap().versions[0].id;
        let f = d.path().join("take.mp3");
        std::fs::write(&f, b"x").unwrap();
        let a = l.add_take(s.id, &f, "from Suno").unwrap();
        let b = l.add_take(s.id, &f, "from Suno").unwrap();
        assert_eq!((a.version_id, l.song(s.id).unwrap().status.as_deref()), (Some(v1), Some("ready")), "no take stage");
        assert_eq!(l.song(s.id).unwrap().master_take, Some(a.id));
        l.delete_take(a.id).unwrap();
        assert_eq!(l.song(s.id).unwrap().master_take, Some(b.id), "the newest remaining take becomes master");
        l.delete_take(b.id).unwrap();
        assert_eq!(l.song(s.id).unwrap().master_take, None);
    }

    #[test]
    fn drafts_become_versions() {
        let (_d, l) = lib();
        let s = l
            .create_request("sad country song", Some("Country"), "Bard")
            .unwrap();
        let d = Draft {
            title: "Porch Light".into(),
            genre: "Country".into(),
            lyrics: "[Verse]\nHello".into(),
            bpm: 80,
            ..Default::default()
        };
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
        assert!(
            l.genres()
                .unwrap()
                .iter()
                .any(|g| g.name == "Rock" && g.origin == "default")
        );
        let s = l
            .create_request("a song", Some("sea shanty"), "Bard")
            .unwrap();
        assert_eq!(s.genre, "Sea Shanty");
        let tag = l
            .genres()
            .unwrap()
            .into_iter()
            .find(|g| g.name == "Sea Shanty")
            .unwrap();
        assert_eq!((tag.origin.as_str(), tag.songs), ("user", 1));
        assert!(
            l.create_genre("sea SHANTY").is_err(),
            "duplicates are refused"
        );
        l.rename_genre(tag.id, "Shanty").unwrap();
        assert_eq!(l.song(s.id).unwrap().genre, "Shanty");
        l.delete_genre(tag.id).unwrap();
        assert_eq!(l.song(s.id).unwrap().genre, "Unsorted");
        let rock = l.ensure_genre("rock", "writer").unwrap();
        assert_eq!(rock, "Rock", "existing tags keep their spelling");
    }

    #[test]
    fn streaming_songs_save_once() {
        let (_d, l) = lib();
        let e = ExternalSong {
            provider: "apple".into(),
            provider_id: "1440833098".into(),
            title: "Song".into(),
            artist: "Band".into(),
            album: "Album".into(),
            genre: "Alternative".into(),
            year: "2019".into(),
            track: Some(3),
            duration: Some(201.0),
            art_url: Some("https://is1-ssl.mzstatic.com/{w}x{h}bb.jpg".into()),
        };
        let a = l.save_external_song(&e).unwrap();
        let b = l.save_external_song(&e).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(
            (a.provider.as_str(), a.provider_id.as_deref(), a.has_audio),
            ("apple", Some("1440833098"), false)
        );
        assert_eq!(l.songs().unwrap().len(), 1);
        assert!(
            l.save_external_song(&ExternalSong {
                provider: "nope".into(),
                ..e
            })
            .is_err()
        );
    }

    #[test]
    fn settings_round_trip() {
        let (_d, l) = lib();
        l.set_setting("sidebarDefault", &serde_json::json!("genres"))
            .unwrap();
        assert_eq!(l.setting_str("sidebarDefault").as_deref(), Some("genres"));
    }
}
