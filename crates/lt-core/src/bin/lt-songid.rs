//! Prints the permanent song ID (see songid.rs) of every audio file in the given files and folders:
//! one line per file, "<id>\t<size>\t<path>". Used for processing a whole music library at once.
use std::path::Path;

fn main() {
    let exts = ["mp3", "m4a", "mp4", "aac", "flac", "ogg", "opus", "wav", "aif", "aiff", "wma", "alac"];
    for arg in std::env::args().skip(1) {
        for e in walkdir::WalkDir::new(&arg).follow_links(true).into_iter().filter_map(|e| e.ok()) {
            let p = e.path();
            let audio = p.extension().and_then(|x| x.to_str()).is_some_and(|x| exts.contains(&x.to_ascii_lowercase().as_str()));
            let hidden = p.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.') && c.as_os_str() != "." && c.as_os_str() != "..");
            if !e.file_type().is_file() || !audio || hidden {
                continue;
            }
            match lt_core::songid::for_file(Path::new(p)) {
                Ok(id) => println!("{id}\t{}\t{}", e.metadata().map(|m| m.len()).unwrap_or(0), p.display()),
                Err(err) => eprintln!("skipped {}: {err}", p.display()),
            }
        }
    }
}
