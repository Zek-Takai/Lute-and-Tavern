//! Scheduled backup of the library folder to a WebDAV connection (lt_core::backup does the work). The
//! schedule is the person's: manual, daily or weekly at a time they choose. A run that was due while the app
//! was closed happens shortly after the next launch.

use crate::AppState;
use chrono::{Datelike, Duration as CDuration, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};

static RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    /// "manual", "daily" or "weekly"
    pub mode: String,
    /// 0 = Sunday … 6 = Saturday (weekly)
    pub day: u32,
    /// "HH:MM", local time
    pub time: String,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule { mode: "weekly".into(), day: 0, time: "16:00".into() }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSettings {
    conn: Option<i64>,
    folder: String,
    schedule: Schedule,
    last: Option<i64>,
    last_report: Value,
    next: Option<i64>,
    running: bool,
}

fn schedule(state: &AppState) -> Schedule {
    state.lib.settings().ok().and_then(|m| m.get("backupSchedule").cloned()).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

fn setting_i64(state: &AppState, k: &str) -> Option<i64> {
    state.lib.settings().ok().and_then(|m| m.get(k).and_then(Value::as_i64))
}

/// The latest scheduled moment at or before now (local time), if the schedule has one.
fn last_slot(s: &Schedule) -> Option<i64> {
    let t = NaiveTime::parse_from_str(&s.time, "%H:%M").ok()?;
    let now = Local::now();
    let today = Local.from_local_datetime(&now.date_naive().and_time(t)).earliest()?;
    let slot = match s.mode.as_str() {
        "daily" => if today <= now { today } else { today - CDuration::days(1) },
        "weekly" => {
            let back = (7 + now.weekday().num_days_from_sunday() as i64 - s.day as i64) % 7;
            let cand = today - CDuration::days(back);
            if cand <= now { cand } else { cand - CDuration::days(7) }
        }
        _ => return None,
    };
    Some(slot.timestamp())
}

fn next_slot(s: &Schedule) -> Option<i64> {
    let last = last_slot(s)?;
    Some(last + if s.mode == "daily" { 86400 } else { 7 * 86400 })
}

fn info(state: &AppState) -> BackupSettings {
    let s = schedule(state);
    BackupSettings {
        conn: setting_i64(state, "backupConn"),
        folder: state.lib.setting_str("backupFolder").unwrap_or_else(|| "Music/Lute & Tavern".into()),
        next: next_slot(&s),
        schedule: s,
        last: setting_i64(state, "backupLast"),
        last_report: state.lib.settings().ok().and_then(|m| m.get("backupLastReport").cloned()).unwrap_or(Value::Null),
        running: RUNNING.load(Ordering::SeqCst),
    }
}

#[tauri::command]
pub fn backup_settings(state: State<'_, AppState>) -> BackupSettings {
    info(&state)
}

#[tauri::command]
pub fn set_backup_settings(state: State<'_, AppState>, conn: Option<i64>, folder: String, schedule: Schedule) -> Result<BackupSettings, String> {
    if let Some(id) = conn {
        let c = state.lib.connection(id).map_err(|e| e.to_string())?;
        if c.kind != "webdav" {
            return Err("Backups go to a WebDAV connection (for Nextcloud: its Files address).".into());
        }
    }
    let l = &state.lib;
    l.set_setting("backupConn", &conn.map(Value::from).unwrap_or(Value::Null)).map_err(|e| e.to_string())?;
    l.set_setting("backupFolder", &Value::String(folder.trim().trim_matches('/').to_string())).map_err(|e| e.to_string())?;
    l.set_setting("backupSchedule", &serde_json::to_value(&schedule).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(info(&state))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupEvent {
    state: String,
    message: String,
}

/// Starts a backup now (once at a time).
pub fn start(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn_id = setting_i64(&state, "backupConn").ok_or("Choose where to back up first.")?;
    let root = state.lib.library_folder().ok().flatten().map(|f| PathBuf::from(f.path)).ok_or("Choose a library folder first.")?;
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("A backup is already running.".into());
    }
    let folder = state.lib.setting_str("backupFolder").unwrap_or_else(|| "Music/Lute & Tavern".into());
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let send = |st: &str, m: String| {
            let _ = app.emit("backup", BackupEvent { state: st.into(), message: m });
        };
        let result = (|| -> anyhow::Result<lt_core::backup::BackupReport> {
            let conn = state.lib.connection(conn_id)?;
            let secret = keyring::Entry::new("Lute & Tavern", &format!("connection-{conn_id}"))?.get_password()?;
            let dav = lt_core::remote_webdav::WebDav::new(&conn, &secret);
            lt_core::backup::run(&state.lib, &dav, conn_id, &folder, &root, &|m| send("running", m))
        })();
        RUNNING.store(false, Ordering::SeqCst);
        let now = chrono::Utc::now().timestamp();
        match result {
            Ok(r) => {
                let msg = format!(
                    "Backed up: {} uploaded, {} moved, {} unchanged{}{}",
                    r.uploaded,
                    r.moved,
                    r.unchanged + r.already_there,
                    if r.bytes > 0 { format!(" ({} MB sent)", r.bytes / 1_048_576) } else { String::new() },
                    if r.failed.is_empty() { String::new() } else { format!(", {} failed", r.failed.len()) }
                );
                let _ = state.lib.set_setting("backupLast", &Value::from(now));
                let _ = state.lib.set_setting("backupLastReport", &json!({"ok": r.failed.is_empty(), "message": msg, "failed": r.failed}));
                send(if r.failed.is_empty() { "done" } else { "partial" }, msg);
                let _ = crate::playlists::sync_now(&app);
            }
            Err(e) => {
                let msg = format!("Backup failed: {e:#}");
                crate::log::write("error", &msg);
                let _ = state.lib.set_setting("backupLastReport", &json!({"ok": false, "message": msg}));
                send("error", msg);
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub fn backup_now(app: AppHandle) -> Result<(), String> {
    start(&app)
}

/// Checks once a minute whether a scheduled backup is due (including one missed while the app was closed).
pub fn start_scheduler(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(90)); // let the app settle after launch
        loop {
            let state = app.state::<AppState>();
            let s = schedule(&state);
            if s.mode != "manual" && setting_i64(&state, "backupConn").is_some() {
                if let Some(slot) = last_slot(&s) {
                    let last = setting_i64(&state, "backupLast").unwrap_or(0);
                    if last < slot && !RUNNING.load(Ordering::SeqCst) {
                        let _ = start(&app);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });
}
