# Sets up the show analyzer's own Python environment (nothing shared with other environments) on Windows.
# Needs Python 3.10 or newer from python.org (with "Add python.exe to PATH" ticked). Downloads PyTorch,
# Demucs and librosa (a few GB). Run it from this folder:
#   powershell -ExecutionPolicy Bypass -File setup.ps1
# Optional: $env:VENV = where the environment goes (default: a "python" folder next to this script)
#           $env:TORCH_INDEX = a PyTorch wheel index to use instead of the automatic choice
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$venv = if ($env:VENV) { $env:VENV } else { Join-Path $here 'python' }
$py = if (Get-Command py -ErrorAction SilentlyContinue) { 'py' } else { 'python' }
& $py -c "import sys; sys.exit(0 if sys.version_info >= (3, 10) else 'Python 3.10 or newer is needed (found ' + sys.version.split()[0] + ')')"
if ($LASTEXITCODE) { exit 1 }
& $py -m venv $venv
$pip = Join-Path $venv 'Scripts\pip.exe'
$python = Join-Path $venv 'Scripts\python.exe'
& $python -m pip install -q --upgrade pip
# With an NVIDIA GPU, the CUDA build of PyTorch; otherwise the standard one (uses the processor).
$index = $env:TORCH_INDEX
if (-not $index -and (Get-Command nvidia-smi -ErrorAction SilentlyContinue)) { $index = 'https://download.pytorch.org/whl/cu130' }
if ($index) { & $pip install -q torch --index-url $index } else { & $pip install -q torch }
& $pip install -q -r (Join-Path $here 'requirements.txt')
& $pip install -q --no-deps demucs==4.0.1
Push-Location $here
& $python -c "import torch, librosa, imageio_ffmpeg; from analyze import best_device; print('Ready. PyTorch', torch.__version__, 'will use:', best_device(torch)); print('librosa', librosa.__version__)"
Pop-Location
