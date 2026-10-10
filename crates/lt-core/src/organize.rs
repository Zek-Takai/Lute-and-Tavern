//! The managed library folder. Files dropped into it are moved into place, and every song keeps
//! everything about it in its own folder:
//!
//! ```text
//! <library>/Songs/<Title>/<Title>.<ext>        the song (songwriter songs, singles, anything with stems)
//! <library>/Songs/<Title>/stems/<Part>.<ext>   its parts: Vocals, Drums...
//! <library>/Songs/<Title>/takes/  sheets/      takes and sheet music added in the app
//! <library>/Songs/<Title>/cover.<ext>          artwork
//! <library>/Songs/<Title>/song.json            everything the app knows: tags, lyrics, Suno fields, versions
//! <library>/Albums/<Album artist>/<Album>/NN - <Title>.<ext>   albums, kept together (compilations under their album artist)
//! ```
//!
//! Only the library folder is organized; other music folders are watched and never changed.

use crate::library::{Folder, Library, now};
use crate::tags;
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const SONGS: &str = "Songs";
pub const ALBUMS: &str = "Albums";
/// Programs and data that aren't songs (the show analyzer and its Python, made shows). Never organized, scanned
/// for songs or tidied; the underscore keeps it at the top of the folder list.
pub const SOFTWARE: &str = "_Software";
pub const SIDECAR: &str = "song.json";

/// A file or folder name that is safe on macOS, Windows and Linux.
pub fn clean_name(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let squeezed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed: String = squeezed
        .trim_matches(|c: char| c == '.' || c == ' ')
        .chars()
        .take(100)
        .collect();
    if trimmed.is_empty() {
        "Untitled".into()
    } else {
        trimmed
    }
}

/// Where a file sits inside the library folder.
#[derive(Debug, PartialEq)]
pub enum Place {
    /// Directly in a song folder: the song itself.
    Main(PathBuf),
    /// In a song folder's `stems/`: one part, named after the file.
    Stem(PathBuf, String),
    /// In a song folder's `takes/` or `sheets/`: managed by the app, never a song of its own.
    Extra,
    /// In `Albums/<Artist>/<Album>/`.
    Album,
    /// Anywhere else: waiting to be put in place.
    Loose,
}

pub fn place_of(root: &Path, path: &Path) -> Place {
    let Ok(rel) = path.strip_prefix(root) else {
        return Place::Loose;
    };
    let parts: Vec<&str> = rel.iter().filter_map(|s| s.to_str()).collect();
    match parts.as_slice() {
        [s, ..] if *s == SOFTWARE => Place::Extra,
        [s, dir, file] if *s == SONGS => {
            if tags::stem_of(Path::new(file)).is_some() {
                Place::Loose
            } else {
                Place::Main(root.join(SONGS).join(dir))
            }
        }
        [s, dir, sub, file] if *s == SONGS && sub.eq_ignore_ascii_case("stems") => {
            let name = Path::new(file)
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("Part")
                .to_string();
            Place::Stem(root.join(SONGS).join(dir), name)
        }
        [s, _, sub, ..]
            if *s == SONGS
                && (sub.eq_ignore_ascii_case("takes") || sub.eq_ignore_ascii_case("sheets")) =>
        {
            Place::Extra
        }
        [a, _, _, _] if *a == ALBUMS => Place::Album,
        _ => Place::Loose,
    }
}

/// A file still being copied in is left alone until it has been quiet for a moment.
fn settled(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| {
            SystemTime::now().duration_since(t).unwrap_or_default() >= Duration::from_millis(1500)
        })
        .unwrap_or(false)
}

fn free_path(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }
    let stem = target
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = target
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let dir = target.parent().unwrap_or(Path::new(""));
    (2..)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|c| !c.exists())
        .expect("unbounded range")
}

