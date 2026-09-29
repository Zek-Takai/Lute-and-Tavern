//! Reads what an audio file says about itself: tags, length and cover art.
//! When a file has no tags, the file name and folder layout fill the gaps.

use crate::genre;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::{Accessor, ItemKey};
use std::path::Path;

pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "flac", "wav", "ogg", "oga", "opus", "aif", "aiff", "wma", "alac"];

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Part names that stem exports (Suno, DAWs) put in brackets after the song title.
const STEM_NAMES: &[&str] = &[
    "vocals", "lead vocals", "backing vocals", "vocal", "instrumental", "drums", "bass", "guitar", "keyboard", "keys",
    "piano", "synth", "strings", "brass", "woodwinds", "percussion", "fx", "other", "melody", "choir", "pad", "organ",
];

/// "Song (Vocals).wav" -> ("Song", "Vocals") when the bracketed part is a known stem name.
pub fn stem_of(path: &Path) -> Option<(String, String)> {
    let stem = path.file_stem()?.to_str()?.trim();
    let inner = stem.strip_suffix(')')?;
    let open = inner.rfind(" (")?;
    let (base, name) = (inner[..open].trim(), inner[open + 2..].trim());
    (!base.is_empty() && STEM_NAMES.contains(&name.to_lowercase().as_str())).then(|| (base.to_string(), name.to_string()))
}

#[derive(Debug, Clone, Default)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// The album's artist ("Various Artists" on compilations); empty when the file does not say.
    pub album_artist: String,
    pub year: String,
    pub genre: String,
    pub track: Option<u32>,
    pub duration: Option<f64>,
    /// Embedded cover art: (bytes, file extension)
    pub picture: Option<(Vec<u8>, &'static str)>,
}

/// Reads a track. Never fails: unreadable tags fall back to the file name.
/// `root` is the watched folder the file sits in, used for Artist/Album/Song layouts.
pub fn read_track(path: &Path, root: Option<&Path>) -> TrackInfo {
    let mut info = from_path(path, root);
    if let Ok(tagged) = lofty::read_from_path(path) {
        let secs = tagged.properties().duration().as_secs_f64();
        if secs > 0.0 {
            info.duration = Some(secs);
        }
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            let set = |dst: &mut String, v: Option<std::borrow::Cow<'_, str>>| {
                if let Some(v) = v.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
                    *dst = v;
                }
            };
            set(&mut info.title, tag.title());
            set(&mut info.artist, tag.artist());
            set(&mut info.album, tag.album());
            if let Some(a) = tag.get_string(ItemKey::AlbumArtist).map(str::trim).filter(|a| !a.is_empty()) {
                info.album_artist = a.to_string();
            }
            if let Some(g) = tag.genre() {
                info.genre = genre::normalize(&g);
            }
            if let Some(t) = tag.track() {
                info.track = Some(t);
            }
            let year = [ItemKey::Year, ItemKey::RecordingDate, ItemKey::OriginalReleaseDate, ItemKey::ReleaseDate]
                .iter()
                .find_map(|k| tag.get_string(*k))
                .map(|d| d.chars().take(4).collect::<String>())
                .filter(|y| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()));
            if let Some(y) = year {
                info.year = y;
            }
            if let Some(pic) = tag.pictures().iter().find(|p| p.pic_type() == lofty::picture::PictureType::CoverFront).or_else(|| tag.pictures().first()) {
                let ext = match pic.mime_type() {
                    Some(lofty::picture::MimeType::Png) => "png",
                    Some(lofty::picture::MimeType::Gif) => "gif",
                    Some(lofty::picture::MimeType::Bmp) => "bmp",
                    _ => "jpg",
                };
                if !pic.data().is_empty() {
                    info.picture = Some((pic.data().to_vec(), ext));
                }
            }
        }
    }
    info
}

/// "Artist - Title.mp3" and "01 Title.mp3" file names, plus Artist/Album/ folders under the root.
fn from_path(path: &Path, root: Option<&Path>) -> TrackInfo {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Untitled").to_string();
    let (mut artist, mut title) = match stem.split_once(" - ") {
        Some((a, t)) if a.trim().chars().all(|c| c.is_ascii_digit()) => (String::new(), t.trim().to_string()),
        Some((a, t)) => (a.trim().to_string(), t.trim().to_string()),
        None => (String::new(), stem.clone()),
    };
    let stripped = title.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches([' ', '.', '_', '-']);
    if stripped.len() < title.len() && !stripped.is_empty() && title.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        title = stripped.to_string();
    }
    let mut album = String::new();
    if let Some(root) = root
        && let Ok(rel) = path.strip_prefix(root) {
            let parts: Vec<_> = rel.parent().map(|p| p.iter().filter_map(|s| s.to_str()).collect()).unwrap_or_default();
            if let Some(a) = parts.last() {
                album = a.to_string();
            }
            if artist.is_empty() && parts.len() >= 2 {
                artist = parts[parts.len() - 2].to_string();
            }
        }
    TrackInfo {
        title,
        artist: if artist.is_empty() { "Unknown artist".into() } else { artist },
        album,
        genre: genre::UNSORTED.into(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_stem_files() {
        assert_eq!(stem_of(Path::new("/m/Every Leaf (Backing Vocals).wav")), Some(("Every Leaf".into(), "Backing Vocals".into())));
        assert_eq!(stem_of(Path::new("/m/Song (FX).mp3")), Some(("Song".into(), "FX".into())));
        assert_eq!(stem_of(Path::new("/m/Song (Live).mp3")), None);
        assert_eq!(stem_of(Path::new("/m/Song.mp3")), None);
    }

    #[test]
    fn falls_back_to_file_and_folder_names() {
        let root = Path::new("/music");
        let t = read_track(Path::new("/music/Some Band/Great Album/03 - Opening.mp3"), Some(root));
        assert_eq!(t.title, "Opening");
        assert_eq!(t.album, "Great Album");
        assert_eq!(t.artist, "Some Band");
        assert_eq!(t.genre, "Unsorted");
        let t = read_track(Path::new("/music/Artist Name - Song Title.flac"), Some(root));
        assert_eq!((t.artist.as_str(), t.title.as_str()), ("Artist Name", "Song Title"));
    }
}
