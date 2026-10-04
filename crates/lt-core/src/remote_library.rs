//! A connected server's songs as part of My music. When a connection has "In My music" on, every song on the
//! server becomes a library song that streams from the server (provider "remote", provider_id
//! "<connection id>:<song id>"), with its tags and cover, so it shows in My music, Albums, Genres and
//! playlists like any other song. Songs the person already has as local files stay local only (the local
//! copy wins; a server song already in playlists moves over to the local copy). Songs gone from the server
//! leave the library on the next sync. A sync only lands while the connection still exists with "In My
//! music" on, so removing it or turning it off during a sync can't bring the songs back.

use crate::library::{Library, now};
use crate::remote::RemoteItem;
use anyhow::{Result, bail};
use rusqlite::params;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySync {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    /// Songs skipped because the same song is already a local file
    pub local: usize,
    pub total: usize,
}

/// Same song regardless of case, spacing or "The": title + artist + album.
fn key(title: &str, artist: &str, album: &str) -> String {
    let n = |s: &str| {
        let s = s.to_lowercase();
        let s = s.strip_prefix("the ").unwrap_or(&s).to_string();
        s.chars().filter(|c| c.is_alphanumeric()).collect::<String>()
    };
    format!("{}|{}|{}", n(title), n(artist), n(album))
}

