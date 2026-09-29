//! MCP (Model Context Protocol) songwriting tools. Any AI app that connects can read the genre tags and
//! written songs, learn how Suno wants a song laid out, and write or revise songs in the library.
//! Transport-free: `handle` takes one JSON-RPC message; the desktop app serves it over local HTTP.

use crate::library::{Draft, Library};
use crate::songwriter::{hook_to_notes, suno_guide};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

pub struct Context<'a> {
    pub lib: &'a Library,
    /// Credit for new songs unless the caller names a songwriter
    pub writer_name: String,
    /// Whether this library uses Suno's "Exclude styles" box
    pub exclude: bool,
    pub version: &'a str,
}

pub struct Handled {
    /// None for notifications, which get no answer
    pub reply: Option<Value>,
    /// The song a tool created or changed, so the app can refresh
    pub changed: Option<i64>,
}

pub fn handle(cx: &Context, msg: &Value) -> Handled {
    let method = msg["method"].as_str().unwrap_or("");
    let Some(id) = msg.get("id").cloned() else {
        return Handled { reply: None, changed: None }; // notification (e.g. notifications/initialized)
    };
    let params = &msg["params"];
    let mut changed = None;
    let result: std::result::Result<Value, (i64, String)> = match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or("");
            let v = if PROTOCOL_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSIONS[0] };
            Ok(json!({
                "protocolVersion": v,
                "capabilities": {"tools": {}, "prompts": {}},
                "serverInfo": {"name": "lute-and-tavern", "title": "Lute & Tavern", "version": cx.version},
                "instructions": "Songwriting tools for Suno. Read suno_songwriting_guide before writing, then save songs with create_song.",
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call(cx, name, &args) {
                Ok((v, id)) => {
                    changed = id;
                    let text = if let Value::String(s) = &v { s.clone() } else { serde_json::to_string_pretty(&v).unwrap_or_default() };
                    let mut r = json!({"content": [{"type": "text", "text": text}], "isError": false});
                    if v.is_object() {
                        r["structuredContent"] = v;
                    }
                    r
                }
                Err(e) => json!({"content": [{"type": "text", "text": format!("{e:#}")}], "isError": true}),
            })
        }
        "prompts/list" => Ok(json!({"prompts": [{
            "name": "write_suno_song",
            "title": "Write a song for Suno",
            "description": "Write an original song for Suno from a request and save it in the Lute & Tavern library.",
            "arguments": [{"name": "request", "description": "What the song should be about, its genre and mood", "required": true}],
        }]})),
        "prompts/get" if params["name"] == "write_suno_song" => {
            let request = params["arguments"]["request"].as_str().unwrap_or("").trim().to_string();
            Ok(json!({
                "description": "Write a song for Suno",
                "messages": [{"role": "user", "content": {"type": "text", "text": format!(
                    "{}\n\nWrite a song for this request and save it with create_song:\n{request}", suno_guide(cx.exclude))}}],
            }))
        }
        "prompts/get" => Err((-32602, "Unknown prompt".into())),
        _ => Err((-32601, format!("Method not found: {method}"))),
    };
    let reply = match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    };
    Handled { reply: Some(reply), changed }
}

fn song_fields(require: bool) -> Value {
    let req = |d: &str| if require { format!("{d} (required)") } else { d.to_string() };
    json!({
        "title": {"type": "string", "description": req("Song title")},
        "style": {"type": "string", "description": req("Suno 'Style of music': comma-separated tags, under 200 characters")},
        "lyrics": {"type": "string", "description": req("Full lyrics with [Verse 1], [Chorus] and other section tags on their own lines")},
        "genre": {"type": "string", "description": "One genre tag; reuse one from list_genre_tags when it fits"},
        "sub_genre": {"type": "string", "description": "A more specific style, e.g. 'Dream pop'"},
        "key": {"type": "string", "description": "Musical key, e.g. C, Am, F#m"},
        "bpm": {"type": "integer", "minimum": 0, "maximum": 300},
        "exclude": {"type": "string", "description": "Suno 'Exclude styles' tags; leave out unless the guide says this library uses them"},
        "chords": {"type": "array", "items": {"type": "string"}, "description": "4 to 8 chord symbols for the chorus, one per 4/4 measure"},
        "hook": {"type": "array", "description": "Chorus melody: [{note: 'G4', len: 'w'|'h'|'q'|'e', lyric: 'syl-'}], each measure 4 beats",
                 "items": {"type": "object", "properties": {"note": {"type": "string"}, "len": {"type": "string"}, "lyric": {"type": "string"}}}},
    })
}

