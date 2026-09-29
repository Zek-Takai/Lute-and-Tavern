//! Lute & Tavern core: the music library, its database, tag reading and the songwriter's data.
//! The desktop app (apps/desktop) is a thin face over this crate.

pub mod assistant;
pub mod audio;
pub mod control;
pub mod db;
pub mod genre;
pub mod library;
pub mod mcp;
pub mod organize;
pub mod songwriter;
pub mod tags;

pub use library::{Draft, Library, Song, SongDetail, SongPatch};
