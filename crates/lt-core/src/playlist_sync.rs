//! Playlists that follow the person across devices. Each playlist is a standard .m3u8 file in a "Playlists"
//! folder on their backup server (other players can read them), with extra #LT tags so another device can
//! match every song to its own library. An index file records each playlist's lasting id, name and change
//! time, and deletions. The newer change wins; a deletion wins unless the playlist was edited after it.

use crate::Song;
use crate::library::{Library, now};
use crate::playlist_io::{best_match, parse_m3u};
use crate::remote_webdav::WebDav;
use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

const INDEX: &str = ".lute-playlists.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    playlists: BTreeMap<String, Entry>,
    #[serde(default)]
    deleted: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    name: String,
    file: String,
    updated: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistSync {
    pub sent: usize,
    pub received: usize,
    pub deleted_here: usize,
    pub deleted_there: usize,
}

struct LocalList {
    name: String,
    updated: i64,
    songs: Vec<i64>,
}

impl Library {
    fn local_lists(&self) -> Result<HashMap<String, LocalList>> {
        self.ensure_playlist_uuids()?;
        let songs: HashMap<i64, Vec<i64>> = self.playlists()?.into_iter().map(|p| (p.id, p.songs)).collect();
        let db = self.db();
        let mut st = db.prepare("SELECT id, uuid, name, updated_at FROM playlists")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?)))?;
        Ok(rows
            .filter_map(|r| r.ok())
            .map(|(id, uuid, name, updated)| (uuid, LocalList { name, updated, songs: songs.get(&id).cloned().unwrap_or_default() }))
            .collect())
    }

    fn tombstones(&self) -> Result<HashMap<String, i64>> {
        let db = self.db();
        let mut st = db.prepare("SELECT uuid, deleted_at FROM playlist_tombstones")?;
        Ok(st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(|r| r.ok()).collect())
    }

    /// Creates or replaces a playlist from another device, keeping its id and change time.
    fn apply_playlist(&self, uuid: &str, name: &str, updated: i64, songs: &[i64]) -> Result<()> {
        let existing: Option<i64> = {
            let db = self.db();
            db.query_row("SELECT id FROM playlists WHERE uuid = ?1", [uuid], |r| r.get(0)).ok()
        };
        let id = match existing {
            Some(id) => {
                self.db().execute("UPDATE playlists SET name = ?2 WHERE id = ?1", params![id, name])?;
                id
            }
            None => {
                let db = self.db();
                db.execute("INSERT INTO playlists(name, created_at, uuid, updated_at) VALUES (?1, ?2, ?3, ?4)", params![name, now(), uuid, updated])?;
                db.last_insert_rowid()
            }
        };
        self.set_playlist_songs(id, songs)?;
        let db = self.db();
        db.execute("UPDATE playlists SET updated_at = ?2 WHERE id = ?1", params![id, updated])?;
        db.execute("DELETE FROM playlist_tombstones WHERE uuid = ?1", [uuid])?;
        Ok(())
    }

    fn drop_playlist(&self, uuid: &str, at: i64) -> Result<()> {
        let db = self.db();
        db.execute("INSERT OR REPLACE INTO playlist_tombstones(uuid, name, deleted_at) SELECT uuid, name, ?2 FROM playlists WHERE uuid = ?1", params![uuid, at])?;
        db.execute("DELETE FROM playlists WHERE uuid = ?1", [uuid])?;
        Ok(())
    }
}

fn file_name(name: &str, uuid: &str, used: &HashSet<String>) -> String {
    let clean: String = name.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { ' ' } else { c }).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean = if clean.is_empty() { "Playlist".to_string() } else { clean };
    let f = format!("{clean}.m3u8");
    if used.contains(&f.to_lowercase()) { format!("{clean} {}.m3u8", &uuid[..6]) } else { f }
}

