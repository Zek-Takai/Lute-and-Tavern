//! The songwriter: any OpenAI-compatible chat endpoint (a local model server or a hosted API)
//! writes songs from a request and rewrites them from feedback.

use crate::library::{Draft, Message, Song};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterConfig {
    /// Base URL ending in /v1, e.g. http://localhost:8000/v1
    pub base_url: String,
    pub model: String,
    #[serde(skip_serializing)]
    pub api_key: Option<String>,
    /// The songwriter's name as shown in the app
    pub name: String,
    /// Also write Suno's "Exclude styles" box (off unless the person turns it on)
    #[serde(default)]
    pub exclude: bool,
}

impl WriterConfig {
    pub fn is_ready(&self) -> bool {
        !self.base_url.trim().is_empty() && !self.model.trim().is_empty()
    }
}

/// What comes back from the songwriter: the song fields plus a short note to the listener.
pub struct Written {
    pub draft: Draft,
    pub reply: String,
}

const FORMAT: &str = r#"Answer with one JSON object and nothing else:
{
  "reply": "one or two friendly sentences to the listener about what you wrote or changed",
  "title": "song title",
  "genre": "ONE genre tag. Use a tag from the list you are given when one fits; otherwise invent a short new tag",
  "sub_genre": "a more specific style, e.g. 'Dream pop'",
  "key": "musical key such as C, G, Am, F#m",
  "bpm": 96,
  "style": "Suno 'Style of Music' prompt: comma-separated genre, instruments, vocal type, mood; under 200 characters",
  "lyrics": "full lyrics with Suno section tags on their own lines: [Intro], [Verse 1], [Pre-Chorus], [Chorus], [Bridge], [Outro]",
  "chords": ["four to eight chord symbols for the chorus, one per 4/4 measure"],
  "hook": [{"note": "G4", "len": "q", "lyric": "first syllable"}]
}
"hook" is the chorus melody for 2 to 4 measures of 4/4: note names C4 to A5, len is w (4 beats), h (2), q (1) or e (half a beat); each measure adds up to 4 beats; one syllable per note, a trailing hyphen marks a split word."#;

/// Guidance for Suno's "Exclude styles" box, shared by the JSON format and the MCP guide.
pub const EXCLUDE_RULE: &str = "Suno 'Exclude styles' box: at most 4 short tags this style could drift into by mistake \
(e.g. for acoustic folk: 'drums, electric guitar, autotune'); not unrelated genres, not moods or tempos the style already sets; \
empty string when nothing needs excluding";

fn format_spec(exclude: bool) -> String {
    if !exclude {
        return FORMAT.to_string();
    }
    let line = format!("  \"exclude\": \"{EXCLUDE_RULE}\",\n  \"lyrics\"");
    FORMAT.replacen("  \"lyrics\"", &line, 1)
}

fn system_prompt(cfg: &WriterConfig, tags: &[String]) -> String {
    format!(
        "You are {}, a songwriter. You write original songs people can generate in Suno and sing. \
         Lyrics are singable, rhyme naturally, and scan evenly within each section. Never copy existing songs.\n\
         Genre tags already in the library: {}.\n\n{}",
        cfg.name,
        if tags.is_empty() {
            "none yet".into()
        } else {
            tags.join(", ")
        },
        format_spec(cfg.exclude)
    )
}

