#!/usr/bin/env bash
# Builds Lute & Tavern for Linux: a .deb (Ubuntu, Debian), an .rpm (Fedora, openSUSE) and an AppImage (any
# distribution), in target/release/bundle/. Needs Rust, Node 20+ and the WebKitGTK 4.1 development libraries.
# The build carries no paths or names from this computer (see clean-paths.sh); the script checks that.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/clean-paths.sh
(cd apps/desktop && npm run tauri build -- --bundles deb,rpm,appimage)
if strings -n 6 target/release/lute-and-tavern | grep -qF "$HOME"; then
  echo "The build still contains this computer's home folder path; not safe to publish." >&2
  exit 1
fi
echo "Built, with no paths from this computer inside. Packages: target/release/bundle/{deb,rpm,appimage}/"
