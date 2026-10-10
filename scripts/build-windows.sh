#!/usr/bin/env bash
# Builds the Lute & Tavern Windows installer (an NSIS setup.exe for 64-bit Windows) on Linux, by cross-compiling
# with cargo-xwin. The installer isn't code-signed by Microsoft, so Windows SmartScreen asks people to confirm
# once ("More info" > "Run anyway"). In-app updates are still signed with the project's update key.
#
#   scripts/build-windows.sh            everyday build
#   scripts/build-windows.sh --release  official release: signed update file, built-in Apple Music access, the
#                                       update address, and a release folder with latest.json
#
# Needs (once): rustup target add x86_64-pc-windows-msvc; cargo install --locked cargo-xwin; and nsis, lld, llvm,
# clang from the system's packages. Releases need the same keys and settings as build-linux.sh --release.
set -euo pipefail
cd "$(dirname "$0")/.."
export LT_TARGET=x86_64-pc-windows-msvc
source scripts/clean-paths.sh
mode=${1:-local}
version=$(node -p "require('./apps/desktop/src-tauri/tauri.conf.json').version")
build=(npm run tauri build -- --runner cargo-xwin --target "$LT_TARGET" --bundles nsis)

if [[ "$mode" != --release ]]; then
  (cd apps/desktop && "${build[@]}")
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
  build_config=$(node -e 'console.log(JSON.stringify({bundle: {createUpdaterArtifacts: true}, plugins: {updater: {pubkey: process.env.LT_UPDATER_PUBKEY}}}))')
  (cd apps/desktop && "${build[@]}" --config "$build_config")
fi

exe=target/$LT_TARGET/release/lute-and-tavern.exe
if strings -n 6 "$exe" | grep -qF "$HOME"; then
  echo "The build still contains this computer's home folder path; not safe to publish." >&2
  exit 1
fi
setup="target/$LT_TARGET/release/bundle/nsis/Lute & Tavern_${version}_x64-setup.exe"
[[ "$mode" != --release ]] && { echo "Built, with no paths from this computer inside: $setup"; exit 0; }

out="target/release/bundle/release-$version-windows"
rm -rf "$out" && mkdir -p "$out"
name="Lute-and-Tavern-$version-windows-x64-setup.exe"
cp "$setup" "$out/$name"
cp "$setup.sig" "$out/$name.sig"
node scripts/update-manifest.mjs "$out/latest.json" "$version" windows-x86_64 "$base/$name" "$setup.sig"
echo "Release ready in $(cd "$out" && pwd), with no paths from this computer inside."
