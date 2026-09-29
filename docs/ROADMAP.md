# Roadmap

## 0.1 (this version)
- Music folders with live sync; tags and cover art read from files
- Genre tags: built-in, your own, from files, from the songwriter; rename, merge, delete
- Playlists; now-playing queue with reorder; repeat playlist or song; seek
- Songwriter over any OpenAI-compatible endpoint: request, rewrite by chat, versions, Suno fields
- Takes for songwriter songs, master take plays in the library
- Sheet music: click-to-write melody editor, chords, lyrics under notes, transpose, PDF/image/MusicXML attachments, MusicXML export
- Background visualizer; settings
- Signed with Developer ID and notarized: `scripts/build-mac.sh`

## Next
- Linux and Windows builds (on Windows, reordering the queue by dragging needs a different drag mode)
- Transcription from audio: split stems, turn the vocal into a melody, detect chords, sync lyrics
- Local music generation: render a songwriter song to audio on your own machine (open models such as ACE-Step), landing as a take
- Lyrics view with timing while a song plays
- In-app updates (Tauri updater): deferred until the app has users. People on versions without it update by downloading the new release
- Edit file tags in place (write back to the audio file), with a clear opt-in
