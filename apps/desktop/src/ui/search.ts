// The main search bar's source picker. "My music" filters the library as you type (ui/library.ts); other
// sources (Apple Music, Radio, the person's own servers) show a results dropdown: Enter or a click opens that
// source's page, the play button plays right away and leaves you where you are.
import { errText } from '../api';
import { appleStatus, searchApple, type AppleItem } from '../apple';
import { playAppleItem, showAppleItem, showAppleSearch } from './apple';
import { connections, dropResults, onConnectionsChange, openSourceSearch, type DropItem } from './sources';
import { rerender, saveSetting, state } from '../state';
import { $, esc, PLAY } from '../util';

/** 'library', 'apple', 'radio' or 'conn:<id>' for a connected server. */
type Source = string;
const placeholder = (s: Source) =>
  s === 'library' ? 'Search songs, artists, albums, lyrics'
  : s === 'apple' ? 'Search Apple Music: songs, albums, artists, playlists'
  : s === 'radio' ? 'Search radio stations: name, city, genre'
  : `Search ${connections.find((c) => `conn:${c.id}` === s)?.name ?? 'your server'}`;
let drops: DropItem[] = [];

function fillSources() {
  const sel = $<HTMLSelectElement>('#qSource');
  const keep = sel.value || (state.settings.searchSource as string) || 'library';
  sel.innerHTML = [['library', 'My music'], ['apple', 'Apple Music'], ['radio', 'Radio'], ...connections.map((c) => [`conn:${c.id}`, c.name])]
    .map(([v, l]) => `<option value="${esc(v)}">${esc(l)}</option>`).join('');
  sel.value = [...sel.options].some((o) => o.value === keep) ? keep : 'library';
  input().placeholder = placeholder(sel.value);
}

let timer = 0;
let seq = 0;
let items: AppleItem[] = [];

const input = () => $<HTMLInputElement>('#q');
const drop = () => $('#qDrop');
const source = () => $<HTMLSelectElement>('#qSource').value as Source;

function close() {
  drop().hidden = true;
  input().setAttribute('aria-expanded', 'false');
}

function row(it: AppleItem, i: number) {
  const kind = { songs: 'Song', albums: 'Album', playlists: 'Playlist', artists: 'Artist' }[it.type] ?? '';
  return `<div class="qr" role="option" data-i="${i}">
    ${it.art ? `<img src="${esc(it.art.replace('300x300', '72x72'))}" alt="">` : '<span class="ph"></span>'}
    <div class="tx"><b>${esc(it.name)}</b><span>${esc([kind, it.artist].filter(Boolean).join(', '))}</span></div>
    <button class="play" data-qplay="${i}" aria-label="Play ${esc(it.name)}" title="Play">${PLAY}</button></div>`;
}

async function lookUpOther(q: string) {
  const mine = ++seq;
  const d = drop();
  d.hidden = false;
  d.innerHTML = '<p class="muted small">Searching…</p>';
  try {
    const groups = await dropResults(source(), q);
    if (mine !== seq) return;
    drops = groups.flatMap((g) => g.items);
    let i = 0;
    d.innerHTML =
      groups.filter((g) => g.items.length).map((g) => `<h4>${esc(g.title)}</h4>` + g.items.map((it) => dropRow(it, i++)).join('')).join('') +
      (drops.length ? `<button class="more" data-qmore>See all results for "${esc(q)}"</button>` : `<p class="muted small">Nothing matches "${esc(q)}".</p>`);
  } catch (e) {
    if (mine === seq) d.innerHTML = `<p class="muted small">Could not search: ${esc(errText(e))}</p>`;
  }
}

function dropRow(it: DropItem, i: number) {
  return `<div class="qr" role="option" data-d="${i}">
    ${it.art ? `<img src="${esc(it.art)}" alt="">` : '<span class="ph"></span>'}
    <div class="tx"><b>${esc(it.name)}</b><span>${esc(it.sub)}</span></div>
    <button class="play" data-dplay="${i}" aria-label="Play ${esc(it.name)}" title="Play">${PLAY}</button></div>`;
}

