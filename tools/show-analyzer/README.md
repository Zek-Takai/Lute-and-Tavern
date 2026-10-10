# Show analyzer

Makes a visualizer "show" for each song, once, ahead of time: the song is split into vocals, drums, bass and
other (Demucs, MIT license), then librosa (ISC license) marks its beats, hits, sections, vocal phrases and key.
The app's visualizer plays the show and varies it on every play. Shows are named by song ID
(`crates/lt-core/src/songid.rs`).

## From the app (any computer)

Settings › Shows walks through it. The app saves these files to `_Software/Show analyzer` in the library
folder, writes the song list (`worklist.tsv`) there, and reads shows from `_Software/Shows`. Everything in
`_Software` is left alone by the organizer, song scans and backups.

1. Install Python 3.10 to 3.14 (python.org).
2. Set up once, in this folder: `bash setup.sh` (macOS, Linux) or
   `powershell -ExecutionPolicy Bypass -File setup.ps1` (Windows). It makes its own Python environment in
   `python/` here and installs PyTorch, Demucs and librosa (a few GB). An NVIDIA GPU or Apple Silicon makes
   it much faster; it works on the processor alone too.
3. In the app: Make song list.
4. Run: `python/bin/python run.py --list worklist.tsv --out "<Shows folder>"`
   (Windows: `python\Scripts\python.exe run.py ...`). Finished shows are skipped, so it can be stopped and
   started again. The first run downloads Demucs's model (about 80 MB) into `models/` here.

## On a server

    python run.py --music <music root> --list worklist.tsv --out <shows folder> [--workers 3]

`worklist.tsv` lines are `<song id>\t<path>`: a full path, or one relative to `--music`. Show format: see
`analyze.py` (`FORMAT`).
