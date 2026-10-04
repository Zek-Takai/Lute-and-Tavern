//! A connected server's songs as part of My music. When a connection has "In My music" on, every song on the
//! server becomes a library song that streams from the server (provider "remote", provider_id
//! "<connection id>:<song id>"), with its tags and cover, so it shows in My music, Albums, Genres and
//! playlists like any other song. Songs the person already has as local files stay local only (the local
//! copy wins). Songs gone from the server leave the library on the next sync.

use crate::library::{Library, now};
use crate::remote::RemoteItem;
use anyhow::Result;
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
    /// `art` maps a song's cover reference to stored artwork (see `store_cover`).
    pub fn sync_remote_songs(&self, conn: i64, items: &[RemoteItem], art: &HashMap<String, String>) -> Result<LibrarySync> {
        let mut report = LibrarySync { total: items.len(), ..Default::default() };
        let locals: HashSet<String> = {
            let db = self.db();
            let mut st = db.prepare("SELECT title, artist, album FROM songs WHERE provider = 'local' AND source = 'library'")?;
            let rows = st.query_map([], |r| Ok(key(&r.get::<_, String>(0)?, &r.get::<_, String>(1)?, &r.get::<_, String>(2)?)))?;
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
            for it in items.iter().filter(|i| i.kind == "song") {
                if locals.contains(&key(&it.name, &it.artist, &it.album)) {
                    report.local += 1;
                    continue;
                }
                let pid = format!("{conn}:{}", it.id);
                keep.insert(pid.clone());
                let cover = it.cover.as_ref().and_then(|c| art.get(c)).cloned();
                let existed: bool = tx.query_row("SELECT COUNT(*) FROM songs WHERE provider = 'remote' AND provider_id = ?1", [&pid], |r| r.get::<_, i64>(0))? > 0;
                tx.execute(
                    "INSERT INTO songs(source, provider, provider_id, title, artist, album, year, genre, track, duration, art, added_at, updated_at) \
                     VALUES ('library', 'remote', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10) \
                     ON CONFLICT(provider, provider_id) WHERE provider_id IS NOT NULL DO UPDATE SET title = excluded.title, artist = excluded.artist, \
                     album = excluded.album, year = excluded.year, genre = excluded.genre, track = excluded.track, duration = excluded.duration, \
                     art = COALESCE(excluded.art, songs.art)",
                    params![pid, it.name, it.artist, it.album, it.year, genres[&it.genre], it.track, it.duration, cover, t],
                )?;
                if existed { report.updated += 1 } else { report.added += 1 }
            }
            let old: Vec<(i64, String)> = {
                let mut st = tx.prepare("SELECT id, provider_id FROM songs WHERE provider = 'remote' AND provider_id LIKE ?1")?;
                st.query_map([format!("{conn}:%")], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(|r| r.ok()).collect()
            };
            for (id, pid) in old {
                if !keep.contains(&pid) {
                    tx.execute("DELETE FROM songs WHERE id = ?1", [id])?;
                    report.removed += 1;
                }
            }
            tx.commit()?;
        }
        Ok(report)
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

    #[test]
    fn server_songs_join_the_library_and_follow_the_server() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(d.path()).unwrap();
        // a local file of the same song
        {
            let db = l.db();
            db.execute("INSERT INTO songs(source, title, artist, album, genre, added_at, updated_at) VALUES ('library', 'Numb', 'Linkin Park', 'Meteora', 'Rock', 1, 1)", []).unwrap();
        }
        let art: HashMap<String, String> = [("c-Meteora".to_string(), "abc.jpg".to_string())].into();
        let items = vec![song("1", "Numb", "Linkin Park", "Meteora"), song("2", "Faint", "Linkin Park", "Meteora"), song("3", "Lying from You", "Linkin Park", "Meteora")];
        let r = l.sync_remote_songs(7, &items, &art).unwrap();
        assert_eq!((r.added, r.local, r.removed), (2, 1, 0), "the local Numb wins");
        let faint = l.songs().unwrap().into_iter().find(|s| s.title == "Faint").unwrap();
        assert_eq!((faint.provider.as_str(), faint.provider_id.as_deref(), faint.art.as_deref(), faint.genre.as_str()), ("remote", Some("7:2"), Some("abc.jpg"), "Rock"));
        // the server drops a song and renames another
        let items = vec![RemoteItem { name: "Faint (Live)".into(), ..song("2", "", "Linkin Park", "Meteora") }];
        let r = l.sync_remote_songs(7, &items, &HashMap::new()).unwrap();
        assert_eq!((r.updated, r.removed), (1, 1));
        let faint = l.songs().unwrap().into_iter().find(|s| s.provider_id.as_deref() == Some("7:2")).unwrap();
        assert_eq!((faint.title.as_str(), faint.art.as_deref()), ("Faint (Live)", Some("abc.jpg")), "artwork is kept when not resent");
        assert_eq!(l.remove_remote_songs(7).unwrap(), 1);
    }
}
