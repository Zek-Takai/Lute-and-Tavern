#!/bin/zsh
# Builds Lute & Tavern for macOS. Run it in Terminal on the Mac: signing needs your unlocked keychain,
# so it cannot run over SSH.
#
#   scripts/build-mac.sh             Everyday build. Signed with your best certificate, not notarized.
#                                    Opens on this Mac; takes about a minute.
#
# Both modes install the new build into /Applications (quitting a running copy first) and open it,
# so the copy you launch is always the one you just built. LT_NO_INSTALL=1 skips that.
#   scripts/build-mac.sh --release   Official release (version milestones such as 0.1.0 -> 1.0.0).
#                                    Developer ID signature, Apple notarization, disk image, and the
#                                    files the in-app updater downloads.
#
# Official releases need, once:
#   1. A Developer ID Application certificate in your keychain.
#   2. A notary login:  xcrun notarytool store-credentials "lute-and-tavern" --apple-id EMAIL --team-id TEAM_ID
#   3. An update-signing key. The script creates ~/.tauri/lute-and-tavern.key the first time and asks you for
#      a password. Back that file up: without it, installed copies can never receive another update.
# Set LT_UPDATE_BASE_URL to the web folder the release files will live in, e.g.
#   LT_UPDATE_BASE_URL=https://downloads.example.com/lute-and-tavern scripts/build-mac.sh --release
# Other overrides: LT_SIGN_IDENTITY (certificate name or part of it), LT_NOTARY_PROFILE (default lute-and-tavern).
set -euo pipefail
cd "${0:A:h}/../apps/desktop"
mode=${1:-local}
[[ "$mode" == "--release" ]] && mode=release
[[ "$mode" == "--local" ]] && mode=local
bundle_dir=../../target/release/bundle

# Replaces /Applications/Lute & Tavern.app with a fresh build and opens it.
install_app() {
  local src="$1" dst="/Applications/Lute & Tavern.app"
  if [[ -n "${LT_NO_INSTALL:-}" ]]; then open -R "$src"; return; fi
  if pgrep -f "Lute & Tavern.app/Contents/MacOS" >/dev/null; then
    print "Quitting the running copy..."
    osascript -e 'tell application "Lute & Tavern" to quit' >/dev/null 2>&1 || true
    for _ in {1..20}; do pgrep -f "Lute & Tavern.app/Contents/MacOS" >/dev/null || break; sleep 0.5; done
    pkill -f "Lute & Tavern.app/Contents/MacOS" 2>/dev/null || true
  fi
  rm -rf "$dst"
  ditto "$src" "$dst"
  codesign --verify --deep --strict "$dst"
  print "Installed in /Applications and opening it."
  open "$dst"
}

# Pick a certificate by its SHA-1 fingerprint: two certificates can share the same name,
# and codesign refuses a name that matches more than one.
ids=$(security find-identity -v -p codesigning)
pick() { print -r -- "$ids" | grep -F "\"$1" | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "\(.*\)"/\1 \2/p' | head -1; }
if [[ -n "${LT_SIGN_IDENTITY:-}" ]]; then
  chosen=$(print -r -- "$ids" | grep -F "$LT_SIGN_IDENTITY" | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "\(.*\)"/\1 \2/p' | head -1)
elif [[ "$mode" == release ]]; then
  chosen=$(pick "Developer ID Application:")
else
  chosen=$(pick "Developer ID Application:"); [[ -z "$chosen" ]] && chosen=$(pick "Apple Development:")
fi
if [[ -z "$chosen" ]]; then
  [[ "$mode" == release ]] && print "An official release needs a Developer ID Application certificate in your keychain." || print "No signing certificate found in your keychain."
  exit 1
fi
hash=${chosen%% *}
identity=${chosen#* }
version=$(node -p "require('./src-tauri/tauri.conf.json').version")
print "Lute & Tavern $version, ${mode} build, signing with: $identity"

[[ -d node_modules ]] || npm install --no-fund --no-audit
print "(Tauri may warn that it is skipping notarization. That is expected: this script notarizes release builds"
print " itself, with the notary login saved in your keychain, so no password ever sits in an environment variable.)"

if [[ "$mode" == local ]]; then
  APPLE_SIGNING_IDENTITY="$hash" npm run tauri build -- --bundles app
  app="$bundle_dir/macos/Lute & Tavern.app"
  codesign --verify --deep --strict "$app"
  install_app "$app"
  exit 0
fi

# ---------- official release ----------
profile="${LT_NOTARY_PROFILE:-lute-and-tavern}"
if ! xcrun notarytool history --keychain-profile "$profile" >/dev/null 2>&1; then
  print "No notary login named \"$profile\" in your keychain. Save it once with:"
  print "  xcrun notarytool store-credentials \"$profile\" --apple-id YOUR_APPLE_ID --team-id ${${identity##*\(}%\)}"
  exit 1
fi

key="$HOME/.tauri/lute-and-tavern.key"
if [[ ! -f "$key" ]]; then
  print "Creating the update-signing key at $key. Choose a password and keep it; back the file up."
  mkdir -p "${key:h}"
  npx tauri signer generate -w "$key"
fi
print -n "Password for the update-signing key: "
read -rs key_password
print

base="${LT_UPDATE_BASE_URL:-}"
base=${base%/}
export TAURI_SIGNING_PRIVATE_KEY="$key"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$key_password"
export LT_UPDATER_PUBKEY="$(cat "$key.pub")"
[[ -n "$base" ]] && export LT_UPDATE_URL="$base/latest.json"

APPLE_SIGNING_IDENTITY="$hash" npm run tauri build -- --bundles app,dmg --config '{"bundle":{"createUpdaterArtifacts":true}}'

app="$bundle_dir/macos/Lute & Tavern.app"
dmg=$(ls -t "$bundle_dir"/dmg/*.dmg | head -1)
tarball="$bundle_dir/macos/Lute & Tavern.app.tar.gz"
codesign --verify --deep --strict "$app"

print "Sending the disk image to Apple for notarization..."
xcrun notarytool submit "$dmg" --keychain-profile "$profile" --wait
xcrun stapler staple "$dmg"
xcrun stapler staple "$app"
spctl -a -vv "$app"

# Everything that gets uploaded goes into one folder.
out="$bundle_dir/release-$version"
rm -rf "$out" && mkdir -p "$out"
cp "$dmg" "$out/Lute-and-Tavern-$version-mac-arm64.dmg"
cp "$tarball" "$out/Lute-and-Tavern-$version-mac-arm64.app.tar.gz"
if [[ -n "$base" ]]; then
  VERSION="$version" URL="$base/Lute-and-Tavern-$version-mac-arm64.app.tar.gz" SIG="$(cat "$tarball.sig")" \
    node -e 'console.log(JSON.stringify({version: process.env.VERSION, notes: "", pub_date: new Date().toISOString(),
      platforms: {"darwin-aarch64": {signature: process.env.SIG, url: process.env.URL}}}, null, 2))' > "$out/latest.json"
  print "\nRelease ready in $(cd "$out" && pwd). Upload everything in it to $base/"
else
  print "\nRelease ready in $(cd "$out" && pwd)."
  print "No LT_UPDATE_BASE_URL was set, so there is no latest.json and this build has no built-in update address."
fi
install_app "$app"
open "$out"
