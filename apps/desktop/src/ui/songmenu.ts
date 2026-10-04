// The right-click menu for a song (library cards, the Now playing list), and dragging songs onto playlists,
// the Now playing list, or to a new place inside a playlist.
import { ask } from '@tauri-apps/plugin-dialog';
import { api, errText, type Song } from '../api';
import { enqueue, paused, playing, playShuffled, playSong, queue, stop, togglePause } from '../player';
import { albumKey, byId, playable, reload, rerender, state, visible, writerName } from '../state';
import { contextMenu, toast, type MenuItem } from '../util';
import { openSong } from './song';

const MIME = 'application/x-lute-songs';

async function after(msg: string) {
  await reload();
  rerender();
  toast(msg);
}

async function addTo(playlist: number, ids: number[]) {
  const p = state.playlists.find((x) => x.id === playlist);
  if (!p) return;
  const fresh = ids.filter((id) => !p.songs.includes(id));
  if (!fresh.length) return toast(ids.length === 1 ? `Already in ${p.name}` : `All of them are already in ${p.name}`);
  try {
    await api.setPlaylistSongs(p.id, [...p.songs, ...fresh]);
  } catch (e) {
    return toast(errText(e));
  }
  await after(fresh.length === 1 ? `Added "${byId(fresh[0])?.title ?? 'song'}" to ${p.name}` : `Added ${fresh.length} songs to ${p.name}`);
}

async function newPlaylistWith(ids: number[]) {
  try {
    const p = await api.createPlaylist(`Playlist ${state.playlists.length + 1}`, ids);
    await after(`Made ${p.name}`);
  } catch (e) {
    toast(errText(e));
  }
}

/** The open playlist, when its songs show in the playlist's own order (so moving them makes sense). */
function openPlaylist() {
  const v = state.view;
  if (v.kind !== 'pl' || ['title', 'artist', 'genre'].includes(state.sort)) return null;
  return state.playlists.find((p) => p.id === v.id) ?? null;
}

async function moveInPlaylist(id: number, to: number | 'top' | 'bottom' | -1 | 1) {
  const p = openPlaylist();
  if (!p) return;
  const order = p.songs.slice();
  const from = order.indexOf(id);
  if (from < 0) return;
  order.splice(from, 1);
  const at = to === 'top' ? 0 : to === 'bottom' ? order.length : to === -1 || to === 1 ? Math.max(0, Math.min(order.length, from + to)) : to;
  order.splice(at, 0, id);
  try {
    await api.setPlaylistSongs(p.id, order);
  } catch (e) {
    return toast(errText(e));
  }
  await reload();
  rerender();
}

/** Everything you can do with a song, from a right-click. `list` is what Play continues with. */
export function songMenu(e: MouseEvent, s: Song, list: Song[] = visible()) {
  const current = !!playing && playing.id === String(s.id);
  const items: MenuItem[] = [];
  if (playable(s)) {
    items.push(current ? [paused ? 'Resume' : 'Pause', () => togglePause()] : ['Play', () => playSong(s, list)]);
    items.push(['Play next', () => { enqueue([s], true); toast(`"${s.title}" plays next`); }]);
    items.push(['Add to Now playing', () => { enqueue([s], false); toast(`Added "${s.title}" to Now playing`); }]);
    if (current) items.push(['Stop', () => stop()]);
  } else if (s.missing) {
    items.push(["Can't play: the file isn't found right now", () => {}]);
  }
  items.push(null);
  const lists: MenuItem[] = state.playlists.map((p): MenuItem =>
    p.songs.includes(s.id)
      ? [`✓ ${p.name} (remove)`, async () => {
          await api.toggleInPlaylist(p.id, s.id, false).catch((err) => toast(errText(err)));
          await after(`Removed from ${p.name}`);
        }]
      : [p.name, () => void addTo(p.id, [s.id])],
  );
  items.push(['Add to playlist', [['New playlist', () => void newPlaylistWith([s.id])], ...(lists.length ? [null, ...lists] : [])]]);
  const pl = openPlaylist();
  if (pl && pl.songs.includes(s.id)) {
    const i = pl.songs.indexOf(s.id), last = pl.songs.length - 1;
    items.push(['Move', [
      ...(i > 0 ? [['To the top', () => void moveInPlaylist(s.id, 'top')], ['Up one', () => void moveInPlaylist(s.id, -1)]] as MenuItem[] : []),
      ...(i < last ? [['Down one', () => void moveInPlaylist(s.id, 1)], ['To the bottom', () => void moveInPlaylist(s.id, 'bottom')]] as MenuItem[] : []),
    ]]);
    items.push([`Remove from ${pl.name}`, async () => {
      await api.toggleInPlaylist(pl.id, s.id, false).catch((err) => toast(errText(err)));
      await after(`Removed from ${pl.name}`);
    }]);
  }
  items.push(null);
  items.push(['Song details', () => void openSong(s.id)]);
  if (s.album.trim()) items.push([`Go to album "${s.album}"`, () => { state.view = { kind: 'album', key: albumKey(s) }; state.filter = 'all'; rerender(); }]);
  if (s.provider === 'local' || s.source === 'ai') {
    const w = writerName();
    items.push(s.source === 'ai'
      ? [`Not written with ${w}`, async () => {
          try { await api.setSongWritten(s.id, false, w); } catch (err) { return toast(errText(err)); }
          await after(`"${s.title}" is now under My music`);
        }]
      : [`Written with ${w}`, async () => {
          try { await api.setSongWritten(s.id, true, w); } catch (err) { return toast(errText(err)); }
          await after(`"${s.title}" now shows under Written with ${w}`);
        }]);
  }
  if (s.hasFile || s.songDir) items.push([navigator.platform.startsWith('Mac') ? 'Show in Finder' : 'Show in folder', () => void api.revealSong(s.id).catch((err) => toast(errText(err)))]);
  if (!s.songDir) {
    items.push(null);
    items.push(['Remove from library', async () => {
      if (!(await ask(`Remove "${s.title}" from your library? Files on disk are not deleted.`, { title: 'Remove song', kind: 'warning', okLabel: 'Remove', cancelLabel: 'Keep' }))) return;
      if (current) stop();
      try { await api.deleteSong(s.id); } catch (err) { return toast(errText(err)); }
      await after(`Removed "${s.title}"`);
    }]);
  }
  contextMenu(e, items);
}

