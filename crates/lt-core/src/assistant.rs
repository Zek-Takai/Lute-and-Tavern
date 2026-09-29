//! The in-app assistant: a chat with the person's own AI (any OpenAI-compatible endpoint with tool calling)
//! that can run the app through the same tools MCP clients get. The conversation is kept in the library
//! database. Voice goes through `transcribe`, which speaks the common Whisper API (`/audio/transcriptions`),
//! so any Whisper setup works: a local server, OpenAI, Groq and others.

use crate::library::Library;
use crate::mcp::{self, Context, Effects};
use crate::songwriter::{WriterConfig, chat_message};
use anyhow::{Context as _, Result, bail};
use rusqlite::params;
use serde::Serialize;
use serde_json::{Value, json};

/// Tool rounds per request before the assistant has to answer.
const MAX_STEPS: usize = 12;
/// Earlier messages sent along for context.
const HISTORY: usize = 40;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatLine {
    pub id: i64,
    /// "user", "assistant" or "action" (a tool the assistant used)
    pub role: String,
    pub text: String,
    pub created_at: i64,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Library {
    fn save_assistant(&self, body: &Value) -> Result<()> {
        self.db().execute("INSERT INTO assistant_messages(body, created_at) VALUES (?1, ?2)", params![body.to_string(), now()])?;
        Ok(())
    }

    fn assistant_raw(&self, limit: usize) -> Result<Vec<(i64, Value, i64)>> {
        let db = self.db();
        let mut st = db.prepare("SELECT id, body, created_at FROM (SELECT * FROM assistant_messages ORDER BY id DESC LIMIT ?1) ORDER BY id")?;
        let rows = st
            .query_map([limit as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?
            .filter_map(|r| r.ok())
            .map(|(id, b, t)| (id, serde_json::from_str(&b).unwrap_or(Value::Null), t))
            .collect();
        Ok(rows)
    }

    /// The conversation as the person sees it: their words, the assistant's answers and what it did.
    pub fn assistant_history(&self, limit: usize) -> Result<Vec<ChatLine>> {
        let mut out = Vec::new();
        for (id, m, t) in self.assistant_raw(limit)? {
            let text = strip_thinking(m["content"].as_str().unwrap_or(""));
            match m["role"].as_str().unwrap_or("") {
                "user" => out.push(ChatLine { id, role: "user".into(), text, created_at: t }),
                "assistant" => {
                    for c in m["tool_calls"].as_array().into_iter().flatten() {
                        out.push(ChatLine { id, role: "action".into(), text: describe(&c["function"]), created_at: t });
                    }
                    if !text.is_empty() {
                        out.push(ChatLine { id, role: "assistant".into(), text, created_at: t });
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    pub fn clear_assistant(&self) -> Result<()> {
        self.db().execute("DELETE FROM assistant_messages", [])?;
        Ok(())
    }
}

/// "search_songs(query: rain)" style summary of a tool call, for the chat.
fn describe(f: &Value) -> String {
    let name = f["name"].as_str().unwrap_or("tool").replace('_', " ");
    let args: Value = f["arguments"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or(json!({}));
    let parts: Vec<String> = args
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| match v {
                    Value::String(s) if s.chars().count() > 60 => format!("{k}: {}…", s.chars().take(60).collect::<String>()),
                    Value::String(s) => format!("{k}: {s}"),
                    Value::Array(a) if a.len() > 8 => format!("{k}: {} items", a.len()),
                    v => format!("{k}: {v}"),
                })
                .collect()
        })
        .unwrap_or_default();
    if parts.is_empty() { name } else { format!("{name} ({})", parts.join(", ")) }
}

fn strip_thinking(s: &str) -> String {
    let s = match s.rfind("</think>") {
        Some(i) => &s[i + 8..],
        None => s,
    };
    s.trim().to_string()
}

fn system_prompt(cx: &Context) -> String {
    format!(
        "You are {name}, the assistant inside Lute & Tavern, a music app. You help the person with their music: \
find and play songs, build playlists to their taste, organize genres and song details, and write songs for Suno. \
Use the tools to act instead of describing what you would do. Look things up before guessing ids: search_songs and \
library_overview return the ids other tools need. When asked to play something, find it and call play. Before deleting \
anything, ask. Keep answers short and friendly; the person may be speaking to you, so no lists of ids unless asked.",
        name = cx.writer_name
    )
}

/// Runs one request: the person's words go in, the assistant may call tools (acting on the app) and answers.
/// `on_effect` is called for each tool effect (refresh the library, play music...) as it happens.
pub fn ask(cfg: &WriterConfig, cx: &Context, text: &str, on_effect: &mut dyn FnMut(&Effects)) -> Result<String> {
    let lib = cx.lib;
    let tools: Vec<Value> = mcp::tool_list(cx)
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|t| json!({"type": "function", "function": {"name": t["name"], "description": t["description"].as_str().unwrap_or(t["title"].as_str().unwrap_or("")), "parameters": t["inputSchema"]}}))
        .collect();
    let user = json!({"role": "user", "content": text.trim()});
    lib.save_assistant(&user)?;
    let mut messages = vec![json!({"role": "system", "content": system_prompt(cx)})];
    // History from the database, starting at a user turn so tool replies are never cut from their calls.
    let history: Vec<Value> = lib.assistant_raw(HISTORY)?.into_iter().map(|(_, m, _)| m).collect();
    let first_user = history.iter().position(|m| m["role"] == "user").unwrap_or(history.len());
    messages.extend(history.into_iter().skip(first_user));

    for _ in 0..MAX_STEPS {
        let reply = chat_message(cfg, json!({"messages": messages, "tools": tools, "temperature": 0.4}))?;
        let calls = reply["tool_calls"].as_array().cloned().unwrap_or_default();
        let msg = json!({"role": "assistant", "content": reply["content"].as_str().unwrap_or(""), "tool_calls": if calls.is_empty() { Value::Null } else { json!(calls) }});
        let msg = if calls.is_empty() { json!({"role": "assistant", "content": msg["content"]}) } else { msg };
        lib.save_assistant(&msg)?;
        messages.push(msg);
        if calls.is_empty() {
            return Ok(strip_thinking(reply["content"].as_str().unwrap_or("")));
        }
        for (i, c) in calls.iter().enumerate() {
            let name = c["function"]["name"].as_str().unwrap_or("");
            let args: Value = match &c["function"]["arguments"] {
                Value::String(s) => serde_json::from_str(s).unwrap_or(json!({})),
                v @ Value::Object(_) => v.clone(),
                _ => json!({}),
            };
            let out = mcp::handle(cx, &json!({"jsonrpc": "2.0", "id": i, "method": "tools/call", "params": {"name": name, "arguments": args}}));
            on_effect(&out.effects);
            let content = out.reply.map(|r| r["result"]["content"][0]["text"].clone()).unwrap_or(Value::Null);
            let id = c["id"].as_str().map(String::from).unwrap_or_else(|| format!("call_{i}"));
            let tool_msg = json!({"role": "tool", "tool_call_id": id, "name": name, "content": content.as_str().unwrap_or("")});
            lib.save_assistant(&tool_msg)?;
            messages.push(tool_msg);
        }
    }
    bail!("That took too many steps. Try asking in smaller pieces.")
}

/// Voice hook settings: any server that speaks the Whisper transcription API.
pub struct SpeechConfig {
    /// Base URL ending in /v1, e.g. http://localhost:8000/v1 or https://api.openai.com/v1
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    /// Language hint such as "en"; empty lets the model detect it
    pub language: String,
}

/// Turns recorded speech into text through `<base_url>/audio/transcriptions`.
pub fn transcribe(cfg: &SpeechConfig, audio: Vec<u8>, file_name: &str, mime: &str) -> Result<String> {
    if cfg.base_url.trim().is_empty() {
        bail!("Set up voice in Settings first: the address of a Whisper server");
    }
    let url = format!("{}/audio/transcriptions", cfg.base_url.trim_end_matches('/'));
    let part = reqwest::blocking::multipart::Part::bytes(audio).file_name(file_name.to_string()).mime_str(mime)?;
    let mut form = reqwest::blocking::multipart::Form::new()
        .part("file", part)
        .text("model", if cfg.model.trim().is_empty() { "whisper-1".to_string() } else { cfg.model.trim().to_string() })
        .text("response_format", "json");
    if !cfg.language.trim().is_empty() {
        form = form.text("language", cfg.language.trim().to_string());
    }
    let mut req = reqwest::blocking::Client::builder().timeout(None).build()?.post(&url).multipart(form);
    if let Some(k) = cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let res = req.send().with_context(|| format!("Could not reach the voice server at {url}"))?;
    let status = res.status();
    let body = res.text()?;
    if !status.is_success() {
        bail!("The voice server answered {status}: {}", body.chars().take(300).collect::<String>());
    }
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::String(body.clone()));
    Ok(v["text"].as_str().or(v.as_str()).unwrap_or("").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_shows_words_and_actions_but_not_thinking() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(d.path()).unwrap();
        l.save_assistant(&json!({"role": "user", "content": "play something calm"})).unwrap();
        l.save_assistant(&json!({"role": "assistant", "content": "", "tool_calls": [{"id": "a", "function": {"name": "search_songs", "arguments": "{\"query\":\"calm\"}"}}]})).unwrap();
        l.save_assistant(&json!({"role": "tool", "tool_call_id": "a", "content": "{}"})).unwrap();
        l.save_assistant(&json!({"role": "assistant", "content": "<think>hmm</think>Playing it now."})).unwrap();
        let h = l.assistant_history(50).unwrap();
        let got: Vec<(&str, &str)> = h.iter().map(|c| (c.role.as_str(), c.text.as_str())).collect();
        assert_eq!(got, [("user", "play something calm"), ("action", "search songs (query: calm)"), ("assistant", "Playing it now.")]);
        l.clear_assistant().unwrap();
        assert!(l.assistant_history(50).unwrap().is_empty());
    }

    #[test]
    fn asks_the_ai_runs_its_tools_and_answers() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(d.path()).unwrap();
        let s = l.create_request("x", None, "Ari").unwrap();
        let draft = crate::Draft { title: "Rain Song".into(), lyrics: "[Verse]\nla".into(), notes: Some(json!([{"p": 60, "d": 4}])), ..Default::default() };
        l.apply_draft(s.id, &draft, "v", false).unwrap();

        // A fake AI: first asks to search, then (after seeing the tool result) plays the song and answers.
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let id = s.id;
        let t = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (i, mut req) in server.incoming_requests().take(3).enumerate() {
                let mut body = String::new();
                req.as_reader().read_to_string(&mut body).unwrap();
                seen.push(body);
                let reply = match i {
                    0 => json!({"role": "assistant", "content": "", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "search_songs", "arguments": "{\"query\":\"rain\"}"}}]}),
                    1 => json!({"role": "assistant", "content": null, "tool_calls": [{"id": "c2", "type": "function", "function": {"name": "play", "arguments": format!("{{\"song_ids\":[{id}]}}")}}]}),
                    _ => json!({"role": "assistant", "content": "<think>ok</think>Playing Rain Song."}),
                };
                req.respond(tiny_http::Response::from_string(json!({"choices": [{"message": reply}]}).to_string())).unwrap();
            }
            seen
        });
        let cfg = WriterConfig { base_url: format!("http://127.0.0.1:{port}/v1"), model: "m".into(), api_key: None, name: "Ari".into(), exclude: false };
        let cx = Context { lib: &l, writer_name: "Ari".into(), exclude: false, full: true, version: "0" };
        let mut ui = Vec::new();
        let answer = ask(&cfg, &cx, "play the rain one", &mut |fx| if let Some(u) = &fx.ui { ui.push(u.clone()) }).unwrap();
        assert_eq!(answer, "Playing Rain Song.");
        assert_eq!(ui, [json!({"action": "play", "songIds": [id], "shuffle": false})]);
        let seen = t.join().unwrap();
        assert!(seen[0].contains("\"tools\"") && seen[1].contains("Rain Song"), "the search result went back to the AI");
        let roles: Vec<String> = l.assistant_history(50).unwrap().into_iter().map(|c| c.role).collect();
        assert_eq!(roles, ["user", "action", "action", "assistant"]);
    }
}