async function lookUp(q: string) {
  if (source() !== 'apple') return lookUpOther(q);
  const mine = ++seq;
  const d = drop();
  d.hidden = false;
  input().setAttribute('aria-expanded', 'true');
  d.innerHTML = '<p class="muted small">Searching Apple Music…</p>';
  const st = await appleStatus().catch(() => null);
  if (!st?.ready) {
    d.innerHTML = '<p class="muted small">Apple Music needs a MusicKit key first. Add yours in Settings › Apple Music.</p>';
    return;
  }
  try {
    const r = await searchApple(q);
    if (mine !== seq) return; // a newer search is on its way
    const groups: [string, AppleItem[]][] = [['Songs', r.songs.slice(0, 6)], ['Albums', r.albums.slice(0, 3)], ['Artists', r.artists.slice(0, 3)], ['Playlists', r.playlists.slice(0, 3)]];
    items = groups.flatMap(([, g]) => g);
    let i = 0;
    d.innerHTML =
      groups.filter(([, g]) => g.length).map(([t, g]) => `<h4>${t}</h4>` + g.map((it) => row(it, i++)).join('')).join('') +
      (items.length ? `<button class="more" data-qmore>See all results for "${esc(q)}" in Apple Music</button>` : `<p class="muted small">Nothing on Apple Music matches "${esc(q)}".</p>`);
  } catch (e) {
    if (mine === seq) d.innerHTML = `<p class="muted small">Apple Music could not search: ${esc(errText(e))}</p>`;
  }
}

function openResults() {
  const q = input().value.trim();
  if (!q) return;
  close();
  if (source() === 'apple') showAppleSearch(q);
  else openSourceSearch(source(), q);
}

let syncedFor = '';

/** The picker follows the page: My music, Apple Music, Radio or the open server. A choice made by hand
 *  stays until the person moves to another page. */
export function syncSearchSource() {
  const sel = document.getElementById('qSource') as HTMLSelectElement | null;
  if (!sel) return;
  const v = state.view;
  const src = v.kind === 'apple' ? 'apple' : v.kind === 'radio' ? 'radio' : v.kind === 'remote' ? `conn:${v.id}` : 'library';
  const key = v.kind === 'remote' ? src : v.kind === 'apple' || v.kind === 'radio' ? v.kind : 'library';
  if (key === syncedFor) return;
  syncedFor = key;
  if (sel.value === src || ![...sel.options].some((o) => o.value === src)) return;
  sel.value = src;
  input().placeholder = placeholder(src);
  input().value = '';
  close();
}

export function initSearch() {
  const sel = $<HTMLSelectElement>('#qSource');
  fillSources();
  onConnectionsChange(fillSources);
  sel.onchange = () => {
    void saveSetting('searchSource', sel.value);
    input().placeholder = placeholder(source());
    close();
    // Leaving "My music": stop filtering the library by what was typed.
    if (source() !== 'library' && state.query) {
      state.query = '';
      rerender();
    } else if (source() === 'library') {
      state.query = input().value;
      rerender();
    }
    input().focus();
    if (source() !== 'library' && input().value.trim()) void lookUp(input().value.trim());
  };
  input().addEventListener('input', () => {
    if (source() === 'library') return;
    clearTimeout(timer);
    const q = input().value.trim();
    if (!q) return close();
    timer = window.setTimeout(() => void lookUp(q), 300);
  });
  input().addEventListener('keydown', (e) => {
    if (source() === 'library') return;
    if (e.key === 'Enter') {
      e.preventDefault();
      clearTimeout(timer);
      openResults();
    } else if (e.key === 'Escape') close();
  });
  input().addEventListener('focus', () => {
    if (source() !== 'library' && input().value.trim() && drop().innerHTML) drop().hidden = false;
  });
  drop().addEventListener('click', (e) => {
    const t = e.target as HTMLElement;
    const dp = t.closest<HTMLElement>('[data-dplay]');
    if (dp) {
      e.stopPropagation();
      drops[Number(dp.dataset.dplay)]?.play();
      return;
    }
    const dr = t.closest<HTMLElement>('[data-d]');
    if (dr) {
      close();
      drops[Number(dr.dataset.d)]?.open();
      return;
    }
    const play = t.closest<HTMLElement>('[data-qplay]');
    if (play) {
      e.stopPropagation();
      void playAppleItem(items[Number(play.dataset.qplay)]); // plays without leaving the page
      return;
    }
    if (t.closest('[data-qmore]')) return openResults();
    const r = t.closest<HTMLElement>('[data-i]');
    if (!r) return;
    const it = items[Number(r.dataset.i)];
    close();
    if (it.type === 'songs') showAppleSearch(input().value.trim());
    else showAppleItem(it);
  });
  document.addEventListener('mousedown', (e) => {
    if (!(e.target as HTMLElement).closest('.search')) close();
  });
}