/// Whether two paths are the same file or folder on disk (e.g. only the letter case differs on macOS or
/// Windows).
fn same_entry(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Renames in two steps, for a change of letter case only on a case-insensitive disk.
fn rename_case(from: &Path, to: &Path) -> std::io::Result<()> {
    let tmp = from.with_file_name(format!(".lt-rename-{}", std::process::id()));
    std::fs::rename(from, &tmp)?;
    std::fs::rename(&tmp, to).or_else(|e| {
        let _ = std::fs::rename(&tmp, from);
        Err(e)
    })
}

/// Moves a file, falling back to copy-and-delete across drives. Never overwrites. If the original can't be
/// removed, the copy is removed again, so a failed move leaves nothing behind.
pub fn move_file(from: &Path, to: &Path) -> Result<PathBuf> {
    if to.exists() && same_entry(from, to) {
        rename_case(from, to).with_context(|| format!("renaming {}", from.display()))?;
        return Ok(to.to_path_buf());
    }
    let to = free_path(to);
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    if std::fs::rename(from, &to).is_err() {
        std::fs::copy(from, &to)
            .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
        if let Err(e) = std::fs::remove_file(from) {
            let _ = std::fs::remove_file(&to);
            return Err(e).with_context(|| format!("{} is in use or read-only, so it was left where it is", from.display()));
        }
    }
    Ok(to)
}

/// Removes folders left empty by moves, never the library folder or its Songs/Albums folders.
fn prune_empty_dirs(root: &Path) {
    let keep = [root.to_path_buf(), root.join(SONGS), root.join(ALBUMS)];
    let mut dirs: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.depth() != 1 || e.file_name() != SOFTWARE)
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.into_path())
        .collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        if keep.contains(&d) {
            continue;
        }
        let only_junk = std::fs::read_dir(&d)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .all(|e| e.file_name() == ".DS_Store")
            })
            .unwrap_or(false);
        if only_junk {
            let _ = std::fs::remove_file(d.join(".DS_Store"));
            let _ = std::fs::remove_dir(&d);
        }
    }
}

impl Library {
    /// The folder the app organizes, if one is set.
    pub fn library_folder(&self) -> Result<Option<Folder>> {
        Ok(self.folders()?.into_iter().find(|f| f.managed))
    }

