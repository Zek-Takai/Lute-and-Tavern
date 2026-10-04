//! Prints the permanent song ID (see songid.rs) of every audio file in the given files and folders:
//! one line per file, "<id>\t<size>\t<path>". Used for processing a whole music library at once.
//!
//! `lt-songid --index <music root>` writes the server's song ID list instead: "<id>\t<size>\t<path relative
//! to the root>", sorted by path, with exact copies of the same audio given their own IDs in path order (the
//! same rule the app uses), so every run gives the same list.
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--index") {
        return index(Path::new(args.get(1).map(String::as_str).unwrap_or(".")));
    }
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

fn index(root: &Path) {
    let exts = ["mp3", "m4a", "mp4", "aac", "flac", "ogg", "opus", "wav", "aif", "aiff", "wma", "alac"];
    let mut rows: Vec<(String, u64, String)> = vec![];
    for e in walkdir::WalkDir::new(root).follow_links(true).into_iter().filter_map(|e| e.ok()) {
        let p = e.path();
        let Ok(rel) = p.strip_prefix(root) else { continue };
        let audio = p.extension().and_then(|x| x.to_str()).is_some_and(|x| exts.contains(&x.to_ascii_lowercase().as_str()));
        let hidden = rel.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
        if !e.file_type().is_file() || !audio || hidden {
            continue;
        }
        match lt_core::songid::for_file(p) {
            Ok(id) => rows.push((id, e.metadata().map(|m| m.len()).unwrap_or(0), rel.to_string_lossy().replace('\\', "/"))),
            Err(err) => eprintln!("skipped {}: {err}", p.display()),
        }
    }
    rows.sort_by(|a, b| a.2.cmp(&b.2));
    let mut seen = std::collections::HashMap::<String, u32>::new();
    let mut used = std::collections::HashSet::<String>::new();
    for (id, size, rel) in rows {
        let n = seen.entry(id.clone()).or_insert(0);
        *n += 1;
        let mut uid = if *n == 1 { id.clone() } else { lt_core::songid::variant(&id, *n) };
        let mut k = *n + 1;
        while !used.insert(uid.clone()) {
            uid = lt_core::songid::variant(&id, k);
            k += 1;
        }
        println!("{uid}\t{size}\t{rel}");
    }
}