/// How to write a song for Suno, for any AI that writes through the app's MCP tools.
pub fn suno_guide(exclude: bool) -> String {
    let mut g = String::from(
        "# Writing songs for Suno in Lute & Tavern\n\n\
Suno's Custom mode takes a song as separate boxes. The person copies each one from the app into Suno. \
Write original songs only: never copy or closely imitate existing lyrics, and never name real artists in the style.\n\n\
## The boxes\n\
- **Song title**: short and memorable.\n\
- **Style of music**: comma-separated tags, most important first: genre and sub-genre, main instruments, vocal type \
(e.g. 'warm female alto', 'gritty baritone male vocal'), mood and energy, production feel. Keep it under 200 characters \
so it fits every Suno model. Describe the sound; do not write sentences or name artists.\n\
- **Lyrics**: the full song with section tags on their own lines in square brackets: [Intro], [Verse 1], [Pre-Chorus], \
[Chorus], [Verse 2], [Bridge], [Outro]. Suno sings everything outside brackets, so put no notes, chords or stage \
directions there. Short cues inside brackets are fine, e.g. [Instrumental Break] or [Fade Out].\n",
    );
    if exclude {
        g.push_str(&format!(
            "- **Exclude styles** (under More Options in Suno): {EXCLUDE_RULE}.\n"
        ));
    } else {
        g.push_str(
            "- Leave **exclude** empty: this library does not use Suno's Exclude styles box.\n",
        );
    }
    g.push_str(
        "\n## Craft\n\
- Lines in a section scan evenly (similar syllable counts) so Suno keeps a steady rhythm.\n\
- The chorus carries the title or hook and repeats word for word each time.\n\
- A typical song is 2 to 3 verses, a chorus after each, and an optional bridge, about 2 to 4 minutes sung.\n\
- Rhymes should feel natural; slant rhymes are fine.\n\n\
## Saving it in the library\n\
Call `create_song` with the title, style, lyrics and one genre tag (reuse one from `list_genre_tags` when it fits). \
Optional fields: sub_genre, key (e.g. 'Am'), bpm, chords (4 to 8 chord symbols for the chorus, one per 4/4 measure), \
hook (the chorus melody as [{\"note\": \"G4\", \"len\": \"q\", \"lyric\": \"first\"}], len w/h/q/e, each measure 4 beats). \
To change a song later, call `revise_song` with its id and only the fields that change; each revision is kept as a version.\n",
    );
    g
}

/// Leaves the exclude box empty when the person has it turned off.
fn apply_exclude_setting(cfg: &WriterConfig, mut w: Written) -> Written {
    if !cfg.exclude {
        w.draft.exclude.clear();
    }
    w
}

pub fn write_song(
    cfg: &WriterConfig,
    request: &str,
    genre_hint: Option<&str>,
    tags: &[String],
) -> Result<Written> {
    let mut user = format!("Write a song for this request:\n{request}");
    if let Some(g) = genre_hint.filter(|g| !g.trim().is_empty() && *g != crate::genre::UNSORTED) {
        user.push_str(&format!("\nUse the genre tag \"{g}\"."));
    }
    let messages = vec![
        json!({"role": "system", "content": system_prompt(cfg, tags)}),
        json!({"role": "user", "content": user}),
    ];
    Ok(apply_exclude_setting(
        cfg,
        parse_written(&chat(cfg, messages)?)?,
    ))
}

pub fn rewrite(
    cfg: &WriterConfig,
    song: &Song,
    history: &[Message],
    instruction: &str,
    tags: &[String],
) -> Result<Written> {
    let current = json!({
        "title": song.title, "genre": song.genre, "sub_genre": song.sub_genre, "key": song.key, "bpm": song.bpm,
        "style": song.style, "exclude": if cfg.exclude { song.exclude.as_str() } else { "" }, "lyrics": song.lyrics, "chords": song.chords,
    });
    let mut messages = vec![json!({"role": "system", "content": system_prompt(cfg, tags)})];
    messages.push(json!({"role": "user", "content": format!("The original request was:\n{}", song.request.clone().unwrap_or_default())}));
    // Earlier feedback gives the model the thread's direction without resending every past draft.
    for m in history
        .iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let role = if m.role == "user" {
            "user"
        } else {
            "assistant"
        };
        messages.push(json!({"role": role, "content": m.text}));
    }
    messages.push(json!({
        "role": "user",
        "content": format!("Here is the current song:\n{}\n\nChange it like this: {instruction}\nReturn the whole updated song. Keep what the listener did not ask to change.", current)
    }));
    Ok(apply_exclude_setting(
        cfg,
        parse_written(&chat(cfg, messages)?)?,
    ))
}

