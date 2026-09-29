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
- Sheet music from audio (optional tool, low priority: sheet music is for people who read it; the app's owner does not).
  Plan when it is built: use the song's Suno vocal stem when present, else split locally with Spleeter (MIT);
  vocal to notes with RMVPE (Apache-2.0), tempo with Beat This! (MIT); lyrics placed from the known words.
  Not Demucs: its weights are licensed for scientific use only. Fine-tune only if stock accuracy on real songs is poor.
- Local music generation: render a songwriter song to audio on your own machine (open models such as ACE-Step), landing as a take
- Lyrics view with timing while a song plays
- In-app updates (Tauri updater): deferred until the app has users. People on versions without it update by downloading the new release
- Edit file tags in place (write back to the audio file), with a clear opt-in
