#!/usr/bin/env bash
# Builds Lute & Tavern for Linux: a .deb (Ubuntu, Debian), an .rpm (Fedora, openSUSE) and an AppImage (any
# distribution), in target/release/bundle/. Needs Rust, Node 20+ and the WebKitGTK 4.1 development libraries.
# The build carries no paths or names from this computer (see clean-paths.sh); the script checks that.
#
#   scripts/build-linux.sh            everyday build
#   scripts/build-linux.sh --release  official release: signed in-app update files, built-in Apple Music access,
#                                     the update address, and a release folder with latest.json
#
# Releases need (the same as build-mac.sh --release):
#   the update-signing key: TAURI_SIGNING_PRIVATE_KEY = its file (default ~/.tauri/lute-and-tavern.key, with .pub
#     beside it), and its password from LT_UPDATE_KEY_PASSWORD_FILE (read once, then deleted) or typed when asked;
#   the MusicKit key for Apple Music's token: LT_MUSICKIT_P8 = the AuthKey_….p8 file, with LT_MUSICKIT_TEAM_ID
#     (and LT_MUSICKIT_KEY_ID unless the file is named AuthKey_<KeyID>.p8). LT_MUSICKIT_SKIP=1 releases without.
#   LT_UPDATE_BASE_URL: where the release files will be (default: this project's GitHub Releases).
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/clean-paths.sh
mode=${1:-local}
version=$(node -p "require('./apps/desktop/src-tauri/tauri.conf.json').version")

if [[ "$mode" != --release ]]; then
  (cd apps/desktop && npm run tauri build -- --bundles deb,rpm,appimage)
else
  key=${TAURI_SIGNING_PRIVATE_KEY:-$HOME/.tauri/lute-and-tavern.key}
  [[ -f "$key" && -f "$key.pub" ]] || { echo "No update-signing key at $key (and $key.pub)." >&2; exit 1; }
  if [[ -n "${LT_UPDATE_KEY_PASSWORD_FILE:-}" && -r "$LT_UPDATE_KEY_PASSWORD_FILE" ]]; then
    key_password=$(<"$LT_UPDATE_KEY_PASSWORD_FILE"); rm -f "$LT_UPDATE_KEY_PASSWORD_FILE"
    echo "Update-signing key password read from LT_UPDATE_KEY_PASSWORD_FILE (file deleted)."
  else
    read -rsp "Password for the update-signing key: " key_password; echo
  fi
  export TAURI_SIGNING_PRIVATE_KEY="$key" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$key_password"
  export LT_UPDATER_PUBKEY="$(cat "$key.pub")"
  base=${LT_UPDATE_BASE_URL:-https://github.com/Zek-Takai/Lute-and-Tavern/releases/latest/download}
  base=${base%/}
  export LT_UPDATE_URL="$base/latest.json"

  if [[ -z "${LT_MUSICKIT_SKIP:-}" ]]; then
    p8=${LT_MUSICKIT_P8:?Set LT_MUSICKIT_P8 to the AuthKey_….p8 file, or LT_MUSICKIT_SKIP=1}
    kid=${LT_MUSICKIT_KEY_ID:-$(basename "$p8" .p8 | sed 's/^AuthKey_//')}
    team=${LT_MUSICKIT_TEAM_ID:?Set LT_MUSICKIT_TEAM_ID (10 characters)}
    LT_MUSICKIT_TOKEN=$(node scripts/musickit-token.mjs "$kid" "$team" 180 < "$p8")
    export LT_MUSICKIT_TOKEN
    echo "Apple Music access built in until $(date -d '+180 days' '+%B %-d, %Y') (Key ID $kid). Release again before then."
  fi

  # The public key also goes into the build's config: Tauri checks the update signatures against it.
  build_config=$(node -e 'console.log(JSON.stringify({bundle: {createUpdaterArtifacts: true}, plugins: {updater: {pubkey: process.env.LT_UPDATER_PUBKEY}}}))')
  (cd apps/desktop && npm run tauri build -- --bundles deb,rpm,appimage --config "$build_config")
fi

if strings -n 6 target/release/lute-and-tavern | grep -qF "$HOME"; then
  echo "The build still contains this computer's home folder path; not safe to publish." >&2
  exit 1
fi
[[ "$mode" != --release ]] && { echo "Built, with no paths from this computer inside. Packages: target/release/bundle/{deb,rpm,appimage}/"; exit 0; }

# Everything that gets uploaded goes into one folder, with latest.json for the in-app updater.
b=target/release/bundle
out="$b/release-$version-linux"
rm -rf "$out" && mkdir -p "$out"
add() { # <built file> <published name> <updater platform>
  cp "$1" "$out/$2"
  if [[ -f "$1.sig" ]]; then
    cp "$1.sig" "$out/$2.sig"
    node scripts/update-manifest.mjs "$out/latest.json" "$version" "$3" "$base/$2" "$1.sig"
  fi
}
add "$b/appimage/Lute & Tavern_${version}_amd64.AppImage" "Lute-and-Tavern-$version-linux-x86_64.AppImage" linux-x86_64
add "$b/deb/Lute & Tavern_${version}_amd64.deb" "Lute-and-Tavern-$version-linux-amd64.deb" linux-x86_64-deb
add "$b/rpm/Lute & Tavern-${version}-1.x86_64.rpm" "Lute-and-Tavern-$version-linux-x86_64.rpm" linux-x86_64-rpm
chmod +x "$out"/*.AppImage
echo "Release ready in $(cd "$out" && pwd), with no paths from this computer inside."
