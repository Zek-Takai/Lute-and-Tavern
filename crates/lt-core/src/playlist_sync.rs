//! Playlists that follow the person across devices. Each playlist is a standard .m3u8 file in a "Playlists"
//! folder on their backup server (other players can read them), with extra #LT tags so another device can
//! match every song to its own library. An index file records each playlist's lasting id, name and change
//! time, and deletions. The newer change wins; a deletion wins unless the playlist was edited after it.

use crate::Song;
use crate::library::{Library, now};
use crate::playlist_io::{PlEntry, best_match, parse_m3u};
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
    id: i64,
    name: String,
    updated: i64,
    songs: Vec<i64>,
}

/// A song of a received playlist that isn't in this library: it keeps its place (after `after`, a song id,
/// or at the start) and goes back out with the playlist, so devices with fewer songs never shorten it.
struct Pending {
    after: Option<i64>,
    entry: PlEntry,
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
            .map(|(id, uuid, name, updated)| (uuid, LocalList { id, name, updated, songs: songs.get(&id).cloned().unwrap_or_default() }))
            .collect())
    }

    fn tombstones(&self) -> Result<HashMap<String, i64>> {
        let db = self.db();
        let mut st = db.prepare("SELECT uuid, deleted_at FROM playlist_tombstones")?;
        Ok(st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(|r| r.ok()).collect())
    }

    fn pending(&self, playlist: i64) -> Result<Vec<Pending>> {
        let db = self.db();
        let mut st = db.prepare("SELECT after_song, entry FROM playlist_pending WHERE playlist_id = ?1 ORDER BY seq")?;
        let rows = st.query_map([playlist], |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, String>(1)?)))?;
        Ok(rows.filter_map(|r| r.ok()).filter_map(|(after, e)| serde_json::from_str(&e).ok().map(|entry| Pending { after, entry })).collect())
    }

    fn set_pending(&self, playlist: i64, pending: &[Pending]) -> Result<()> {
        let db = self.db();
        db.execute("DELETE FROM playlist_pending WHERE playlist_id = ?1", [playlist])?;
        for (seq, p) in pending.iter().enumerate() {
            db.execute(
                "INSERT INTO playlist_pending(playlist_id, after_song, seq, entry) VALUES (?1, ?2, ?3, ?4)",
                params![playlist, p.after, seq as i64, serde_json::to_string(&p.entry)?],
            )?;
        }
        Ok(())
    }

    /// Songs that arrived since (a scan, a server sync) take their place in the playlists that were waiting
    /// for them. This doesn't count as an edit: the playlist is the same everywhere.
    fn place_pending(&self, own: &[Song], apple: &[Song]) -> Result<()> {
        let lists: Vec<i64> = {
            let db = self.db();
            let mut st = db.prepare("SELECT DISTINCT playlist_id FROM playlist_pending")?;
            st.query_map([], |r| r.get(0))?.filter_map(|r| r.ok()).collect()
        };
        for pl in lists {
            let pending = self.pending(pl)?;
            let mut songs = self.playlists()?.into_iter().find(|p| p.id == pl).map(|p| p.songs).unwrap_or_default();
            let mut still = Vec::new();
            let mut placed = false;
            for p in pending {
                match best_match(&p.entry, own).or_else(|| best_match(&p.entry, apple)) {
                    Some(id) if !songs.contains(&id) => {
                        let at = p.after.and_then(|a| songs.iter().position(|x| *x == a)).map(|i| i + 1).unwrap_or(if p.after.is_some() { songs.len() } else { 0 });
                        songs.insert(at, id);
                        placed = true;
                    }
                    Some(_) => placed = true,
                    None => still.push(p),
                }
            }
            if placed {
                let updated: i64 = self.db().query_row("SELECT updated_at FROM playlists WHERE id = ?1", [pl], |r| r.get(0))?;
                self.set_playlist_songs(pl, &songs)?;
                self.db().execute("UPDATE playlists SET updated_at = ?2 WHERE id = ?1", params![pl, updated])?;
                self.set_pending(pl, &still)?;
            }
        }
        Ok(())
    }

    /// Creates or replaces a playlist from another device, keeping its id and change time.
    fn apply_playlist(&self, uuid: &str, name: &str, updated: i64, songs: &[i64], pending: &[Pending]) -> Result<()> {
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
        self.set_pending(id, pending)?;
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
fn render(lib: &Library, name: &str, songs: &[i64], pending: &[Pending], all: &HashMap<i64, &Song>, root: Option<&Path>) -> String {
    let mut out = format!("#EXTM3U\n#PLAYLIST:{name}\n");
    let entry = |out: &mut String, e: &PlEntry| {
        out.push_str(&format!("#EXTINF:{},{} - {}\n", e.duration.map(|d| d.round() as i64).unwrap_or(-1), e.artist, e.title));
        out.push_str(&format!("#LT:{}\n", json!({"title": e.title, "artist": e.artist, "album": e.album, "duration": e.duration})));
        out.push_str(&format!("{}\n", e.path.clone().unwrap_or_else(|| format!("lute-and-tavern:{}", e.title.replace(['\n', '\r'], " ")))));
    };
    // songs this device doesn't have go back out in their place; ones whose neighbour left go at the end
    let placed = |after: Option<i64>| pending.iter().filter(move |p| p.after == after);
    for p in placed(None) {
        entry(&mut out, &p.entry);
    }
    for p in pending.iter().filter(|p| p.after.is_some_and(|a| !songs.contains(&a))) {
        entry(&mut out, &p.entry);
    }
    for id in songs {
        let Some(s) = all.get(id) else { continue };
        out.push_str(&format!("#EXTINF:{},{} - {}\n", s.duration.map(|d| d.round() as i64).unwrap_or(-1), s.artist, s.title));
        out.push_str(&format!("#LT:{}\n", json!({"title": s.title, "artist": s.artist, "album": s.album, "duration": s.duration})));
        let file = lib.audio_path(s.id).ok().flatten();
        let local = root.and_then(|r| file.as_deref().and_then(|f| f.strip_prefix(r).ok()).map(|rel| format!("../{}", rel.to_string_lossy())));
        out.push_str(&format!("{}\n", local.unwrap_or_else(|| format!("lute-and-tavern:{}", s.title.replace(['\n', '\r'], " ")))));
        for p in placed(Some(*id)) {
            entry(&mut out, &p.entry);
        }
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
    // No index yet: start one. One that can't be read (a maintenance page, a cut-off upload) stops the sync,
    // so deletions recorded in it are never lost.
    let mut index: Index = match dav.get_bytes(&at(INDEX))? {
        None => Index::default(),
        Some(b) => serde_json::from_slice(&b).map_err(|_| anyhow::anyhow!("The playlist list on the server ({INDEX}) couldn't be read, so playlists weren't synced. If it stays this way, delete that file there and sync again."))?,
    };
    let songs = lib.songs()?;
    let (apple, own): (Vec<Song>, Vec<Song>) = songs.iter().cloned().partition(|s| s.provider == "apple");
    lib.place_pending(&own, &apple)?;
    let locals = lib.local_lists()?;
    let tombs = lib.tombstones()?;
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
            dav.put_bytes(&at(&file), render(lib, &l.name, &l.songs, &lib.pending(l.id)?, &by_id, root.as_deref()).into_bytes())?;
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
                let mut ids: Vec<i64> = Vec::new();
                let mut pending = Vec::new();
                for entry in parsed.entries {
                    match best_match(&entry, &own).or_else(|| best_match(&entry, &apple)) {
                        Some(id) if !ids.contains(&id) => ids.push(id),
                        Some(_) => {}
                        None => pending.push(Pending { after: ids.last().copied(), entry }),
                    }
                }
                lib.apply_playlist(&uuid, &e.name, e.updated, &ids, &pending)?;
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

        // B lacks a song: B's edit must not shorten the playlist on A, and the song finds its place once B has it
        let a3 = song(&a, "Only on A");
        let pa = a.playlists().unwrap()[0].clone();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        a.set_playlist_songs(pa.id, &[a2, a3, a1]).unwrap();
        sync(&a, &da, "Music").unwrap();
        sync(&b, &db, "Music").unwrap();
        let pb = b.playlists().unwrap()[0].clone();
        assert_eq!((pb.songs.clone(), pb.missing), (vec![b2, b1], 1));
        std::thread::sleep(std::time::Duration::from_millis(1100));
        b.rename_playlist(pb.id, "Night Drive III").unwrap();
        sync(&b, &db, "Music").unwrap();
        sync(&a, &da, "Music").unwrap();
        assert_eq!(a.playlists().unwrap()[0].songs, vec![a2, a3, a1], "the song B doesn't have kept its place");
        let b3 = song(&b, "Only on A");
        sync(&b, &db, "Music").unwrap();
        let pb = b.playlists().unwrap()[0].clone();
        assert_eq!((pb.songs.clone(), pb.missing), (vec![b2, b3, b1], 0));

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
