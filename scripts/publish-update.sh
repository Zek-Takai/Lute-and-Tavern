#!/usr/bin/env bash
# Sends a finished release folder (made by build-mac.sh --release) to the update server, where installed
# copies of Lute & Tavern find it through their built-in updater (Settings › General › Updates). Any web
# host works: the app only reads <update address>/latest.json and the files it names. GitHub isn't needed.
#
#   scripts/publish-update.sh <release folder>
#
# Set LT_UPDATE_UPLOAD to the command that copies a folder to your server; {dir} is replaced with the folder:
#   LT_UPDATE_UPLOAD='rclone copy {dir} myserver:lute-and-tavern'                 (any rclone remote, S3, R2, WebDAV…)
#   LT_UPDATE_UPLOAD='rsync -av {dir}/ me@host:/var/www/lute-and-tavern/'         (your own web server)
# latest.json goes up last, so nobody is offered an update before its files are there.
set -euo pipefail
dir=${1:?Usage: scripts/publish-update.sh <release folder>}
dir=$(cd "$dir" && pwd)
[[ -f "$dir/latest.json" ]] || { echo "No latest.json in $dir: build the release with LT_UPDATE_BASE_URL set." >&2; exit 1; }
if [[ -z "${LT_UPDATE_UPLOAD:-}" ]]; then
  echo "Release files are ready in: $dir"
  echo "No LT_UPDATE_UPLOAD command is set, so nothing was sent. Copy the files to your update server,"
  echo "latest.json last, or set LT_UPDATE_UPLOAD and run: scripts/publish-update.sh \"$dir\""
  exit 0
fi
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
find "$dir" -maxdepth 1 -type f ! -name latest.json -exec cp {} "$stage/" \;
run() { local cmd=${LT_UPDATE_UPLOAD//\{dir\}/\"$1\"}; echo "+ $cmd"; bash -c "$cmd"; }
run "$stage"
rm -f "$stage"/* && cp "$dir/latest.json" "$stage/"
run "$stage"
echo "Published. Installed copies will offer version $(node -p "require('$dir/latest.json').version") on their next update check."
