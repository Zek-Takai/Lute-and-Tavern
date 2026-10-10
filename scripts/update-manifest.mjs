#!/usr/bin/env node
// Adds (or replaces) one platform's entry in a release's latest.json, the file the app's updater reads.
// Mac and Linux builds each add their own entry to the same file, so one address serves every platform.
//
//   node scripts/update-manifest.mjs <latest.json> <version> <platform> <download url> <signature file>
//
// platform is Tauri's name for it: darwin-aarch64, linux-x86_64 (AppImage), linux-x86_64-deb, linux-x86_64-rpm.
// A latest.json for an older version is started over, so it never mixes versions.
import fs from 'node:fs';

const [file, version, platform, url, sigFile] = process.argv.slice(2);
if (!file || !version || !platform || !url || !sigFile) {
  console.error('Usage: node scripts/update-manifest.mjs <latest.json> <version> <platform> <url> <signature file>');
  process.exit(2);
}
let manifest = {};
try {
  manifest = JSON.parse(fs.readFileSync(file, 'utf8'));
} catch {}
if (manifest.version !== version) manifest = { version, notes: '', pub_date: new Date().toISOString(), platforms: {} };
manifest.platforms ??= {};
manifest.platforms[platform] = { signature: fs.readFileSync(sigFile, 'utf8').trim(), url };
manifest.pub_date = new Date().toISOString();
fs.writeFileSync(file, JSON.stringify(manifest, null, 2) + '\n');
console.log(`${file}: ${version}, platforms: ${Object.keys(manifest.platforms).join(', ')}`);
