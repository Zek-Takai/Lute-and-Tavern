// Sheet music tab: uploaded sheets (PDF, images, MusicXML) and a click-to-write melody editor.
import { open as pickFile, save as pickSave } from '@tauri-apps/plugin-dialog';
import { api, errText, lt, type Note, type SongDetail } from '../api';
import { fromMusicXML, toMusicXML } from '../music/musicxml';
import { DUR_NAME, keyName, staffSVG, toMeasures, type Placed } from '../music/notation';
import { blip, playing, playMelody, setMelodyListener, stop } from '../player';
import { $, copyText, esc, toast } from '../util';

let sel = -1;
let dur: Note['d'] = 'q';
let transpose = 0;
let song: SongDetail | null = null;
let host: HTMLElement | null = null;
let onSaved: () => Promise<void> = async () => {};
let saveTimer = 0;

export function resetSheet() {
  sel = -1;
  transpose = 0;
}

function save() {
  clearTimeout(saveTimer);
  const s = song!;
  saveTimer = window.setTimeout(async () => {
    try {
      await api.saveSheetMusic(s.id, s.notes, s.chords);
      await onSaved();
    } catch (e) {
      toast(errText(e));
    }
  }, 500);
}

export function renderSheet(b: HTMLElement, s: SongDetail, afterChange: () => Promise<void>) {
  song = s;
  host = b;
  onSaved = afterChange;
  const att = s.sheets
    .map((f) => {
      const rm = `<button class="btn small" data-rm-sheet="${f.id}">Remove</button>`;
      if (f.kind === 'image') return `<figure><img src="${lt('sheet', f.id)}" alt="${esc(f.name)}"><figcaption>${esc(f.name)} ${rm}</figcaption></figure>`;
      if (f.kind === 'pdf') return `<figure><iframe class="pdf" src="${lt('sheet', f.id)}" title="${esc(f.name)}"></iframe><figcaption>${esc(f.name)} ${rm}</figcaption></figure>`;
      return `<p class="file"><b>${esc(f.name)}</b> ${rm}<br>MusicXML file, loaded into the editor below. <button class="btn small" data-load-xml="${f.id}">Load it again</button></p>`;
    })
    .join('');
  b.innerHTML = `${att ? `<div class="sheets">${att}</div>` : ''}
    <div class="editor-tools">
      <div class="seg" aria-label="Note length">${Object.entries(DUR_NAME).map(([k, l]) => `<button data-dur="${k}" aria-pressed="${dur === k}">${l}</button>`).join('')}</div>
      <div class="seg"><button id="nUp" aria-label="Move note up">▲</button><button id="nDown" aria-label="Move note down">▼</button><button id="nDel">Delete</button></div>
      <input class="inline-input" id="nLyric" placeholder="Lyric for the selected note" style="flex:1 1 180px;width:auto" ${sel < 0 ? 'disabled' : ''} value="${esc(sel >= 0 && s.notes[sel] ? s.notes[sel].w : '')}">
    </div>
    <label class="chordin" for="chordsIn">Chords, one per measure <input class="inline-input" id="chordsIn" value="${esc(s.chords.join(' '))}" placeholder="C G Am F"></label>
    <div class="paper"><p class="title">${esc(s.title)}</p><p class="sub">${s.key ? keyName(s.key, transpose) + ', ' : ''}${s.bpm ? `♩ = ${s.bpm}` : 'Tempo not set'}</p><div class="rows" id="rows"></div></div>
    <div class="sheet-tools" style="margin-top:14px">
      <div class="transpose"><button id="tDown" aria-label="Transpose down a semitone">−</button><span>${transpose ? `Transposed ${transpose > 0 ? '+' : ''}${transpose}` : 'Original key'}</span><button id="tUp" aria-label="Transpose up a semitone">+</button></div>
      <button class="btn small" id="sPlay">Play melody</button>
      <button class="btn small" id="xCopy">Copy MusicXML</button>
      <button class="btn small" id="xSave">Save MusicXML…</button>
      <button class="btn small" id="sUp">Add sheet music…</button>
      <button class="btn small" id="sClear">Clear notes</button>
    </div>
    <p class="small muted">Click the staff to place a note at that pitch. Click a note to select it; the ↑ ↓ keys move it, ← → pick the next note, Delete removes it, and the lyric box puts a word under it. Changes save on their own. Add sheet music takes a PDF, an image, or a MusicXML file from MuseScore.</p>`;
  renderRows();
  b.querySelectorAll<HTMLElement>('[data-dur]').forEach((x) => (x.onclick = () => {
    dur = x.dataset.dur as Note['d'];
    if (sel >= 0) { s.notes[sel].d = dur; save(); }
    renderSheet(b, s, afterChange);
  }));
  $('#nUp').onclick = () => moveSel(1);
  $('#nDown').onclick = () => moveSel(-1);
  $('#nDel').onclick = delSel;
  $<HTMLInputElement>('#nLyric').oninput = (e) => {
    if (sel < 0) return;
    s.notes[sel].w = (e.target as HTMLInputElement).value.trim();
    renderRows();
    save();
  };
  $<HTMLInputElement>('#chordsIn').onchange = (e) => {
    s.chords = (e.target as HTMLInputElement).value.split(/[\s,|]+/).filter(Boolean);
    renderRows();
    save();
  };
  $('#tDown').onclick = () => { transpose--; renderSheet(b, s, afterChange); };
  $('#tUp').onclick = () => { transpose++; renderSheet(b, s, afterChange); };
  $('#sPlay').onclick = () => {
    if (!s.notes.length) return toast('Add some notes first.');
    if (playing && playing.id === String(s.id)) return stop();
    playMelody(s, transpose);
  };
  $('#xCopy').onclick = () => copyText(toMusicXML(s.title, s.notes, s.chords, s.bpm), 'MusicXML');
  $('#xSave').onclick = async () => {
    const path = await pickSave({ defaultPath: `${s.title}.musicxml`, filters: [{ name: 'MusicXML', extensions: ['musicxml'] }] });
    if (!path) return;
    await api.exportText(path, toMusicXML(s.title, s.notes, s.chords, s.bpm)).then(() => toast('MusicXML saved'), (e) => toast(errText(e)));
  };
  $('#sClear').onclick = () => { s.notes = []; sel = -1; save(); renderSheet(b, s, afterChange); };
  $('#sUp').onclick = async () => {
    const path = await pickFile({ multiple: false, filters: [{ name: 'Sheet music', extensions: ['pdf', 'png', 'jpg', 'jpeg', 'gif', 'webp', 'musicxml', 'xml'] }] });
    if (typeof path !== 'string') return;
    try {
      const sheet = await api.addSheet(s.id, path);
      if (sheet.kind === 'musicxml') await loadXml(sheet.id);
      s.sheets.push(sheet);
      renderSheet(b, s, afterChange);
      toast(`${sheet.name} added`);
    } catch (e) {
      toast(errText(e));
    }
  };
  b.querySelectorAll<HTMLElement>('[data-rm-sheet]').forEach((x) => (x.onclick = async () => {
    const id = Number(x.dataset.rmSheet);
    await api.removeSheet(id).catch((e) => toast(errText(e)));
    s.sheets = s.sheets.filter((f) => f.id !== id);
    renderSheet(b, s, afterChange);
  }));
  b.querySelectorAll<HTMLElement>('[data-load-xml]').forEach((x) => (x.onclick = async () => { await loadXml(Number(x.dataset.loadXml)); renderSheet(b, s, afterChange); }));
}

