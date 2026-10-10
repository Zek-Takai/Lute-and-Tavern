//! Watches every music folder. New, changed and deleted files update the library on their own,
//! and the interface hears about it through the `library-changed` event.

use crate::AppState;
use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

static RESCAN_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Default)]
pub struct Watcher(Mutex<Option<Debouncer<RecommendedWatcher>>>);

impl Watcher {
    /// Rebuilds the watch list from the library's folders.
    pub fn refresh(&self, app: &tauri::AppHandle) {
        let handle = app.clone();
        let debouncer = new_debouncer(Duration::from_secs(2), move |res: DebounceEventResult| {
            let Ok(events) = res else { return };
            let state = handle.state::<AppState>();
            let lib = &state.lib;
            let mut changed = false;
            // The library folder is organized as a whole: new files get moved into song folders first.
            if let Ok(Some(lib_folder)) = lib.library_folder() {
                let root = Path::new(&lib_folder.path);
                let software = root.join(lt_core::organize::SOFTWARE);
                let inside: Vec<std::path::PathBuf> = events.iter().filter(|e| e.path.starts_with(root) && !e.path.starts_with(&software)).map(|e| e.path.clone()).collect();
                if !inside.is_empty() {
                    if let Ok(r) = lib.scan_folder(&lib_folder) {
                        changed |= r.added + r.updated + r.removed > 0;
                    }
                    // Files still arriving (a download, a sync app) are left until they settle; look again shortly.
                    let later = handle.clone();
                    let folder = lib_folder.clone();
                    // One follow-up at a time, however many changes arrive meanwhile.
                    if !RESCAN_PENDING.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(20));
                        RESCAN_PENDING.store(false, std::sync::atomic::Ordering::SeqCst);
                        let state = later.state::<AppState>();
                        if state.lib.scan_folder(&folder).is_ok_and(|r| r.added + r.updated + r.removed > 0) {
                            crate::emit_library_changed(&later);
                        }
                    });
                    }
                    crate::sync::changed(&handle, inside);
                }
            }
            for ev in events {
                let path = ev.path.as_path();
                if path.is_dir() || !lt_core::tags::is_audio(path) || lt_core::sync::conflict_of(path).is_some() {
                    continue;
                }
                if lib.folder_for(path).ok().flatten().is_some_and(|f| f.managed) {
                    continue;
                }
                if path.exists() {
                    if let Ok(Some(folder)) = lib.folder_for(path) {
                        changed |= matches!(
                            lib.import_file(path, Some(folder.id), Some(Path::new(&folder.path))),
                            Ok(lt_core::library::Imported::Added { .. } | lt_core::library::Imported::Updated)
                        );
                    }
                } else {
                    changed |= lib.forget_path(path).unwrap_or(false);
                }
            }
            if changed {
                crate::emit_library_changed(&handle);
            }
        });
        let Ok(mut debouncer) = debouncer else { return };
        let folders = app.state::<AppState>().lib.folders().unwrap_or_default();
        for f in folders {
            let _ = debouncer.watcher().watch(Path::new(&f.path), RecursiveMode::Recursive);
        }
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(debouncer);
    }
}