/// The .m3u8 text for a playlist: plain paths for local songs (relative to the Playlists folder), and #LT tags
/// with each song's tags so another device can find it in its own library.
fn render(lib: &Library, name: &str, songs: &[i64], all: &HashMap<i64, &Song>, root: Option<&Path>) -> String {
    let mut out = format!("#EXTM3U\n#PLAYLIST:{name}\n");
    for id in songs {
        let Some(s) = all.get(id) else { continue };
        out.push_str(&format!("#EXTINF:{},{} - {}\n", s.duration.map(|d| d.round() as i64).unwrap_or(-1), s.artist, s.title));
        out.push_str(&format!("#LT:{}\n", json!({"title": s.title, "artist": s.artist, "album": s.album, "duration": s.duration})));
        let file = lib.audio_path(s.id).ok().flatten();
        let local = root.and_then(|r| file.as_deref().and_then(|f| f.strip_prefix(r).ok()).map(|rel| format!("../{}", rel.to_string_lossy())));
        out.push_str(&format!("{}\n", local.unwrap_or_else(|| format!("lute-and-tavern:{}", s.title.replace(['\n', '\r'], " ")))));
    }
    out
}

/// Changes whenever a playlist is made, changed or deleted on this device (cheap; polled by the app).
pub fn stamp(lib: &Library) -> Result<String> {
    let db = lib.db();
    Ok(db.query_row(
        "SELECT (SELECT COUNT(*) || ':' || COALESCE(MAX(updated_at), 0) FROM playlists) || ':' || (SELECT COUNT(*) || ':' || COALESCE(MAX(deleted_at), 0) FROM playlist_tombstones)",
        [],
        |r| r.get(0),
    )?)
}

