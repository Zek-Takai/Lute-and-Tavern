#!/bin/zsh
# Runs ONE live development copy of Lute & Tavern on this Mac. Changes pushed to this checkout appear
# on their own: the interface reloads instantly; Rust changes rebuild and the window reopens.
#
#   scripts/dev-mac.sh          start (does nothing if it is already running)
#   scripts/dev-mac.sh stop     stop it
#   scripts/dev-mac.sh status   show whether it runs
#
# The installed copy in /Applications is quit first: both would share one library database and
# both would organize the library folder. Log: ~/Library/Logs/lute-and-tavern-dev.log
set -uo pipefail
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
cd "${0:A:h}/../apps/desktop"
log="$HOME/Library/Logs/lute-and-tavern-dev.log"
running() { pgrep -f "tauri dev" >/dev/null || pgrep -f "target/debug/lute-and-tavern" >/dev/null; }

case "${1:-start}" in
  stop)
    pkill -f "tauri dev"; pkill -f "vite"; sleep 2; pkill -f "target/debug/lute-and-tavern"
    print "Stopped."; exit 0 ;;
  status)
    running && print "Running." || print "Not running."; exit 0 ;;
esac

if running; then print "Already running."; exit 0; fi
if pgrep -f "Lute & Tavern.app/Contents/MacOS" >/dev/null; then
  osascript -e 'tell application "Lute & Tavern" to quit' >/dev/null 2>&1
  sleep 2; pkill -f "Lute & Tavern.app/Contents/MacOS" 2>/dev/null
fi
[[ -d node_modules ]] || npm install --no-fund --no-audit
nohup npm run tauri dev >"$log" 2>&1 &
print "Started. The window opens when the first build finishes. Log: $log"
