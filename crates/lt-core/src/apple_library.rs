//! The person's Apple Music library in My music: their saved songs and their playlists. The interface reads
//! them through MusicKit (Apple only allows that in the signed-in web player) and hands them over here.
//! A song they already have as a file (or on one of their servers) is used instead of the Apple copy, so
//! nothing shows twice and their own files play first. Apple playlists keep a lasting id ("apple-<id>"), so a
//! later pull updates the same playlist, and it follows to their other devices like any playlist.

use crate::library::{ExternalSong, Library, now};
use crate::Song;
use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const SYNCED_FROM: &str = "apple-library";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplePlaylist {
    pub id: String,
    pub name: String,
    /// Apple's last change time (unix seconds), when it gives one.
    #[serde(default)]
    pub modified: Option<i64>,
    pub tracks: Vec<ExternalSong>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplePull {
    pub songs: usize,
    pub added: usize,
    /// Songs skipped because the person already has them as a file or on their server
    pub have_own: usize,
    pub removed: usize,
    pub playlists: usize,
    pub playlists_updated: usize,
}

/// Same song regardless of case, spacing, "The" or "(Remastered)"-style extras: title + artist.
fn key(title: &str, artist: &str) -> String {
    let n = |s: &str| {
        let s = s.to_lowercase();
        let s = s.split(['(', '[']).next().unwrap_or(&s).split(" - ").next().unwrap_or("").to_string();
        let s = s.trim().strip_prefix("the ").unwrap_or(s.trim()).to_string();
        s.chars().filter(|c| c.is_alphanumeric()).collect::<String>()
    };
    format!("{}|{}", n(title), n(artist))
}

/// The person's own copies (local files and their servers' songs) by title + artist, with lengths.
fn own_songs(songs: &[Song]) -> HashMap<String, Vec<(i64, Option<f64>)>> {
    let mut m: HashMap<String, Vec<(i64, Option<f64>)>> = HashMap::new();
    for s in songs.iter().filter(|s| s.provider != "apple") {
        m.entry(key(&s.title, &s.artist)).or_default().push((s.id, s.duration));
    }
    m
}

fn own_copy(own: &HashMap<String, Vec<(i64, Option<f64>)>>, e: &ExternalSong) -> Option<i64> {
    let c = own.get(&key(&e.title, &e.artist))?;
    // the same length when both are known (a live or extended version is a different song)
    c.iter().find(|(_, d)| matches!((d, e.duration), (Some(a), Some(b)) if (a - b).abs() <= 3.0)).or_else(|| c.iter().find(|(_, d)| d.is_none() || e.duration.is_none())).map(|(id, _)| *id)
}

impl Library {
    /// Saves an Apple song (or finds it), marking it as part of the Apple library when `from_library`.
    fn put_apple_song(&self, e: &ExternalSong, from_library: bool) -> Result<(i64, bool)> {
        let genre = self.ensure_genre(if e.genre.is_empty() { "Unsorted" } else { &e.genre }, "file")?;
        let t = now();
        let db = self.db();
        let n = db.execute(
            "INSERT INTO songs(source, provider, provider_id, title, artist, album, year, genre, track, duration, art_url, added_at, updated_at, synced_from) \
             VALUES ('library', 'apple', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) ON CONFLICT(provider, provider_id) WHERE provider_id IS NOT NULL DO NOTHING",
            params![e.provider_id, e.title, e.artist, e.album, e.year, genre, e.track, e.duration, e.art_url, e.added.unwrap_or(t), t, from_library.then_some(SYNCED_FROM)],
        )?;
        let id: i64 = db.query_row("SELECT id FROM songs WHERE provider = 'apple' AND provider_id = ?1", [&e.provider_id], |r| r.get(0))?;
        if n == 0 && from_library {
            db.execute("UPDATE songs SET synced_from = ?2 WHERE id = ?1 AND synced_from IS NULL", params![id, SYNCED_FROM])?;
        }
        Ok((id, n > 0))
    }

    /// Brings the person's Apple Music songs and playlists into the library (a full list each time, so songs
    /// they removed in Apple Music leave here too, unless they are in one of their playlists).
    pub fn pull_apple_library(&self, songs: &[ExternalSong], playlists: &[ApplePlaylist]) -> Result<ApplePull> {
        let mut r = ApplePull { songs: songs.len(), playlists: playlists.len(), ..Default::default() };
        let all = self.songs()?;
        let own = own_songs(&all);
        let mut keep: HashSet<i64> = HashSet::new();
        let mut ids_by_provider: HashMap<String, i64> = HashMap::new();
        for e in songs {
            if let Some(id) = own_copy(&own, e) {
                r.have_own += 1;
                ids_by_provider.insert(e.provider_id.clone(), id);
                continue;
            }
            let (id, new) = self.put_apple_song(e, true)?;
            keep.insert(id);
            ids_by_provider.insert(e.provider_id.clone(), id);
            r.added += new as usize;
        }

        let pulled: HashMap<String, i64> = {
            let m = self.settings()?;
            m.get("applePlaylistsPulled").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
        };
        let mut seen = pulled.clone();
        for p in playlists {
            let uuid = format!("apple-{}", p.id);
            let exists: bool = self.db().query_row("SELECT COUNT(*) FROM playlists WHERE uuid = ?1", [&uuid], |r| r.get::<_, i64>(0))? > 0;
            let deleted: bool = self.db().query_row("SELECT COUNT(*) FROM playlist_tombstones WHERE uuid = ?1", [&uuid], |r| r.get::<_, i64>(0))? > 0;
            let modified = p.modified.unwrap_or(0);
            // Unchanged in Apple Music since the last pull: leave the person's own edits alone. Deleted
            // here: stays deleted until it changes in Apple Music.
            if pulled.get(&p.id).is_some_and(|m| *m >= modified && modified > 0) && (exists || deleted) {
                for e in &p.tracks {
                    if let Some(id) = ids_by_provider.get(&e.provider_id) {
                        keep.insert(*id);
                    }
                }
                continue;
            }
            let mut ids = Vec::new();
            for e in &p.tracks {
                let id = match ids_by_provider.get(&e.provider_id) {
                    Some(id) => *id,
                    None => match own_copy(&own, e) {
                        Some(id) => id,
                        None => self.put_apple_song(e, false)?.0,
                    },
                };
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            self.set_playlist_by_uuid(&uuid, &p.name, &ids)?;
            seen.insert(p.id.clone(), modified);
            r.playlists_updated += 1;
        }
        self.set_setting("applePlaylistsPulled", &serde_json::to_value(&seen)?)?;

        // Songs that left the Apple library: gone here too, unless a playlist holds them (then they stay as
        // a saved song).
        let db = self.db();
        let gone: Vec<i64> = {
            let mut st = db.prepare("SELECT id FROM songs WHERE provider = 'apple' AND synced_from = ?1")?;
            st.query_map([SYNCED_FROM], |r| r.get(0))?.filter_map(|r| r.ok()).filter(|id| !keep.contains(id)).collect()
        };
        for id in gone {
            let in_pl: bool = db.query_row("SELECT COUNT(*) FROM playlist_songs WHERE song_id = ?1", [id], |r| r.get::<_, i64>(0))? > 0;
            if in_pl {
                db.execute("UPDATE songs SET synced_from = NULL WHERE id = ?1", [id])?;
            } else {
                db.execute("DELETE FROM songs WHERE id = ?1", [id])?;
                r.removed += 1;
            }
        }
        Ok(r)
    }

    /// Creates or replaces the playlist with this lasting id.
    pub fn set_playlist_by_uuid(&self, uuid: &str, name: &str, songs: &[i64]) -> Result<i64> {
        let existing: Option<i64> = self.db().query_row("SELECT id FROM playlists WHERE uuid = ?1", [uuid], |r| r.get(0)).ok();
        let id = match existing {
            Some(id) => {
                self.db().execute("UPDATE playlists SET name = ?2 WHERE id = ?1", params![id, name])?;
                id
            }
            None => {
                let db = self.db();
                db.execute("INSERT INTO playlists(name, created_at, uuid, updated_at) VALUES (?1, ?2, ?3, ?2)", params![name, now(), uuid])?;
                db.execute("DELETE FROM playlist_tombstones WHERE uuid = ?1", [uuid])?;
                db.last_insert_rowid()
            }
        };
        self.set_playlist_songs(id, songs)?;
        Ok(id)
    }

    /// Adds Apple songs to a playlist (saving them first), e.g. from the Apple Music page or to fill songs an
    /// import could not find. `after` puts each one right after that song (an import keeps its order).
    pub fn add_apple_to_playlist(&self, playlist: i64, items: &[(ExternalSong, Option<i64>)]) -> Result<usize> {
        let mut list = self.playlists()?.into_iter().find(|p| p.id == playlist).map(|p| p.songs).ok_or_else(|| anyhow::anyhow!("That playlist no longer exists."))?;
        let own = own_songs(&self.songs()?);
        let mut added = 0;
        for (e, after) in items {
            let id = match own_copy(&own, e) {
                Some(id) => id,
                None => self.put_apple_song(e, false)?.0,
            };
            if list.contains(&id) {
                continue;
            }
            let at = match after {
                Some(a) => list.iter().position(|x| x == a).map(|i| i + 1).unwrap_or(list.len()),
                None if items.iter().any(|(_, a)| a.is_some()) => 0,
                None => list.len(),
            };
            list.insert(at, id);
            added += 1;
        }
        self.set_playlist_songs(playlist, &list)?;
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ext(id: &str, title: &str, artist: &str, dur: f64) -> ExternalSong {
        ExternalSong {
            provider: "apple".into(),
            provider_id: id.into(),
            title: title.into(),
            artist: artist.into(),
            album: "Album".into(),
            genre: "Rock".into(),
            year: "2003".into(),
            track: None,
            duration: Some(dur),
            art_url: None,
            added: Some(100),
        }
    }

    #[test]
    fn pulls_songs_and_playlists_preferring_own_files() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(&d.path().join("app")).unwrap();
        l.db().execute("INSERT INTO songs(source, title, artist, duration, added_at, updated_at) VALUES ('library', 'Numb', 'Linkin Park', 185.5, 1, 1)", []).unwrap();
        let own = l.db().last_insert_rowid();
        let songs = vec![ext("1", "Numb (Remastered)", "Linkin Park", 186.0), ext("2", "Faint", "Linkin Park", 162.0), ext("3", "Lying from You", "Linkin Park", 175.0)];
        let pl = ApplePlaylist { id: "p.1".into(), name: "Meteora".into(), modified: Some(10), tracks: vec![songs[1].clone(), songs[0].clone(), ext("9", "Breaking the Habit", "Linkin Park", 196.0)] };
        let r = l.pull_apple_library(&songs, &[pl.clone()]).unwrap();
        assert_eq!((r.added, r.have_own, r.playlists_updated), (2, 1, 1));
        let p = l.playlists().unwrap().into_iter().find(|p| p.name == "Meteora").unwrap();
        assert_eq!(p.songs.len(), 3);
        assert_eq!(p.songs[1], own, "the person's own file stands in for the Apple copy");

        // a local edit survives a pull when Apple's playlist didn't change
        l.set_playlist_songs(p.id, &p.songs[..1]).unwrap();
        l.pull_apple_library(&songs, &[pl.clone()]).unwrap();
        assert_eq!(l.playlists().unwrap().iter().find(|x| x.id == p.id).unwrap().songs.len(), 1);

        // "Lying from You" was removed in Apple Music and is in no playlist: it leaves; "Faint" stays (in a playlist)
        let r = l.pull_apple_library(&songs[..1], &[pl]).unwrap();
        assert_eq!(r.removed, 1);
        let titles: Vec<String> = l.songs().unwrap().into_iter().map(|s| s.title).collect();
        assert!(titles.contains(&"Faint".to_string()) && !titles.contains(&"Lying from You".to_string()), "{titles:?}");

        // adding to a playlist after a given song keeps import order
        let n = l.add_apple_to_playlist(p.id, &[(ext("7", "Somewhere I Belong", "Linkin Park", 213.0), None)]).unwrap();
        assert_eq!(n, 1);
        assert_eq!(l.playlists().unwrap().iter().find(|x| x.id == p.id).unwrap().songs.len(), 2);
    }
}
