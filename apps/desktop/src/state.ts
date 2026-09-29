// App state: the library snapshot from Rust plus what the person is looking at.
import { api, type Snapshot, type Song, type WriterSettings } from './api';

export type View = { kind: 'all' | 'ai' | 'library' } | { kind: 'pl'; id: number } | { kind: 'album'; key: string } | { kind: 'apple' };
export type SideTab = 'library' | 'albums' | 'playlists' | 'genres' | 'apple';
export type Repeat = 'off' | 'all' | 'one';

export const state = {
  songs: [] as Song[],
  playlists: [] as Snapshot['playlists'],
  genres: [] as Snapshot['genres'],
  folders: [] as Snapshot['folders'],
  settings: {} as Record<string, unknown>,
  writer: { baseUrl: '', model: '', name: 'Bard', hasKey: false } as WriterSettings,
  view: { kind: 'all' } as View,
  genre: 'all',
  filter: 'all',
  sideTab: 'library' as SideTab,
  query: '',
  sort: 'recent',
};

let renderAll: () => void = () => {};
export const onRender = (fn: () => void) => (renderAll = fn);
export const rerender = () => renderAll();

export async function reload() {
  const snap = await api.library();
  state.songs = snap.songs;
  state.playlists = snap.playlists;
  state.genres = snap.genres;
  state.folders = snap.folders;
  state.settings = snap.settings;
}

export const setting = <T>(key: string, fallback: T): T => (state.settings[key] as T) ?? fallback;
export async function saveSetting(key: string, value: unknown) {
  state.settings[key] = value;
  await api.setSetting(key, value);
}

export const writerName = () => state.writer.name || 'Bard';

/** Where a written song stands: being written, lyrics ready for Suno, or it has audio (a take or stems). */
export const stage = (s: Song) => (s.status === 'writing' && !s.lyrics ? 'writing' : s.hasAudio ? 'audio' : 'ready');
export const libraryFolder = () => state.folders.find((f) => f.managed) ?? null;
export const byId = (id: number) => state.songs.find((s) => s.id === id);
export const gkey = (g: string) => (g || 'Unsorted').toLowerCase();

/** Genres that have songs, most used first; Unsorted last. */
/** Songs that belong in the main library views (Apple songs can be kept out in Settings). */
export const inMainLibrary = (s: Song) => !(s.provider === 'apple' && state.settings.showAppleInLibrary === false);

export function genresInUse() {
  const map = new Map<string, { name: string; n: number }>();
  for (const s of state.songs.filter(inMainLibrary)) {
    const k = gkey(s.genre);
    if (!map.has(k)) map.set(k, { name: s.genre || 'Unsorted', n: 0 });
    map.get(k)!.n++;
  }
  return [...map.entries()].sort(
    (a, b) => Number(a[0] === 'unsorted') - Number(b[0] === 'unsorted') || b[1].n - a[1].n || a[1].name.localeCompare(b[1].name),
  );
}

/** Songs without an album gather under this key in the album list. */
export const SINGLES = '';
export const albumKey = (s: Song) => (s.album.trim() ? `${s.album.trim().toLowerCase()}\u001f${s.artist.trim().toLowerCase()}` : SINGLES);

/** Albums with their songs; songs without an album form "Singles", listed first. */
export function albums() {
  const map = new Map<string, { key: string; name: string; artist: string; songs: Song[] }>();
  for (const s of state.songs.filter(inMainLibrary)) {
    const key = albumKey(s);
    if (!map.has(key)) map.set(key, { key, name: key === SINGLES ? 'Singles' : s.album.trim(), artist: key === SINGLES ? '' : s.artist, songs: [] });
    map.get(key)!.songs.push(s);
  }
  return [...map.values()].sort((a, b) => Number(b.key === SINGLES) - Number(a.key === SINGLES) || a.name.localeCompare(b.name) || a.artist.localeCompare(b.artist));
}

export const playable = (s: Song) => s.provider === 'apple' || s.hasAudio || s.notes.length > 0;

function inView(s: Song) {
  const v = state.view;
  if (v.kind === 'apple') return false;
  if (v.kind === 'ai') return s.source === 'ai';
  if (v.kind === 'library') return s.source === 'library' && s.provider === 'local';
  if (s.provider === 'apple' && state.settings.showAppleInLibrary === false && v.kind !== 'pl') return false;
  if (v.kind === 'pl') return !!state.playlists.find((p) => p.id === v.id)?.songs.includes(s.id);
  if (v.kind === 'album') return albumKey(s) === v.key;
  return true;
}

export function visible(): Song[] {
  const q = state.query.trim().toLowerCase();
  const list = state.songs.filter(
    (s) =>
      inView(s) &&
      (state.genre === 'all' || gkey(s.genre) === state.genre) &&
      (state.view.kind !== 'ai' || state.filter === 'all' || stage(s) === state.filter) &&
      (!q || [s.title, s.artist, s.album, s.genre, s.subGenre, s.lyrics, s.request ?? ''].join(' ').toLowerCase().includes(q)),
  );
  const v = state.view;
  switch (state.sort) {
    case 'title':
      return list.sort((a, b) => a.title.localeCompare(b.title));
    case 'artist':
      return list.sort((a, b) => a.artist.localeCompare(b.artist) || (a.album || '').localeCompare(b.album || '') || (a.track ?? 0) - (b.track ?? 0));
    case 'genre':
      return list.sort((a, b) => a.genre.localeCompare(b.genre) || a.title.localeCompare(b.title));
    default:
      if (v.kind === 'album') return list.sort((a, b) => (a.track ?? 999) - (b.track ?? 999) || a.title.localeCompare(b.title));
      if (v.kind === 'pl') {
        const order = state.playlists.find((p) => p.id === v.id)?.songs ?? [];
        return list.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id));
      }
      return list.sort((a, b) => b.addedAt - a.addedAt || b.id - a.id);
  }
}

export function viewName() {
  const v = state.view;
  if (v.kind === 'pl') return state.playlists.find((p) => p.id === v.id)?.name ?? 'Playlist';
  if (v.kind === 'album') return albums().find((a) => a.key === v.key)?.name ?? 'Album';
  if (v.kind === 'apple') return 'Apple Music';
  const base = { all: 'All music', ai: `Written with ${writerName()}`, library: 'My music' }[v.kind];
  const g = state.genre === 'all' ? '' : genresInUse().find(([k]) => k === state.genre)?.[1].name;
  return g ? `${base}, ${g}` : base;
}
