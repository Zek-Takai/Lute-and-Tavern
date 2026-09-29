// The song popup: Song (Suno fields), Talk to the songwriter, Sheet music, Takes, Details, Versions.
import { open as pickFile } from '@tauri-apps/plugin-dialog';
import { api, errText, lt, type SongDetail } from '../api';
import { keyName } from '../music/notation';
import { pickStems, playing, playSong, playStems, playTake, setStemMix, stemMix, stop, syncIcons } from '../player';
import { byId, playable, reload, rerender, state, visible, writerName } from '../state';
import { $, copyText, date, esc, PLAY, toast } from '../util';
import { sourceBadge, STATUS } from './library';
import { renderSheet, resetSheet } from './sheet';

export let cur: SongDetail | null = null;
let tab = 'song';

const tabsFor = (s: SongDetail): [string, string][] => {
  const stems: [string, string][] = s.stemFiles.length ? [['stems', 'Stems']] : [];
  return s.source === 'ai'
    ? [['song', 'Song'], ['chat', `Talk to ${writerName()}`], ['sheet', 'Sheet music'], ['takes', 'Takes'], ...stems, ['details', 'Details'], ['versions', 'Versions']]
    : [['details', 'Details'], ...stems, ['sheet', 'Sheet music']];
};

export async function openSong(id: number, startTab?: string) {
  try {
    cur = await api.songDetail(id);
  } catch (e) {
    return toast(errText(e));
  }
  tab = startTab ?? tabsFor(cur)[0][0];
  resetSheet();
  $('#tabs').innerHTML = tabsFor(cur).map(([k, l]) => `<button class="tab" role="tab" data-tab="${k}">${esc(l)}</button>`).join('');
  renderHeader();
  renderTab();
  const d = $<HTMLDialogElement>('#dlg');
  if (!d.open) d.showModal();
}

/** Reloads the open song after something changed it (a rewrite finished, a file was added). */
export async function refreshSong(id: number) {
  if (!cur || cur.id !== id || !$<HTMLDialogElement>('#dlg').open) return;
  cur = await api.songDetail(id);
  renderHeader();
  if (tab !== 'sheet') renderTab();
}

function renderHeader() {
  const s = cur!;
  $('#mTitle').textContent = s.title;
  const mp = $('#mPlay');
  mp.dataset.play = String(s.id);
  mp.hidden = !playable(s);
  const m: string[] = [];
  if (s.source === 'ai' && s.status) m.push(`<span class="status ${STATUS[s.status][1]}">${STATUS[s.status][0]}</span>`);
  m.push(sourceBadge(s), `<span class="gtag">${esc(s.genre)}</span>`);
  if (s.source === 'ai' && s.lyrics)
    m.push(...[s.subGenre, keyName(s.key), s.bpm ? `${s.bpm} BPM` : '', `Written by ${s.artist}`].filter(Boolean).map((v) => `<span class="meta">${esc(v)}</span>`));
  if (s.source === 'library') m.push(...[s.artist, s.album, s.year].filter(Boolean).map((v) => `<span class="meta">${esc(v)}</span>`));
  $('#mMeta').innerHTML = m.join('');
  syncIcons();
}

async function afterChange() {
  await reload();
  rerender();
  if (cur) await refreshSong(cur.id);
}

