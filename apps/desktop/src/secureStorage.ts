// Web storage without files: the page's localStorage lives in the system key store.
// Web SDKs (MusicKit) keep sign-in tokens in localStorage, which WebKit would otherwise write to disk
// in the app's data folder. The app itself keeps no settings here (they are in the database).
import { invoke } from '@tauri-apps/api/core';

const SLOT = 'web-storage';
const mem = new Map<string, string>();
let timer = 0;

function persist() {
  clearTimeout(timer);
  timer = window.setTimeout(() => {
    const data = mem.size ? invoke('secure_set', { name: SLOT, value: JSON.stringify(Object.fromEntries(mem)) }) : invoke('secure_delete', { name: SLOT });
    data.catch(() => {});
  }, 300);
}

export async function initSecureStorage() {
  const saved = await invoke<string | null>('secure_get', { name: SLOT }).catch(() => null);
  if (saved) {
    try {
      for (const [k, v] of Object.entries(JSON.parse(saved) as Record<string, string>)) mem.set(k, String(v));
    } catch {
      /* unreadable blob: start empty */
    }
  }
  const P = Storage.prototype;
  const real = { get: P.getItem, set: P.setItem, remove: P.removeItem, clear: P.clear, key: P.key };
  const length = Object.getOwnPropertyDescriptor(P, 'length')!;
  const ls = window.localStorage;
  // Anything already written to disk moves into the key store, and the disk copy is wiped.
  const n = length.get!.call(ls) as number;
  for (let i = 0; i < n; i++) {
    const k = real.key.call(ls, i);
    if (k !== null && !mem.has(k)) mem.set(k, real.get.call(ls, k) ?? '');
  }
  real.clear.call(ls);
  if (n) persist();

  const isLocal = (s: Storage) => s === ls;
  P.getItem = function (k: string) { return isLocal(this) ? (mem.get(String(k)) ?? null) : real.get.call(this, k); };
  P.setItem = function (k: string, v: string) { if (!isLocal(this)) return real.set.call(this, k, v); mem.set(String(k), String(v)); persist(); };
  P.removeItem = function (k: string) { if (!isLocal(this)) return real.remove.call(this, k); mem.delete(String(k)); persist(); };
  P.clear = function () { if (!isLocal(this)) return real.clear.call(this); mem.clear(); persist(); };
  P.key = function (i: number) { return isLocal(this) ? ([...mem.keys()][i] ?? null) : real.key.call(this, i); };
  Object.defineProperty(P, 'length', { configurable: true, get() { return isLocal(this) ? mem.size : length.get!.call(this); } });
}
