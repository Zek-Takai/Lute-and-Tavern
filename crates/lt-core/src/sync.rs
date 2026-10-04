//! Working alongside a sync app the person already uses (Syncthing, Dropbox, iCloud Drive, OneDrive,
//! Nextcloud, Resilio, Synology Drive, rclone...). The app never installs, runs or updates one. It:
//!   - leaves sync apps' own files alone (hidden temp files and version folders are already skipped),
//!   - recognizes the conflict copies they make and lists them instead of importing duplicates,
//!   - can tell the sync app the library changed: Syncthing through its local API, or any tool through a
//!     command the person sets (e.g. an rclone or unison line, or their own script).

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Which sync app made a conflict copy, from its file name.
pub fn conflict_of(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?.to_lowercase();
    if name.contains(".sync-conflict-") {
        Some("Syncthing")
    } else if name.contains("conflicted copy") {
        Some("Dropbox or Nextcloud")
    } else if name.contains("(conflict") || name.contains("[conflict") {
        Some("your sync app")
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub path: String,
    /// The file it is a copy of, when it still exists
    pub original: Option<String>,
    pub by: String,
}

/// Conflict copies of audio files in the library folder.
pub fn conflicts(root: &Path) -> Vec<Conflict> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && crate::tags::is_audio(e.path()))
        .filter_map(|e| {
            let by = conflict_of(e.path())?;
            let original = original_of(e.path()).filter(|o| o.exists()).map(|o| o.to_string_lossy().into_owned());
            Some(Conflict { path: e.path().to_string_lossy().into_owned(), original, by: by.into() })
        })
        .collect()
}

/// "Song.sync-conflict-20261003-120000-ABCDEFG.mp3" → "Song.mp3"; "Song (Alex's conflicted copy 2026-10-03).mp3" → "Song.mp3".
fn original_of(p: &Path) -> Option<PathBuf> {
    let name = p.file_name()?.to_str()?;
    let (stem, ext) = name.rsplit_once('.')?;
    let base = if let Some(i) = stem.find(".sync-conflict-") {
        &stem[..i]
    } else if let Some(i) = stem.to_lowercase().find(" (") {
        &stem[..i]
    } else {
        return None;
    };
    Some(p.with_file_name(format!("{base}.{ext}")))
}

// ---------- Syncthing (local REST API) ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// "idle", "syncing", "scanning", "error", "paused" or "unknown"
    pub state: String,
    pub summary: String,
}

pub struct Syncthing<'a> {
    /// e.g. http://127.0.0.1:8384
    pub address: &'a str,
    pub api_key: &'a str,
}

