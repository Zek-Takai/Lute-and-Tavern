//! MCP tools that let a connected AI run the app for the person: search the library, edit song details,
//! organize genre tags, build and change playlists, and play music. Only offered when the person turns on
//! "Full control" in Settings; the songwriting tools in `mcp` are always there once MCP is on.

use crate::library::{Library, SongPatch};
use crate::mcp::{Context, Effects};
use crate::Song;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

const NAMES: &[&str] = &[
    "library_overview", "search_songs", "get_song", "update_song", "list_playlists", "get_playlist",
    "create_playlist", "add_to_playlist", "remove_from_playlist", "reorder_playlist", "rename_playlist",
    "delete_playlist", "create_genre_tag", "rename_genre_tag", "delete_genre_tag", "rescan_library",
    "play", "queue_songs", "playback", "now_playing", "search_radio", "play_radio",
];

pub fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

pub fn tools() -> Value {
    let ro = json!({"readOnlyHint": true});
    let edit = json!({"readOnlyHint": false, "destructiveHint": false});
    let destroy = json!({"readOnlyHint": false, "destructiveHint": true});
    let ids = json!({"type": "array", "items": {"type": "integer"}});
    let obj = |props: Value, req: &[&str]| json!({"type": "object", "properties": props, "required": req});
    json!([
        {"name": "library_overview", "title": "Library overview",
         "description": "Counts, genre tags, playlists, the most common artists and what is playing. Start here to get your bearings.",
         "inputSchema": obj(json!({}), &[]), "annotations": ro},
        {"name": "search_songs", "title": "Search songs",
         "description": "Finds songs. `query` matches words in title, artist, album, genre, style and lyrics; the other fields narrow the results. Returns ids to use with the other tools.",
         "inputSchema": obj(json!({
            "query": {"type": "string"}, "genre": {"type": "string", "description": "Exact genre tag"},
            "artist": {"type": "string"}, "album": {"type": "string"},
            "year_from": {"type": "integer"}, "year_to": {"type": "integer"},
            "source": {"type": "string", "enum": ["any", "my_music", "written", "apple_music"], "description": "my_music: the person's own files; written: songs written in the app; apple_music: saved Apple Music songs"},
            "playable_only": {"type": "boolean"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "description": "Default 100"}}), &[]),
         "annotations": ro},
        {"name": "get_song", "title": "Get a song",
         "description": "Everything about one song, including which playlists it is in; written songs also get lyrics and Suno fields.",
         "inputSchema": obj(json!({"id": {"type": "integer"}}), &["id"]), "annotations": ro},
        {"name": "update_song", "title": "Edit song details",
         "description": "Changes a song's title, artist, album, year or genre tag (a new tag is created when needed). Files in the library folder are renamed to match.",
         "inputSchema": obj(json!({"id": {"type": "integer"}, "title": {"type": "string"}, "artist": {"type": "string"},
            "album": {"type": "string"}, "year": {"type": "string"}, "genre": {"type": "string"}}), &["id"]),
         "annotations": edit},
        {"name": "list_playlists", "title": "List playlists", "description": "Every playlist with its id and song count.",
         "inputSchema": obj(json!({}), &[]), "annotations": ro},
        {"name": "get_playlist", "title": "Get a playlist", "description": "A playlist's songs in order.",
         "inputSchema": obj(json!({"id": {"type": "integer"}}), &["id"]), "annotations": ro},
        {"name": "create_playlist", "title": "Create a playlist", "description": "Makes a playlist from song ids, in the order given.",
         "inputSchema": obj(json!({"name": {"type": "string"}, "song_ids": ids}), &["name"]), "annotations": edit},
        {"name": "add_to_playlist", "title": "Add to a playlist", "description": "Adds songs to the end of a playlist (songs already in it are skipped).",
         "inputSchema": obj(json!({"playlist_id": {"type": "integer"}, "song_ids": ids}), &["playlist_id", "song_ids"]), "annotations": edit},
        {"name": "remove_from_playlist", "title": "Remove from a playlist", "description": "Takes songs out of a playlist. The songs stay in the library.",
         "inputSchema": obj(json!({"playlist_id": {"type": "integer"}, "song_ids": ids}), &["playlist_id", "song_ids"]), "annotations": edit},
        {"name": "reorder_playlist", "title": "Reorder a playlist", "description": "Sets the playlist's full song order. Songs left out are removed from it.",
         "inputSchema": obj(json!({"playlist_id": {"type": "integer"}, "song_ids": ids}), &["playlist_id", "song_ids"]), "annotations": edit},
        {"name": "rename_playlist", "title": "Rename a playlist",
         "inputSchema": obj(json!({"playlist_id": {"type": "integer"}, "name": {"type": "string"}}), &["playlist_id", "name"]), "annotations": edit},
        {"name": "delete_playlist", "title": "Delete a playlist", "description": "Deletes the playlist. Its songs stay in the library. Confirm with the person first.",
         "inputSchema": obj(json!({"playlist_id": {"type": "integer"}}), &["playlist_id"]), "annotations": destroy},
        {"name": "create_genre_tag", "title": "Create a genre tag",
         "inputSchema": obj(json!({"name": {"type": "string"}}), &["name"]), "annotations": edit},
        {"name": "rename_genre_tag", "title": "Rename a genre tag", "description": "Renames a tag; every song with it follows.",
         "inputSchema": obj(json!({"name": {"type": "string"}, "new_name": {"type": "string"}}), &["name", "new_name"]), "annotations": edit},
        {"name": "delete_genre_tag", "title": "Delete a genre tag", "description": "Deletes a tag; its songs become Unsorted. Confirm with the person first.",
         "inputSchema": obj(json!({"name": {"type": "string"}}), &["name"]), "annotations": destroy},
        {"name": "rescan_library", "title": "Check music folders", "description": "Looks for songs added, changed or removed in the music folders.",
         "inputSchema": obj(json!({}), &[]), "annotations": edit},
        {"name": "play", "title": "Play music",
         "description": "Plays songs in the app now, replacing the queue: pass song_ids (in order), or a playlist_id. Optional shuffle.",
         "inputSchema": obj(json!({"song_ids": ids, "playlist_id": {"type": "integer"}, "shuffle": {"type": "boolean"}}), &[]),
         "annotations": edit},
        {"name": "queue_songs", "title": "Queue songs", "description": "Adds songs to the queue: next, or at the end.",
         "inputSchema": obj(json!({"song_ids": ids, "play_next": {"type": "boolean"}}), &["song_ids"]), "annotations": edit},
        {"name": "playback", "title": "Playback controls",
         "inputSchema": obj(json!({"action": {"type": "string", "enum": ["pause", "resume", "next", "previous", "stop"]}}), &["action"]),
         "annotations": edit},
        {"name": "search_radio", "title": "Search radio stations",
         "description": "Finds internet radio stations (most real FM/AM stations stream online): by name or city, genre tag (jazz, news, classic rock...) and two-letter country code. Returns uuids for play_radio.",
         "inputSchema": obj(json!({"query": {"type": "string"}, "genre": {"type": "string"}, "country": {"type": "string", "description": "e.g. US, GB, DE"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 50}}), &[]), "annotations": ro},
        {"name": "play_radio", "title": "Play a radio station",
         "description": "Plays a station in the app: give its uuid from search_radio, or a query (name/genre) to play the best match.",
         "inputSchema": obj(json!({"uuid": {"type": "string"}, "query": {"type": "string"}, "genre": {"type": "string"}, "country": {"type": "string"}}), &[]),
         "annotations": edit},
        {"name": "now_playing", "title": "What's playing", "description": "The current song, whether it is paused, and the queue.",
         "inputSchema": obj(json!({}), &[]), "annotations": ro},
    ])
}

