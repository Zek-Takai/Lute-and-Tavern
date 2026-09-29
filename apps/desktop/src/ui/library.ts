// Sidebar (Library / Playlists / Genres), the view header, status chips and the song grid.
import { api, errText, lt, type Song } from '../api';
import { playSong, syncIcons } from '../player';
import { keyName, staffSVG, toMeasures } from '../music/notation';
import { genresInUse, playable, rerender, reload, state, viewName, visible, writerName, type SideTab } from '../state';
import { $, esc, hueOf, PLAY, toast } from '../util';
import { openSong } from './song';

export const STATUS: Record<string, [string, string]> = {
  writing: ['Writing', 's-draft'],
  ready: ['Ready for Suno', 's-ready'],
  takes: ['Picking a take', 's-takes'],
  done: ['Finished', 's-done'],
};

const nav = (attr: string, val: string | number, label: string, count: number, on: boolean) =>
  `<button class="nav" ${attr}="${esc(val)}" aria-pressed="${on}"><span>${esc(label)}</span><span class="n">${count}</span></button>`;

export function renderSide() {
  const n = (k: string) => state.songs.filter((s) => k === 'all' || s.source === k).length;
  const v = state.view;
  let body = '';
  if (state.sideTab === 'library')
    body =
      nav('data-view', 'all', 'All music', n('all'), v.kind === 'all') +
      nav('data-view', 'ai', `Written with ${writerName()}`, n('ai'), v.kind === 'ai') +
      nav('data-view', 'library', 'My music', n('library'), v.kind === 'library');
  if (state.sideTab === 'playlists')
    body =
      state.playlists.map((p) => nav('data-view', 'pl:' + p.id, p.name, p.songs.length, v.kind === 'pl' && v.id === p.id)).join('') +
      '<button class="nav add" id="plNew">+ New playlist</button>';
  if (state.sideTab === 'genres')
    body = nav('data-g', 'all', 'All genres', state.songs.length, state.genre === 'all') + genresInUse().map(([k, g]) => nav('data-g', k, g.name, g.n, state.genre === k)).join('');
  const tabs: [SideTab, string][] = [['library', 'Library'], ['playlists', 'Playlists'], ['genres', 'Genres']];
  $('#side').innerHTML =
    `<div class="seg side-seg" role="tablist" aria-label="Sidebar">${tabs.map(([k, l]) => `<button role="tab" data-st="${k}" aria-pressed="${state.sideTab === k}" aria-selected="${state.sideTab === k}">${l}</button>`).join('')}</div><section>${body}</section>`;
  $('#genreList').innerHTML = state.genres.map((g) => `<option value="${esc(g.name)}">`).join('');
}

export function renderHead() {
  const list = visible(), v = state.view;
  const p = v.kind === 'pl' ? state.playlists.find((x) => x.id === v.id) : undefined;
  const title = p ? `<input id="plName" value="${esc(p.name)}" aria-label="Playlist name" size="${Math.max(6, p.name.length)}">` : `<h2>${esc(viewName())}</h2>`;
  $('#viewHead').innerHTML =
    title +
    `<span class="meta">${list.length} song${list.length === 1 ? '' : 's'}</span>` +
    (list.some(playable) ? `<button class="btn small" id="playAll">${PLAY.replace('<svg', '<svg width="12" height="12"')}Play all</button>` : '') +
    (p ? '<button class="btn small" id="plDel">Delete playlist</button>' : '');
}

export function renderChips() {
  const chips = $('#chips');
  chips.hidden = state.view.kind !== 'ai';
  if (chips.hidden) return;
  const pool = state.songs.filter((s) => s.source === 'ai');
  const counts: Record<string, number> = { all: pool.length };
  pool.forEach((s) => (counts[s.status ?? ''] = (counts[s.status ?? ''] || 0) + 1));
  const opts = [['all', 'All'], ...Object.entries(STATUS).map(([k, [l]]) => [k, l])];
  chips.innerHTML = opts.map(([k, l]) => `<button class="chip" aria-pressed="${state.filter === k}" data-f="${k}">${l}<span class="n">${counts[k] || 0}</span></button>`).join('');
}

function cover(s: Song) {
  if (s.art) return `<div class="cover" style="background-image:url('${lt('art', s.art)}')"></div>`;
  if (s.source === 'ai' && s.status === 'writing' && !s.lyrics) return `<div class="staff"><div class="placeholder">${esc(writerName())} is writing…</div></div>`;
  if (s.notes.length)
    return `<div class="staff">${staffSVG(toMeasures(s.notes).slice(0, 2), { g: 6.5, W: 300, lyrics: false, timesig: false, chords: s.chords, title: s.title })}</div>`;
  const h = hueOf(s.title + s.artist);
  return `<div class="cover" style="background-image:linear-gradient(135deg,hsl(${h} 55% 38%),hsl(${(h + 50) % 360} 60% 18%))">${esc((s.title[0] || '♪').toUpperCase())}</div>`;
}