impl Syncthing<'_> {
    fn client() -> Result<reqwest::blocking::Client> {
        Ok(reqwest::blocking::Client::builder().timeout(Duration::from_secs(10)).build()?)
    }

    fn get(&self, path: &str) -> Result<Value> {
        let res = Self::client()?
            .get(format!("{}{path}", self.address.trim_end_matches('/')))
            .header("X-API-Key", self.api_key)
            .send()
            .map_err(|_| anyhow!("Syncthing isn't answering at {}. Is it running?", self.address))?;
        if res.status().as_u16() == 403 {
            bail!("Syncthing refused the API key. Copy it from Syncthing's Settings › General.");
        }
        Ok(res.error_for_status()?.json()?)
    }

    /// The Syncthing folder that holds `root` (the library folder or one above it).
    pub fn folder_for(&self, root: &Path) -> Result<(String, String)> {
        let folders = self.get("/rest/config/folders")?;
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        folders
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| {
                let p = PathBuf::from(f["path"].as_str()?.replace('~', &std::env::var("HOME").unwrap_or_default()));
                let p = p.canonicalize().unwrap_or(p);
                root.starts_with(&p).then(|| (f["id"].as_str().unwrap_or("").to_string(), f["label"].as_str().unwrap_or("").to_string(), p.components().count()))
            })
            .max_by_key(|(_, _, depth)| *depth)
            .map(|(id, label, _)| (id, label))
            .ok_or_else(|| anyhow!("Syncthing isn't syncing this library folder yet. Add {} as a folder in Syncthing.", root.display()))
    }

    pub fn status(&self, root: &Path) -> Result<SyncStatus> {
        let (id, label) = self.folder_for(root)?;
        let s = self.get(&format!("/rest/db/status?folder={id}"))?;
        let state = s["state"].as_str().unwrap_or("unknown").to_string();
        let need = s["needFiles"].as_i64().unwrap_or(0);
        let conns = self.get("/rest/system/connections").ok();
        let online = conns.as_ref().and_then(|c| c["connections"].as_object()).map(|m| m.values().filter(|d| d["connected"] == true).count()).unwrap_or(0);
        let name = if label.is_empty() { id } else { label };
        let summary = match state.as_str() {
            "idle" if need == 0 => format!("\"{name}\" is up to date, {online} device{} connected", if online == 1 { "" } else { "s" }),
            "idle" => format!("\"{name}\" is waiting for {need} file{} from other devices", if need == 1 { "" } else { "s" }),
            "syncing" | "sync-preparing" => format!("\"{name}\" is syncing, {need} file{} to go", if need == 1 { "" } else { "s" }),
            "scanning" => format!("\"{name}\" is checking for changes"),
            other => format!("\"{name}\": {other}"),
        };
        let state = match state.as_str() {
            "sync-preparing" => "syncing".to_string(),
            s => s.to_string(),
        };
        Ok(SyncStatus { state, summary })
    }

    /// Asks Syncthing to look at the changed paths now instead of at its next scan.
    pub fn nudge(&self, root: &Path, changed: &[PathBuf]) -> Result<()> {
        let (id, _) = self.folder_for(root)?;
        let folders = self.get("/rest/config/folders")?;
        let base = folders
            .as_array()
            .into_iter()
            .flatten()
            .find(|f| f["id"] == id.as_str())
            .and_then(|f| f["path"].as_str())
            .map(|p| PathBuf::from(p.replace('~', &std::env::var("HOME").unwrap_or_default())))
            .ok_or_else(|| anyhow!("Syncthing folder {id} has no path"))?;
        let base = base.canonicalize().unwrap_or(base);
        let mut url = reqwest::Url::parse(&format!("{}/rest/db/scan", self.address.trim_end_matches('/')))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("folder", &id);
            for p in changed.iter().take(50) {
                let p = p.canonicalize().unwrap_or_else(|_| p.clone());
                if let Ok(rel) = p.strip_prefix(&base) {
                    q.append_pair("sub", &rel.to_string_lossy());
                }
            }
        }
        Self::client()?.post(url).header("X-API-Key", self.api_key).send()?.error_for_status()?;
        Ok(())
    }
}

// ---------- any tool: a command the person sets ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRun {
    pub ok: bool,
    pub code: Option<i32>,
    /// The last lines it printed
    pub output: String,
    pub at: i64,
}

