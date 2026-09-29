# Roadmap

## 0.1 (this version)
- Music folders with live sync; tags and cover art read from files
- Genre tags: built-in, your own, from files, from the songwriter; rename, merge, delete
- Playlists; now-playing queue with reorder; repeat playlist or song; seek
- Songwriter over any OpenAI-compatible endpoint: request, rewrite by chat, versions, Suno fields
- Takes for songwriter songs, master take plays in the library
- Sheet music: click-to-write melody editor, chords, lyrics under notes, transpose, PDF/image/MusicXML attachments, MusicXML export
- Background visualizer; settings
- Everyday builds signed locally; version milestones get the official build (`scripts/build-mac.sh --release`: Developer ID, notarization, update files)
- In-app updates built in, off by default (Settings > Updates); only official releases can update themselves

## Next
- Linux and Windows builds (on Windows, reordering the queue by dragging needs a different drag mode)
  - Audio formats: the system web engine plays MP3, AAC/M4A and FLAC everywhere, but Windows lacks Apple Lossless
    (ALAC) and AIFF, and Linux depends on the installed GStreamer codecs. Check what the engine can play at startup
    (`canPlayType`) and fall back to a built-in decoder (Symphonia, pure Rust: MP3, AAC, ALAC, FLAC, Vorbis, WAV,
    AIFF; libopus for Opus-in-M4A) served through the `lt://` protocol, so seeking and the visualizer keep working.
  - Decided: the small decoders ship inside the app (about 2 MB) so every common format plays offline on any
    system. Big optional tools are never bundled or downloaded: if the person has FFmpeg installed, the app uses it
    (WMA and other rare formats); if not, it says clearly the file can't play here and how to convert it.
- Sheet music from audio (optional tool, low priority: sheet music is for people who read it; the app's owner does not).
  Plan when it is built: use the song's Suno vocal stem when present, else split locally with Spleeter (MIT);
  vocal to notes with RMVPE (Apache-2.0), tempo with Beat This! (MIT); lyrics placed from the known words.
  Not Demucs: its weights are licensed for scientific use only. Fine-tune only if stock accuracy on real songs is poor.
- Local music generation: render a songwriter song to audio on your own machine (open models such as ACE-Step), landing as a take
- Lyrics view with timing while a song plays
- Edit file tags in place (write back to the audio file), with a clear opt-in

## Streaming services
- **YouTube: dropped** (2026-09-29). Only its visible embedded player is allowed, and ad-free playback cannot be
  had legitimately inside a desktop app (Google blocks sign-in in app web views; embeds need third-party cookies).
  Adding it would turn into fighting ads, which is not what this app is for.
- Friendlier options researched, not yet chosen:
  - Apple Music via MusicKit (official, no ads, needs each user's Apple Music subscription; audio is DRM, so no
    visualizer or stems for those songs; needs a Swift bridge).
  - Audius (free open API, no ads, real streams; independent catalog; credit artists).
  - Jamendo (Creative Commons catalog; free API for non-commercial apps).
  - Self-hosted server on the NAS (Navidrome/Jellyfin, Subsonic API): stream your own collection anywhere.
  - Not viable: Spotify (DRM, 250k-MAU gate), TIDAL (player SDK only for web/iOS/Android), SoundCloud (invite-only API).
