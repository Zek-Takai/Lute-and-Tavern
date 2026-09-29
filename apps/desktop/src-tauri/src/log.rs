//! A small log file in the app's data folder (`logs/app.log`) so problems on someone's machine can be
//! read back afterwards. The interface reports errors here too. Kept under 2 MB.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static FILE: OnceLock<Mutex<PathBuf>> = OnceLock::new();
const MAX_BYTES: u64 = 2 * 1024 * 1024;

pub fn init(data_dir: &std::path::Path) {
    let dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let _ = FILE.set(Mutex::new(dir.join("app.log")));
    write("info", &format!("Lute & Tavern {} started", env!("CARGO_PKG_VERSION")));
}

pub fn write(level: &str, msg: &str) {
    let Some(lock) = FILE.get() else { return };
    let path = lock.lock().unwrap_or_else(|e| e.into_inner());
    if std::fs::metadata(&*path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&*path, path.with_extension("old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&*path) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "{t} [{level}] {msg}");
    }
}

/// Lets the interface add lines (errors from playback, unexpected exceptions).
#[tauri::command]
pub fn log_event(level: String, message: String) {
    write(&level, &format!("ui: {message}"));
}
