// Apple Music through Apple's MusicKit on the Web. Playback is Apple's own protected player, so these songs
// never reach the app's audio graph (no visualizer reaction, no stems). Sign-in tokens MusicKit keeps go to
// the system key store (see secureStorage.ts); Apple ID cookies are wiped after sign-in.
import { invoke } from '@tauri-apps/api/core';
import type { Song } from './api';

/* eslint-disable @typescript-eslint/no-explicit-any */
declare global {
  interface Window { MusicKit: any }
}

export interface AppleStatus { ready: boolean; builtIn: boolean; keyId: string; teamId: string }
export const appleStatus = () => invoke<AppleStatus>('musickit_status');
export const setAppleKey = (path: string, keyId: string, teamId: string) => invoke<void>('musickit_set_key', { path, keyId, teamId });
export const forgetAppleKey = () => invoke<void>('musickit_forget_key');

let mk: any = null;
let loading: Promise<any> | null = null;
const listeners = new Set<() => void>();
export const onAppleChange = (fn: () => void) => listeners.add(fn);
const changed = () => listeners.forEach((f) => f());

/** Loads and configures MusicKit once. Fails with a readable message when no developer token exists. */
export function musicKit(): Promise<any> {
  if (mk) return Promise.resolve(mk);
  loading ??= (async () => {
    const token = await invoke<string>('musickit_token');
    if (!window.MusicKit) {
      await new Promise<void>((resolve, reject) => {
        document.addEventListener('musickitloaded', () => resolve(), { once: true });
        const s = document.createElement('script');
        s.src = 'https://js-cdn.music.apple.com/musickit/v3/musickit.js';
        s.async = true;
        s.onerror = () => reject(new Error('Apple Music could not load. Check the internet connection.'));
        document.head.appendChild(s);
      });
    }
    await window.MusicKit.configure({ developerToken: token, app: { name: 'Lute & Tavern', build: '0.1.0' } });
    mk = window.MusicKit.getInstance();
    mk.addEventListener('authorizationStatusDidChange', changed);
    return mk;
  })().catch((e) => {
    loading = null;
    throw e;
  });
  return loading;
}

export const isConnected = () => !!mk?.isAuthorized;

/** Apple ID sign-in in Apple's own window. Afterwards the sign-in cookies are wiped from the app. */
export async function connectApple() {
  const m = await musicKit();
  try {
    await m.authorize();
  } finally {
    await invoke('close_signin_windows').catch(() => {});
  }
  await invoke('clear_web_data').catch(() => {});
  changed();
}

export async function disconnectApple() {
  const m = await musicKit();
  await m.unauthorize();
  await invoke('clear_web_data').catch(() => {});
  changed();
}

// ---------- catalog and library ----------

export type AppleKind = 'songs' | 'albums' | 'playlists' | 'library-songs' | 'library-playlists' | 'library-albums' | 'stations' | 'artists';
export interface AppleItem { id: string; type: string; name: string; artist: string; art: string | null; raw: any }

const art = (a: any, size = 300) => (a?.artwork?.url ? String(a.artwork.url).replace('{w}', String(size)).replace('{h}', String(size)) : null);
export const artUrl = (template: string | null | undefined, size = 300) => (template ? template.replace('{w}', String(size)).replace('{h}', String(size)) : null);

function item(r: any): AppleItem {
  const a = r.attributes ?? {};
  return { id: r.id, type: r.type, name: a.name ?? 'Untitled', artist: a.artistName ?? a.curatorName ?? '', art: art(a), raw: r };
}

async function get(path: string, params: Record<string, unknown> = {}) {
  const m = await musicKit();
  const { data } = await m.api.music(path, params);
  return data;
}

export async function searchApple(term: string) {
  const d = await get('/v1/catalog/{{storefrontId}}/search', { term, types: 'songs,albums,playlists,artists', limit: 25 });
  const r = d.results ?? {};
  return {
    songs: (r.songs?.data ?? []).map(item),
    albums: (r.albums?.data ?? []).map(item),
    playlists: (r.playlists?.data ?? []).map(item),
    artists: (r.artists?.data ?? []).map(item),
  };
}

export async function librarySongs(): Promise<AppleItem[]> {
  return ((await get('/v1/me/library/songs', { limit: 100 })).data ?? []).map(item);
}
export async function libraryPlaylists(): Promise<AppleItem[]> {
  return ((await get('/v1/me/library/playlists', { limit: 100 })).data ?? []).map(item);
}
export async function recentlyPlayed(): Promise<AppleItem[]> {
  return ((await get('/v1/me/recent/played', { limit: 25 })).data ?? []).map(item);
}
export async function recommendations() {
  const d = (await get('/v1/me/recommendations', { limit: 10 })).data ?? [];
  return d.flatMap((rec: any) => (rec.relationships?.contents?.data ?? []).map(item));
}
export async function charts() {
  const d = await get('/v1/catalog/{{storefrontId}}/charts', { types: 'songs,albums,playlists', limit: 25 });
  const c = d.results ?? {};
  return {
    songs: (c.songs?.[0]?.data ?? []).map(item),
    albums: (c.albums?.[0]?.data ?? []).map(item),
    playlists: (c.playlists?.[0]?.data ?? []).map(item),
  };
}

