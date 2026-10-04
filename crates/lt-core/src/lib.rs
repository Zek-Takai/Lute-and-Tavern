//! Lute & Tavern core: the music library, its database, tag reading and the songwriter's data.
//! The desktop app (apps/desktop) is a thin face over this crate.

pub mod assistant;
pub mod audio;
pub mod backup;
pub mod control;
pub mod db;
pub mod genre;
pub mod library;
pub mod mcp;
pub mod organize;
pub mod playlist_io;
pub mod playlist_sync;
pub mod radio;
pub mod remote;
pub mod remote_jellyfin;
pub mod remote_library;
pub mod remote_plex;
pub mod remote_subsonic;
pub mod remote_webdav;
pub mod songwriter;
pub mod sync;
pub mod tags;

pub use library::{Draft, Library, Song, SongDetail, SongPatch};