    /// Makes `path` the library folder (creating it if needed) and organizes what is already in it.
    pub fn set_library_folder(&self, path: &Path) -> Result<Folder> {
        std::fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))?;
        let f = self.add_folder(path)?;
        self.db()
            .execute("UPDATE folders SET managed = (id = ?1)", [f.id])?;
        Ok(Folder { managed: true, ..f })
    }

    pub(crate) fn managed_root(&self, folder_id: Option<i64>) -> Option<PathBuf> {
        let id = folder_id?;
        self.db()
            .query_row(
                "SELECT path FROM folders WHERE id = ?1 AND managed = 1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .ok()?
            .map(PathBuf::from)
    }

    /// The Songs/<Title> folder for a song. A folder already holding a different song of the same
    /// title gets the artist added to the name.
    pub(crate) fn song_dir_for(
        &self,
        root: &Path,
        title: &str,
        artist: &str,
        me: Option<i64>,
    ) -> Result<PathBuf> {
        let songs = root.join(SONGS);
        let unknown = |a: &str| a.is_empty() || a == "Unknown artist";
        let candidates = [
            clean_name(title),
            clean_name(&format!("{title} ({artist})")),
        ];
        for (i, name) in candidates
            .iter()
            .chain(
                (2..50)
                    .map(|n| format!("{} ({n})", clean_name(title)))
                    .collect::<Vec<_>>()
                    .iter(),
            )
            .enumerate()
        {
            if i == 1 && unknown(artist) {
                continue;
            }
            let dir = songs.join(name);
            let owner: Option<(i64, String, String)> = self
                .db()
                .query_row(
                    "SELECT id, title, artist FROM songs WHERE song_dir = ?1",
                    [dir.to_string_lossy()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            match owner {
                None => return Ok(dir),
                Some((id, _, _)) if Some(id) == me => return Ok(dir),
                Some((_, t, a))
                    if t.eq_ignore_ascii_case(title)
                        && (a == artist || unknown(&a) || unknown(artist))
                        && me.is_none() =>
                {
                    return Ok(dir);
                }
                _ => continue,
            }
        }
        Ok(songs.join(format!("{} {}", clean_name(title), now())))
    }

    /// Puts loose files in the library folder where they belong. Returns how many files moved.
    /// Files still being written (changed in the last moment) wait for the next pass.
    pub fn organize(&self, folder: &Folder) -> Result<usize> {
        self.organize_files(folder, true)
    }

    /// Organizes and scans right away, for files the app itself just moved in (they are complete).
    pub fn scan_folder_now(&self, folder: &Folder) -> Result<crate::library::ScanReport> {
        self.organize_files(folder, false)?;
        self.scan_folder(folder)
    }

    fn organize_files(&self, folder: &Folder, wait_for_writes: bool) -> Result<usize> {
        if !folder.managed {
            return Ok(0);
        }
        let root = PathBuf::from(&folder.path);
        let loose: Vec<PathBuf> = walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_entry(|e| e.depth() != 1 || e.file_name() != SOFTWARE)
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file() && tags::is_audio(e.path()))
            .map(|e| e.into_path())
            .filter(|p| {
                let rel = p.strip_prefix(&root).unwrap_or(p);
                !rel.iter()
                    .any(|c| c.to_str().is_some_and(|s| s.starts_with('.')))
                    && place_of(&root, p) == Place::Loose
                    && crate::sync::conflict_of(p).is_none()
                    && (!wait_for_writes || settled(p))
            })
            .collect();
        // Whole songs first so their folders exist before their stems arrive.
        let mut loose = loose;
        loose.sort_by_key(|p| (tags::stem_of(p).is_some(), p.clone()));
        let mut moved = 0;
        for p in loose {
            if self.organize_failed.lock().unwrap_or_else(|e| e.into_inner()).contains(&p) {
                continue;
            }
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp3")
                .to_ascii_lowercase();
            let (target, song_dir) = if let Some((base, name)) = tags::stem_of(&p) {
                let dir = self.song_dir_for(&root, &base, "", None)?;
                (
                    dir.join("stems")
                        .join(format!("{}.{ext}", clean_name(&name))),
                    None,
                )
            } else {
                // Tags only: folder names around a loose file are not an album.
                let info = tags::read_track(&p, None);
                let album = info.album.trim();
                if !album.is_empty() && !album.to_lowercase().ends_with(" stems") {
                    let n = info.track.map(|t| format!("{t:02} - ")).unwrap_or_default();
                    let album_artist = if info.album_artist.is_empty() {
                        &info.artist
                    } else {
                        &info.album_artist
                    };
                    let f = root
                        .join(ALBUMS)
                        .join(clean_name(album_artist))
                        .join(clean_name(album))
                        .join(format!("{n}{}.{ext}", clean_name(&info.title)));
                    (f, None)
                } else {
                    let dir = self.song_dir_for(&root, &info.title, &info.artist, None)?;
                    (
                        dir.join(format!("{}.{ext}", clean_name(&info.title))),
                        Some(dir),
                    )
                }
            };
            if target == p {
                continue;
            }
            // A file that can't be moved (in use, read-only) stays where it is and is still imported there.
            let new = match move_file(&p, &target) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("organize: {e:#}");
                    self.organize_failed.lock().unwrap_or_else(|e| e.into_inner()).insert(p.clone());
                    continue;
                }
            };
            let (old_s, new_s) = (
                p.to_string_lossy().to_string(),
                new.to_string_lossy().to_string(),
            );
            let db = self.db();
            db.execute(
                "UPDATE songs SET path = ?2, song_dir = COALESCE(?3, song_dir) WHERE path = ?1",
                params![
                    old_s,
                    new_s,
                    song_dir.map(|d| d.to_string_lossy().to_string())
                ],
            )?;
            db.execute(
                "UPDATE stems SET path = ?2 WHERE path = ?1",
                params![old_s, new_s],
            )?;
            moved += 1;
        }
        if moved > 0 {
            prune_empty_dirs(&root);
        }
        Ok(moved)
    }

    /// Finds or creates the song a song folder belongs to (used for stems in `Songs/<Title>/stems/`).
    pub(crate) fn song_for_dir(&self, dir: &Path, folder_id: Option<i64>) -> Result<(i64, bool)> {
        let d = dir.to_string_lossy().to_string();
        let found: Option<i64> = self
            .db()
            .query_row("SELECT id FROM songs WHERE song_dir = ?1", [&d], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(id) = found {
            return Ok((id, false));
        }
        let title = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled")
            .to_string();
        let t = now();
        let id = {
            let db = self.db();
            db.execute(
                "INSERT INTO songs(source, title, folder_id, song_dir, stem_group, added_at, updated_at) VALUES ('library', ?1, ?2, ?3, ?3, ?4, ?4)",
                params![title, folder_id, d, t],
            )?;
            db.last_insert_rowid()
        };
        self.read_sidecar(id)?;
        Ok((id, true))
    }

    /// When a song folder's own file is imported, a song already made for that folder (from its stems,
    /// or by the songwriter) takes the file instead of a second song appearing.
    pub(crate) fn adopt_main_file(
        &self,
        dir: &Path,
        path: &Path,
        mtime: Option<i64>,
        duration: Option<f64>,
    ) -> Result<Option<i64>> {
        let db = self.db();
        let owner: Option<i64> = db
            .query_row(
                "SELECT id FROM songs WHERE song_dir = ?1 AND path IS NULL",
                [dir.to_string_lossy()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = owner {
            db.execute(
                "UPDATE songs SET path = ?2, file_mtime = ?3, duration = COALESCE(duration, ?4), stem_group = NULL, updated_at = ?5 WHERE id = ?1",
                params![id, path.to_string_lossy(), mtime, duration, now()],
            )?;
        }
        Ok(owner)
    }

    /// Gives a songwriter song its folder in the library once it has a title.
    pub fn ensure_song_dir(&self, id: i64) -> Result<Option<PathBuf>> {
        let (dir, title, artist): (Option<String>, String, String) = self.db().query_row(
            "SELECT song_dir, title, artist FROM songs WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if let Some(d) = dir {
            return Ok(Some(PathBuf::from(d)));
        }
        let Some(lib) = self.library_folder()? else {
            return Ok(None);
        };
        let root = PathBuf::from(&lib.path);
        let d = self.song_dir_for(&root, &title, &artist, Some(id))?;
        std::fs::create_dir_all(&d)?;
        self.db().execute(
            "UPDATE songs SET song_dir = ?2, folder_id = COALESCE(folder_id, ?3) WHERE id = ?1",
            params![id, d.to_string_lossy(), lib.id],
        )?;
        self.write_sidecar(id)?;
        Ok(Some(d))
    }

    /// Renames a song's folder and file after its title or artist changes, keeping every stored path right.
    pub fn relocate(&self, id: i64) -> Result<()> {
        let (dir, path, title, artist, folder_id): (
            Option<String>,
            Option<String>,
            String,
            String,
            Option<i64>,
        ) = self.db().query_row(
            "SELECT song_dir, path, title, artist, folder_id FROM songs WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        let Some(root) = self.managed_root(folder_id) else {
            return Ok(());
        };
        let Some(dir) = dir.map(PathBuf::from) else {
            return Ok(());
        };
        let want = self.song_dir_for(&root, &title, &artist, Some(id))?;
        let mut dir_now = dir.clone();
        if want != dir && dir.exists() {
            let want = if want.exists() && same_entry(&dir, &want) {
                rename_case(&dir, &want).with_context(|| format!("renaming {}", dir.display()))?;
                want
            } else {
                let want = free_path(&want);
                std::fs::rename(&dir, &want).with_context(|| format!("renaming {}", dir.display()))?;
                want
            };
            let new = want.to_string_lossy().to_string();
            let db = self.db();
            // Paths inside the old folder, matched exactly (not with LIKE: '_' and '%' in names are wildcards).
            for table in ["songs", "stems", "takes", "sheets"] {
                let rows: Vec<(i64, String)> = {
                    let mut st = db.prepare(&format!("SELECT rowid, path FROM {table} WHERE path IS NOT NULL"))?;
                    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(|r| r.ok()).collect()
                };
                for (rowid, p) in rows {
                    if let Ok(rest) = Path::new(&p).strip_prefix(&dir) {
                        db.execute(&format!("UPDATE {table} SET path = ?2 WHERE rowid = ?1"), params![rowid, want.join(rest).to_string_lossy()])?;
                    }
                }
            }
            db.execute("UPDATE songs SET song_dir = ?2, stem_group = CASE WHEN stem_group IS NULL THEN NULL ELSE ?2 END WHERE id = ?1", params![id, new])?;
            dir_now = want;
        }
        // The song's own file follows its title.
        let path: Option<String> = if dir_now != dir {
            self.db()
                .query_row("SELECT path FROM songs WHERE id = ?1", [id], |r| r.get(0))?
        } else {
            path
        };
        if let Some(p) = path.map(PathBuf::from)
            && p.parent() == Some(dir_now.as_path())
        {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("mp3");
            let target = dir_now.join(format!("{}.{ext}", clean_name(&title)));
            if target != p && p.exists() {
                let new = move_file(&p, &target)?;
                self.db().execute(
                    "UPDATE songs SET path = ?2 WHERE id = ?1",
                    params![id, new.to_string_lossy()],
                )?;
            }
        }
        self.write_sidecar(id)
    }

    /// Where a song's takes or sheets go: its folder in the library, or the app's own data folder.
    pub(crate) fn extras_dir(&self, id: i64, kind: &str) -> Result<PathBuf> {
        match self.ensure_song_dir(id)? {
            Some(d) => Ok(d.join(kind)),
            None => Ok(match kind {
                "takes" => self.dirs.takes.join(id.to_string()),
                "stems" => self.dirs.takes.join(id.to_string()).join("stems"),
                _ => self.dirs.sheets.join(id.to_string()),
            }),
        }
    }

    /// Copies the song's artwork into its folder as cover.<ext>.
    pub(crate) fn write_cover(&self, id: i64) -> Result<()> {
        let (dir, art): (Option<String>, Option<String>) =
            self.db()
                .query_row("SELECT song_dir, art FROM songs WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
        let (Some(dir), Some(art)) = (dir, art) else {
            return Ok(());
        };
        let Some(src) = self.art_path(&art) else {
            return Ok(());
        };
        let dir = PathBuf::from(dir);
        if !dir.exists() {
            return Ok(());
        }
        for old in std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
        {
            let image = old.extension().and_then(|e| e.to_str()).is_some_and(|e| ["jpg", "jpeg", "png", "gif", "webp", "bmp"].contains(&e.to_ascii_lowercase().as_str()));
            if image && old.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("cover")) {
                let _ = std::fs::remove_file(old);
            }
        }
        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
        std::fs::copy(&src, dir.join(format!("cover.{ext}")))?;
        Ok(())
    }

    /// Writes song.json: everything the app knows about the song, so the folder stands on its own.
    pub fn write_sidecar(&self, id: i64) -> Result<()> {
        let dir: Option<String> =
            self.db()
                .query_row("SELECT song_dir FROM songs WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?;
        let Some(dir) = dir.map(PathBuf::from) else {
            return Ok(());
        };
        if !dir.exists() {
            return Ok(());
        }
        let d = self.song_detail(id)?;
        let s = &d.song;
        let versions: Vec<Value> = {
            let db = self.db();
            let mut st = db.prepare("SELECT label, note, title, lyrics, style, created_at FROM versions WHERE song_id = ?1 ORDER BY id")?;
            st.query_map([id], |r| {
                Ok(json!({"label": r.get::<_, String>(0)?, "note": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?,
                          "lyrics": r.get::<_, String>(3)?, "style": r.get::<_, String>(4)?, "createdAt": r.get::<_, i64>(5)?}))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let doc = json!({
            "app": "Lute & Tavern", "format": 1, "uid": s.uid,
            "title": s.title, "artist": s.artist, "album": s.album, "year": s.year, "genre": s.genre,
            "source": s.source, "request": s.request, "status": s.status,
            "subGenre": s.sub_genre, "key": s.key, "bpm": s.bpm,
            "suno": {"style": s.style, "exclude": s.exclude, "lyrics": s.lyrics},
            "sheetMusic": {"notes": s.notes, "chords": s.chords},
            "versions": versions,
            "conversation": d.messages.iter().map(|m| json!({"role": m.role, "text": m.text, "createdAt": m.created_at})).collect::<Vec<_>>(),
        });
        let tmp = dir.join(".song.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&doc)?)?;
        std::fs::rename(&tmp, dir.join(SIDECAR))?;
        Ok(())
    }

    /// Restores what a song.json knows when a song folder arrives (for example copied from another Mac).
    pub(crate) fn read_sidecar(&self, id: i64) -> Result<()> {
        let dir: Option<String> =
            self.db()
                .query_row("SELECT song_dir FROM songs WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?;
        let Some(file) = dir.map(|d| PathBuf::from(d).join(SIDECAR)) else {
            return Ok(());
        };
        let Ok(text) = std::fs::read_to_string(&file) else {
            return Ok(());
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            return Ok(());
        };
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        let genre = self.ensure_genre(&s("genre"), "file")?;
        let source = if s("source") == "ai" { "ai" } else { "library" };
        let db = self.db();
        db.execute(
            "UPDATE songs SET title = CASE WHEN ?2 = '' THEN title ELSE ?2 END, artist = ?3, album = ?4, year = ?5, genre = ?6, source = ?7, \
             request = ?8, status = ?9, sub_genre = ?10, song_key = ?11, bpm = ?12, style = ?13, exclude = ?14, lyrics = ?15, \
             notes_json = ?16, chords_json = ?17 WHERE id = ?1",
            params![
                id, s("title"), s("artist"), s("album"), s("year"), genre, source,
                v["request"].as_str(), v["status"].as_str(), s("subGenre"), s("key"), v["bpm"].as_i64().unwrap_or(0),
                v["suno"]["style"].as_str().unwrap_or(""), v["suno"]["exclude"].as_str().unwrap_or(""), v["suno"]["lyrics"].as_str().unwrap_or(""),
                v["sheetMusic"]["notes"].to_string(), v["sheetMusic"]["chords"].to_string(),
            ],
        )?;
        for ver in v["versions"].as_array().into_iter().flatten() {
            db.execute(
                "INSERT INTO versions(song_id, label, note, title, lyrics, style, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, ver["label"].as_str().unwrap_or("v"), ver["note"].as_str().unwrap_or(""), ver["title"].as_str().unwrap_or(""),
                        ver["lyrics"].as_str().unwrap_or(""), ver["style"].as_str().unwrap_or(""), ver["createdAt"].as_i64().unwrap_or(0)],
            )?;
        }
        for m in v["conversation"].as_array().into_iter().flatten() {
            let role = if m["role"].as_str() == Some("user") {
                "user"
            } else {
                "writer"
            };
            db.execute(
                "INSERT INTO messages(song_id, role, text, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![
                    id,
                    role,
                    m["text"].as_str().unwrap_or(""),
                    m["createdAt"].as_i64().unwrap_or(0)
                ],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::SongPatch;

    fn old_file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(60))
            .unwrap();
    }

    #[test]
    fn dropped_song_and_stems_get_one_tidy_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let l = Library::open(&tmp.path().join("data")).unwrap();
        let lib = tmp.path().join("Library");
        let stems = lib.join("Every Leaf Is Listening Stems");
        for f in [
            "Every Leaf Is Listening (Vocals).wav",
            "Every Leaf Is Listening (Vocals).mp3",
            "Every Leaf Is Listening (Drums).mp3",
        ] {
            old_file(&stems.join(f));
        }
        old_file(&lib.join("Every Leaf Is Listening.m4a"));
        let f = l.set_library_folder(&lib).unwrap();
        l.scan_folder(&f).unwrap();

        let root = lib.canonicalize().unwrap();
        let dir = root.join("Songs/Every Leaf Is Listening");
        assert!(dir.join("Every Leaf Is Listening.m4a").is_file());
        assert!(dir.join("stems/Vocals.wav").is_file());
        assert!(dir.join("stems/Vocals.mp3").is_file());
        assert!(dir.join("stems/Drums.mp3").is_file());
        assert!(dir.join("song.json").is_file());
        assert!(
            !root.join("Every Leaf Is Listening Stems").exists(),
            "the emptied stems folder is removed"
        );

        let songs = l.songs().unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!((songs[0].stems, songs[0].has_file), (3, true));
        assert_eq!(
            l.scan_folder(&f).unwrap().added,
            0,
            "a second scan changes nothing"
        );

        // Renaming the song renames its folder and file; every stored path follows.
        l.update_song(
            songs[0].id,
            &SongPatch {
                title: Some("Every Leaf".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let dir2 = root.join("Songs/Every Leaf");
        assert!(dir2.join("Every Leaf.m4a").is_file());
        assert!(dir2.join("stems/Drums.mp3").is_file());
        assert!(!dir.exists());
        assert_eq!(
            l.audio_path(songs[0].id).unwrap().unwrap(),
            dir2.join("Every Leaf.m4a")
        );
        let json: Value =
            serde_json::from_str(&std::fs::read_to_string(dir2.join("song.json")).unwrap())
                .unwrap();
        assert_eq!(json["title"], "Every Leaf");
        assert!(
            l.delete_song(songs[0].id).is_err(),
            "library songs are removed by moving their folder"
        );
        assert_eq!(l.scan_folder(&f).unwrap().added, 0);
    }

    #[test]
    fn songwriter_songs_live_in_their_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let l = Library::open(&tmp.path().join("data")).unwrap();
        let f = l.set_library_folder(&tmp.path().join("Library")).unwrap();
        let s = l.create_request("a sea shanty", None, "Bard").unwrap();
        let d = crate::Draft {
            title: "Salt and Rope".into(),
            lyrics: "[Chorus]\nHeave".into(),
            style: "shanty".into(),
            ..Default::default()
        };
        l.apply_draft(s.id, &d, "First draft", false).unwrap();
        let dir = PathBuf::from(&f.path).join("Songs/Salt and Rope");
        let json: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("song.json")).unwrap()).unwrap();
        assert_eq!(json["suno"]["lyrics"], "[Chorus]\nHeave");
        assert_eq!(json["conversation"][0]["text"], "a sea shanty");
        let take = tmp.path().join("take.mp3");
        std::fs::write(&take, b"x").unwrap();
        l.add_take(s.id, &take, "from Suno").unwrap();
        assert!(dir.join("takes/take.mp3").is_file());
        assert_eq!(
            l.scan_folder(&f).unwrap().added,
            0,
            "takes are not songs of their own"
        );
        assert_eq!(l.songs().unwrap().len(), 1);
    }

    #[test]
    fn a_copied_song_folder_restores_from_song_json() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("Library");
        let dir = lib.join("Songs/Old Tune");
        old_file(&dir.join("stems/Vocals.mp3"));
        std::fs::write(dir.join("song.json"), r#"{"title":"Old Tune","genre":"Folk","source":"ai","suno":{"lyrics":"la la"},"versions":[{"label":"v1","note":"First draft"}]}"#).unwrap();
        let l = Library::open(&tmp.path().join("data")).unwrap();
        let f = l.set_library_folder(&lib).unwrap();
        l.scan_folder(&f).unwrap();
        let songs = l.songs().unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!(
            (
                songs[0].title.as_str(),
                songs[0].genre.as_str(),
                songs[0].source.as_str(),
                songs[0].lyrics.as_str()
            ),
            ("Old Tune", "Folk", "ai", "la la")
        );
        assert_eq!(l.song_detail(songs[0].id).unwrap().versions.len(), 1);
    }

    #[test]
    fn names_are_safe() {
        assert_eq!(clean_name("AC/DC: Live?"), "AC DC Live");
        assert_eq!(clean_name("  ..  "), "Untitled");
        assert_eq!(
            clean_name("Every Leaf Is Listening"),
            "Every Leaf Is Listening"
        );
    }

    /// Suno's M4A downloads come without artwork; its MP3 of the same song has it. Choosing the MP3 as the
    /// artwork (Song details › Add artwork) must copy its cover into the M4A in the library.
    #[test]
    fn cover_from_an_mp3_is_embedded_in_the_m4a() {
        let ff = |args: &[&str]| std::process::Command::new("ffmpeg").args(["-v", "error", "-y"]).args(args).status().is_ok_and(|s| s.success());
        let tmp = tempfile::tempdir().unwrap();
        let (png, mp3, m4a) = (tmp.path().join("c.png"), tmp.path().join("Song.mp3"), tmp.path().join("in.m4a"));
        let made = ff(&["-f", "lavfi", "-i", "color=c=red:s=16x16", "-frames:v", "1", png.to_str().unwrap()])
            && ff(&["-f", "lavfi", "-i", "sine=d=1", "-i", png.to_str().unwrap(), "-map", "0", "-map", "1", "-c:v", "png",
                    "-disposition:v", "attached_pic", "-id3v2_version", "3", mp3.to_str().unwrap()])
            && ff(&["-f", "lavfi", "-i", "sine=d=1", "-c:a", "aac", m4a.to_str().unwrap()]);
        if !made {
            eprintln!("ffmpeg not available; skipping");
            return;
        }
        let cover = tags::read_track(&mp3, None).picture.expect("the MP3 has a cover").0;
        assert!(tags::read_track(&m4a, None).picture.is_none(), "the M4A starts without a cover");

        let l = Library::open(&tmp.path().join("data")).unwrap();
        let lib = tmp.path().join("Library");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::copy(&m4a, lib.join("Song.m4a")).unwrap();
        let f = l.set_library_folder(&lib).unwrap();
        l.scan_folder(&f).unwrap();
        let song = l.songs().unwrap().into_iter().next().expect("the M4A was imported");

        let song = l.set_art_from_file(song.id, &mp3).unwrap();
        assert!(song.art.is_some(), "the song has artwork");
        let path = l.audio_path(song.id).unwrap().unwrap();
        assert_eq!(path.extension().unwrap(), "m4a");
        assert_eq!(tags::read_track(&path, None).picture.map(|p| p.0), Some(cover), "the M4A now carries the MP3's cover");
    }

    #[test]
    fn software_folder_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let l = Library::open(&tmp.path().join("data")).unwrap();
        let lib = tmp.path().join("Library");
        let test_wav = lib.join(SOFTWARE).join("Show analyzer/python/lib/pkg/tests/data/test.wav");
        old_file(&test_wav);
        std::fs::create_dir_all(lib.join(SOFTWARE).join("Shows")).unwrap();
        old_file(&lib.join("A Real Song.mp3"));
        let f = l.set_library_folder(&lib).unwrap();
        l.scan_folder(&f).unwrap();

        assert!(test_wav.is_file(), "a file in _Software was moved");
        assert!(lib.join(SOFTWARE).join("Shows").is_dir(), "an empty folder in _Software was removed");
        let titles: Vec<String> = l.songs().unwrap().into_iter().map(|s| s.title).collect();
        assert_eq!(titles, vec!["A Real Song".to_string()]);
    }

    #[test]
    fn places() {
        let r = Path::new("/lib");
        assert_eq!(
            place_of(r, Path::new("/lib/_Software/Show analyzer/python/lib/scipy/io/tests/data/test.wav")),
            Place::Extra
        );
        assert_eq!(
            place_of(r, Path::new("/lib/Songs/A/A.mp3")),
            Place::Main(r.join("Songs/A"))
        );
        assert_eq!(
            place_of(r, Path::new("/lib/Songs/A/stems/Vocals.wav")),
            Place::Stem(r.join("Songs/A"), "Vocals".into())
        );
        assert_eq!(
            place_of(r, Path::new("/lib/Songs/A/takes/t.mp3")),
            Place::Extra
        );
        assert_eq!(
            place_of(r, Path::new("/lib/Albums/X/Y/01 - Z.mp3")),
            Place::Album
        );
        assert_eq!(place_of(r, Path::new("/lib/A (Vocals).wav")), Place::Loose);
        assert_eq!(
            place_of(r, Path::new("/lib/Songs/A/A (Vocals).wav")),
            Place::Loose
        );
        assert_eq!(place_of(r, Path::new("/lib/new.mp3")), Place::Loose);
    }
}
