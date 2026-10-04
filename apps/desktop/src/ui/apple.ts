// The Apple Music tab: connect, search the catalog, your Apple Music library, recently played, for you, charts.
import { api, errText, type Song } from '../api';
import {
  appleStatus, asExternal, charts, connectApple, isConnected, libraryPlaylists, librarySongs, musicKit, recentlyPlayed,
  recommendations, searchApple, songsOf, toSong, tracksOf, type AppleItem,
} from '../apple';
import { playSong, syncIcons } from '../player';
import { reload, rerender, state } from '../state';
import { $, esc, PLAY, toast } from '../util';

export type AppleSec = 'search' | 'songs' | 'playlists' | 'recent' | 'foryou' | 'charts' | 'open';
interface Page { title: string; sub?: string; songs: AppleItem[]; groups: { title: string; items: AppleItem[] }[] }

let sec: AppleSec = 'search';
let term = '';
let opened: AppleItem | null = null;
let page: Page | null = null;
let busy = false;
let myPlaylists: AppleItem[] = [];

/** Rows for the side panel when the Apple Music icon is open. */
export async function applePanel(): Promise<string> {
  const st = await appleStatus().catch(() => null);
  if (!st?.ready)
    return `<p class="muted small side-empty">Apple Music needs a MusicKit key. Add yours in Settings › Apple Music.</p>
      <button class="btn small" data-apple="settings">Open Settings</button>`;
  try {
    await musicKit();
  } catch (e) {
    return `<p class="muted small side-empty">${esc(errText(e))}</p>`;
  }
  if (!isConnected())
    return `<p class="muted small side-empty">Sign in with your Apple ID to play Apple Music here. You need an Apple Music subscription.</p>
      <button class="btn primary small" data-apple="connect">Connect Apple Music</button>`;
  if (!myPlaylists.length) myPlaylists = await libraryPlaylists().catch(() => []);
  const row = (k: AppleSec, label: string) =>
    `<button class="nav" data-asec="${k}" aria-pressed="${state.view.kind === 'apple' && sec === k}"><span>${label}</span></button>`;
  return (
    row('search', 'Search Apple Music') + row('foryou', 'For you') + row('recent', 'Recently played') + row('charts', 'Top charts') + row('songs', 'Your songs') +
    `<h3 class="side-sub">Your playlists</h3>` +
    myPlaylists.map((p, i) => `<button class="nav" data-apl="${i}" aria-pressed="${state.view.kind === 'apple' && sec === 'open' && opened?.id === p.id}"><span>${esc(p.name)}</span></button>`).join('')
  );
}

export async function handleApplePanelClick(t: HTMLElement, openSettings: () => void) {
  const a = t.closest<HTMLElement>('[data-apple]')?.dataset.apple;
  if (a === 'settings') return openSettings(), true;
  if (a === 'connect') {
    try {
      await connectApple();
      toast('Apple Music connected');
    } catch (e) {
      toast(`Apple Music: ${errText(e)}`);
    }
    return rerender(), true;
  }
  const s = t.closest<HTMLElement>('[data-asec]')?.dataset.asec as AppleSec | undefined;
  const pl = t.closest<HTMLElement>('[data-apl]')?.dataset.apl;
  if (!s && pl === undefined) return false;
  if (s) {
    sec = s;
    opened = null;
  } else {
    sec = 'open';
    opened = myPlaylists[Number(pl)];
  }
  state.view = { kind: 'apple' };
  page = null;
  rerender();
  return true;
}

/** Opens the Apple Music page where the person left it (the sidebar icon). */
export function openApplePage() {
  state.view = { kind: 'apple' };
  state.sideTab = 'apple';
  rerender();
}

/** Opens the Apple Music page on a search (from the main search bar). */
export function showAppleSearch(q: string) {
  term = q;
  sec = 'search';
  opened = null;
  page = null;
  state.view = { kind: 'apple' };
  state.sideTab = 'apple';
  rerender();
}

/** Opens an Apple Music album, playlist or artist page. */
export function showAppleItem(it: AppleItem) {
  opened = it;
  sec = 'open';
  page = null;
  state.view = { kind: 'apple' };
  state.sideTab = 'apple';
  rerender();
}

/** Plays a song, or a whole album, playlist or artist's top songs, without changing the page. */
export async function playAppleItem(it: AppleItem) {
  const songs = it.type === 'songs' || it.type === 'library-songs' ? songsOf([it]) : songsOf(await tracksOf(it));
  const list = songs.map((x) => savedFor(x) ?? x);
  if (!list.length) return toast('Nothing to play there.');
  playSong(list[0], list);
}

export const appleTitle = () =>
  ({ search: 'Apple Music', songs: 'Your songs', playlists: 'Your playlists', recent: 'Recently played', foryou: 'For you', charts: 'Top charts', open: opened?.name ?? 'Apple Music' })[sec];

export function appleHead() {
  return `<h2>${esc(appleTitle())}</h2>${opened?.artist ? `<span class="meta">${esc(opened.artist)}</span>` : ''}<span class="src src-apple">Apple Music</span>
    ${sec === 'search' ? `<form id="aSearchForm" class="row" style="flex:1 1 260px"><input class="inline-input" id="aSearch" type="search" placeholder="Search songs, albums, artists, playlists" value="${esc(term)}" style="flex:1"><button class="btn small">Search</button></form>` : ''}
    ${page?.songs.length ? `<button class="btn small" id="aPlayAll">${PLAY.replace('<svg', '<svg width="12" height="12"')}Play all</button>` : ''}`;
}

