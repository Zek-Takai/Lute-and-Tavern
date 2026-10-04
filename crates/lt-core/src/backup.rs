//! Backup of the library folder to a WebDAV location (Nextcloud Files, a NAS share, a WebDAV cloud drive), in
//! the same layout, so songs land "in their place" on the server. One way and additive: new and changed files
//! are uploaded, songs that moved or were renamed locally are moved on the server instead of re-sent, and
//! nothing is ever deleted there. A manifest of what was sent keeps later runs short.

use crate::library::{Library, now};
use crate::remote_webdav::WebDav;
use anyhow::Result;
use rusqlite::params;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupReport {
    pub uploaded: usize,
    pub moved: usize,
    pub unchanged: usize,
    /// Already on the server with the same size (first runs, or files put there another way)
    pub already_there: usize,
    pub bytes: u64,
    pub failed: Vec<String>,
}

struct Local {
    size: u64,
    mtime: i64,
}

impl Library {
    fn backup_manifest(&self, conn: i64) -> Result<HashMap<String, (u64, i64)>> {
        let db = self.db();
        let mut st = db.prepare("SELECT rel, size, mtime FROM backup_files WHERE conn_id = ?1")?;
        let rows = st.query_map([conn], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, i64>(1)? as u64, r.get::<_, i64>(2)?))))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    fn backup_record(&self, conn: i64, rel: &str, size: u64, mtime: i64) -> Result<()> {
        self.db().execute(
            "INSERT INTO backup_files(conn_id, rel, size, mtime, uploaded_at) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(conn_id, rel) DO UPDATE SET size = excluded.size, mtime = excluded.mtime, uploaded_at = excluded.uploaded_at",
            params![conn, rel, size as i64, mtime, now()],
        )?;
        Ok(())
    }

    fn backup_forget(&self, conn: i64, rel: &str) -> Result<()> {
        self.db().execute("DELETE FROM backup_files WHERE conn_id = ?1 AND rel = ?2", params![conn, rel])?;
        Ok(())
    }
}

/// Files in the library folder worth backing up (not hidden, not a sync app's conflict copy).
fn local_files(root: &Path) -> HashMap<String, Local> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && crate::sync::conflict_of(e.path()).is_none())
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            let rel = e.path().strip_prefix(root).ok()?.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            let mtime = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
            Some((rel, Local { size: m.len(), mtime }))
        })
        .collect()
}