impl Library {
    /// Remote songs of a connection already in the library: provider_id → art.
    pub fn remote_art(&self, conn: i64) -> Result<HashMap<String, Option<String>>> {
        let db = self.db();
        let mut st = db.prepare("SELECT provider_id, art FROM songs WHERE provider = 'remote' AND provider_id LIKE ?1")?;
        let rows = st.query_map([format!("{conn}:%")], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Makes the library match the server: adds and updates `items`, removes songs no longer there.
    /// `art` maps a song's cover reference to stored artwork (see `store_cover`). `items` must be the
    /// server's complete list. Fails without changing anything when the connection was removed or "In My
    /// music" was turned off meanwhile.
    pub fn sync_remote_songs(&self, conn: i64, items: &[RemoteItem], art: &HashMap<String, String>) -> Result<LibrarySync> {
        let mut report = LibrarySync { total: items.len(), ..Default::default() };
        // same song as a local file → that local song's id
        let locals: HashMap<String, i64> = {
            let db = self.db();
            let mut st = db.prepare("SELECT id, title, artist, album FROM songs WHERE provider = 'local' AND source = 'library' ORDER BY id DESC")?;
            let rows = st.query_map([], |r| Ok((key(&r.get::<_, String>(1)?, &r.get::<_, String>(2)?, &r.get::<_, String>(3)?), r.get::<_, i64>(0)?)))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        // genres first (ensure_genre takes the lock itself)
        let mut genres = HashMap::new();
        for it in items {
            if !genres.contains_key(&it.genre) {
                genres.insert(it.genre.clone(), self.ensure_genre(&it.genre, "file")?);
            }
        }
        let t = now();
        let mut keep = HashSet::new();
        {
            let mut db = self.db();
            let tx = db.transaction()?;
            // Checked under the same lock as the writes, so a removal or turn-off can't slip in between.
            let in_library = tx
                .query_row("SELECT extra FROM connections WHERE id = ?1", [conn], |r| r.get::<_, String>(0))
                .ok()
                .and_then(|e| serde_json::from_str::<serde_json::Value>(&e).ok())
                .is_some_and(|e| e["inLibrary"].as_bool() == Some(true));
            if !in_library {
                bail!("This server was removed or taken out of My music, so its songs were not added");
            }
            for it in items.iter().filter(|i| i.kind == "song") {
                if locals.contains_key(&key(&it.name, &it.artist, &it.album)) {
                    report.local += 1;
                    continue;
                }
                let pid = format!("{conn}:{}", it.id);
                keep.insert(pid.clone());
                let cover = it.cover.as_ref().and_then(|c| art.get(c)).cloned();
                let existed: bool = tx.query_row("SELECT COUNT(*) FROM songs WHERE provider = 'remote' AND provider_id = ?1", [&pid], |r| r.get::<_, i64>(0))? > 0;
                // Added = when the server got the song (unknown → oldest), so server songs never bury new local ones.
                tx.execute(
                    "INSERT INTO songs(source, provider, provider_id, title, artist, album, year, genre, track, duration, art, added_at, updated_at, remote_path, remote_size) \
                     VALUES ('library', 'remote', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?11, ?10, ?12, ?13) \
                     ON CONFLICT(provider, provider_id) WHERE provider_id IS NOT NULL DO UPDATE SET title = excluded.title, artist = excluded.artist, \
                     album = excluded.album, year = excluded.year, genre = excluded.genre, track = excluded.track, duration = excluded.duration, \
                     art = COALESCE(excluded.art, songs.art), added_at = excluded.added_at, \
                     remote_path = COALESCE(excluded.remote_path, songs.remote_path), remote_size = COALESCE(excluded.remote_size, songs.remote_size)",
                    params![pid, it.name, it.artist, it.album, it.year, genres[&it.genre], it.track, it.duration, cover, t, it.added.unwrap_or(0), it.path, it.size],
                )?;
                if existed { report.updated += 1 } else { report.added += 1 }
            }
            let old: Vec<(i64, String, String)> = {
                let mut st = tx.prepare("SELECT id, provider_id, title, artist, album FROM songs WHERE provider = 'remote' AND provider_id LIKE ?1")?;
                st.query_map([format!("{conn}:%")], |r| Ok((r.get(0)?, r.get(1)?, key(&r.get::<_, String>(2)?, &r.get::<_, String>(3)?, &r.get::<_, String>(4)?))))?
                    .filter_map(|r| r.ok())
                    .collect()
            };
            for (id, pid, k) in old {
                if !keep.contains(&pid) {
                    // Now a local file: playlists, notes and sheets move to the local copy instead of being lost.
                    if let Some(local) = locals.get(&k) {
                        tx.execute("UPDATE OR IGNORE playlist_songs SET song_id = ?2 WHERE song_id = ?1", [id, *local])?;
                        tx.execute("UPDATE messages SET song_id = ?2 WHERE song_id = ?1", [id, *local])?;
                        tx.execute("UPDATE sheets SET song_id = ?2 WHERE song_id = ?1", [id, *local])?;
                    }
                    tx.execute("DELETE FROM songs WHERE id = ?1", [id])?;
                    report.removed += 1;
                }
            }
            tx.commit()?;
        }
        Ok(report)
    }

    /// Gives server songs their song IDs from the server's song ID list (`Shows/song-ids.tsv`, made by
    /// lt-songid --index over the server's music folder): each line "<id>\t<size>\t<path>". A server song
    /// matches the line whose path ends its own reported path's end, with the same size. Returns how many got one.
    pub fn map_remote_uids(&self, list: &str) -> Result<usize> {
        let mut by_name: HashMap<String, Vec<(&str, i64, &str)>> = HashMap::new();
        for line in list.lines() {
            let mut f = line.splitn(3, '\t');
            let (Some(uid), Some(size), Some(rel)) = (f.next(), f.next(), f.next()) else { continue };
            if !uid.starts_with("LT-") {
                continue;
            }
            let name = rel.rsplit('/').next().unwrap_or(rel).to_lowercase();
            by_name.entry(name).or_default().push((uid, size.parse().unwrap_or(-1), rel));
        }
        let pending: Vec<(i64, String, Option<i64>)> = {
            let db = self.db();
            let mut st = db.prepare("SELECT id, remote_path, remote_size FROM songs WHERE provider = 'remote' AND uid IS NULL AND remote_path IS NOT NULL")?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.filter_map(|r| r.ok()).collect()
        };
        let mut n = 0;
        for (id, path, size) in pending {
            let path = path.replace('\\', "/");
            let name = path.rsplit('/').next().unwrap_or(&path).to_lowercase();
            let Some(cands) = by_name.get(&name) else { continue };
            let same_size = |s: i64| size.is_none_or(|z| z == s);
            let hit = cands
                .iter()
                .find(|(_, s, rel)| same_size(*s) && (path == *rel || path.ends_with(&format!("/{rel}"))))
                // the server counts paths from a different starting folder: the one file of that name and size
                .or_else(|| {
                    let same: Vec<_> = cands.iter().filter(|(_, s, _)| size.is_some() && same_size(*s)).collect();
                    (same.len() == 1).then(|| same[0])
                });
            let Some((uid, _, _)) = hit else { continue };
            // an ID already taken here (the same audio as a song on this device): this one is a copy
            let mut u = uid.to_string();
            let mut k = 2;
            while self.db().query_row("SELECT COUNT(*) FROM songs WHERE uid = ?1", [&u], |r| r.get::<_, i64>(0))? > 0 {
                u = crate::songid::variant(uid, k);
                k += 1;
            }
            self.db().execute("UPDATE songs SET uid = ?2 WHERE id = ?1", params![id, u])?;
            n += 1;
        }
        Ok(n)
    }

    /// Takes a connection's songs out of the library (connection removed or "In My music" turned off).
    pub fn remove_remote_songs(&self, conn: i64) -> Result<usize> {
        Ok(self.db().execute("DELETE FROM songs WHERE provider = 'remote' AND provider_id LIKE ?1", [format!("{conn}:%")])?)
    }

    /// Stores a downloaded cover and returns its artwork name.
    pub fn store_cover(&self, bytes: &[u8]) -> Result<String> {
        let ext = if bytes.starts_with(b"\x89PNG") { "png" } else { "jpg" };
        self.store_art(bytes, ext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str, title: &str, artist: &str, album: &str) -> RemoteItem {
        RemoteItem { id: id.into(), kind: "song".into(), name: title.into(), artist: artist.into(), album: album.into(), genre: "Rock".into(), cover: Some(format!("c-{album}")), ..Default::default() }
    }

    fn conn(l: &Library) -> i64 {
        l.add_connection("subsonic", "Home", "http://nas.local:4533", "listener", &serde_json::json!({"inLibrary": true})).unwrap().id
    }

    #[test]
    fn server_songs_join_the_library_and_follow_the_server() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(d.path()).unwrap();
        let c = conn(&l);
        // a local file of the same song
        {
            let db = l.db();
            db.execute("INSERT INTO songs(source, title, artist, album, genre, added_at, updated_at) VALUES ('library', 'Numb', 'Linkin Park', 'Meteora', 'Rock', 1, 1)", []).unwrap();
        }
        let art: HashMap<String, String> = [("c-Meteora".to_string(), "abc.jpg".to_string())].into();
        let items = vec![song("1", "Numb", "Linkin Park", "Meteora"), song("2", "Faint", "Linkin Park", "Meteora"), song("3", "Lying from You", "Linkin Park", "Meteora")];
        let r = l.sync_remote_songs(c, &items, &art).unwrap();
        assert_eq!((r.added, r.local, r.removed), (2, 1, 0), "the local Numb wins");
        let faint = l.songs().unwrap().into_iter().find(|s| s.title == "Faint").unwrap();
        assert_eq!((faint.provider.as_str(), faint.provider_id.as_deref(), faint.art.as_deref(), faint.genre.as_str()), ("remote", Some(format!("{c}:2").as_str()), Some("abc.jpg"), "Rock"));
        // the server drops a song and renames another
        let items = vec![RemoteItem { name: "Faint (Live)".into(), ..song("2", "", "Linkin Park", "Meteora") }];
        let r = l.sync_remote_songs(c, &items, &HashMap::new()).unwrap();
        assert_eq!((r.updated, r.removed), (1, 1));
        let faint = l.songs().unwrap().into_iter().find(|s| s.provider_id.as_deref() == Some(format!("{c}:2").as_str())).unwrap();
        assert_eq!((faint.title.as_str(), faint.art.as_deref()), ("Faint (Live)", Some("abc.jpg")), "artwork is kept when not resent");
        assert_eq!(l.remove_remote_songs(c).unwrap(), 1);
    }

    #[test]
    fn a_sync_never_lands_after_the_server_was_removed_or_turned_off() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(d.path()).unwrap();
        let c = conn(&l);
        let items = vec![song("1", "Faint", "Linkin Park", "Meteora")];
        l.set_connection_extra(c, "inLibrary", serde_json::Value::Bool(false)).unwrap();
        assert!(l.sync_remote_songs(c, &items, &HashMap::new()).is_err());
        l.remove_connection(c).unwrap();
        assert!(l.sync_remote_songs(c, &items, &HashMap::new()).is_err());
        assert!(l.songs().unwrap().is_empty(), "nothing came back");
    }

    #[test]
    fn a_new_local_copy_takes_over_the_server_song_in_playlists() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(d.path()).unwrap();
        let c = conn(&l);
        l.sync_remote_songs(c, &[song("2", "Faint", "Linkin Park", "Meteora")], &HashMap::new()).unwrap();
        let remote = l.songs().unwrap().into_iter().find(|s| s.title == "Faint").unwrap().id;
        let local = {
            let db = l.db();
            db.execute("INSERT INTO playlists(id, name, created_at) VALUES (1, 'Road', 1)", []).unwrap();
            db.execute("INSERT INTO playlist_songs(playlist_id, song_id, pos) VALUES (1, ?1, 0)", [remote]).unwrap();
            db.execute("INSERT INTO songs(source, title, artist, album, genre, added_at, updated_at) VALUES ('library', 'Faint', 'Linkin Park', 'Meteora', 'Rock', 1, 1)", []).unwrap();
            db.last_insert_rowid()
        };
        let r = l.sync_remote_songs(c, &[song("2", "Faint", "Linkin Park", "Meteora")], &HashMap::new()).unwrap();
        assert_eq!((r.local, r.removed), (1, 1));
        let in_playlist: Vec<i64> = {
            let db = l.db();
            let mut st = db.prepare("SELECT song_id FROM playlist_songs WHERE playlist_id = 1").unwrap();
            st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
        };
        assert_eq!(in_playlist, [local], "the playlist now holds the local copy");
    }

    #[test]
    fn server_songs_get_their_ids_from_the_server_list() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(&d.path().join("app")).unwrap();
        {
            let db = l.db();
            // as Nextcloud Music reports them: from the user's files root
            db.execute("INSERT INTO songs(source, provider, provider_id, title, added_at, updated_at, remote_path, remote_size) VALUES ('library', 'remote', '1:a', 'Numb', 1, 1, '/Music/Lute & Tavern/Albums/Linkin Park/Meteora/13 - Numb.mp3', 4000)", []).unwrap();
            db.execute("INSERT INTO songs(source, provider, provider_id, title, added_at, updated_at, remote_path, remote_size) VALUES ('library', 'remote', '1:b', 'Esaul', 1, 1, '/Music/Lute & Tavern/Albums/Linkin Park/EP/03 - Esaul.mp3', 300)", []).unwrap();
            db.execute("INSERT INTO songs(source, provider, provider_id, title, added_at, updated_at, remote_path, remote_size) VALUES ('library', 'remote', '1:c', 'Gone', 1, 1, '/Music/elsewhere/x.mp3', 1)", []).unwrap();
        }
        let list = "LT-AAAA-0000-0000-0000-0000-0000-0000-0001\t4000\tLute & Tavern/Albums/Linkin Park/Meteora/13 - Numb.mp3\n\
                    LT-AAAA-0000-0000-0000-0000-0000-0000-0002\t300\tLute & Tavern/Albums/Linkin Park/Demo/03 - Esaul.mp3\n\
                    LT-AAAA-0000-0000-0000-0000-0000-0000-0003\t300\tLute & Tavern/Albums/Linkin Park/EP/03 - Esaul.mp3\n";
        assert_eq!(l.map_remote_uids(list).unwrap(), 2);
        let uid = |pid: &str| l.db().query_row("SELECT uid FROM songs WHERE provider_id = ?1", [pid], |r| r.get::<_, Option<String>>(0)).unwrap();
        assert_eq!(uid("1:a").as_deref(), Some("LT-AAAA-0000-0000-0000-0000-0000-0000-0001"));
        assert_eq!(uid("1:b").as_deref(), Some("LT-AAAA-0000-0000-0000-0000-0000-0000-0003"), "same name and size: the path decides");
        assert_eq!(uid("1:c"), None, "not in the list: waits");
    }
}