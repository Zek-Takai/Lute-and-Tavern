// Lute & Tavern: boots the interface, wires dialogs, drag and drop, and events from the Rust side.
import '@fontsource/bricolage-grotesque/500.css';
import '@fontsource/bricolage-grotesque/700.css';
import '@fontsource/instrument-sans/400.css';
import '@fontsource/instrument-sans/500.css';
import '@fontsource/instrument-sans/600.css';
import '@fontsource/noto-music/400.css';
import './styles.css';
import './app.css';

import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open as pickFiles } from '@tauri-apps/plugin-dialog';
import { api, errText } from './api';
import { aiControl, cycleRepeat, initPlayer, initQueue, refreshQueue, restoreNowPlaying, step, stop, togglePlay } from './player';
import { onRender, reload, state, visible, writerName } from './state';
import { initLibrary, renderChips, renderGrid, renderHead, renderSide } from './ui/library';
import { initAssistant } from './ui/assistant';
import { initSearch } from './ui/search';
import { initSources } from './ui/sources';
import { addMusicFolder, addSongsToLibrary, checkForUpdatesAtStart, chooseLibrary, openSettings } from './ui/settings';
import { initSheetKeys } from './ui/sheet';
import { initApple } from './ui/apple';
import { onAppleChange } from './apple';
import { initSecureStorage } from './secureStorage';
import { activeTab, cur, initSong, refreshSong } from './ui/song';
import { $, toast } from './util';
import { startVisualizer } from './visualizer';

function renderAll() {
  refreshQueue();
  renderSide();
  renderHead();
  renderChips();
  renderGrid();
  $('#writerName').textContent = writerName();
}

async function openRequest() {
  if (!state.writer.baseUrl || !state.writer.model) {
    toast('Set up your songwriter first: an address and a model.');
    return openSettings('writer');
  }
  $<HTMLTextAreaElement>('#reqText').value = '';
  $<HTMLInputElement>('#reqGenre').value = '';
  $<HTMLDialogElement>('#req').showModal();
}

function initRequest() {
  $('#newSong').onclick = () => void openRequest();
  $('#reqCancel').onclick = () => $<HTMLDialogElement>('#req').close();
  $('#reqQuick').innerHTML = ['Upbeat', 'Sad', 'Love song', 'Female vocal', 'Male vocal', 'Acoustic', 'Big chorus']
    .map((q) => `<button type="button" class="chip" data-add="${q}">${q}</button>`)
    .join('');
  $('#reqQuick').onclick = (e) => {
    const x = (e.target as HTMLElement).closest<HTMLElement>('[data-add]');
    if (!x) return;
    const t = $<HTMLTextAreaElement>('#reqText');
    t.value = (t.value.trim() ? t.value.trim().replace(/\.?$/, '. ') : '') + x.dataset.add + '.';
    t.focus();
  };
  $<HTMLFormElement>('#reqForm').onsubmit = async (e) => {
    e.preventDefault();
    const text = $<HTMLTextAreaElement>('#reqText').value.trim();
    if (!text) return;
    try {
      const genre = $<HTMLInputElement>('#reqGenre').value.trim() || null;
      await api.requestSong(text, genre);
      $<HTMLDialogElement>('#req').close();
      await reload();
      state.view = { kind: 'ai' };
      state.filter = 'all';
      state.genre = 'all';
      renderAll();
      toast(`Request sent to ${writerName()}`);
    } catch (err) {
      toast(errText(err));
    }
  };
}

function initImport() {
  $('#addMusic').onclick = () => $<HTMLDialogElement>('#imp').showModal();
  $('#impClose').onclick = () => $<HTMLDialogElement>('#imp').close();
  $('#impFolder').onclick = async () => {
    $<HTMLDialogElement>('#imp').close();
    await addMusicFolder();
  };
  $('#impFiles').onclick = async () => {
    const paths = await pickFiles({ multiple: true, filters: [{ name: 'Audio', extensions: ['mp3', 'm4a', 'aac', 'flac', 'wav', 'ogg', 'opus', 'aiff', 'aif'] }] });
    $<HTMLDialogElement>('#imp').close();
    if (Array.isArray(paths) && paths.length) await addSongsToLibrary(paths);
  };
}

async function importPaths(paths: string[]) {
  toast('Reading your music…');
  try {
    const r = await api.importPaths(paths);
    await reload();
    state.view = { kind: 'library' };
    state.genre = 'all';
    renderAll();
    toast(r.added ? `Added ${r.added} song${r.added === 1 ? '' : 's'}${r.untagged ? `. ${r.untagged} had no genre tag and wait in Unsorted.` : '.'}` : 'Nothing new to add.');
  } catch (e) {
    toast(errText(e));
  }
}

async function initDragDrop() {
  const over = $('#dropOver');
  await getCurrentWebview().onDragDropEvent((ev) => {
    const p = ev.payload;
    if (p.type === 'enter' || p.type === 'over') over.hidden = false;
    else if (p.type === 'leave') over.hidden = true;
    else if (p.type === 'drop') {
      over.hidden = true;
      if ($<HTMLDialogElement>('#dlg').open || !p.paths.length) return;
      void (state.folders.some((f) => f.managed) ? addSongsToLibrary(p.paths) : importPaths(p.paths));
    }
  });
}

async function initEvents() {
  await listen('library-changed', async () => {
    await reload();
    renderAll();
  });
  await listen<number>('song-updated', async (e) => {
    await reload();
    renderAll();
    await refreshSong(e.payload);
    const s = state.songs.find((x) => x.id === e.payload);
    if (s && !(cur && cur.id === s.id && $<HTMLDialogElement>('#dlg').open)) toast(`${writerName()} finished "${s.title}"`);
  });
  await listen<Parameters<typeof aiControl>[0]>('ai-control', (e) => aiControl(e.payload));
  await listen<{ id: number; message: string }>('song-failed', async (e) => {
    await reload();
    renderAll();
    await refreshSong(e.payload.id);
    toast(e.payload.message);
  });
}

async function boot() {
  await initSecureStorage();
  await reload();
  state.writer = await api.writerConfig().catch(() => state.writer);
  onRender(renderAll);
  initPlayer();
  initQueue();
  initLibrary({ addFolder: () => void addMusicFolder(), request: () => void openRequest(), setupLibrary: (suggested: boolean) => void chooseLibrary(suggested), settings: () => void openSettings() });
  initSong();
  initApple();
  await initSources();
  initSearch();
  onAppleChange(() => renderAll());
  initSheetKeys(() => $<HTMLDialogElement>('#dlg').open && activeTab() === 'sheet');
  initRequest();
  initImport();
  $('#pBtn').onclick = () => togglePlay(visible());
  $('#pNext').onclick = () => step(1);
  $('#pPrev').onclick = () => step(-1);
  $('#pStop').onclick = () => stop();
  $('#rMode').onclick = () => cycleRepeat();
  renderAll();
  restoreNowPlaying();
  startVisualizer($<HTMLCanvasElement>('#viz'));
  await initEvents();
  await initDragDrop();
  await initAssistant();
  void checkForUpdatesAtStart();
}

addEventListener('error', (e) => void api.logEvent('error', `${e.message} at ${e.filename}:${e.lineno}`));
addEventListener('unhandledrejection', (e) => void api.logEvent('error', `unhandled: ${errText(e.reason)}`));
boot().catch((e) => {
  void api.logEvent('error', `boot failed: ${errText(e)}`);
  toast(errText(e));
});