fn int(a: &Value, k: &str) -> Result<i64> {
    a[k].as_i64().ok_or_else(|| anyhow!("{k} is required"))
}

fn id_list(a: &Value, k: &str) -> Vec<i64> {
    a[k].as_array().map(|v| v.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default()
}

fn src_label(s: &Song) -> &'static str {
    if s.provider == "apple" {
        "apple_music"
    } else if s.source == "ai" {
        "written"
    } else {
        "my_music"
    }
}

fn playable(s: &Song) -> bool {
    s.provider == "apple" || s.has_audio || s.stems > 0 || s.notes.as_array().is_some_and(|n| !n.is_empty())
}

fn row(s: &Song) -> Value {
    json!({"id": s.id, "title": s.title, "artist": s.artist, "album": s.album, "year": s.year, "genre": s.genre,
           "duration": s.duration.map(|d| d.round()), "source": src_label(s), "playable": playable(s)})
}

fn songs_by_id(lib: &Library, ids: &[i64]) -> Result<Vec<Song>> {
    let all = lib.songs()?;
    ids.iter()
        .map(|id| all.iter().find(|s| s.id == *id).cloned().ok_or_else(|| anyhow!("No song with id {id}")))
        .collect()
}

fn genre_id(lib: &Library, name: &str) -> Result<i64> {
    lib.genres()?
        .into_iter()
        .find(|g| g.name.eq_ignore_ascii_case(name.trim()))
        .map(|g| g.id)
        .ok_or_else(|| anyhow!("No genre tag called \"{name}\""))
}