/// Runs a backup of `root` into `folder` (path pieces under the WebDAV connection's address).
pub fn run(lib: &Library, dav: &WebDav, conn: i64, folder: &str, root: &Path, progress: &dyn Fn(String)) -> Result<BackupReport> {
    let mut r = BackupReport::default();
    let base: Vec<String> = folder.split('/').map(str::trim).filter(|p| !p.is_empty()).map(String::from).collect();
    let base_refs: Vec<&str> = base.iter().map(String::as_str).collect();
    dav.make_folders(&base_refs)?;
    let locals = local_files(root);
    let manifest = lib.backup_manifest(conn)?;
    let url = |rel: &str| {
        let mut p: Vec<&str> = base_refs.clone();
        p.extend(rel.split('/'));
        dav.url_for(&p, false)
    };
    // Files sent before that are no longer where they were: candidates for a move.
    let mut gone: HashMap<(u64, i64), Vec<String>> = HashMap::new();
    let mut gone_by_name: HashMap<(u64, String), Vec<String>> = HashMap::new();
    for (rel, (size, mtime)) in &manifest {
        if !locals.contains_key(rel) {
            gone.entry((*size, *mtime)).or_default().push(rel.clone());
            let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
            gone_by_name.entry((*size, name)).or_default().push(rel.clone());
        }
    }
    let mut made: HashSet<String> = HashSet::new();
    let mut ensure_parent = |rel: &str| -> Result<()> {
        if let Some((dir, _)) = rel.rsplit_once('/') {
            if made.insert(dir.to_string()) {
                let mut p: Vec<&str> = base_refs.clone();
                p.extend(dir.split('/'));
                dav.make_folders(&p)?;
            }
        }
        Ok(())
    };
    let mut todo: Vec<(&String, &Local)> = locals.iter().collect();
    todo.sort_by(|a, b| a.0.cmp(b.0));
    let total = todo.len();
    for (i, (rel, l)) in todo.into_iter().enumerate() {
        if i % 20 == 0 {
            progress(format!("Checking {}/{total}…", i));
        }
        if manifest.get(rel.as_str()) == Some(&(l.size, l.mtime)) {
            r.unchanged += 1;
            continue;
        }
        let res: Result<()> = (|| {
            // moved or renamed since the last backup? move it on the server instead of sending it again
            if !manifest.contains_key(rel.as_str()) {
                let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
                let old = gone.get_mut(&(l.size, l.mtime)).and_then(|v| v.pop()).or_else(|| gone_by_name.get_mut(&(l.size, name)).and_then(|v| v.pop()));
                if let Some(old) = old {
                    if dav.stat(&url(&old))?.is_some() && dav.stat(&url(rel))?.is_none() {
                        ensure_parent(rel)?;
                        dav.move_to(&url(&old), &url(rel))?;
                        lib.backup_forget(conn, &old)?;
                        lib.backup_record(conn, rel, l.size, l.mtime)?;
                        r.moved += 1;
                        return Ok(());
                    }
                }
                if dav.stat(&url(rel))? == Some(Some(l.size)) {
                    lib.backup_record(conn, rel, l.size, l.mtime)?;
                    r.already_there += 1;
                    return Ok(());
                }
            }
            ensure_parent(rel)?;
            progress(format!("Uploading {rel}…"));
            dav.upload(&url(rel), &root.join(rel))?;
            lib.backup_record(conn, rel, l.size, l.mtime)?;
            r.uploaded += 1;
            r.bytes += l.size;
            Ok(())
        })();
        if let Err(e) = res {
            r.failed.push(format!("{rel}: {e:#}"));
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::Connection;
    use std::sync::{Arc, Mutex};

    /// A tiny in-memory WebDAV server: PROPFIND depth 0, MKCOL, PUT, MOVE.
    fn fake_dav() -> (String, Arc<Mutex<HashMap<String, Option<Vec<u8>>>>>) {
        let store: Arc<Mutex<HashMap<String, Option<Vec<u8>>>>> = Arc::new(Mutex::new(HashMap::new()));
        store.lock().unwrap().insert("/dav/".into(), None);
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let st = store.clone();
        std::thread::spawn(move || {
            for mut req in server.incoming_requests() {
                let path = req.url().split('?').next().unwrap().to_string();
                let key = path.trim_end_matches('/').to_string();
                let method = req.method().as_str().to_string();
                let mut s = st.lock().unwrap();
                let find = |s: &HashMap<String, Option<Vec<u8>>>| s.iter().find(|(k, _)| k.trim_end_matches('/') == key).map(|(k, v)| (k.clone(), v.clone()));
                let res = match method.as_str() {
                    "PROPFIND" => match find(&s) {
                        Some((k, v)) => {
                            let k = k.replace('&', "&amp;");
                            let body = match v {
                                None => format!("<d:multistatus xmlns:d=\"DAV:\"><d:response><d:href>{k}</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response></d:multistatus>"),
                                Some(b) => format!("<d:multistatus xmlns:d=\"DAV:\"><d:response><d:href>{k}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>{}</d:getcontentlength></d:prop></d:propstat></d:response></d:multistatus>", b.len()),
                            };
                            tiny_http::Response::from_string(body).with_status_code(207)
                        }
                        None => tiny_http::Response::from_string("").with_status_code(404),
                    },
                    "MKCOL" => {
                        s.insert(format!("{key}/"), None);
                        tiny_http::Response::from_string("").with_status_code(201)
                    }
                    "PUT" => {
                        let mut b = Vec::new();
                        std::io::Read::read_to_end(req.as_reader(), &mut b).unwrap();
                        s.insert(key.clone(), Some(b));
                        tiny_http::Response::from_string("").with_status_code(201)
                    }
                    "MOVE" => {
                        let dest = req.headers().iter().find(|h| h.field.equiv("Destination")).unwrap().value.to_string();
                        let dest = reqwest::Url::parse(&dest).unwrap().path().to_string();
                        let v = find(&s).map(|(k, v)| (k, v));
                        if let Some((k, v)) = v {
                            s.remove(&k);
                            s.insert(dest, v);
                        }
                        tiny_http::Response::from_string("").with_status_code(201)
                    }
                    _ => tiny_http::Response::from_string("").with_status_code(405),
                };
                drop(s);
                let _ = req.respond(res);
            }
        });
        (format!("http://127.0.0.1:{port}/dav"), store)
    }

    #[test]
    fn uploads_new_files_moves_renamed_ones_and_skips_unchanged() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(&d.path().join("app")).unwrap();
        let root = d.path().join("Library");
        std::fs::create_dir_all(root.join("Songs/Porch Light")).unwrap();
        std::fs::write(root.join("Songs/Porch Light/Porch Light.mp3"), b"audio").unwrap();
        std::fs::write(root.join("Songs/Porch Light/.hidden"), b"x").unwrap();
        let (url, store) = fake_dav();
        let c = l.add_connection("webdav", "NC", &url, "listener", &serde_json::json!({})).unwrap();
        let dav = WebDav::new(&c, "pw");
        let r = run(&l, &dav, c.id, "Music/Lute & Tavern", &root, &|_| {}).unwrap();
        assert_eq!((r.uploaded, r.failed.len()), (1, 0), "{:?}", r.failed);
        assert!(store.lock().unwrap().contains_key("/dav/Music/Lute%20&%20Tavern/Songs/Porch%20Light/Porch%20Light.mp3"), "{:?}", store.lock().unwrap().keys().collect::<Vec<_>>());
        let r = run(&l, &dav, c.id, "Music/Lute & Tavern", &root, &|_| {}).unwrap();
        assert_eq!((r.uploaded, r.unchanged), (0, 1), "second run sends nothing");
        // the song is renamed locally (same file, new folder): moved on the server, not re-sent
        std::fs::create_dir_all(root.join("Songs/Porch Lights")).unwrap();
        std::fs::rename(root.join("Songs/Porch Light/Porch Light.mp3"), root.join("Songs/Porch Lights/Porch Light.mp3")).unwrap();
        let r = run(&l, &dav, c.id, "Music/Lute & Tavern", &root, &|_| {}).unwrap();
        assert_eq!((r.moved, r.uploaded), (1, 0), "{:?}", r.failed);
        let keys: Vec<String> = store.lock().unwrap().keys().cloned().collect();
        assert!(keys.iter().any(|k| k.ends_with("Porch%20Lights/Porch%20Light.mp3")) && !keys.iter().any(|k| k.ends_with("Porch%20Light/Porch%20Light.mp3")), "{keys:?}");
        let _: &Connection = &c;
    }
}
