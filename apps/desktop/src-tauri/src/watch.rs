//! Watches every music folder. New, changed and deleted files update the library on their own,
//! and the interface hears about it through the `library-changed` event.

use crate::AppState;
use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

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
            for ev in events {
                let path = ev.path.as_path();
                if path.is_dir() || !lt_core::tags::is_audio(path) {
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
