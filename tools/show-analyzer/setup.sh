#!/usr/bin/env bash
# Sets up the show analyzer's own Python environment (nothing shared with other environments) on macOS or
# Linux. Needs Python 3.10 or newer. Downloads PyTorch, Demucs and librosa (a few GB).
#   VENV=<folder>          where the environment goes (default: a "python" folder next to this script)
#   TORCH_INDEX=<url>      a PyTorch wheel index to use instead of the automatic choice
set -euo pipefail
cd "$(dirname "$0")"
VENV=${VENV:-$PWD/python}
PY=${PYTHON:-python3}
"$PY" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else "Python 3.10 or newer is needed (found " + sys.version.split()[0] + ")")'
if ! "$PY" -c 'import ensurepip' 2>/dev/null; then
  echo "This Python can't make environments yet. On Ubuntu or Debian: sudo apt install python3-venv" >&2
  exit 1
fi
"$PY" -m venv "$VENV"
"$VENV/bin/pip" install -q --upgrade pip
# macOS: the standard PyTorch uses Apple Silicon's GPU by itself. Linux: the CUDA build with an NVIDIA GPU,
# else the much smaller processor-only build.
if [[ -z "${TORCH_INDEX:-}" && "$(uname)" == Linux ]]; then
  if command -v nvidia-smi >/dev/null; then TORCH_INDEX=https://download.pytorch.org/whl/cu130
  else TORCH_INDEX=https://download.pytorch.org/whl/cpu; fi
fi
"$VENV/bin/pip" install -q torch ${TORCH_INDEX:+--index-url "$TORCH_INDEX"}
"$VENV/bin/pip" install -q -r requirements.txt
"$VENV/bin/pip" install -q --no-deps demucs==4.0.1
"$VENV/bin/python" - <<'PY'
import torch, demucs, librosa, imageio_ffmpeg
from analyze import best_device
print("Ready. PyTorch", torch.__version__, "will use:", best_device(torch))
print("librosa", librosa.__version__, "· ffmpeg", imageio_ffmpeg.get_ffmpeg_exe())
PY