export function renderTab() {
  const s = cur!, b = $('#mBody');
  document.querySelectorAll<HTMLElement>('.tab').forEach((t) => t.setAttribute('aria-selected', String(t.dataset.tab === tab)));
  if (tab === 'song') {
    const ask = `<div class="ask"><small>You asked for</small>${esc(s.request)}</div>`;
    if (!s.lyrics) {
      b.innerHTML = ask + `<p class="muted">${esc(writerName())} is writing this one. The title, lyrics and Suno style appear here when the first draft is ready.</p>`;
      return;
    }
    const f: [string, string][] = [['Song title', s.title], ['Style of music', s.style], ['Exclude styles', s.exclude], ['Lyrics', s.lyrics]];
    b.innerHTML =
      ask +
      `<p class="small muted" style="margin-top:0">Each box matches a field in Suno's custom mode. Copy one, paste it, come back for the next. Want changes? Use the Talk to ${esc(writerName())} tab.</p>` +
      f.map(([l, v], i) => `<section class="field"><header><b>${l}</b><span class="count">${v.length} characters</span><button class="btn small" data-copy="${i}">Copy</button></header><pre>${esc(v)}</pre></section>`).join('');
    b.querySelectorAll<HTMLElement>('[data-copy]').forEach((btn) => (btn.onclick = () => copyText(f[Number(btn.dataset.copy)][1], f[Number(btn.dataset.copy)][0])));
  }
  if (tab === 'chat') {
    const busy = s.messages.length > 0 && s.messages[s.messages.length - 1].role === 'user';
    const quick = ['Make it sadder', 'Shorter chorus', 'Try a different genre', 'Rewrite verse 2', 'Punchier title'];
    b.innerHTML = `<div class="thread" id="thread">${s.messages
      .map((m) => (m.role === 'user' ? `<div class="msg me">${esc(m.text)}</div>` : `<div class="msg ai"><b>${esc(writerName())}</b>${esc(m.text)}</div>`))
      .join('')}${busy ? `<div class="msg ai typing">${esc(writerName())} is writing…</div>` : ''}</div>
      <div class="quick">${quick.map((q) => `<button class="chip" data-q="${q}">${q}</button>`).join('')}</div>
      <form class="composer" id="chatForm"><textarea id="chatText" rows="2" placeholder="Tell ${esc(writerName())} what to change"></textarea><button class="btn primary"${busy ? ' disabled' : ''}>Send</button></form>`;
    b.querySelector('#thread')?.lastElementChild?.scrollIntoView({ block: 'nearest' });
    b.querySelectorAll<HTMLElement>('[data-q]').forEach((x) => (x.onclick = () => { $<HTMLTextAreaElement>('#chatText').value = x.dataset.q!; $('#chatText').focus(); }));
    $<HTMLFormElement>('#chatForm').onsubmit = async (e) => {
      e.preventDefault();
      const text = $<HTMLTextAreaElement>('#chatText').value.trim();
      if (!text) return;
      try {
        await api.rewriteSong(s.id, text);
        cur = await api.songDetail(s.id);
        renderTab();
      } catch (err) {
        toast(errText(err));
      }
    };
  }
  if (tab === 'sheet') renderSheet(b, s, afterChange);
  if (tab === 'takes') {
    b.innerHTML =
      (s.takes.length
        ? s.takes
            .map((t, i) => {
              const master = s.masterTake === t.id || (s.masterTake == null && i === 0);
              return `<div class="take${master ? ' master' : ''}"><button class="play" aria-label="Play take" data-take="${t.id}" data-play="${s.id}:t${t.id}">${PLAY}</button>
            <div><b>Take ${i + 1}</b> <span class="muted small">${esc(t.name)}, ${esc(t.origin)}, ${date(t.addedAt)}</span></div>
            ${master ? '<span class="badge">Master</span>' : `<button class="btn small" data-master="${t.id}">Make master</button>`}</div>`;
            })
            .join('')
        : `<p class="muted" style="margin-top:0">No takes yet. Generate the song in Suno with the fields on the Song tab, then add the audio file here.</p>`) +
      `<div class="row"><button class="btn" id="addTake">Add a take</button>${s.status !== 'done' && s.takes.length ? '<button class="btn" id="markDone">Mark song finished</button>' : ''}</div>
      <p class="small muted">The master take is what plays in your library. Turning a take into sheet music automatically is planned for a later version.</p>`;
    b.querySelectorAll<HTMLElement>('[data-take]').forEach((x) => (x.onclick = () => {
      const t = s.takes.find((t) => t.id === Number(x.dataset.take))!;
      playTake(byId(s.id) ?? s, t.id, t.name);
    }));
    b.querySelectorAll<HTMLElement>('[data-master]').forEach((x) => (x.onclick = async () => {
      await api.setMasterTake(s.id, Number(x.dataset.master)).catch((e) => toast(errText(e)));
      await afterChange();
      toast('Master take set');
    }));
    $('#addTake').onclick = async () => {
      const path = await pickFile({ multiple: false, filters: [{ name: 'Audio', extensions: ['mp3', 'wav', 'flac', 'm4a', 'ogg', 'aac', 'opus', 'aiff'] }] });
      if (typeof path !== 'string') return;
      try {
        await api.addTake(s.id, path, 'from Suno');
        await afterChange();
        toast('Take added');
      } catch (e) {
        toast(errText(e));
      }
    };
    const done = b.querySelector<HTMLElement>('#markDone');
    if (done) done.onclick = async () => { await api.setStatus(s.id, 'done'); await afterChange(); toast('Marked finished'); };
  }
  if (tab === 'details') {
    const fields: [keyof SongDetail, string][] = [['title', 'Title'], ['artist', 'Artist'], ['album', 'Album'], ['year', 'Year'], ['genre', 'Genre']];
    b.innerHTML = `<div class="details">
      <div><div class="art" style="${s.art ? `background-image:url('${lt('art', s.art)}')` : ''}">${s.art ? '' : 'No artwork'}</div>
        <div class="row"><button class="btn small" id="artSet">${s.art ? 'Change artwork' : 'Add artwork'}</button>${s.art ? '<button class="btn small" id="artClear">Remove</button>' : ''}</div></div>
      <div class="form">${fields
        .map(([k, l]) => `<label for="d_${k}">${l}</label><input id="d_${k}" data-k="${k}" value="${esc(s[k])}"${k === 'genre' ? ' list="genreList" placeholder="Pick a tag or type a new one"' : ''}${k === 'artist' && s.source === 'ai' ? ' readonly' : ''}>`)
        .join('')}</div>
    </div>
    <h4 class="sub">Playlists</h4>
    <div class="pl-list">${state.playlists.map((p) => `<label><input type="checkbox" data-pl="${p.id}" ${s.playlists.includes(p.id) ? 'checked' : ''}>${esc(p.name)}</label>`).join('')}
      <button class="btn small" id="plAddNew">New playlist with this song</button></div>
    <p class="small muted">Every song carries one genre tag. Pick one of yours or type a new tag and it is created for you.</p>
    <dl class="kv facts">
      ${s.duration ? `<dt>Length</dt><dd>${Math.floor(s.duration / 60)}:${String(Math.round(s.duration % 60)).padStart(2, '0')}</dd>` : ''}
      ${s.stemFiles.length ? `<dt>Stems</dt><dd>${pickStems(s.stemFiles).length} parts</dd>` : ''}
      ${s.takes.length ? `<dt>Takes</dt><dd>${s.takes.length}</dd>` : ''}
      <dt>Added</dt><dd>${date(s.addedAt)}</dd>
      ${s.songDir ? `<dt>Folder</dt><dd class="path-text">${esc(s.songDir)}</dd>` : ''}
    </dl>
    ${s.songDir
      ? '<p class="small muted">This song lives in your library folder. To remove it, move its folder out of the library or to the Trash.</p>'
      : '<div class="row danger-zone"><button class="btn small" id="delSong">Remove from library</button><span class="muted small" id="delNote"></span></div>'}`;
    b.querySelectorAll<HTMLInputElement>('[data-k]').forEach((inp) => (inp.onchange = async () => {
      try {
        await api.updateSong(s.id, { [inp.dataset.k!]: inp.value });
        await afterChange();
      } catch (e) {
        toast(errText(e));
      }
    }));
    $('#artSet').onclick = async () => {
      const path = await pickFile({ multiple: false, filters: [{ name: 'Images', extensions: ['jpg', 'jpeg', 'png', 'gif', 'webp'] }] });
      if (typeof path !== 'string') return;
      await api.setArt(s.id, path).catch((e) => toast(errText(e)));
      await afterChange();
      toast('Artwork added');
    };
    b.querySelector<HTMLElement>('#artClear')?.addEventListener('click', async () => { await api.clearArt(s.id); await afterChange(); });
    b.querySelectorAll<HTMLInputElement>('[data-pl]').forEach((cb) => (cb.onchange = async () => {
      await api.toggleInPlaylist(Number(cb.dataset.pl), s.id, cb.checked).catch((e) => toast(errText(e)));
      await afterChange();
    }));
    $('#plAddNew').onclick = async () => {
      const p = await api.createPlaylist(`Playlist ${state.playlists.length + 1}`, [s.id]);
      await afterChange();
      toast(`Added to ${p.name}`);
    };
    let armed = false;
    const del = document.getElementById('delSong');
    if (del) del.onclick = async () => {
      if (!armed) {
        armed = true;
        $('#delSong').textContent = 'Yes, remove it';
        $('#delNote').textContent = s.source === 'library' ? 'The file stays on your computer. Rescanning its folder brings it back.' : 'This deletes the song, its versions, takes and sheets.';
        return;
      }
      await api.deleteSong(s.id).catch((e) => toast(errText(e)));
      $<HTMLDialogElement>('#dlg').close();
      await reload();
      rerender();
      toast(`Removed ${s.title}`);
    };
  }
  if (tab === 'stems') {
    const parts = pickStems(s.stemFiles);
    const formats = (name: string) => [...new Set(s.stemFiles.filter((f) => f.name === name).map((f) => f.format.toUpperCase()))].join(' + ');
    const live = playing?.kind === 'stems' && playing.id === String(s.id);
    b.innerHTML = `<div class="row" style="margin-bottom:12px"><button class="btn primary" id="stemPlay">${live ? 'Stop' : 'Play the stems together'}</button>
        <span class="muted small">${parts.length} parts. Mute, solo or turn down any part while it plays.${s.hasFile ? ' The play button at the top plays the full song file.' : ''}</span></div>
      <ul class="stem-list">${parts
        .map((f) => {
          const m = stemMix(s.id, f.name);
          return `<li data-stem="${esc(f.name)}">
            <b>${esc(f.name)}</b><span class="muted small">${formats(f.name)}</span>
            <button class="chip" data-act="mute" aria-pressed="${m.mute}">Mute</button>
            <button class="chip" data-act="solo" aria-pressed="${m.solo}">Solo</button>
            <input type="range" min="0" max="1.5" step="0.05" value="${m.vol}" aria-label="${esc(f.name)} volume">
          </li>`;
        })
        .join('')}</ul>
      <p class="small muted">Stems are the separate parts of a song (vocals, drums, bass...). When a folder has only stems, Lute &amp; Tavern plays them together as the song.</p>`;
    $('#stemPlay').onclick = () => {
      if (playing?.kind === 'stems' && playing.id === String(s.id)) stop();
      else void playStems(byId(s.id) ?? s);
      setTimeout(renderTab, 50);
    };
    b.querySelectorAll<HTMLElement>('[data-stem]').forEach((li) => {
      const name = li.dataset.stem!;
      li.querySelectorAll<HTMLElement>('[data-act]').forEach((btn) => (btn.onclick = () => {
        const key = btn.dataset.act as 'mute' | 'solo';
        const on = !stemMix(s.id, name)[key];
        setStemMix(s.id, name, { [key]: on });
        btn.setAttribute('aria-pressed', String(on));
      }));
      li.querySelector<HTMLInputElement>('input[type=range]')!.oninput = (e) => setStemMix(s.id, name, { vol: Number((e.target as HTMLInputElement).value) });
    });
  }
  if (tab === 'versions') {
    b.innerHTML = s.versions.length
      ? `<ol class="versions">${s.versions.map((v) => `<li><b>${esc(v.label)}</b><span class="muted small">${date(v.createdAt)}</span><div>${esc(v.note)}</div></li>`).join('')}</ol>`
      : `<p class="muted" style="margin-top:0">No versions yet. The first draft becomes v1.</p>`;
  }
}

export function initSong() {
  $('#tabs').onclick = (e) => {
    const t = (e.target as HTMLElement).closest<HTMLElement>('[data-tab]');
    if (!t || !cur) return;
    tab = t.dataset.tab!;
    renderTab();
  };
  $('#mPlay').onclick = () => {
    if (!cur) return;
    const s = byId(cur.id) ?? cur;
    playSong(s, visible());
  };
  $('#mClose').onclick = () => $<HTMLDialogElement>('#dlg').close();
  $('#dlg').addEventListener('click', (e) => { if (e.target === $('#dlg')) $<HTMLDialogElement>('#dlg').close(); });
}

export const activeTab = () => tab;