/** Songs inside an album or playlist (catalog or the person's library). */
export async function tracksOf(it: AppleItem) {
  const path =
    it.type === 'albums' ? `/v1/catalog/{{storefrontId}}/albums/${it.id}/tracks`
    : it.type === 'playlists' ? `/v1/catalog/{{storefrontId}}/playlists/${it.id}/tracks`
    : it.type === 'library-playlists' ? `/v1/me/library/playlists/${it.id}/tracks`
    : it.type === 'library-albums' ? `/v1/me/library/albums/${it.id}/tracks`
    : it.type === 'artists' ? `/v1/catalog/{{storefrontId}}/artists/${it.id}/view/top-songs`
    : null;
  if (!path) return [];
  return ((await get(path, { limit: 100 })).data ?? []).map(item);
}

const isSong = (it: AppleItem) => it.type === 'songs' || it.type === 'library-songs';

/** An Apple song as a library-shaped Song (not saved: negative id) so it can play and queue. */
export function toSong(it: AppleItem, saved?: Song): Song {
  if (saved) return saved;
  const a = it.raw.attributes ?? {};
  const playId = a.playParams?.catalogId ?? a.playParams?.id ?? it.id;
  return {
    id: -Math.abs(hash(playId)), source: 'library', provider: 'apple', providerId: String(playId), artUrl: a.artwork?.url ?? null,
    title: it.name, artist: it.artist, album: a.albumName ?? '', year: String(a.releaseDate ?? '').slice(0, 4), genre: a.genreNames?.[0] ?? 'Unsorted',
    track: a.trackNumber ?? null, duration: a.durationInMillis ? a.durationInMillis / 1000 : null,
    hasAudio: false, hasFile: false, stems: 0, songDir: null, art: null, addedAt: 0, updatedAt: 0,
    request: null, status: null, subGenre: '', key: '', bpm: 0, style: '', exclude: '', lyrics: '', masterTake: null, notes: [], chords: [],
  };
}
export const songsOf = (items: AppleItem[]) => items.filter(isSong).map((it) => toSong(it));

function hash(s: string) {
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) | 0;
  return h || 1;
}

// ---------- the person's Apple Music library in My music ----------

/** Every page of a library list (Apple gives at most 100 at a time). */
async function paged(path: string): Promise<any[]> {
  const out: any[] = [];
  for (let offset = 0; ; ) {
    const d = await get(path, { limit: 100, offset });
    const rows = d.data ?? [];
    out.push(...rows);
    if (!d.next || !rows.length) return out;
    offset += rows.length;
  }
}

export interface ExternalSong {
  provider: 'apple'; providerId: string; title: string; artist: string; album: string; genre: string; year: string;
  track: number | null; duration: number | null; artUrl: string | null; added: number | null;
}

const secs = (iso?: string) => (iso && !Number.isNaN(Date.parse(iso)) ? Math.floor(Date.parse(iso) / 1000) : null);

/** An Apple song (catalog or library) in the form the library saves. */
export function externalOf(r: any): ExternalSong {
  const a = r.attributes ?? {};
  const pp = a.playParams ?? {};
  return {
    provider: 'apple', providerId: String(pp.catalogId ?? pp.id ?? r.id), title: a.name ?? 'Untitled', artist: a.artistName ?? '', album: a.albumName ?? '',
    genre: a.genreNames?.[0] ?? 'Unsorted', year: String(a.releaseDate ?? '').slice(0, 4), track: a.trackNumber ?? null,
    duration: a.durationInMillis ? a.durationInMillis / 1000 : null, artUrl: a.artwork?.url ?? null, added: secs(a.dateAdded),
  };
}
const playable = (r: any) => !!r.attributes?.playParams;

export interface ApplePull { songs: number; added: number; haveOwn: number; removed: number; playlists: number; playlistsUpdated: number; unreadable: string[] }

