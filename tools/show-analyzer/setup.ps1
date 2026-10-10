# Sets up the show analyzer's own Python environment (nothing shared with other environments) on Windows.
# Needs Python 3.10 to 3.14 from python.org (with "Add python.exe to PATH" ticked). Downloads PyTorch,
# Demucs and librosa (a few GB). Run it from this folder:
#   powershell -ExecutionPolicy Bypass -File setup.ps1
# Optional: $env:VENV = where the environment goes (default: a "python" folder next to this script)
#           $env:TORCH_INDEX = a PyTorch wheel index to use instead of the automatic choice
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$venv = if ($env:VENV) { $env:VENV } else { Join-Path $here 'python' }
$py = if (Get-Command py -ErrorAction SilentlyContinue) { 'py' } else { 'python' }
& $py -c "import sys; sys.exit(0 if (3, 10) <= sys.version_info[:2] <= (3, 14) else 'Python 3.10 to 3.14 is needed (found ' + sys.version.split()[0] + ')')"
if ($LASTEXITCODE) { exit 1 }
& $py -m venv $venv
$pip = Join-Path $venv 'Scripts\pip.exe'
$python = Join-Path $venv 'Scripts\python.exe'
& $python -m pip install -q --upgrade pip
# With an NVIDIA GPU, the CUDA build of PyTorch; otherwise the standard one (uses the processor). PyTorch for
# AMD graphics cards on Windows comes from AMD itself; see the note at the end.
$index = $env:TORCH_INDEX
if (-not $index -and (Get-Command nvidia-smi -ErrorAction SilentlyContinue)) { $index = 'https://download.pytorch.org/whl/cu130' }
$amd = @(Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue | Where-Object { $_.Name -match 'Radeon|AMD' }).Count -gt 0
# torch and torchaudio must be the same version (Demucs's filtering loads torchaudio); 2.11 is torchaudio's last.
if ($index) { & $pip install -q 'torch==2.11.*' 'torchaudio==2.11.*' --index-url $index } else { & $pip install -q 'torch==2.11.*' 'torchaudio==2.11.*' }
& $pip install -q -r (Join-Path $here 'requirements.txt')
& $pip install -q --no-deps demucs==4.0.1
Push-Location $here
& $python -c "import torch, librosa, imageio_ffmpeg; from analyze import best_device; print('Ready. PyTorch', torch.__version__, 'will use:', best_device(torch)); print('librosa', librosa.__version__)"
Pop-Location
if ($amd -and -not $index) {
  Write-Host ''
  Write-Host 'AMD graphics card found. This setup uses the processor. To use the card (Radeon RX 7000 or 9000 series),'
  Write-Host 'install AMD''s PyTorch for Windows into this analyzer by following AMD''s guide (Settings > Shows has the'
  Write-Host 'link), using .\python\Scripts\pip.exe in place of pip. AMD''s build needs Python 3.12.'
}