/// Lists the models the endpoint serves; used by Settings to check the connection.
pub fn list_models(cfg: &WriterConfig) -> Result<Vec<String>> {
    let url = format!("{}/models", cfg.base_url.trim_end_matches('/'));
    let mut req = client()?.get(&url);
    if let Some(k) = cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let res = req
        .send()
        .with_context(|| format!("Could not reach {url}"))?;
    if !res.status().is_success() {
        bail!("{url} answered {}", res.status());
    }
    let v: Value = res.json()?;
    Ok(v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

fn client() -> Result<reqwest::blocking::Client> {
    // No request timeout: local models can take minutes to write a full song.
    Ok(reqwest::blocking::Client::builder().timeout(None).build()?)
}

fn chat(cfg: &WriterConfig, messages: Vec<Value>) -> Result<String> {
    let msg = chat_message(cfg, json!({"messages": messages, "temperature": 0.8}))?;
    msg["content"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow!("The songwriter's answer was empty"))
}

/// One chat completion; `body` holds messages and any extras (tools, temperature). Returns the reply message.
pub fn chat_message(cfg: &WriterConfig, mut body: Value) -> Result<Value> {
    if !cfg.is_ready() {
        bail!("Set up the songwriter in Settings first: an address and a model");
    }
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    body["model"] = json!(cfg.model);
    let mut req = client()?.post(&url).json(&body);
    if let Some(k) = cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let res = req
        .send()
        .with_context(|| format!("Could not reach the AI at {url}"))?;
    let status = res.status();
    let body: Value = res
        .json()
        .context("The AI sent back something that is not JSON")?;
    if !status.is_success() {
        let msg = body["error"]["message"]
            .as_str()
            .or(body["message"].as_str())
            .unwrap_or("no details");
        bail!("The AI's server answered {status}: {msg}");
    }
    Ok(body["choices"][0]["message"].clone())
}

fn parse_written(text: &str) -> Result<Written> {
    let v = extract_json(text).ok_or_else(|| anyhow!("The songwriter's answer was not in the expected format. Try again, or try another model."))?;
    let s = |k: &str| v[k].as_str().unwrap_or("").trim().to_string();
    let chords: Vec<String> = v["chords"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str().map(|c| c.trim().to_string()))
                .collect()
        })
        .unwrap_or_default();
    let draft = Draft {
        title: s("title"),
        genre: s("genre"),
        sub_genre: s("sub_genre"),
        key: s("key"),
        bpm: v["bpm"]
            .as_i64()
            .or_else(|| v["bpm"].as_str().and_then(|b| b.parse().ok()))
            .unwrap_or(0)
            .clamp(0, 300),
        style: s("style"),
        exclude: s("exclude"),
        lyrics: s("lyrics"),
        notes: hook_to_notes(&v["hook"]),
        chords: (!chords.is_empty()).then_some(chords),
    };
    if draft.lyrics.is_empty() {
        bail!("The songwriter did not send any lyrics. Try again, or try another model.");
    }
    Ok(Written {
        reply: s("reply"),
        draft,
    })
}

/// Finds the JSON object in a model's answer, skipping any thinking or code fences around it.
fn extract_json(text: &str) -> Option<Value> {
    let text = match text.rfind("</think>") {
        Some(i) => &text[i + 8..],
        None => text,
    };
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end]).ok()
}

/// Converts [{"note":"G4","len":"q","lyric":"la"}] into the app's sheet notes, where step 0 is E4.
pub fn hook_to_notes(hook: &Value) -> Option<Value> {
    let notes: Vec<Value> = hook
        .as_array()?
        .iter()
        .filter_map(|n| {
            let name = n["note"].as_str()?.trim();
            let mut chars = name.chars();
            let letter = chars.next()?.to_ascii_uppercase();
            let idx = "CDEFGAB".find(letter)? as i64;
            let octave: i64 = name
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '-')
                .collect::<String>()
                .parse()
                .ok()?;
            let step = (octave * 7 + idx - 30).clamp(-8, 16);
            let len = match n["len"].as_str().unwrap_or("q") {
                l @ ("w" | "h" | "q" | "e") => l,
                _ => "q",
            };
            Some(json!({"s": step, "d": len, "w": n["lyric"].as_str().unwrap_or("")}))
        })
        .collect();
    (!notes.is_empty()).then_some(Value::Array(notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_answer_with_thinking_and_fences() {
        let text = "<think>plan the song</think>\n```json\n{\"reply\":\"Here it is.\",\"title\":\"Porch Light\",\"genre\":\"Country\",\"bpm\":\"84\",\
                    \"lyrics\":\"[Verse 1]\\nHello\",\"chords\":[\"G\",\"C\"],\"hook\":[{\"note\":\"E4\",\"len\":\"q\",\"lyric\":\"Hel-\"},{\"note\":\"G4\",\"len\":\"h\",\"lyric\":\"lo\"}]}\n```";
        let w = parse_written(text).unwrap();
        assert_eq!(w.draft.title, "Porch Light");
        assert_eq!(w.draft.bpm, 84);
        assert_eq!(w.reply, "Here it is.");
        let notes = w.draft.notes.unwrap();
        assert_eq!(notes[0]["s"], 0);
        assert_eq!(notes[1]["s"], 2);
        assert_eq!(notes[1]["d"], "h");
    }

    #[test]
    fn refuses_answers_without_lyrics() {
        assert!(parse_written("{\"title\":\"x\"}").is_err());
        assert!(parse_written("no json here").is_err());
    }
}
