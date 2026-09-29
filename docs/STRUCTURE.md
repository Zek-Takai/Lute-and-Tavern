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
│           ├── organize.rs    the managed library folder: song folders, stems, song.json, moves and renames
│           ├── tags.rs        reading audio files: tags, length, cover art, stems, file-name fallbacks
│           ├── genre.rs       genre normalization (ID3 numbers, casing)
│           ├── songwriter.rs  OpenAI-compatible client: write and rewrite songs
│           ├── audio.rs       WAV to FLAC for stems, verified sample by sample
│           ├── mcp.rs         MCP songwriting tools (JSON-RPC, transport-free)
│           ├── control.rs     MCP full-control tools: search, playlists, tags, playback
│           └── assistant.rs   in-app assistant (tool-calling chat, history) and the Whisper voice hook
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
│       └── src-tauri/         Rust side of the app: commands, lt:// file protocol, folder watcher,
│                              local MCP server, assistant and luteandtavern:// links
├── scripts/
│   ├── dev-mac.sh             runs one live development copy on the Mac
│   └── build-mac.sh           builds and signs the macOS app (run on the Mac)
└── design/
    └── icon/                  app icon source (SVG) and master PNG
```

## The library folder (the person's music)

One folder is the library; the app organizes it. Other music folders are only watched, never changed.

```
<library>/Songs/<Title>/<Title>.<ext>         the song (songwriter songs, singles, anything with stems)
<library>/Songs/<Title>/stems/<Part>.<ext>    its parts: Vocals, Drums... (wav and mp3 of a part may both exist)
<library>/Songs/<Title>/takes/  sheets/       takes and sheet music added in the app
<library>/Songs/<Title>/cover.<ext>           artwork
<library>/Songs/<Title>/song.json             everything the app knows; a copied folder restores from it
<library>/Albums/<Artist>/<Album>/NN - <Title>.<ext>   other artists' albums, kept together
```

Files dropped anywhere else in the library are moved into place once they finish copying.
Stem files named "Title (Part).ext" join their song. Renaming a song renames its folder and file.

## Where the app keeps its data

Per-user app data folder (macOS: `~/Library/Application Support/com.luteandtavern.app/`):

```
library.db             the whole library in SQLite
art/<sha1>.<ext>       cover art, deduplicated by content
sheets/<song id>/      uploaded sheet music for a song
takes/<song id>/       audio takes for songwriter songs
```

Music files in other watched folders are never modified, moved or deleted by the app. In the
library folder the app moves and renames files into the layout above, but never deletes music.
The songwriter's API key lives in the system keychain, never in the database.

## Boundaries

- `lt-core` must not depend on Tauri. Anything a future face (CLI, server, mobile) needs lives there.
- Faces talk to the engine only through its public API.
- Schema changes are new entries at the end of `MIGRATIONS` in `db.rs`; old entries are never edited.
