# Lute & Tavern

A desktop music app: your music library, an AI songwriter, and sheet music in one place.

- **Your music.** Point it at your music folders. Songs are read from their own tags (title, artist, album, year, genre, cover art) and the folders stay in sync as you add, change or delete files.
- **Genre tags.** Every song carries one genre tag. Built-in tags make picking quick, you can create your own, and tags from your files and from the songwriter join the list.
- **Playlists, queue and repeat.** Build playlists, reorder what plays next, repeat a playlist or a single song.
- **A songwriter.** Describe a song and an AI of your choice writes the lyrics, a Suno style prompt and a chorus melody. Ask for changes in plain words; every rewrite is kept as a version. Works with any OpenAI-compatible server, local or hosted.
- **Sheet music.** Write a melody by clicking on the staff, attach PDFs or images of sheet music, and import or export MusicXML for MuseScore.
- **A living background** that swells with whatever is playing.

## Layout

See [docs/STRUCTURE.md](docs/STRUCTURE.md). In short: `crates/lt-core` is the engine (database, tags, folders, songwriter), `apps/desktop` is the Tauri app that puts a face on it.

## Build

Requirements: Rust (stable), Node 20+, and on macOS the Xcode command line tools.

```sh
cd apps/desktop
npm install
npm run tauri dev      # run in development
npm run tauri build    # produce the app bundle (macOS: .app and .dmg under target/release/bundle)
```

On a Mac, run the build script in Terminal (not over SSH, since signing needs your keychain):

- `scripts/build-mac.sh` is the everyday build: signed, not notarized, ready in about a minute.
- `scripts/build-mac.sh --release` is for version milestones: Developer ID signature, Apple notarization, a disk image, and in-app update files. See the notes at the top of the script.

In-app updates are built in but off by default; each person turns them on in Settings, and only official releases can update themselves.

Tests: `cargo test --workspace` from the repository root.

## Status

Version 0.1: library, folders, playlists, queue, genre tags, songwriter, sheet editor. See [docs/ROADMAP.md](docs/ROADMAP.md) for what comes next.