/** Right-click on a playlist in the side panel. */
export function playlistMenu(e: MouseEvent, id: number) {
  const p = state.playlists.find((x) => x.id === id);
  if (!p) return;
  const songs = p.songs.map(byId).filter((s): s is Song => !!s && playable(s));
  contextMenu(e, [
    ['Play', () => songs.length && playSong(songs[0], songs)],
    ['Shuffle', () => playShuffled(songs)],
    ['Add to Now playing', () => { enqueue(songs, false); toast(`Added ${songs.length} songs to Now playing`); }],
    null,
    ['Delete playlist', async () => {
      if (!(await ask(`Delete the playlist "${p.name}"? The songs stay in your library.`, { title: 'Delete playlist', kind: 'warning', okLabel: 'Delete', cancelLabel: 'Keep' }))) return;
      try { await api.deletePlaylist(p.id); } catch (err) { return toast(errText(err)); }
      if (state.view.kind === 'pl' && state.view.id === p.id) state.view = { kind: 'all' };
      await after(`Deleted ${p.name}`);
    }],
  ]);
}

/** Songs being dragged, if the drag came from a song card. */
export const draggedSongs = (e: DragEvent): number[] => {
  const raw = e.dataTransfer?.getData(MIME);
  return raw ? raw.split(',').map(Number).filter(Boolean) : [];
};
const isSongDrag = (e: DragEvent) => !!e.dataTransfer?.types.includes(MIME);

/** Right-click and drag for the song grid and the side panel. */
export function initSongMenus() {
  const grid = document.getElementById('grid')!;
  const side = document.getElementById('side')!;
  grid.addEventListener('contextmenu', (e) => {
    if (['apple', 'radio', 'remote'].includes(state.view.kind)) return;
    const c = (e.target as HTMLElement).closest<HTMLElement>('.card[data-id]');
    const s = c && byId(Number(c.dataset.id));
    if (s) songMenu(e, s);
  });
  document.getElementById('qList')!.addEventListener('contextmenu', (e) => {
    const li = (e.target as HTMLElement).closest<HTMLElement>('.qi');
    const s = li && queue[Number(li.dataset.i)];
    if (s) songMenu(e, s, queue);
  });
  side.addEventListener('contextmenu', (e) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>('[data-view^="pl:"]');
    if (row) playlistMenu(e, Number(row.dataset.view!.slice(3)));
  });

  // Dragging a card: onto a playlist in the side panel adds it; onto "+ New" makes a playlist with it; onto
  // another card in a playlist moves it there; onto the Now playing list queues it.
  grid.addEventListener('dragstart', (e) => {
    const c = (e.target as HTMLElement).closest<HTMLElement>('.card[data-id]');
    if (!c || !e.dataTransfer) return;
    e.dataTransfer.setData(MIME, c.dataset.id!);
    e.dataTransfer.setData('text/plain', byId(Number(c.dataset.id))?.title ?? '');
    e.dataTransfer.effectAllowed = 'copyMove';
    c.classList.add('dragging');
    document.body.classList.add('song-drag');
  });
  grid.addEventListener('dragend', () => {
    document.querySelectorAll('.dragging, .drop-on').forEach((x) => x.classList.remove('dragging', 'drop-on'));
    document.body.classList.remove('song-drag');
  });
  const target = (e: DragEvent) =>
    (e.target as HTMLElement).closest<HTMLElement>('[data-view^="pl:"], #plNew, #grid .card[data-id], #queue');
  const over = (e: DragEvent) => {
    if (!isSongDrag(e)) return;
    const t = target(e);
    if (!t || (t.classList.contains('card') && !openPlaylist())) return;
    e.preventDefault();
    e.dataTransfer!.dropEffect = t.classList.contains('card') ? 'move' : 'copy';
    document.querySelectorAll('.drop-on').forEach((x) => x !== t && x.classList.remove('drop-on'));
    t.classList.add('drop-on');
  };
  const drop = (e: DragEvent) => {
    const ids = draggedSongs(e);
    const t = target(e);
    document.querySelectorAll('.drop-on').forEach((x) => x.classList.remove('drop-on'));
    if (!ids.length || !t) return;
    e.preventDefault();
    if (t.id === 'plNew') return void newPlaylistWith(ids);
    if (t.id === 'queue') {
      enqueue(ids.map(byId).filter((s): s is Song => !!s), false);
      return toast('Added to Now playing');
    }
    if (t.dataset.view) return void addTo(Number(t.dataset.view.slice(3)), ids);
    const p = openPlaylist();
    if (p && t.dataset.id) void moveInPlaylist(ids[0], p.songs.filter((x) => x !== ids[0]).indexOf(Number(t.dataset.id)));
  };
  for (const el of [grid, side, document.getElementById('queue')!]) {
    el.addEventListener('dragover', over);
    el.addEventListener('drop', drop);
    el.addEventListener('dragleave', (e) => (e.target as HTMLElement).classList?.remove('drop-on'));
  }
}