fn playlist(lib: &Library, id: i64) -> Result<crate::library::Playlist> {
    lib.playlists()?.into_iter().find(|p| p.id == id).ok_or_else(|| anyhow!("No playlist with id {id}"))
}

fn now_playing(lib: &Library) -> Value {
    let np = lib.settings().ok().and_then(|m| m.get("nowPlaying").cloned()).unwrap_or(Value::Null);
    let Some(sid) = np["songId"].as_i64() else { return json!({"playing": false}) };
    let song = lib.song(sid).ok().map(|s| row(&s));
    let queue: Vec<i64> = np["queue"].as_array().map(|q| q.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
    let upcoming: Vec<Value> = songs_by_id(lib, &queue)
        .unwrap_or_default()
        .iter()
        .skip(np["qi"].as_u64().unwrap_or(0) as usize + 1)
        .take(20)
        .map(|s| json!({"id": s.id, "title": s.title, "artist": s.artist}))
        .collect();
    json!({"song": song, "paused": np["paused"].as_bool().unwrap_or(true), "positionSeconds": np["pos"], "from": np["from"], "upNext": upcoming})
}

pub fn call(cx: &Context, name: &str, a: &Value) -> Result<(Value, Effects)> {
    let lib = cx.lib;
    let none = Effects::default();
    let changed = Effects { library: true, ..Default::default() };
    match name {
        "library_overview" => {
            let songs = lib.songs()?;
            let mut by_source: BTreeMap<&str, usize> = BTreeMap::new();
            let mut artists: BTreeMap<String, usize> = BTreeMap::new();
            let mut albums = HashSet::new();
            for s in &songs {
                *by_source.entry(src_label(s)).or_default() += 1;
                if !s.artist.is_empty() {
                    *artists.entry(s.artist.clone()).or_default() += 1;
                }
                if !s.album.is_empty() {
                    albums.insert((s.artist.to_lowercase(), s.album.to_lowercase()));
                }
            }
            let mut top: Vec<_> = artists.into_iter().collect();
            top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let genres: Vec<Value> = lib.genres()?.into_iter().filter(|g| g.songs > 0).map(|g| json!({"name": g.name, "songs": g.songs})).collect();
            let playlists: Vec<Value> = lib.playlists()?.into_iter().map(|p| json!({"id": p.id, "name": p.name, "songs": p.songs.len()})).collect();
            Ok((json!({
                "songs": songs.len(), "bySource": by_source, "albums": albums.len(),
                "topArtists": top.into_iter().take(40).map(|(a, n)| json!({"artist": a, "songs": n})).collect::<Vec<_>>(),
                "genres": genres, "playlists": playlists, "nowPlaying": now_playing(lib),
            }), none))
        }
        "search_songs" => {
            let words: Vec<String> = a["query"].as_str().unwrap_or("").to_lowercase().split_whitespace().map(String::from).collect();
            let lc = |k: &str| a[k].as_str().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
            let (genre, artist, album) = (lc("genre"), lc("artist"), lc("album"));
            let source = a["source"].as_str().unwrap_or("any");
            let (from, to) = (a["year_from"].as_i64(), a["year_to"].as_i64());
            let limit = a["limit"].as_u64().unwrap_or(100).clamp(1, 1000) as usize;
            let matched: Vec<Song> = lib
                .songs()?
                .into_iter()
                .filter(|s| source == "any" || src_label(s) == source)
                .filter(|s| genre.as_ref().is_none_or(|g| s.genre.to_lowercase() == *g))
                .filter(|s| artist.as_ref().is_none_or(|x| s.artist.to_lowercase().contains(x.as_str())))
                .filter(|s| album.as_ref().is_none_or(|x| s.album.to_lowercase().contains(x.as_str())))
                .filter(|s| {
                    let y: Option<i64> = s.year.get(..4).and_then(|y| y.parse().ok());
                    (from.is_none() && to.is_none()) || y.is_some_and(|y| from.is_none_or(|f| y >= f) && to.is_none_or(|t| y <= t))
                })
                .filter(|s| !a["playable_only"].as_bool().unwrap_or(false) || playable(s))
                .filter(|s| {
                    let hay = [&s.title, &s.artist, &s.album, &s.genre, &s.sub_genre, &s.style, &s.lyrics].map(|f| f.to_lowercase()).join(" ");
                    words.iter().all(|w| hay.contains(w.as_str()))
                })
                .collect();
            let total = matched.len();
            let rows: Vec<Value> = matched.iter().take(limit).map(row).collect();
            Ok((json!({"total": total, "shown": rows.len(), "songs": rows}), none))
        }
        "get_song" => {
            let id = int(a, "id")?;
            let d = lib.song_detail(id)?;
            let s = &d.song;
            let mut v = row(s);
            v["trackNumber"] = json!(s.track);
            v["playlists"] = json!(d.playlists);
            v["stems"] = json!(s.stems);
            v["takes"] = json!(d.takes.len());
            if s.source == "ai" {
                for (k, x) in [("lyrics", &s.lyrics), ("style", &s.style), ("sub_genre", &s.sub_genre), ("key", &s.key)] {
                    v[k] = json!(x);
                }
                v["request"] = json!(s.request);
            }
            Ok((v, none))
        }
        "update_song" => {
            let id = int(a, "id")?;
            let t = |k: &str| a[k].as_str().map(|s| s.trim().to_string());
            let patch = SongPatch { title: t("title"), artist: t("artist"), album: t("album"), year: t("year"), genre: t("genre") };
            let s = lib.update_song(id, &patch)?;
            Ok((row(&s), changed))
        }
        "list_playlists" => {
            let p: Vec<Value> = lib.playlists()?.into_iter().map(|p| json!({"id": p.id, "name": p.name, "songs": p.songs.len()})).collect();
            Ok((json!({"playlists": p}), none))
        }
        "get_playlist" => {
            let p = playlist(lib, int(a, "id")?)?;
            let songs: Vec<Value> = songs_by_id(lib, &p.songs)?.iter().map(row).collect();
            Ok((json!({"id": p.id, "name": p.name, "songs": songs}), none))
        }
        "create_playlist" => {
            let name = a["name"].as_str().unwrap_or("").trim();
            if name.is_empty() {
                bail!("name is required");
            }
            let ids = id_list(a, "song_ids");
            songs_by_id(lib, &ids)?;
            let p = lib.create_playlist(name, &ids)?;
            Ok((json!({"id": p.id, "name": p.name, "songs": p.songs.len()}), changed))
        }
        "add_to_playlist" | "remove_from_playlist" | "reorder_playlist" => {
            let p = playlist(lib, int(a, "playlist_id")?)?;
            let ids = id_list(a, "song_ids");
            songs_by_id(lib, &ids)?;
            let new: Vec<i64> = match name {
                "add_to_playlist" => p.songs.iter().copied().chain(ids.iter().copied().filter(|i| !p.songs.contains(i))).collect(),
                "remove_from_playlist" => p.songs.iter().copied().filter(|i| !ids.contains(i)).collect(),
                _ => ids,
            };
            lib.set_playlist_songs(p.id, &new)?;
            Ok((json!({"id": p.id, "name": p.name, "songs": new.len()}), changed))
        }
        "rename_playlist" => {
            let id = int(a, "playlist_id")?;
            playlist(lib, id)?;
            lib.rename_playlist(id, a["name"].as_str().unwrap_or("").trim())?;
            Ok((json!({"id": id, "name": a["name"]}), changed))
        }
        "delete_playlist" => {
            let p = playlist(lib, int(a, "playlist_id")?)?;
            lib.delete_playlist(p.id)?;
            Ok((json!({"deleted": p.name}), changed))
        }
        "create_genre_tag" => {
            let g = lib.create_genre(a["name"].as_str().unwrap_or(""))?;
            Ok((json!({"name": g.name}), changed))
        }
        "rename_genre_tag" => {
            let id = genre_id(lib, a["name"].as_str().unwrap_or(""))?;
            lib.rename_genre(id, a["new_name"].as_str().unwrap_or(""))?;
            Ok((json!({"renamed": a["name"], "to": a["new_name"]}), changed))
        }
        "delete_genre_tag" => {
            let id = genre_id(lib, a["name"].as_str().unwrap_or(""))?;
            lib.delete_genre(id)?;
            Ok((json!({"deleted": a["name"]}), changed))
        }
        "rescan_library" => {
            let (mut added, mut updated, mut removed) = (0, 0, 0);
            for f in lib.folders()? {
                let r = lib.scan_folder(&f)?;
                (added, updated, removed) = (added + r.added, updated + r.updated, removed + r.removed);
            }
            Ok((json!({"added": added, "updated": updated, "removed": removed}), changed))
        }
        "play" => {
            let mut ids = match a["playlist_id"].as_i64() {
                Some(pid) => playlist(lib, pid)?.songs,
                None => id_list(a, "song_ids"),
            };
            if ids.is_empty() {
                bail!("Give song_ids or a playlist_id to play");
            }
            let songs = songs_by_id(lib, &ids)?;
            ids.retain(|i| songs.iter().any(|s| s.id == *i && playable(s)));
            if ids.is_empty() {
                bail!("None of those songs has audio to play yet");
            }
            let first = songs.iter().find(|s| s.id == ids[0]).map(row);
            let ui = json!({"action": "play", "songIds": ids, "shuffle": a["shuffle"].as_bool().unwrap_or(false)});
            Ok((json!({"playing": first, "queued": ids.len()}), Effects { ui: Some(ui), ..Default::default() }))
        }
        "queue_songs" => {
            let ids = id_list(a, "song_ids");
            let songs = songs_by_id(lib, &ids)?;
            let ids: Vec<i64> = songs.iter().filter(|s| playable(s)).map(|s| s.id).collect();
            if ids.is_empty() {
                bail!("None of those songs has audio to play yet");
            }
            let ui = json!({"action": "queue", "songIds": ids, "next": a["play_next"].as_bool().unwrap_or(false)});
            Ok((json!({"queued": ids.len()}), Effects { ui: Some(ui), ..Default::default() }))
        }
        "playback" => {
            let action = a["action"].as_str().unwrap_or("");
            if !["pause", "resume", "next", "previous", "stop"].contains(&action) {
                bail!("action must be pause, resume, next, previous or stop");
            }
            Ok((json!({"done": action}), Effects { ui: Some(json!({"action": action})), ..Default::default() }))
        }
        "now_playing" => Ok((now_playing(lib), none)),
        "search_radio" | "play_radio" => {
            let s = |k: &str| a[k].as_str().unwrap_or("").trim().to_string();
            let uuid = s("uuid");
            let found: Vec<crate::radio::Station> = if !uuid.is_empty() {
                match lib.station(&uuid)? {
                    Some(st) => vec![st],
                    None => crate::radio::search("", "", "", 200)?.into_iter().filter(|st| st.uuid == uuid).collect(),
                }
            } else {
                crate::radio::search(&s("query"), &s("genre").to_lowercase(), &s("country").to_uppercase(), a["limit"].as_u64().unwrap_or(10).clamp(1, 50) as usize)?
            };
            if name == "search_radio" {
                let rows: Vec<Value> = found.iter().map(|st| json!({"uuid": st.uuid, "name": st.name, "country": st.country, "genres": st.tags, "bitrate": st.bitrate})).collect();
                return Ok((json!({"stations": rows}), none));
            }
            let st = found.into_iter().next().ok_or_else(|| anyhow!("No station matches that"))?;
            let ui = json!({"action": "play_station", "station": st});
            Ok((json!({"playing": st.name, "country": st.country}), Effects { ui: Some(ui), ..Default::default() }))
        }
        _ => bail!("Unknown tool: {name}"),
    }
}