async function loadXml(sheetId: number) {
  const s = song!;
  const r = fromMusicXML(await api.readSheetText(sheetId));
  if (!r || !r.notes.length) return toast('No melody found in that file. Export it from MuseScore as uncompressed MusicXML.');
  s.notes = r.notes;
  if (r.chords.length) s.chords = r.chords;
  sel = -1;
  save();
  toast(`Loaded ${r.notes.length} notes`);
}

function renderRows() {
  const s = song!;
  let ms: Placed[][] = toMeasures(s.notes);
  if (!s.notes.length) ms = [[]];
  const rows: Placed[][][] = [];
  for (let i = 0; i < ms.length; i += 4) {
    const r = ms.slice(i, i + 4);
    while (r.length < 4) r.push([]);
    rows.push(r);
  }
  const box = $('#rows');
  box.innerHTML = rows.map((r, ri) => staffSVG(r, { chords: s.chords, chordAt: ri * 4, semis: transpose, timesig: ri === 0, edit: true, sel, title: s.title })).join('');
  box.querySelectorAll<SVGSVGElement>('svg').forEach((svg) => (svg.onclick = onStaffClick));
}

function onStaffClick(e: MouseEvent) {
  const s = song!, svg = e.currentTarget as SVGSVGElement;
  const hit = (e.target as Element).closest<SVGElement>('.nh');
  if (hit) {
    sel = Number(hit.dataset.n);
    dur = s.notes[sel].d;
    return renderSheet(host!, s, onSaved);
  }
  const pt = svg.createSVGPoint();
  pt.x = e.clientX;
  pt.y = e.clientY;
  const p = pt.matrixTransform(svg.getScreenCTM()!.inverse());
  const top = Number(svg.dataset.top), g = Number(svg.dataset.g), shift = Number(svg.dataset.shift);
  const st = Math.max(-6, Math.min(14, Math.round((top + 4 * g - p.y) / (g / 2)))) - shift;
  const at = sel >= 0 ? sel + 1 : s.notes.length;
  s.notes.splice(at, 0, { s: st, d: dur, w: '' });
  sel = at;
  blip(st, transpose);
  save();
  renderSheet(host!, s, onSaved);
}

function moveSel(d: number) {
  if (sel < 0 || !song) return;
  const n = song.notes[sel];
  n.s = Math.max(-6, Math.min(14, n.s + d));
  blip(n.s, transpose);
  renderRows();
  save();
}

function delSel() {
  if (sel < 0 || !song) return;
  song.notes.splice(sel, 1);
  sel = Math.min(sel, song.notes.length - 1);
  save();
  renderSheet(host!, song, onSaved);
}

/** Keyboard editing while the sheet tab is open. */
export function initSheetKeys(isSheetOpen: () => boolean) {
  document.addEventListener('keydown', (e) => {
    if (!isSheetOpen() || !song || /INPUT|TEXTAREA|SELECT/.test((e.target as HTMLElement).tagName)) return;
    if (e.key === 'ArrowUp') { e.preventDefault(); moveSel(1); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); moveSel(-1); }
    else if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); delSel(); }
    else if (e.key === 'ArrowRight' && sel < song.notes.length - 1) { sel++; renderSheet(host!, song, onSaved); }
    else if (e.key === 'ArrowLeft' && sel > 0) { sel--; renderSheet(host!, song, onSaved); }
  });
  setMelodyListener((s, i) => {
    document.querySelectorAll('#rows .nh.on').forEach((x) => x.classList.remove('on'));
    if (song && s.id === song.id && i >= 0) document.querySelector(`#rows .nh[data-n="${i}"]`)?.classList.add('on');
  });
}
