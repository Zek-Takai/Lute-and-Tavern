# Show analyzer

Makes a visualizer "show" for each song, once, ahead of time: the song is split into vocals, drums, bass and
other (Demucs, MIT license), then librosa (ISC license) marks its beats, hits, sections, vocal phrases and key.
The app's visualizer plays the show and varies it on every play. Shows are named by song ID
(`crates/lt-core/src/songid.rs`) and kept in the `Shows` folder next to the server's song ID list.

    python run.py --music <music root> --list worklist.tsv --out <shows folder> [--workers 3]

`worklist.tsv` lines are `<song id>\t<path relative to the music root>`; finished shows are skipped, so a
run can be stopped and started again. Show format: see `analyze.py` (`FORMAT`).
