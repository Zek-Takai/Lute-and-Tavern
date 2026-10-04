#!/usr/bin/env bash
# Sets up the analyzer's own Python environment (nothing shared with other environments).
set -euo pipefail
VENV=${VENV:-$HOME/venvs/lute-shows}
python3 -m venv "$VENV"
"$VENV/bin/pip" install -q --upgrade pip
"$VENV/bin/pip" install -q torch --index-url https://download.pytorch.org/whl/cu130
"$VENV/bin/pip" install -q -r "$(dirname "$0")/requirements.txt"
"$VENV/bin/pip" install -q --no-deps demucs==4.0.1
"$VENV/bin/python" - <<'PY'
import torch, demucs, librosa, imageio_ffmpeg
print("torch", torch.__version__, "cuda", torch.cuda.is_available(), torch.cuda.get_device_name(0) if torch.cuda.is_available() else "")
print("librosa", librosa.__version__, "ffmpeg", imageio_ffmpeg.get_ffmpeg_exe())
PY