/// Brings this device's playlists and the server's in line. `folder` is the backup folder on the server.
pub fn sync(lib: &Library, dav: &WebDav, folder: &str) -> Result<PlaylistSync> {
    let mut r = PlaylistSync::default();
    let mut pieces: Vec<String> = folder.split('/').map(str::trim).filter(|p| !p.is_empty()).map(String::from).collect();
    pieces.push("Playlists".into());
    let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
    dav.make_folders(&refs)?;
    let at = |file: &str| {
        let mut p = refs.clone();
        p.push(file);
        dav.url_for(&p, false)
    };
    let mut index: Index = dav.get_bytes(&at(INDEX))?.and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let locals = lib.local_lists()?;
    let tombs = lib.tombstones()?;
    let songs = lib.songs()?;
    let by_id: HashMap<i64, &Song> = songs.iter().map(|s| (s.id, s)).collect();
    let root = lib.library_folder()?.map(|f| std::path::PathBuf::from(f.path));
    let mut used: HashSet<String> = index.playlists.values().map(|e| e.file.to_lowercase()).collect();
    let uuids: HashSet<String> = locals.keys().chain(index.playlists.keys()).chain(index.deleted.keys()).chain(tombs.keys()).cloned().collect();

    for uuid in uuids {
        let local = locals.get(&uuid);
        let remote = index.playlists.get(&uuid).cloned();
        let tomb_here = tombs.get(&uuid).copied();
        let tomb_there = index.deleted.get(&uuid).copied();
        // deleted on this device
        if let Some(t) = tomb_here {
            match &remote {
                Some(e) if e.updated > t => {} // edited elsewhere after the delete: bring it back below
                _ => {
                    if let Some(e) = index.playlists.remove(&uuid) {
                        dav.delete(&at(&e.file))?;
                        r.deleted_there += 1;
                    }
                    index.deleted.insert(uuid.clone(), t);
                    continue;
                }
            }
        }
        // deleted on another device
        if let Some(t) = tomb_there {
            match local {
                Some(l) if l.updated > t => {
                    index.deleted.remove(&uuid); // edited here after the delete: keep it (sent below)
                }
                Some(_) => {
                    lib.drop_playlist(&uuid, t)?;
                    r.deleted_here += 1;
                    continue;
                }
                None => continue,
            }
        }
        let send = |l: &LocalList, index: &mut Index, used: &mut HashSet<String>| -> Result<()> {
            let old = index.playlists.get(&uuid).map(|e| e.file.clone());
            let file = match &old {
                Some(f) if index.playlists[&uuid].name == l.name => f.clone(),
                _ => file_name(&l.name, &uuid, used),
            };
            dav.put_bytes(&at(&file), render(lib, &l.name, &l.songs, &by_id, root.as_deref()).into_bytes())?;
            if let Some(f) = old.filter(|f| *f != file) {
                dav.delete(&at(&f))?;
                used.remove(&f.to_lowercase());
            }
            used.insert(file.to_lowercase());
            index.playlists.insert(uuid.clone(), Entry { name: l.name.clone(), file, updated: l.updated.max(1) });
            Ok(())
        };
        match (local, remote) {
            (Some(l), None) => {
                send(l, &mut index, &mut used)?;
                r.sent += 1;
            }
            (Some(l), Some(e)) if l.updated > e.updated || (l.updated == e.updated && l.name != e.name) => {
                send(l, &mut index, &mut used)?;
                r.sent += 1;
            }
            (local, Some(e)) if local.is_none_or(|l| e.updated > l.updated) || tomb_here.is_some() => {
                let Some(bytes) = dav.get_bytes(&at(&e.file))? else { continue };
                let parsed = parse_m3u(&String::from_utf8_lossy(&bytes), &e.name);
                let mut ids = Vec::new();
                for entry in &parsed.entries {
                    if let Some(id) = best_match(entry, &songs) {
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                }
                lib.apply_playlist(&uuid, &e.name, e.updated, &ids)?;
                r.received += 1;
            }
            _ => {}
        }
    }
    dav.put_bytes(&at(INDEX), serde_json::to_vec_pretty(&index)?)?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::tests::fake_dav;

    fn song(l: &Library, title: &str) -> i64 {
        let db = l.db();
        db.execute("INSERT INTO songs(source, title, artist, added_at, updated_at) VALUES ('library', ?1, 'Listener', 1, 1)", [title]).unwrap();
        db.last_insert_rowid()
    }

    #[test]
    fn playlists_travel_between_two_devices_including_deletions() {
        let d = tempfile::tempdir().unwrap();
        let (url, _store) = fake_dav();
        let a = Library::open(&d.path().join("a")).unwrap();
        let b = Library::open(&d.path().join("b")).unwrap();
        let (a1, a2) = (song(&a, "Porch Light"), song(&a, "Lantern"));
        let (b2, b1) = (song(&b, "Lantern"), song(&b, "Porch Light"));
        let ca = a.add_connection("webdav", "NC", &url, "listener", &json!({})).unwrap();
        let cb = b.add_connection("webdav", "NC", &url, "listener", &json!({})).unwrap();
        let (da, db) = (WebDav::new(&ca, "pw"), WebDav::new(&cb, "pw"));

        let p = a.create_playlist("Night Drive", &[a2, a1]).unwrap();
        assert_eq!(sync(&a, &da, "Music").unwrap().sent, 1);
        assert_eq!(sync(&b, &db, "Music").unwrap().received, 1);
        let got = b.playlists().unwrap();
        assert_eq!((got.len(), got[0].name.as_str(), got[0].songs.clone()), (1, "Night Drive", vec![b2, b1]));

        // edited on B (later), comes back to A
        std::thread::sleep(std::time::Duration::from_millis(1100));
        b.rename_playlist(got[0].id, "Night Drive II").unwrap();
        assert_eq!(sync(&b, &db, "Music").unwrap().sent, 1);
        assert_eq!(sync(&a, &da, "Music").unwrap().received, 1);
        assert_eq!(a.playlists().unwrap()[0].name, "Night Drive II");

        // deleted on A, disappears from B and stays gone
        a.delete_playlist(p.id).unwrap();
        assert_eq!(sync(&a, &da, "Music").unwrap().deleted_there, 1);
        assert_eq!(sync(&b, &db, "Music").unwrap().deleted_here, 1);
        assert!(b.playlists().unwrap().is_empty());
        let again = sync(&a, &da, "Music").unwrap();
        assert_eq!((again.sent, again.received), (0, 0));
        assert!(a.playlists().unwrap().is_empty());
    }
}
