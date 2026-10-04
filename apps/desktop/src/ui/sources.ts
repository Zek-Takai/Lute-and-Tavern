// Music from outside the library: internet radio (Radio Browser) and the person's own music servers
// (Subsonic API connections). Pages, sidebar panels, the search-bar dropdown hooks, and playback through
// the app's local stream relay.
import { listen } from '@tauri-apps/api/event';
import { api, errText, type Connection, type RemoteItem, type RemoteResults, type Song, type Station } from '../api';
import { aiHooks, playSong, setNowSub, streamSources } from '../player';
import { rerender, state } from '../state';
import { $, cssUrl, esc, hueOf, PLAY, toast } from '../util';
import { openSettings } from './settings';

// ---------- shared ----------

let relay = '';
export let connections: Connection[] = [];
/** Latest sync message per server (for Settings). */
export const syncStatus = new Map<number, { state: string; message: string }>();
const onConnections = new Set<() => void>();
export const onConnectionsChange = (fn: () => void) => onConnections.add(fn);

export async function loadConnections() {
  connections = await api.connections().catch(() => []);
  onConnections.forEach((f) => f());
}

function hash(s: string) {
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) | 0;
  return Math.abs(h) || 1;
}

/** Plain http images are blocked by the page's rules, so they come through the local relay. */
function viaRelay(url: string) {
  if (!url || !url.startsWith('http://') || !relay) return url;
  const b = btoa(String.fromCharCode(...new TextEncoder().encode(url))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  return relay + b;
}

/** A Song-shaped record for something that isn't in the library (negative id, not saved). */
function external(p: Partial<Song> & Pick<Song, 'id' | 'title' | 'provider' | 'providerId'>): Song {
  return {
    source: 'library', artUrl: null, artist: '', album: '', year: '', genre: '', track: null, duration: null,
    hasAudio: false, hasFile: false, stems: 0, songDir: null, art: null, addedAt: 0, updatedAt: 0,
    request: null, status: null, subGenre: '', key: '', bpm: 0, style: '', exclude: '', lyrics: '', masterTake: null, notes: [], chords: [],
    ...p,
  } as Song;
}

const cover = (art: string | null, title: string) => {
  if (art) return `<div class="cover" style="background-image:${esc(cssUrl(art))}"></div>`;
  const h = hueOf(title);
  return `<div class="cover" style="background-image:linear-gradient(135deg,hsl(${h} 55% 38%),hsl(${(h + 50) % 360} 60% 18%))">${esc((title[0] || '♪').toUpperCase())}</div>`;
};

// ---------- radio ----------

const stations = new Map<string, Station>();
export const stationSong = (st: Station) => {
  stations.set(st.uuid, st);
  const where = [st.country, st.tags.split(',').slice(0, 2).join(', ')].filter(Boolean).join(', ');
  return external({ id: -hash('radio:' + st.uuid), title: st.name, artist: where, genre: st.tags.split(',')[0] || 'Radio', provider: 'radio', providerId: st.uuid, artUrl: viaRelay(st.favicon) || null });
};
export const playStation = (st: Station) => playSong(stationSong(st), [stationSong(st)]);

type RadioMode = { mode: 'home' } | { mode: 'search'; q: string } | { mode: 'tag'; tag: string };
let radio: RadioMode = { mode: 'home' };
let radioData: { title: string; groups: { title: string; list: Station[] }[]; tags: string[] } | null = null;
/** The radio page being loaded, so a rerender mid-load doesn't start it twice. */
let radioLoading: RadioMode | null = null;
/** The person's saved stations for the side panel: read once, then kept up to date as hearts are toggled. */
let favStations: Station[] | null = null;
let favLoad: Promise<Station[]> | null = null;

/** Opens Radio where the person left it; the first time after launch that is Radio home. */
export function openRadio() {
  state.view = { kind: 'radio' };
  state.sideTab = 'radio';
  rerender();
}

/** Opens the last server page, or the first server's home; with no servers, just the panel. */
export function openServers() {
  state.sideTab = 'servers';
  if (remote && connections.some((c) => c.id === remote!.id)) state.view = { kind: 'remote', id: remote.id };
  else if (connections.length) return showRemote({ id: connections[0].id, mode: 'home' });
  rerender();
}

export function showRadio(m: RadioMode = { mode: 'home' }) {
  radio = m;
  radioData = null;
  state.view = { kind: 'radio' };
  state.sideTab = 'radio';
  rerender();
}

async function loadRadio() {
  if (radio.mode === 'home') {
    const h = await api.radioHome();
    favStations = h.favorites;
    return { title: 'Radio', tags: h.tags.slice(0, 30), groups: [{ title: 'Your stations', list: h.favorites }, { title: 'Recently played', list: h.recent }, { title: 'Popular around the world', list: h.top }] };
  }
  const list = radio.mode === 'search' ? await api.radioSearch(radio.q) : await api.radioSearch('', radio.tag);
  return { title: radio.mode === 'search' ? `Stations for "${radio.q}"` : `${radio.tag[0].toUpperCase()}${radio.tag.slice(1)} radio`, tags: [], groups: [{ title: '', list }] };
}

function stationCard(st: Station) {
  return `<article class="card station" data-st-uuid="${esc(st.uuid)}">${cover(viaRelay(st.favicon) || null, st.name)}
    <div class="body"><h3>${esc(st.name)}</h3><div class="meta">${esc([st.country, st.tags.split(',').slice(0, 3).join(', ')].filter(Boolean).join(', '))}</div>
      <div class="foot"><span class="src src-radio">${esc([st.codec, st.bitrate ? st.bitrate + 'k' : ''].filter(Boolean).join(' ') || 'Radio')}</span><span style="margin-right:auto"></span>
        <button class="btn small fav" data-fav="${esc(st.uuid)}" aria-pressed="${st.favorite}" title="${st.favorite ? 'Remove from your stations' : 'Save to your stations'}">${st.favorite ? '♥' : '♡'}</button>
        <button class="play" data-st-play="${esc(st.uuid)}" aria-label="Play ${esc(st.name)}">${PLAY}</button></div></div></article>`;
}

export function radioHead() {
  const q = radio.mode === 'search' ? radio.q : '';
  return `<h2>${esc(radioData?.title ?? 'Radio')}</h2>${radio.mode !== 'home' ? '<button class="btn small" data-radio-home>Radio home</button>' : ''}
    <form id="rSearchForm" class="row" style="flex:1 1 260px"><input class="inline-input" id="rSearch" type="search" placeholder="Find a station: name, city, genre" value="${esc(q)}" style="flex:1"><button class="btn small">Search</button></form>`;
}

export async function renderRadioGrid() {
  const grid = $('#grid');
  if (!radioData) {
    grid.innerHTML = '<p class="empty">Tuning in…</p>';
    if (radioLoading === radio) return;
    const want = (radioLoading = radio);
    let data: Awaited<ReturnType<typeof loadRadio>>;
    try {
      data = await loadRadio();
    } catch (e) {
      if (radio !== want) return;
      radioLoading = null;
      if (state.view.kind === 'radio') grid.innerHTML = `<p class="empty">The radio directory could not be reached: ${esc(errText(e))}</p>`;
      return;
    }
    // The person searched or opened another genre meanwhile: that page has its own load.
    if (radio !== want) return;
    radioLoading = null;
    radioData = data;
    rerender();
    return;
  }
  radioData.groups.forEach((g) => g.list.forEach((s) => stations.set(s.uuid, s)));
  const parts = radioData.groups
    .filter((g) => g.list.length)
    .map((g) => (g.title ? `<h3 class="grid-sub">${esc(g.title)}</h3>` : '') + g.list.map(stationCard).join(''));
  if (radioData.tags.length)
    parts.splice(radioData.groups[0].list.length ? 1 : 0, 0, `<h3 class="grid-sub">Genres</h3><div class="tag-row">${radioData.tags.map((t) => `<button class="chip" data-radio-tag="${esc(t)}">${esc(t)}</button>`).join('')}</div>`);
  grid.innerHTML = parts.join('') || '<p class="empty">No stations found. Try another name, city or genre.</p>';
}

export async function radioPanel() {
  // Read from the app once; later redraws use the kept list (radio home also asks the directory, which can be slow).
  favLoad ??= api.radioFavorites().then((f) => (favStations ??= f)).catch(() => [] as Station[]).finally(() => (favLoad = null));
  const favs = favStations ?? (await favLoad);
  favs.forEach((s) => stations.set(s.uuid, s));
  const on = state.view.kind === 'radio';
  return `<button class="nav" data-radio-home aria-pressed="${on && radio.mode === 'home'}"><span>Radio home</span></button>
    <h3 class="side-sub">Your stations</h3>` +
    (favs.map((s) => `<button class="nav" data-st-play="${esc(s.uuid)}" title="Play ${esc(s.name)}"><span>${esc(s.name)}</span></button>`).join('') ||
      '<p class="muted small side-empty">Tap ♡ on a station to keep it here.</p>');
}

// ---------- connected servers ----------

type Remote = { id: number; mode: 'home' } | { id: number; mode: 'search'; q: string } | { id: number; mode: 'open'; item: RemoteItem };
let remote: Remote | null = null;
let remoteData: { title: string; sub: string; songs: RemoteItem[]; groups: { title: string; items: RemoteItem[] }[] } | null = null;
/** The server page being loaded, so a rerender mid-load doesn't start it twice. */
let remoteLoading: Remote | null = null;
const remoteItems = new Map<string, RemoteItem>();

const conn = (id: number) => connections.find((c) => c.id === id);
export const remoteSong = (id: number, it: RemoteItem) =>
  external({ id: -hash(`remote:${id}:${it.id}`), title: it.name, artist: it.artist, album: it.album, year: it.year, genre: it.genre || 'Unsorted', track: it.track, duration: it.duration, provider: 'remote', providerId: `${id}:${it.id}`, artUrl: it.art });

export function showRemote(r: Remote) {
  remote = r;
  remoteData = null;
  state.view = { kind: 'remote', id: r.id };
  state.sideTab = 'servers';
  rerender();
}

/** Plays a song now, or all of an album's, playlist's or artist's songs, without changing the page. */
export async function playRemote(id: number, it: RemoteItem, list?: RemoteItem[]) {
  try {
    if (it.kind === 'song') {
      const songs = (list ?? [it]).map((x) => remoteSong(id, x));
      return playSong(remoteSong(id, it), songs);
    }
    const songs = (await api.remoteSongs(id, it.kind, it.id)).map((x) => remoteSong(id, x));
    if (!songs.length) return toast('Nothing to play there.');
    playSong(songs[0], songs);
  } catch (e) {
    toast(errText(e));
  }
}

async function loadRemote(r: Remote) {
  const c = conn(r.id);
  const name = c?.name ?? 'Server';
  if (r.mode === 'open') {
    const items = await api.remoteOpen(r.id, r.item.kind, r.item.id);
    if (r.item.kind === 'artist') return { title: r.item.name, sub: name, songs: [], groups: [{ title: 'Albums', items }] };
    return {
      title: r.item.name,
      sub: [r.item.artist, name].filter(Boolean).join(', '),
      songs: items.filter((i) => i.kind === 'song'),
      groups: [{ title: 'Folders', items: items.filter((i) => i.kind === 'folder') }],
    };
  }
  const res: RemoteResults = r.mode === 'search' ? await api.remoteSearch(r.id, r.q) : await api.remoteHome(r.id);
  return {
    title: r.mode === 'search' ? `"${r.q}" on ${name}` : name,
    sub: r.mode === 'home' ? (c?.baseUrl ?? '') : '',
    songs: res.songs,
    groups: [{ title: 'Folders', items: res.folders }, { title: 'Playlists', items: res.playlists }, { title: r.mode === 'home' ? 'Newest albums' : 'Albums', items: res.albums }, { title: 'Artists', items: res.artists }],
  };
}

export function remoteHead() {
  const q = remote?.mode === 'search' ? remote.q : '';
  return `<h2>${esc(remoteData?.title ?? conn(remote?.id ?? 0)?.name ?? 'Server')}</h2>${remoteData?.sub ? `<span class="meta">${esc(remoteData.sub)}</span>` : ''}
    ${remote && remote.mode !== 'home' ? '<button class="btn small" data-remote-home>Server home</button>' : ''}
    ${remoteData?.songs.length ? `<button class="btn small" data-remote-all>${PLAY.replace('<svg', '<svg width="12" height="12"')}Play all</button>` : ''}
    <form id="sSearchForm" class="row" style="flex:1 1 260px"><input class="inline-input" id="sSearch" type="search" placeholder="Search this server" value="${esc(q)}" style="flex:1"><button class="btn small">Search</button></form>`;
}

function remoteSongCard(it: RemoteItem, i: number) {
  return `<article class="card" data-r-song="${i}">${cover(it.art, it.name)}
    <div class="body"><h3>${esc(it.name)}</h3><div class="meta">${esc([it.artist, it.album].filter(Boolean).join(', '))}</div>
      <div class="foot"><span class="src src-remote">${esc(conn(remote!.id)?.name ?? 'Server')}</span><span style="margin-right:auto"></span>
        <button class="play" data-r-play-song="${i}" aria-label="Play ${esc(it.name)}">${PLAY}</button></div></div></article>`;
}

function remoteBox(it: RemoteItem) {
  remoteItems.set(`${it.kind}:${it.id}`, it);
  const sub = it.kind === 'artist' ? `${it.count ?? ''} albums` : it.kind === 'playlist' ? `${it.count ?? ''} songs` : it.kind === 'folder' ? 'Folder' : [it.artist, it.year].filter(Boolean).join(', ');
  return `<article class="card" data-r-open="${esc(it.kind + ':' + it.id)}" tabindex="0" role="button" aria-label="Open ${esc(it.name)}">${cover(it.art, it.name)}
    <div class="body"><h3>${esc(it.name)}</h3><div class="meta">${esc(sub)}</div>
      <div class="foot"><span class="src src-remote">${esc(it.kind)}</span><span style="margin-right:auto"></span>
        <button class="play" data-r-play="${esc(it.kind + ':' + it.id)}" aria-label="Play ${esc(it.name)}">${PLAY}</button></div></div></article>`;
}

export async function renderRemoteGrid() {
  const grid = $('#grid');
  if (!remote) return;
  if (!remoteData) {
    grid.innerHTML = '<p class="empty">Loading from your server…</p>';
    if (remoteLoading === remote) return;
    const want = (remoteLoading = remote);
    let data: Awaited<ReturnType<typeof loadRemote>>;
    try {
      data = await loadRemote(want);
    } catch (e) {
      if (remote !== want) return;
      remoteLoading = null;
      if (state.view.kind === 'remote') grid.innerHTML = `<p class="empty">${esc(errText(e))}</p>`;
      return;
    }
    // Another server or page was opened meanwhile: these items belong to the old one, so they are dropped.
    if (remote !== want) return;
    remoteLoading = null;
    remoteData = data;
    rerender();
    return;
  }
  const parts: string[] = [];
  if (remoteData.songs.length) parts.push(remoteData.songs.map(remoteSongCard).join(''));
  for (const g of remoteData.groups) if (g.items.length) parts.push(`<h3 class="grid-sub">${esc(g.title)}</h3>` + g.items.map(remoteBox).join(''));
  grid.innerHTML = parts.join('') || '<p class="empty">Nothing here.</p>';
}

export function serversPanel() {
  const v = state.view;
  return (
    connections
      .map((c) => `<button class="nav" data-remote-open="${c.id}" aria-pressed="${v.kind === 'remote' && v.id === c.id}"><span>${esc(c.name)}</span></button>`)
      .join('') || '<p class="muted small side-empty">Connect music you keep elsewhere: Plex, Jellyfin or Emby, a music server (Nextcloud Music, Navidrome and others) or a plain WebDAV folder.</p>'
  ) + '<button class="btn small" data-add-server>Add a server…</button>';
}

// ---------- search-bar dropdown ----------

export interface DropItem { name: string; sub: string; art: string | null; play: () => void; open: () => void }

/** Top matches for the search bar's dropdown, grouped. */
export async function dropResults(source: string, q: string): Promise<{ title: string; items: DropItem[] }[]> {
  if (source === 'radio') {
    const list = await api.radioSearch(q, '', '', 12);
    list.forEach((s) => stations.set(s.uuid, s));
    return [{ title: 'Stations', items: list.map((s) => ({ name: s.name, sub: [s.country, s.tags.split(',')[0]].filter(Boolean).join(', '), art: viaRelay(s.favicon) || null, play: () => playStation(s), open: () => showRadio({ mode: 'search', q }) })) }];
  }
  const id = Number(source.slice(5));
  const r = await api.remoteSearch(id, q);
  const item = (it: RemoteItem, list?: RemoteItem[]): DropItem => ({
    name: it.name, sub: [it.kind[0].toUpperCase() + it.kind.slice(1), it.artist].filter(Boolean).join(', '), art: it.art,
    play: () => void playRemote(id, it, list),
    open: () => (it.kind === 'song' ? showRemote({ id, mode: 'search', q }) : showRemote({ id, mode: 'open', item: it })),
  });
  return [
    { title: 'Songs', items: r.songs.slice(0, 6).map((s) => item(s, r.songs)) },
    { title: 'Albums', items: r.albums.slice(0, 3).map((a) => item(a)) },
    { title: 'Artists', items: r.artists.slice(0, 3).map((a) => item(a)) },
    { title: 'Playlists', items: r.playlists.slice(0, 3).map((p) => item(p)) },
    { title: 'Folders', items: r.folders.slice(0, 4).map((f) => item(f)) },
  ];
}

export function openSourceSearch(source: string, q: string) {
  if (source === 'radio') return showRadio({ mode: 'search', q });
  showRemote({ id: Number(source.slice(5)), mode: 'search', q });
}

// ---------- wiring ----------

/** Handles clicks in the sidebar panels for Radio and Servers. Returns true when it handled one. */
export function handleSourcePanelClick(t: HTMLElement) {
  if (t.closest('[data-radio-home]')) return showRadio(), true;
  const sp = t.closest<HTMLElement>('[data-st-play]')?.dataset.stPlay;
  if (sp && stations.get(sp)) return playStation(stations.get(sp)!), true;
  const ro = t.closest<HTMLElement>('[data-remote-open]')?.dataset.remoteOpen;
  if (ro) return showRemote({ id: Number(ro), mode: 'home' }), true;
  if (t.closest('[data-add-server]')) return void openSettings('servers'), true;
  return false;
}

export async function initSources() {
  relay = await api.relayBase().catch(() => '');
  await loadConnections();
  streamSources.radio = async (s) => {
    const st = stations.get(s.providerId ?? '');
    if (!st) throw new Error('station details are missing');
    return api.radioPlay(st);
  };
  streamSources.remote = async (s) => {
    const [id, song] = (s.providerId ?? '').split(/:(.*)/s);
    return api.remoteStream(Number(id), song);
  };
  aiHooks.play_station = (cmd) => playStation(cmd.station as Station);
  await listen<{ state: string; message: string }>('backup', (e) => {
    const el = document.getElementById('bStatus');
    if (el) el.textContent = e.payload.message;
    if (e.payload.state !== 'running') {
      toast(e.payload.message);
      const b = document.getElementById('bNow') as HTMLButtonElement | null;
      if (b) b.disabled = false;
    }
  });
  await listen<{ id: number; state: string; message: string }>('remote-sync', (e) => {
    syncStatus.set(e.payload.id, { state: e.payload.state, message: e.payload.message });
    const el = document.querySelector<HTMLElement>(`[data-sync-status="${e.payload.id}"]`);
    if (el) el.textContent = e.payload.message;
    const name = connections.find((c) => c.id === e.payload.id)?.name ?? 'Server';
    if (e.payload.state === 'done') toast(`${name}: ${e.payload.message}`);
    if (e.payload.state === 'error') toast(`${name}: ${e.payload.message}`);
  });
  // Station song titles arrive from the relay while a station plays.
  await listen<{ stream: string; title: string }>('radio-title', (e) => {
    for (const st of stations.values()) {
      if (e.payload.stream.includes(btoa(String.fromCharCode(...new TextEncoder().encode(st.url))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, ''))) {
        setNowSub(stationSong(st).id, `${e.payload.title} · ${st.name}`);
      }
    }
  });

  const grid = $('#grid');
  grid.addEventListener('click', async (e) => {
    const t = e.target as HTMLElement;
    if (state.view.kind === 'radio') {
      const play = t.closest<HTMLElement>('[data-st-play]')?.dataset.stPlay;
      const fav = t.closest<HTMLElement>('[data-fav]')?.dataset.fav;
      const tag = t.closest<HTMLElement>('[data-radio-tag]')?.dataset.radioTag;
      if (play || fav || tag) e.stopImmediatePropagation();
      if (play && stations.get(play)) playStation(stations.get(play)!);
      if (fav && stations.get(fav)) {
        const st = stations.get(fav)!;
        try {
          await api.radioFavorite({ ...st, favorite: !st.favorite }, !st.favorite);
        } catch (err) {
          return toast(errText(err));
        }
        st.favorite = !st.favorite;
        if (favStations) favStations = st.favorite ? [...favStations.filter((x) => x.uuid !== st.uuid), st] : favStations.filter((x) => x.uuid !== st.uuid);
        toast(st.favorite ? `${st.name} saved to your stations` : `${st.name} removed from your stations`);
        rerender();
      }
      if (tag) showRadio({ mode: 'tag', tag });
      if (!play && !fav && !tag && t.closest('[data-st-uuid]')) {
        e.stopImmediatePropagation();
        const st = stations.get(t.closest<HTMLElement>('[data-st-uuid]')!.dataset.stUuid!);
        if (st) playStation(st);
      }
    }
    if (state.view.kind === 'remote' && remote && remoteData) {
      const id = remote.id;
      const ps = t.closest<HTMLElement>('[data-r-play-song]') ?? t.closest<HTMLElement>('[data-r-song]');
      const pl = t.closest<HTMLElement>('[data-r-play]')?.dataset.rPlay;
      const op = t.closest<HTMLElement>('[data-r-open]')?.dataset.rOpen;
      if (ps || pl || op) e.stopImmediatePropagation();
      if (ps) {
        const i = Number(ps.dataset.rPlaySong ?? ps.dataset.rSong);
        void playRemote(id, remoteData.songs[i], remoteData.songs);
      } else if (pl && remoteItems.get(pl)) void playRemote(id, remoteItems.get(pl)!);
      else if (op && remoteItems.get(op)) showRemote({ id, mode: 'open', item: remoteItems.get(op)! });
    }
  }, true);

  $('#viewHead').addEventListener('submit', (e) => {
    const f = e.target as HTMLElement;
    if (f.id === 'rSearchForm') {
      e.preventDefault();
      const q = $<HTMLInputElement>('#rSearch').value.trim();
      showRadio(q ? { mode: 'search', q } : { mode: 'home' });
    }
    if (f.id === 'sSearchForm' && remote) {
      e.preventDefault();
      const q = $<HTMLInputElement>('#sSearch').value.trim();
      showRemote(q ? { id: remote.id, mode: 'search', q } : { id: remote.id, mode: 'home' });
    }
  });
  $('#viewHead').addEventListener('click', (e) => {
    const t = e.target as HTMLElement;
    if (t.closest('[data-radio-home]')) showRadio();
    if (t.closest('[data-remote-home]') && remote) showRemote({ id: remote.id, mode: 'home' });
    if (t.closest('[data-remote-all]') && remote && remoteData?.songs.length) void playRemote(remote.id, remoteData.songs[0], remoteData.songs);
  });
}
