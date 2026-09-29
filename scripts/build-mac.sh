#!/bin/zsh
# Builds Lute & Tavern for macOS and signs it with a certificate from your login keychain.
# Run it in Terminal on the Mac (signing needs your unlocked keychain, so it cannot run over SSH).
#
# With a Developer ID certificate and a saved notary login, the disk image is also sent to Apple
# for notarization and the approval is stapled on, so it opens on any Mac. Save the login once with:
#   xcrun notarytool store-credentials "lute-and-tavern" --apple-id YOUR_APPLE_ID --team-id YOUR_TEAM_ID
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
dmg=$(ls -t ../../target/release/bundle/dmg/*.dmg | head -1)

profile="${LT_NOTARY_PROFILE:-lute-and-tavern}"
if [[ "$identity" == "Developer ID Application:"* ]]; then
  if xcrun notarytool history --keychain-profile "$profile" >/dev/null 2>&1; then
    print "Sending the disk image to Apple for notarization (usually a few minutes)..."
    xcrun notarytool submit "$dmg" --keychain-profile "$profile" --wait
    xcrun stapler staple "$dmg"
    xcrun stapler staple "$app"
    spctl -a -vv "$app"
  else
    print "\nSigned with Developer ID but not notarized: no notary login named \"$profile\" in your keychain."
    print "Save it once, then run this script again:"
    print "  xcrun notarytool store-credentials \"$profile\" --apple-id YOUR_APPLE_ID --team-id ${identity##*\(}" | tr -d ')'
  fi
fi
print "\nDone. The app is in: $(cd "${app:h}" && pwd)/${app:t}"
print "The installer disk image is in: $(cd ../../target/release/bundle/dmg && pwd)"
open -R "$app"