fn tools() -> Value {
    let read_only = json!({"readOnlyHint": true});
    let mut create = song_fields(true);
    create["request"] = json!({"type": "string", "description": "What the person asked for, kept with the song"});
    create["note"] = json!({"type": "string", "description": "One or two sentences to the person about the song"});
    create["songwriter"] = json!({"type": "string", "description": "Name to credit; defaults to the songwriter name set in the app"});
    let mut revise = song_fields(false);
    revise["id"] = json!({"type": "integer", "description": "The song's id"});
    revise["note"] = json!({"type": "string", "description": "What changed, shown in the song's version history"});
    json!([
        {"name": "suno_songwriting_guide", "title": "Suno songwriting guide",
         "description": "How to lay out a song for Suno (title, style, lyrics and section tags) and how to save it here. Read this before writing.",
         "inputSchema": {"type": "object", "properties": {}}, "annotations": read_only},
        {"name": "list_genre_tags", "title": "List genre tags",
         "description": "The genre tags already in the library, with how many songs use each.",
         "inputSchema": {"type": "object", "properties": {}}, "annotations": read_only},
        {"name": "list_songs", "title": "List written songs",
         "description": "Songs written with the app's songwriter tools (newest first), optionally filtered by words in the title, genre or style.",
         "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer", "minimum": 1, "maximum": 200}}},
         "annotations": read_only},
        {"name": "get_song", "title": "Get a song",
         "description": "Everything about one written song: the Suno fields, chords, request, versions and conversation.",
         "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}, "annotations": read_only},
        {"name": "create_song", "title": "Save a new song",
         "description": "Saves a finished song for Suno in the library. Returns its id.",
         "inputSchema": {"type": "object", "properties": create, "required": ["title", "style", "lyrics"]},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false}},
        {"name": "revise_song", "title": "Revise a song",
         "description": "Changes a written song. Send only the fields that change; the rest stay. Each revision is kept as a version.",
         "inputSchema": {"type": "object", "properties": revise, "required": ["id"]},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false}},
    ])
}

fn text(a: &Value, k: &str) -> Option<String> {
    a[k].as_str().map(|s| s.trim().to_string())
}

fn draft_from(a: &Value, base: Draft) -> Draft {
    let chords = a["chords"].as_array().map(|c| c.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).collect::<Vec<_>>());
    Draft {
        title: text(a, "title").unwrap_or(base.title),
        genre: text(a, "genre").unwrap_or(base.genre),
        sub_genre: text(a, "sub_genre").unwrap_or(base.sub_genre),
        key: text(a, "key").unwrap_or(base.key),
        bpm: a["bpm"].as_i64().map(|b| b.clamp(0, 300)).unwrap_or(base.bpm),
        style: text(a, "style").unwrap_or(base.style),
        exclude: text(a, "exclude").unwrap_or(base.exclude),
        lyrics: text(a, "lyrics").unwrap_or(base.lyrics),
        notes: hook_to_notes(&a["hook"]),
        chords: chords.filter(|c| !c.is_empty()),
    }
}

fn written(lib: &Library, id: i64) -> Result<crate::Song> {
    let s = lib.song(id)?;
    if s.source != "ai" {
        bail!("Song {id} is not a written song. These tools only change songs written for Suno.");
    }
    Ok(s)
}