/** Reads the person's Apple Music songs and playlists and brings them into My music. */
export async function pullAppleLibrary(progress: (m: string) => void): Promise<ApplePull> {
  const m = await musicKit();
  if (!m.isAuthorized) throw new Error('Connect Apple Music first (Settings › Apple Music).');
  progress('Reading your Apple Music songs…');
  const songs = (await paged('/v1/me/library/songs')).filter(playable).map(externalOf);
  progress(`Read ${songs.length} songs. Reading your playlists…`);
  const lists = await paged('/v1/me/library/playlists');
  const playlists: { id: string; name: string; modified: number | null; tracks: ExternalSong[] }[] = [];
  const unreadable: string[] = [];
  for (const [i, p] of lists.entries()) {
    const name = p.attributes?.name ?? 'Playlist';
    progress(`Reading playlist ${i + 1} of ${lists.length}: ${name}`);
    let tracks: any[];
    try {
      tracks = await paged(`/v1/me/library/playlists/${p.id}/tracks`);
    } catch (e: any) {
      // Apple answers "not found" for an empty playlist; anything else is reported and that playlist is left as it is
      if (e?.errorCode === 'NOT_FOUND' || /not found|404/i.test(String(e?.message ?? e))) tracks = [];
      else {
        unreadable.push(name);
        continue;
      }
    }
    playlists.push({ id: p.id, name, modified: secs(p.attributes?.lastModifiedDate), tracks: tracks.filter(playable).map(externalOf) });
  }
  progress('Adding them to My music…');
  const r = await invoke<Omit<ApplePull, 'unreadable'>>('pull_apple_library', { songs, playlists });
  return { ...r, unreadable };
}

const words = (s: string) => s.toLowerCase().replace(/\(.*?\)|\[.*?\]/g, ' ').split(/[^\p{L}\p{N}]+/u).filter((w) => w && w !== 'the');
function alike(a: string, b: string) {
  const x = words(a), y = words(b);
  if (!x.length || !y.length) return 0;
  return x.filter((w) => y.includes(w)).length / Math.max(x.length, y.length);
}

/** The best Apple Music match for a song an import could not find (the person's Apple library first, then the catalog). */
export async function findOnApple(e: { title: string; artist: string; album: string; duration: number | null }): Promise<ExternalSong | null> {
  const term = `${e.title} ${e.artist}`.trim();
  const tries = [
    async () => ((await get('/v1/me/library/search', { term, types: 'library-songs', limit: 10 })).results?.['library-songs']?.data ?? []) as any[],
    async () => ((await get('/v1/catalog/{{storefrontId}}/search', { term, types: 'songs', limit: 10 })).results?.songs?.data ?? []) as any[],
  ];
  for (const t of tries) {
    const rows = (await t().catch(() => [])).filter(playable);
    let best: { s: ExternalSong; score: number } | null = null;
    for (const r of rows) {
      const s = externalOf(r);
      const ti = alike(e.title, s.title);
      if (ti < 0.75) continue;
      const ar = e.artist ? alike(e.artist, s.artist) : 0.5;
      const sameLen = e.duration && s.duration ? Math.abs(e.duration - s.duration) <= 3 : false;
      if (ar < 0.5 && !sameLen) continue;
      const score = ti + ar + (sameLen ? 0.5 : 0) + (e.album && alike(e.album, s.album) > 0.6 ? 0.2 : 0);
      if (!best || score > best.score) best = { s, score };
    }
    if (best) return best.s;
  }
  return null;
}

// ---------- playback ----------

/** Newest Apple song handed to the player; an older one that finishes loading late must not touch the player. */
let appleReq = 0;

/** Plays one Apple song. `onEnd` runs when it finishes so the app's own queue moves on. `isCurrent` turns false
 *  once the person picks something else or presses Stop; a start that is no longer wanted then plays nothing. */
export async function playApple(s: Song, startAt: number, onEnd: () => void, isCurrent: () => boolean = () => true) {
  const m = await musicKit();
  if (!isCurrent()) return;
  if (!m.isAuthorized) await connectApple();
  if (!isCurrent()) return;
  const PS = window.MusicKit.PlaybackStates;
  const handler = ({ state }: { state: number }) => {
    if (state === PS.completed || state === PS.ended) {
      m.removeEventListener('playbackStateDidChange', handler);
      onEnd();
    }
  };
  m.removeEventListener('playbackStateDidChange', (m as any).__ltEnd);
  (m as any).__ltEnd = handler;
  m.addEventListener('playbackStateDidChange', handler);
  const req = ++appleReq;
  await m.setQueue({ song: s.providerId, startPlaying: true, startTime: startAt || 0 });
  if (isCurrent()) return;
  // Something else was picked while Apple queued this song. A newer Apple song owns the player; otherwise silence it.
  m.removeEventListener('playbackStateDidChange', handler);
  if (req === appleReq) await m.stop();
}

export const applePause = () => mk?.pause();
export const appleResume = () => mk?.play();
export const appleStop = () => { mk?.removeEventListener('playbackStateDidChange', mk?.__ltEnd); return mk?.stop(); };
export const appleSeek = (sec: number) => mk?.seekToTime(sec);
export const applePosition = () => ({ cur: mk?.currentPlaybackTime ?? 0, dur: mk?.currentPlaybackDuration ?? 0 });

/** The saved-library form of an Apple song, for "Save to library". */
export function asExternal(s: Song) {
  return { provider: 'apple', providerId: s.providerId!, title: s.title, artist: s.artist, album: s.album, genre: s.genre, year: s.year, track: s.track, duration: s.duration, artUrl: s.artUrl };
}