/// Runs the person's sync command through their shell, with LT_LIBRARY (the library folder) and
/// LT_CHANGED (changed paths, one per line) set. Waits up to `limit` for it to finish.
pub fn run_command(command: &str, root: &Path, changed: &[PathBuf], limit: Duration) -> CommandRun {
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (shell, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("/bin/sh", "-c") };
    let child = std::process::Command::new(shell)
        .arg(flag)
        .arg(command)
        .env("LT_LIBRARY", root)
        .env("LT_CHANGED", changed.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return CommandRun { ok: false, code: None, output: format!("Could not start it: {e}"), at },
    };
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                return CommandRun { ok: false, code: None, output: format!("Still running after {} s, so it was stopped.", limit.as_secs()), at };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return CommandRun { ok: false, code: None, output: e.to_string(), at },
        }
    }
    match child.wait_with_output() {
        Ok(o) => {
            let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            let tail: Vec<&str> = text.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();
            CommandRun { ok: o.status.success(), code: o.status.code(), output: tail.join("\n"), at }
        }
        Err(e) => CommandRun { ok: false, code: None, output: e.to_string(), at },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_copies_are_recognized_with_their_originals() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("Songs/Porch Light");
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["Porch Light.mp3", "Porch Light.sync-conflict-20261003-120000-ABCDEFG.mp3", "Other (Alex's conflicted copy 2026-10-03).flac", "Fine (Live).mp3"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let mut c = conflicts(d.path());
        c.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!(c[0].by, "Dropbox or Nextcloud");
        assert!(c[0].original.is_none(), "its original is gone");
        assert_eq!(c[1].by, "Syncthing");
        assert!(c[1].original.as_deref().unwrap().ends_with("Porch Light/Porch Light.mp3"));
        assert!(conflict_of(Path::new("Fine (Live).mp3")).is_none());
    }

    #[test]
    fn commands_get_the_library_and_changes() {
        if cfg!(windows) {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let r = run_command("echo \"$LT_LIBRARY\"; echo \"$LT_CHANGED\"", d.path(), &[PathBuf::from("/a/b.mp3")], Duration::from_secs(10));
        assert!(r.ok && r.output.contains("/a/b.mp3") && r.output.contains(&*d.path().to_string_lossy()), "{r:?}");
        let r = run_command("exit 3", d.path(), &[], Duration::from_secs(10));
        assert_eq!((r.ok, r.code), (false, Some(3)));
        let r = run_command("sleep 5", d.path(), &[], Duration::from_millis(300));
        assert!(!r.ok && r.output.contains("stopped"));
    }

    #[test]
    fn syncthing_finds_the_folder_reports_status_and_scans_changes() {
        let d = tempfile::tempdir().unwrap();
        let synced = d.path().join("Music");
        let lib = synced.join("Lute & Tavern");
        std::fs::create_dir_all(lib.join("Songs/X")).unwrap();
        let (synced_s, other) = (synced.to_string_lossy().to_string(), d.path().join("Other").to_string_lossy().to_string());
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let scans = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = scans.clone();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                if !req.headers().iter().any(|h| h.field.equiv("X-API-Key") && h.value == "k") {
                    let _ = req.respond(tiny_http::Response::from_string("").with_status_code(403));
                    continue;
                }
                let url = req.url().to_string();
                let body = if url.starts_with("/rest/config/folders") {
                    serde_json::json!([{"id": "abc", "label": "Music", "path": synced_s}, {"id": "zzz", "label": "Other", "path": other}])
                } else if url.starts_with("/rest/db/status") {
                    serde_json::json!({"state": "idle", "needFiles": 0})
                } else if url.starts_with("/rest/system/connections") {
                    serde_json::json!({"connections": {"NAS": {"connected": true}, "Phone": {"connected": false}}})
                } else {
                    seen.lock().unwrap().push(url);
                    serde_json::json!({})
                };
                let _ = req.respond(tiny_http::Response::from_string(body.to_string()));
            }
        });
        let addr = format!("http://127.0.0.1:{port}");
        let st = Syncthing { address: &addr, api_key: "k" };
        assert_eq!(st.folder_for(&lib).unwrap().0, "abc", "the library sits inside the synced Music folder");
        let s = st.status(&lib).unwrap();
        assert_eq!((s.state.as_str(), s.summary.as_str()), ("idle", "\"Music\" is up to date, 1 device connected"));
        st.nudge(&lib, &[lib.join("Songs/X/X.mp3")]).unwrap();
        let scans = scans.lock().unwrap();
        assert!(scans[0].starts_with("/rest/db/scan?folder=abc&sub=Lute+%26+Tavern%2FSongs%2FX%2FX.mp3"), "{scans:?}");
        let bad = Syncthing { address: &addr, api_key: "wrong" };
        assert!(bad.status(&lib).unwrap_err().to_string().contains("API key"));
    }
}
