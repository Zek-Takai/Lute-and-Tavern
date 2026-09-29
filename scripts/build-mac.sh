#!/bin/zsh
# Builds Lute & Tavern for macOS and signs it with a certificate from your login keychain.
# Run it in Terminal on the Mac (signing needs your unlocked keychain, so it cannot run over SSH).
#
#   scripts/build-mac.sh            signs with your Developer ID Application certificate if you have one,
#                                   otherwise with your Apple Development certificate
#   LT_SIGN_IDENTITY="..." scripts/build-mac.sh    signs with the exact certificate you name
set -euo pipefail
cd "${0:A:h}/../apps/desktop"

identity="${LT_SIGN_IDENTITY:-}"
if [[ -z "$identity" ]]; then
  ids=$(security find-identity -v -p codesigning)
  identity=$(print -r -- "$ids" | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p' | head -1)
  [[ -z "$identity" ]] && identity=$(print -r -- "$ids" | sed -n 's/.*"\(Apple Development: .*\)"/\1/p' | head -1)
fi
if [[ -z "$identity" ]]; then
  print "No signing certificate found in your keychain."; exit 1
fi
print "Signing with: $identity"

[[ -d node_modules ]] || npm install --no-fund --no-audit
APPLE_SIGNING_IDENTITY="$identity" npm run tauri build -- --bundles app,dmg

app="../../target/release/bundle/macos/Lute & Tavern.app"
codesign --verify --deep --strict "$app"
print "\nDone. The app is in: $(cd "${app:h}" && pwd)/${app:t}"
print "The installer disk image is in: $(cd ../../target/release/bundle/dmg && pwd)"
open -R "$app"
