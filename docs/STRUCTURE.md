# Structure

Every file has one home. New work goes where this map says; if nothing fits, extend the map first.

```
lute-and-tavern/
├── Cargo.toml                 workspace: the engine and each app face
├── crates/
│   └── lt-core/               THE ENGINE. No UI, no Tauri. Tested with `cargo test`.
│       └── src/
│           ├── db.rs          SQLite schema and migrations (append-only list)
│           ├── library.rs     songs, folders, playlists, genre tags, sheets, takes, settings
│           ├── tags.rs        reading audio files: tags, length, cover art, file-name fallbacks
│           ├── genre.rs       genre normalization (ID3 numbers, casing)
│           └── songwriter.rs  OpenAI-compatible client: write and rewrite songs
├── apps/
│   └── desktop/               THE DESKTOP FACE (Tauri 2). macOS first, then Linux and Windows.
│       ├── index.html
│       ├── src/               interface (TypeScript, no framework)
│       │   ├── api.ts         typed bridge to the Rust commands
│       │   ├── state.ts       library snapshot and view state
│       │   ├── player.ts      playback, queue, repeat, mini visualizer
│       │   ├── visualizer.ts  background visualizer (three.js)
│       │   ├── music/         notation drawing and MusicXML
│       │   └── ui/            library grid and sidebar, song popup, sheet editor, settings
│       └── src-tauri/         Rust side of the app: commands, lt:// file protocol, folder watcher
├── scripts/
│   └── build-mac.sh           builds and signs the macOS app (run on the Mac)
└── design/
    └── icon/                  app icon source (SVG) and master PNG
```

## Where the app keeps its data

Per-user app data folder (macOS: `~/Library/Application Support/com.luteandtavern.app/`):

```
library.db             the whole library in SQLite
art/<sha1>.<ext>       cover art, deduplicated by content
sheets/<song id>/      uploaded sheet music for a song
takes/<song id>/       audio takes for songwriter songs
```

Music files in watched folders are never modified, moved or deleted by the app.
The songwriter's API key lives in the system keychain, never in the database.

## Boundaries

- `lt-core` must not depend on Tauri. Anything a future face (CLI, server, mobile) needs lives there.
- Faces talk to the engine only through its public API.
- Schema changes are new entries at the end of `MIGRATIONS` in `db.rs`; old entries are never edited.