export function renderGrid() {
  const list = visible(), q = state.query.trim();
  const empty = !state.songs.length
    ? `<div class="empty welcome"><h2>Welcome to the tavern</h2><p>Add the folder where your music lives, or ask ${esc(writerName())} to write you a song.</p>
        <div class="row" style="justify-content:center"><button class="btn primary" data-act="addFolder">Add a music folder</button><button class="btn" data-act="request">Request a song</button></div></div>`
    : `<p class="empty">${q ? `Nothing matches "${esc(q)}". Try a title, artist, album or lyric.` : state.view.kind === 'pl' ? 'This playlist is empty. Open any song and tick this playlist under Details.' : 'No songs here yet.'}</p>`;
  $('#grid').innerHTML = list.length
    ? list
        .map((s) => {
          const st = s.source === 'ai' && s.status ? STATUS[s.status] : null;
          const writing = s.source === 'ai' && s.status === 'writing' && !s.lyrics;
          const sub = s.source === 'ai'
            ? writing ? esc((s.request ?? '').slice(0, 70)) + ((s.request ?? '').length > 70 ? '…' : '') : [s.subGenre, keyName(s.key), s.bpm ? `${s.bpm} BPM` : ''].filter(Boolean).map(esc).join(', ')
            : esc([s.artist, s.album].filter(Boolean).join(', '));
          return `<article class="card${writing ? ' writing' : ''}" tabindex="0" role="button" data-id="${s.id}" aria-label="Open ${esc(s.title)}">
        ${cover(s)}
        <div class="body">
          <h3>${esc(s.title)}</h3>
          <div class="meta">${sub}</div>
          <div class="foot">${st ? `<span class="status ${st[1]}">${st[0]}</span>` : ''}<span class="gtag">${esc(s.genre)}</span><span style="margin-right:auto"></span>
            ${playable(s) ? `<button class="play" data-play="${s.id}" aria-label="Play">${PLAY}</button>` : ''}</div>
        </div></article>`;
        })
        .join('')
    : empty;
  syncIcons();
}

export function initLibrary(actions: { addFolder: () => void; request: () => void }) {
  $('#side').onclick = async (e) => {
    const t = e.target as HTMLElement;
    const tab = t.closest<HTMLElement>('[data-st]');
    if (tab) {
      state.sideTab = tab.dataset.st as SideTab;
      return renderSide();
    }
    if (t.id === 'plNew') {
      try {
        const n = state.playlists.length + 1;
        const p = await api.createPlaylist(`Playlist ${n}`, []);
        await reload();
        state.view = { kind: 'pl', id: p.id };
        rerender();
        const inp = $<HTMLInputElement>('#plName');
        inp?.focus();
        inp?.select();
      } catch (err) {
        toast(errText(err));
      }
      return;
    }
    const v = t.closest<HTMLElement>('[data-view]'), g = t.closest<HTMLElement>('[data-g]');
    if (v) {
      const x = v.dataset.view!;
      state.view = x.startsWith('pl:') ? { kind: 'pl', id: Number(x.slice(3)) } : { kind: x as 'all' | 'ai' | 'library' };
      state.filter = 'all';
    }
    if (g) state.genre = g.dataset.g!;
    if (v || g) rerender();
  };
  const head = $('#viewHead');
  head.addEventListener('click', async (e) => {
    const t = e.target as HTMLElement;
    if (t.closest('#playAll')) {
      const list = visible().filter(playable);
      if (list.length) playSong(list[0], list);
    }
    if (t.closest('#plDel') && state.view.kind === 'pl') {
      const id = state.view.id, name = state.playlists.find((p) => p.id === id)?.name;
      await api.deletePlaylist(id).catch((err) => toast(errText(err)));
      state.view = { kind: 'all' };
      await reload();
      rerender();
      toast(`Deleted ${name}`);
    }
  });
  head.addEventListener('change', async (e) => {
    const t = e.target as HTMLInputElement;
    if (t.id !== 'plName' || state.view.kind !== 'pl') return;
    try {
      await api.renamePlaylist(state.view.id, t.value);
      await reload();
      renderSide();
    } catch (err) {
      toast(errText(err));
    }
  });
  head.addEventListener('input', (e) => {
    const t = e.target as HTMLInputElement;
    if (t.id === 'plName') t.size = Math.max(6, t.value.length);
  });
  $('#chips').onclick = (e) => {
    const b = (e.target as HTMLElement).closest<HTMLElement>('[data-f]');
    if (!b) return;
    state.filter = b.dataset.f!;
    rerender();
  };
  $<HTMLInputElement>('#q').oninput = (e) => {
    state.query = (e.target as HTMLInputElement).value;
    renderHead();
    renderGrid();
  };
  $<HTMLSelectElement>('#sort').onchange = (e) => {
    state.sort = (e.target as HTMLSelectElement).value;
    renderGrid();
  };
  const grid = $('#grid');
  grid.onclick = (e) => {
    const t = e.target as HTMLElement;
    const act = t.closest<HTMLElement>('[data-act]')?.dataset.act;
    if (act === 'addFolder') return actions.addFolder();
    if (act === 'request') return actions.request();
    const p = t.closest<HTMLElement>('[data-play]');
    if (p) {
      e.stopPropagation();
      const s = state.songs.find((x) => x.id === Number(p.dataset.play));
      if (s) playSong(s, visible());
      return;
    }
    const c = t.closest<HTMLElement>('.card');
    if (c) void openSong(Number(c.dataset.id));
  };
  grid.onkeydown = (e) => {
    const t = e.target as HTMLElement;
    if ((e.key === 'Enter' || e.key === ' ') && t.classList.contains('card')) {
      e.preventDefault();
      void openSong(Number(t.dataset.id));
    }
  };
  state.sideTab = (state.settings.sidebarDefault as SideTab) || 'library';
}
