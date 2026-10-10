#!/usr/bin/env bash
# Sets up the show analyzer's own Python environment (nothing shared with other environments) on macOS or
# Linux. Needs Python 3.10 to 3.14. Downloads PyTorch, Demucs and librosa (a few GB).
#   VENV=<folder>          where the environment goes (default: a "python" folder next to this script)
#   TORCH_INDEX=<url>      a PyTorch wheel index to use instead of the automatic choice
set -euo pipefail
cd "$(dirname "$0")"
VENV=${VENV:-$PWD/python}
PY=${PYTHON:-python3}
"$PY" -c 'import sys; sys.exit(0 if (3, 10) <= sys.version_info[:2] <= (3, 14) else "Python 3.10 to 3.14 is needed (found " + sys.version.split()[0] + ")")'
if ! "$PY" -c 'import ensurepip' 2>/dev/null; then
  echo "This Python can't make environments yet. On Ubuntu or Debian: sudo apt install python3-venv" >&2
  exit 1
fi
"$PY" -m venv "$VENV"
"$VENV/bin/pip" install -q --upgrade pip
# macOS: the standard PyTorch uses the Mac's GPU by itself (Apple Silicon, or AMD in Intel Macs). Linux: the
# CUDA build for an NVIDIA GPU, the newest ROCm build PyTorch publishes for an AMD GPU (needs AMD's amdgpu
# driver, which provides /dev/kfd), else the much smaller processor-only build.
newest_rocm() {
  "$VENV/bin/python" - <<'PY'
import urllib.request
for v in ["8.0", "7.6", "7.5", "7.4", "7.3", "7.2", "7.1", "7.0", "6.4"]:
    try:
        urllib.request.urlopen(f"https://download.pytorch.org/whl/rocm{v}/torch/", timeout=10)
        print(f"https://download.pytorch.org/whl/rocm{v}")
        break
    except Exception:
        pass
PY
}
# AMD graphics PyTorch's ROCm build supports (gfx target versions: Instinct MI100/MI200/MI300/MI350, Radeon RX 6800-6950
# and PRO W6800, RX 7000, Ryzen AI Max, RX 9000). Built-in Radeon graphics in most Ryzen chips aren't on the list.
amd_supported() {
  local f v
  for f in /sys/class/kfd/kfd/topology/nodes/*/properties; do
    v=$(awk '/^gfx_target_version/{print $2}' "$f" 2>/dev/null)
    case "$v" in 90008|90010|90402|90500|100300|110000|110001|110002|110501|120000|120001) return 0 ;; esac
  done
  return 1
}
if [[ -z "${TORCH_INDEX:-}" && "$(uname)" == Linux ]]; then
  if command -v nvidia-smi >/dev/null; then TORCH_INDEX=https://download.pytorch.org/whl/cu130
  elif amd_supported; then TORCH_INDEX=$(newest_rocm); echo "AMD graphics card found: using ${TORCH_INDEX:-the standard PyTorch}"
  elif [[ -e /dev/kfd ]]; then TORCH_INDEX=https://download.pytorch.org/whl/cpu; echo "AMD graphics found, but PyTorch doesn't support this model: using the processor."
  else TORCH_INDEX=https://download.pytorch.org/whl/cpu; fi
fi
# torch and torchaudio must be the same version (Demucs's filtering loads torchaudio); 2.11 is torchaudio's last.
"$VENV/bin/pip" install -q "torch==2.11.*" "torchaudio==2.11.*" ${TORCH_INDEX:+--index-url "$TORCH_INDEX"}
"$VENV/bin/pip" install -q -r requirements.txt
"$VENV/bin/pip" install -q --no-deps demucs==4.0.1
"$VENV/bin/python" - <<'PY'
import torch, demucs, librosa, imageio_ffmpeg
from analyze import best_device
print("Ready. PyTorch", torch.__version__, "will use:", best_device(torch))
print("librosa", librosa.__version__, "· ffmpeg", imageio_ffmpeg.get_ffmpeg_exe())
PY