async function load(): Promise<Page> {
  switch (sec) {
    case 'search': {
      if (!term.trim()) return { title: 'Apple Music', songs: [], groups: [] };
      const r = await searchApple(term.trim());
      return { title: 'Apple Music', songs: r.songs, groups: [{ title: 'Albums', items: r.albums }, { title: 'Artists', items: r.artists }, { title: 'Playlists', items: r.playlists }] };
    }
    case 'songs': return { title: 'Your songs', songs: await librarySongs(), groups: [] };
    case 'recent': {
      const items = await recentlyPlayed();
      return { title: 'Recently played', songs: items.filter((i: AppleItem) => i.type.endsWith('songs')), groups: [{ title: 'Albums and playlists', items: items.filter((i: AppleItem) => !i.type.endsWith('songs')) }] };
    }
    case 'foryou': {
      const items = await recommendations();
      return { title: 'For you', songs: items.filter((i: AppleItem) => i.type.endsWith('songs')), groups: [{ title: 'Picked for you', items: items.filter((i: AppleItem) => !i.type.endsWith('songs')) }] };
    }
    case 'charts': {
      const c = await charts();
      return { title: 'Top charts', songs: c.songs, groups: [{ title: 'Albums', items: c.albums }, { title: 'Playlists', items: c.playlists }] };
    }
    case 'open': return { title: opened?.name ?? '', songs: opened ? await tracksOf(opened) : [], groups: [] };
    default: return { title: '', songs: [], groups: [] };
  }
}

const savedFor = (s: Song) => state.songs.find((x) => x.provider === 'apple' && x.providerId === s.providerId);

function songCard(it: AppleItem, i: number) {
  const s = toSong(it);
  const saved = savedFor(s);
  return `<article class="card apple" data-ai="${i}">
    <div class="cover" style="${it.art ? `background-image:url('${it.art}')` : ''}"></div>
    <div class="body"><h3>${esc(it.name)}</h3><div class="meta">${esc([it.artist, it.raw.attributes?.albumName].filter(Boolean).join(', '))}</div>
      <div class="foot"><span class="src src-apple">Apple Music</span><span style="margin-right:auto"></span>
        <button class="btn small" data-asave="${i}" ${saved ? 'disabled' : ''}>${saved ? 'Saved' : '+ Save'}</button>
        <button class="play" data-aplay="${i}" data-play="${s.id}" aria-label="Play">${PLAY}</button></div></div></article>`;
}
function boxCard(it: AppleItem, g: number, i: number) {
  return `<article class="card apple" data-open="${g}:${i}" tabindex="0" role="button" aria-label="Open ${esc(it.name)}">
    <div class="cover" style="${it.art ? `background-image:url('${it.art}')` : ''}"></div>
    <div class="body"><h3>${esc(it.name)}</h3><div class="meta">${esc(it.artist)}</div>
      <div class="foot"><span class="src src-apple">Apple Music</span></div></div></article>`;
}

/** Draws the Apple Music page into the main grid. */
export async function renderAppleGrid() {
  const grid = $('#grid');
  if (!isConnected()) {
    grid.innerHTML = '<p class="empty">Connect Apple Music from the panel on the left to search and play here.</p>';
    return;
  }
  if (!page && !busy) {
    busy = true;
    grid.innerHTML = '<p class="empty">Loading Apple Music…</p>';
    try {
      page = await load();
    } catch (e) {
      busy = false;
      grid.innerHTML = `<p class="empty">Apple Music could not load this: ${esc(errText(e))}</p>`;
      return;
    }
    busy = false;
    rerender();
    return;
  }
  if (!page) return;
  const parts: string[] = [];
  if (page.songs.length) parts.push(page.songs.map(songCard).join(''));
  page.groups.forEach((g, gi) => {
    if (g.items.length) parts.push(`<h3 class="grid-sub">${esc(g.title)}</h3>` + g.items.map((it, i) => boxCard(it, gi, i)).join(''));
  });
  grid.innerHTML = parts.join('') || `<p class="empty">${sec === 'search' ? 'Search for any song, album, artist or playlist on Apple Music.' : 'Nothing here yet.'}</p>`;
  syncIcons();
}

export function initApple() {
  const grid = $('#grid');
  grid.addEventListener('click', async (e) => {
    if (state.view.kind !== 'apple' || !page) return;
    const t = e.target as HTMLElement;
    const play = t.closest<HTMLElement>('[data-aplay]');
    if (play) {
      e.stopImmediatePropagation();
      const list = songsOf(page.songs);
      const s = list[Number(play.dataset.aplay)];
      if (s) playSong(savedFor(s) ?? s, list.map((x) => savedFor(x) ?? x));
      return;
    }
    const save = t.closest<HTMLElement>('[data-asave]');
    if (save) {
      e.stopImmediatePropagation();
      const s = toSong(page.songs[Number(save.dataset.asave)]);
      try {
        await api.saveExternalSong(asExternal(s));
        await reload();
        rerender();
        toast(`Saved "${s.title}" to your library`);
      } catch (err) {
        toast(errText(err));
      }
      return;
    }
    const box = t.closest<HTMLElement>('[data-open]');
    if (box) {
      e.stopImmediatePropagation();
      const [g, i] = box.dataset.open!.split(':').map(Number);
      opened = page.groups[g].items[i];
      sec = 'open';
      page = null;
      rerender();
    }
  }, true);
  $('#viewHead').addEventListener('submit', (e) => {
    if ((e.target as HTMLElement).id !== 'aSearchForm') return;
    e.preventDefault();
    term = $<HTMLInputElement>('#aSearch').value;
    page = null;
    rerender();
  });
  $('#viewHead').addEventListener('click', (e) => {
    if (!(e.target as HTMLElement).closest('#aPlayAll') || !page) return;
    const list = songsOf(page.songs).map((x) => savedFor(x) ?? x);
    if (list.length) playSong(list[0], list);
  });
}
