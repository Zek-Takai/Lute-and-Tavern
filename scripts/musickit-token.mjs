#!/usr/bin/env node
// Makes an Apple Music (MusicKit) developer token for an official release: an ES256 JWT signed with the
// developer's MusicKit private key (the AuthKey_<KeyID>.p8 file). Apple allows at most about six months.
//
//   node scripts/musickit-token.mjs <Key ID> <Team ID> [days, default 180] < AuthKey_XXXXXXXXXX.p8
//
// Reads the key from standard input (as text, or hex as macOS `security -w` prints multi-line items) and
// prints only the token. The key itself is never printed or written anywhere.
import crypto from 'node:crypto';

const [keyId, teamId, daysArg] = process.argv.slice(2);
if (!/^[A-Z0-9]{10}$/.test(keyId ?? '') || !/^[A-Z0-9]{10}$/.test(teamId ?? '')) {
  console.error('Usage: node scripts/musickit-token.mjs <Key ID> <Team ID> [days] < AuthKey.p8  (both IDs are 10 characters)');
  process.exit(2);
}
const days = Math.min(Number(daysArg ?? 180), 180); // Apple's limit is 15,777,000 seconds (about 182 days)
let input = '';
for await (const chunk of process.stdin) input += chunk;
input = input.trim();
const pem = /^[0-9a-fA-F]+$/.test(input) ? Buffer.from(input, 'hex').toString('utf8') : input;
if (!pem.includes('PRIVATE KEY')) {
  console.error('That is not a MusicKit key (expected the contents of an AuthKey_….p8 file).');
  process.exit(1);
}
const b64url = (v) => Buffer.from(typeof v === 'string' ? v : JSON.stringify(v)).toString('base64url');
const now = Math.floor(Date.now() / 1000);
const data = `${b64url({ alg: 'ES256', kid: keyId })}.${b64url({ iss: teamId, iat: now, exp: now + days * 86400 })}`;
const sig = crypto.sign('sha256', Buffer.from(data), { key: pem, dsaEncoding: 'ieee-p1363' });
process.stdout.write(`${data}.${sig.toString('base64url')}`);