fn call(cx: &Context, name: &str, a: &Value) -> Result<(Value, Option<i64>)> {
    let lib = cx.lib;
    match name {
        "suno_songwriting_guide" => Ok((Value::String(suno_guide(cx.exclude)), None)),
        "list_genre_tags" => {
            let tags: Vec<Value> = lib.genres()?.into_iter().map(|g| json!({"name": g.name, "songs": g.songs})).collect();
            Ok((json!({"tags": tags}), None))
        }
        "list_songs" => {
            let q = text(a, "query").unwrap_or_default().to_lowercase();
            let limit = a["limit"].as_u64().unwrap_or(50).clamp(1, 200) as usize;
            let songs: Vec<Value> = lib
                .songs()?
                .into_iter()
                .filter(|s| s.source == "ai")
                .filter(|s| q.is_empty() || [&s.title, &s.genre, &s.style].iter().any(|f| f.to_lowercase().contains(&q)))
                .take(limit)
                .map(|s| json!({"id": s.id, "title": s.title, "songwriter": s.artist, "genre": s.genre, "status": s.status, "updatedAt": s.updated_at}))
                .collect();
            Ok((json!({"songs": songs}), None))
        }
        "get_song" => {
            let id = a["id"].as_i64().ok_or_else(|| anyhow!("id is required"))?;
            written(lib, id)?;
            let d = lib.song_detail(id)?;
            let s = &d.song;
            let mut v = json!({
                "id": s.id, "title": s.title, "songwriter": s.artist, "genre": s.genre, "sub_genre": s.sub_genre, "key": s.key,
                "bpm": s.bpm, "style": s.style, "lyrics": s.lyrics, "chords": s.chords, "request": s.request, "status": s.status,
                "versions": d.versions.iter().map(|v| json!({"label": v.label, "note": v.note, "createdAt": v.created_at})).collect::<Vec<_>>(),
                "conversation": d.messages.iter().map(|m| json!({"role": m.role, "text": m.text})).collect::<Vec<_>>(),
            });
            if cx.exclude {
                v["exclude"] = json!(s.exclude);
            }
            Ok((v, None))
        }
        "create_song" => {
            let d = draft_from(a, Draft::default());
            for (k, v) in [("title", &d.title), ("style", &d.style), ("lyrics", &d.lyrics)] {
                if v.is_empty() {
                    bail!("{k} is required");
                }
            }
            let d = Draft { exclude: if cx.exclude { d.exclude } else { String::new() }, ..d };
            let who = text(a, "songwriter").filter(|s| !s.is_empty()).unwrap_or_else(|| cx.writer_name.clone());
            let request = text(a, "request").filter(|s| !s.is_empty()).unwrap_or_else(|| format!("\"{}\"", d.title));
            let song = lib.create_request(&request, None, &who)?;
            let song = lib.apply_draft(song.id, &d, "First draft", false)?;
            if let Some(note) = text(a, "note").filter(|s| !s.is_empty()) {
                lib.add_message(song.id, "writer", &note)?;
            }
            Ok((json!({"id": song.id, "title": song.title, "genre": song.genre, "songwriter": song.artist}), Some(song.id)))
        }
        "revise_song" => {
            let id = a["id"].as_i64().ok_or_else(|| anyhow!("id is required"))?;
            let s = written(lib, id)?;
            let base = Draft {
                title: s.title, genre: s.genre, sub_genre: s.sub_genre, key: s.key, bpm: s.bpm, style: s.style,
                exclude: s.exclude, lyrics: s.lyrics, notes: None, chords: None,
            };
            let d = draft_from(a, base);
            let d = Draft { exclude: if cx.exclude { d.exclude } else { String::new() }, ..d };
            let note = text(a, "note").filter(|s| !s.is_empty()).unwrap_or_else(|| "Revised".into());
            let keep_genre = text(a, "genre").is_none_or(|g| g.is_empty());
            let song = lib.apply_draft(id, &d, &note, keep_genre)?;
            lib.add_message(id, "writer", &note)?;
            Ok((json!({"id": song.id, "title": song.title, "genre": song.genre}), Some(id)))
        }
        _ => bail!("Unknown tool: {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> (tempfile::TempDir, Library) {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(d.path()).unwrap();
        (d, l)
    }

    fn rpc(cx: &Context, method: &str, params: Value) -> Value {
        handle(cx, &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})).reply.unwrap()
    }

    #[test]
    fn tools_write_and_revise_songs() {
        let (_d, l) = lib();
        let cx = Context { lib: &l, writer_name: "Ari".into(), exclude: false, version: "0" };
        let init = rpc(&cx, "initialize", json!({"protocolVersion": "2025-06-18"}));
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert!(handle(&cx, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).reply.is_none());
        assert_eq!(rpc(&cx, "tools/list", json!({}))["result"]["tools"].as_array().unwrap().len(), 6);

        let args = json!({"title": "Lantern", "style": "folk, acoustic guitar", "lyrics": "[Verse 1]\nHi", "genre": "folk", "exclude": "drums"});
        let r = rpc(&cx, "tools/call", json!({"name": "create_song", "arguments": args}));
        assert_eq!(r["result"]["isError"], false, "{r}");
        let id = r["result"]["structuredContent"]["id"].as_i64().unwrap();
        let s = l.song(id).unwrap();
        assert_eq!((s.artist.as_str(), s.genre.as_str(), s.exclude.as_str()), ("Ari", "Folk", ""), "exclude stays off");

        let r = rpc(&cx, "tools/call", json!({"name": "revise_song", "arguments": {"id": id, "title": "Lanterns", "note": "Plural"}}));
        assert_eq!(r["result"]["isError"], false, "{r}");
        let s = l.song(id).unwrap();
        assert_eq!((s.title.as_str(), s.style.as_str(), s.genre.as_str()), ("Lanterns", "folk, acoustic guitar", "Folk"));
        assert_eq!(l.song_detail(id).unwrap().versions.len(), 2);

        let r = rpc(&cx, "tools/call", json!({"name": "create_song", "arguments": {"title": "x"}}));
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(rpc(&cx, "nope", json!({}))["error"]["code"], -32601);
    }
}
