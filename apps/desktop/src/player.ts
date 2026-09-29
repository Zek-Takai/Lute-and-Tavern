// Playback: real audio through the lt:// protocol, a synth preview for songs that only have a melody,
// the now-playing queue and repeat modes. The analyser feeds both visualizers.
import { api, lt, type Song, type Stem } from './api';
import { BEATS, IDX, SCALE, keyName } from './music/notation';
import { byId, playable, saveSetting, setting, viewName, type Repeat } from './state';
import { $, esc, hueOf, PLAY, STOP, toast } from './util';

let ac: AudioContext | null = null;
let master: GainNode | null = null;
export let analyser: AnalyserNode | null = null;
let media: HTMLAudioElement | null = null;

type StemTrack = { name: string; el: HTMLAudioElement; gain: GainNode; src: MediaElementAudioSourceNode };
type Playing =
  | { kind: 'file'; id: string }
  | { kind: 'synth'; id: string; nodes: OscillatorNode[]; tick: number }
  | { kind: 'stems'; id: string; tracks: StemTrack[] };
export let playing: Playing | null = null;
let nowSong: Song | null = null;
let lastPlay: (() => void) | null = null;
export let queue: Song[] = [];
export let qi = -1;
let queueFrom = '';
export let repeat: Repeat = 'off';

/** Called with the index of the note sounding during a melody preview (-1 when it stops). */
export let onMelodyNote: (song: Song, i: number) => void = () => {};
export const setMelodyListener = (fn: typeof onMelodyNote) => (onMelodyNote = fn);

function ensureAudio() {
  if (ac) return true;
  try {
    ac = new AudioContext();
    master = ac.createGain();
    analyser = ac.createAnalyser();
    analyser.fftSize = 1024;
    analyser.smoothingTimeConstant = 0.78;
    master.connect(analyser);
    analyser.connect(ac.destination);
    return true;
  } catch {
    toast('Audio is not available on this computer');
    return false;
  }
}

export function stop() {
  if (playing) {
    if (playing.kind === 'synth') {
      playing.nodes.forEach((n) => { try { n.stop(); } catch { /* already stopped */ } });
      clearInterval(playing.tick);
      if (nowSong) onMelodyNote(nowSong, -1);
    } else if (playing.kind === 'stems') {
      for (const t of playing.tracks) {
        t.el.pause();
        t.src.disconnect();
        t.gain.disconnect();
        t.el.removeAttribute('src');
      }
    } else media?.pause();
    playing = null;
  }
  $('#pProg').style.width = '0';
  syncIcons();
}

export function playSong(s: Song, list?: Song[]) {
  if (list) {
    queue = list.filter(playable);
    qi = queue.findIndex((x) => x.id === s.id);
    queueFrom = viewName();
  }
  if (playing && playing.id === String(s.id)) {
    stop();
    return;
  }
  if (s.hasFile) playFile(lt('audio', s.id), s, [s.artist, s.album].filter(Boolean).join(', '), String(s.id));
  else if (s.stems > 0) void playStems(s);
  else if (s.notes.length) playMelody(s);
  else toast('This song has no audio or melody yet.');
}

export function playTake(s: Song, takeId: number, label: string) {
  const key = `${s.id}:t${takeId}`;
  if (playing && playing.id === key) return stop();
  playFile(lt('take', takeId), s, label, key);
}

function playFile(url: string, s: Song, sub: string, id: string) {
  stop();
  if (!ensureAudio()) return;
  void ac!.resume();
  if (!media) {
    media = new Audio();
    media.crossOrigin = 'anonymous';
    ac!.createMediaElementSource(media).connect(master!);
    media.ontimeupdate = () => {
      if (playing?.kind === 'file' && media!.duration) $('#pProg').style.width = (media!.currentTime / media!.duration) * 100 + '%';
    };
    media.onended = onEnded;
    media.onerror = () => {
      const e = media!.error;
      void api.logEvent('error', `audio element failed: code=${e?.code} message=${e?.message ?? ''} src=${media!.currentSrc}`);
    };
  }
  media.src = url;
  media.play().catch((err) => {
    void api.logEvent('error', `play() rejected for ${url}: ${err?.name ?? ''} ${err?.message ?? err}`);
    stop();
    toast('This file could not be played. The format may not be supported.');
  });
  playing = { kind: 'file', id };
  showNow(s, sub);
  lastPlay = () => playFile(url, s, sub, id);
  syncIcons();
}

// ---------- stems: parts of one song played together ----------

/** Per-part mixer settings, kept while the app runs. Keyed by song id, then part name. */
const mix = new Map<number, Map<string, { vol: number; mute: boolean; solo: boolean }>>();
export function stemMix(songId: number, name: string) {
  let m = mix.get(songId);
  if (!m) mix.set(songId, (m = new Map()));
  if (!m.has(name)) m.set(name, { vol: 1, mute: false, solo: false });
  return m.get(name)!;
}
function applyMix(songId: number) {
  if (playing?.kind !== 'stems' || playing.id !== String(songId)) return;
  const m = mix.get(songId);
  const anySolo = !!m && [...m.values()].some((v) => v.solo);
  for (const t of playing.tracks) {
    const v = stemMix(songId, t.name);
    const audible = anySolo ? v.solo : !v.mute;
    t.gain.gain.setTargetAtTime(audible ? v.vol : 0, ac!.currentTime, 0.02);
  }
}
export function setStemMix(songId: number, name: string, patch: Partial<{ vol: number; mute: boolean; solo: boolean }>) {
  Object.assign(stemMix(songId, name), patch);
  applyMix(songId);
}

/** One file per part: MP3 is lighter to stream than WAV when both exist. */
export function pickStems(files: Stem[]) {
  const byName = new Map<string, Stem>();
  for (const f of files) {
    const cur = byName.get(f.name);
    if (!cur || (cur.format !== 'mp3' && f.format === 'mp3')) byName.set(f.name, f);
  }
  return [...byName.values()];
}

export async function playStems(s: Song) {
  stop();
  if (!ensureAudio()) return;
  void ac!.resume();
  const files = pickStems((await api.songDetail(s.id)).stemFiles);
  if (!files.length) return toast('This song has no stems left.');
  const tracks: StemTrack[] = files.map((f) => {
    const el = new Audio();
    el.crossOrigin = 'anonymous';
    el.preload = 'auto';
    el.src = lt('stem', f.id);
    el.onerror = () => void api.logEvent('error', `stem ${f.name} failed: code=${el.error?.code} ${el.error?.message ?? ''} src=${el.currentSrc}`);
    const src = ac!.createMediaElementSource(el);
    const gain = ac!.createGain();
    src.connect(gain).connect(master!);
    return { name: f.name, el, gain, src };
  });
  const id = String(s.id);
  playing = { kind: 'stems', id, tracks };
  applyMix(s.id);
  showNow(s, `${tracks.length} stems${[s.artist].filter(Boolean).map((a) => ', ' + a).join('')}`);
  lastPlay = () => void playStems(s);
  syncIcons();
  // Start every part together once each can play, so they stay in step.
  await Promise.all(tracks.map((t) => new Promise<void>((res) => {
    if (t.el.readyState >= 3) return res();
    t.el.addEventListener('canplay', () => res(), { once: true });
    t.el.addEventListener('error', () => res(), { once: true });
  })));
  if (playing?.kind !== 'stems' || playing.id !== id) return;
  const lead = tracks[0].el;
  lead.ontimeupdate = () => {
    if (playing?.kind === 'stems' && lead.duration) $('#pProg').style.width = (lead.currentTime / lead.duration) * 100 + '%';
  };
  lead.onended = onEnded;
  await Promise.all(tracks.map((t) => t.el.play().catch((err) => api.logEvent('error', `stem ${t.name} play() rejected: ${err?.name ?? ''} ${err?.message ?? err}`))));
}

export function playMelody(song: Song, semis = 0) {
  stop();
  if (!ensureAudio()) return;
  void ac!.resume();
  const spb = 60 / (song.bpm || 100), t0 = ac!.currentTime + 0.05, nodes: OscillatorNode[] = [], times: number[] = [];
  const base = song.key && (IDX[song.key.replace('m', '')] ?? 0) % 12 > 6 ? -12 : 0;
  let t = t0;
  for (const nt of song.notes) {
    const midi = SCALE[Math.max(0, Math.min(SCALE.length - 1, nt.s + 2))] + base + semis;
    const f = 440 * Math.pow(2, (midi - 69) / 12), d = BEATS[nt.d] * spb;
    const o = ac!.createOscillator(), gn = ac!.createGain();
    o.type = 'triangle';
    o.frequency.value = f;
    gn.gain.setValueAtTime(0, t);
    gn.gain.linearRampToValueAtTime(0.22, t + 0.02);
    gn.gain.exponentialRampToValueAtTime(0.001, t + d * 0.95);
    o.connect(gn).connect(master!);
    o.start(t);
    o.stop(t + d);
    nodes.push(o);
    times.push(t - t0);
    t += d;
  }
  const total = t - t0;
  let lastNote = -2;
  playing = {
    kind: 'synth', id: String(song.id), nodes,
    tick: window.setInterval(() => {
      const el = ac!.currentTime - t0;
      $('#pProg').style.width = Math.min(100, (el / total) * 100) + '%';
      const i = times.findIndex((x, k) => el >= x && (k === times.length - 1 || el < times[k + 1]));
      if (i !== lastNote) onMelodyNote(song, (lastNote = i));
      if (el > total + 0.1) onEnded();
    }, 60),
  };
  showNow(song, `Melody preview${song.key ? ', ' + keyName(song.key, semis) : ''}`);
  lastPlay = () => playMelody(song, semis);
  syncIcons();
}

/** Plays a single note, used when placing notes in the editor. */
export function blip(step: number, semis = 0) {
  if (!ensureAudio()) return;
  void ac!.resume();
  const midi = SCALE[Math.max(0, Math.min(SCALE.length - 1, step + 2))] + semis;
  const o = ac!.createOscillator(), gn = ac!.createGain();
  o.type = 'triangle';
  o.frequency.value = 440 * Math.pow(2, (midi - 69) / 12);
  gn.gain.setValueAtTime(0.2, ac!.currentTime);
  gn.gain.exponentialRampToValueAtTime(0.001, ac!.currentTime + 0.35);
  o.connect(gn).connect(master!);
  o.start();
  o.stop(ac!.currentTime + 0.4);
}

function onEnded() {
  stop();
  if (repeat === 'one' && lastPlay) return lastPlay();
  if (!queue.length || qi < 0) return;
  if (qi + 1 >= queue.length) {
    if (repeat === 'all') playSong(queue[(qi = 0)]);
    return;
  }
  playSong(queue[++qi]);
}

export function step(dir: 1 | -1) {
  if (!queue.length) return;
  let n = qi + dir;
  if (n >= queue.length) {
    if (repeat !== 'all') return toast('End of the list. Turn on Repeat playlist to loop it.');
    n = 0;
  }
  if (n < 0) n = repeat === 'all' ? queue.length - 1 : 0;
  qi = n;
  stop();
  playSong(queue[qi]);
}

export function togglePlay(fallback: Song[]) {
  if (playing) return stop();
  if (lastPlay) return lastPlay();
  const l = fallback.filter(playable);
  if (l.length) playSong(l[0], l);
}

export function setRepeat(m: Exclude<Repeat, 'off'>) {
  repeat = repeat === m ? 'off' : m;
  void saveSetting('repeat', repeat);
  syncRepeat();
  toast(repeat === 'all' ? 'Repeat playlist on' : repeat === 'one' ? 'Repeat song on' : 'Repeat off');
}
function syncRepeat() {
  $('#rAll').setAttribute('aria-pressed', String(repeat === 'all'));
  $('#rOne').setAttribute('aria-pressed', String(repeat === 'one'));
}
export function initPlayer() {
  repeat = setting<Repeat>('repeat', 'off');
  syncRepeat();
  $('#pBtn').innerHTML = PLAY;
}

function showNow(s: Song, sub: string) {
  nowSong = s;
  $('#pTitle').textContent = s.title;
  $('#pSub').textContent = sub;
  const a = $('#pArt');
  a.hidden = !s.art;
  if (s.art) a.style.backgroundImage = `url('${lt('art', s.art)}')`;
}

/** After the library reloads, keep queue entries pointing at fresh song objects. */
export function refreshQueue() {
  queue = queue.map((s) => byId(s.id)).filter((s): s is Song => !!s);
  if (nowSong) nowSong = byId(nowSong.id) ?? nowSong;
  qi = nowSong ? queue.findIndex((s) => s.id === nowSong!.id) : -1;
}

export function syncIcons() {
  document.querySelectorAll<HTMLElement>('[data-play]').forEach((b) => {
    const on = !!playing && playing.id === b.dataset.play;
    b.innerHTML = on ? STOP : PLAY;
    b.setAttribute('aria-label', on ? 'Stop' : 'Play');
  });
  $('#pBtn').innerHTML = playing ? STOP : PLAY;
  renderQueue();
}

// ---------- now playing list ----------

export function thumb(s: Song) {
  if (s.art) return `<span class="th" style="background-image:url('${lt('art', s.art)}')"></span>`;
  const h = hueOf(s.title + s.artist);
  return `<span class="th" style="background-image:linear-gradient(135deg,hsl(${h} 55% 38%),hsl(${(h + 50) % 360} 60% 18%))">${esc((s.title[0] || '♪').toUpperCase())}</span>`;
}

export function renderQueue() {
  if ($('#queue').hidden) return;
  $('#qFrom').textContent = queue.length ? `${queue.length} song${queue.length === 1 ? '' : 's'}${queueFrom ? `, from ${queueFrom}` : ''}` : '';
  $('#qList').innerHTML = queue.length
    ? queue
        .map((s, i) => {
          const now = i === qi && nowSong?.id === s.id;
          return `<li class="qi${now ? ' now' : ''}" draggable="true" data-i="${i}">
        <span class="grip" aria-hidden="true">⋮⋮</span>${thumb(s)}
        <span class="nm"><b>${esc(s.title)}</b><span>${esc([s.artist, s.genre].filter(Boolean).join(', '))}</span></span>
        ${now ? '<canvas id="qEq" width="56" height="36" aria-hidden="true"></canvas>' : '<span></span>'}
        <span class="acts"><button data-mv="-1" aria-label="Move up">▲</button><button data-mv="1" aria-label="Move down">▼</button><button data-rm aria-label="Remove from list">✕</button></span>
      </li>`;
        })
        .join('')
    : '<li class="qempty">Nothing queued yet. Press play on a song, or use Play all on a playlist or genre.</li>';
}

function moveQueue(from: number, to: number) {
  if (to < 0 || to >= queue.length || from === to) return;
  const cur = queue[qi];
  const [x] = queue.splice(from, 1);
  queue.splice(to, 0, x);
  qi = queue.indexOf(cur);
  renderQueue();
}

export function initQueue() {
  const toggle = (open: boolean) => {
    $('#queue').hidden = !open;
    $('#qBtn').setAttribute('aria-expanded', String(open));
    if (open) {
      renderQueue();
      $('#qList').querySelector('.qi.now')?.scrollIntoView({ block: 'nearest' });
    }
  };
  $('#qBtn').onclick = () => toggle($('#queue').hidden === true);
  $('#qClose').onclick = () => toggle(false);
  $('#qList').onclick = (e) => {
    const t = e.target as HTMLElement, li = t.closest<HTMLElement>('.qi');
    if (!li) return;
    const i = Number(li.dataset.i);
    const mv = t.closest<HTMLElement>('[data-mv]');
    if (mv) return moveQueue(i, i + Number(mv.dataset.mv));
    if (t.closest('[data-rm]')) {
      const wasNow = i === qi;
      queue.splice(i, 1);
      if (i < qi || wasNow) qi--;
      return renderQueue();
    }
    if (i === qi && playing) return stop();
    qi = i;
    stop();
    playSong(queue[i]);
  };
  let dragFrom = -1;
  const list = $('#qList');
  list.addEventListener('dragstart', (e) => {
    const li = (e.target as HTMLElement).closest<HTMLElement>('.qi');
    if (!li) return;
    dragFrom = Number(li.dataset.i);
    li.classList.add('drag');
    e.dataTransfer!.effectAllowed = 'move';
    e.dataTransfer!.setData('text/plain', 'queue');
  });
  list.addEventListener('dragover', (e) => {
    const li = (e.target as HTMLElement).closest('.qi');
    if (dragFrom < 0 || !li) return;
    e.preventDefault();
    list.querySelectorAll('.over').forEach((x) => x.classList.remove('over'));
    li.classList.add('over');
  });
  list.addEventListener('drop', (e) => {
    const li = (e.target as HTMLElement).closest<HTMLElement>('.qi');
    if (dragFrom < 0 || !li) return;
    e.preventDefault();
    const f = dragFrom;
    dragFrom = -1;
    moveQueue(f, Number(li.dataset.i));
  });
  list.addEventListener('dragend', () => {
    dragFrom = -1;
    renderQueue();
  });
  requestAnimationFrame(drawEq);
}

const eqBuf = new Uint8Array(512);
const eqLv = [0, 0, 0, 0, 0];
function drawEq() {
  requestAnimationFrame(drawEq);
  const c = document.getElementById('qEq') as HTMLCanvasElement | null;
  if (!c || $('#queue').hidden) return;
  const x = c.getContext('2d')!, on = !!playing && !!analyser;
  if (on) analyser!.getByteFrequencyData(eqBuf);
  x.clearRect(0, 0, 56, 36);
  x.fillStyle = getComputedStyle(document.documentElement).getPropertyValue('--accent') || '#9AA6FF';
  [[1, 4], [4, 10], [10, 24], [24, 60], [60, 160]].forEach(([a, b], i) => {
    let v = 0;
    if (on) {
      for (let k = a; k < b; k++) v += eqBuf[k];
      v /= (b - a) * 255;
    }
    eqLv[i] += ((on ? Math.min(1, v * 1.3) : 0.08) - eqLv[i]) * 0.35;
    const h = Math.max(3, eqLv[i] * 34);
    x.fillRect(2 + i * 11, 36 - h, 7, h);
  });
}

/** Click on the progress bar to seek. */
export function initSeek() {
  $('#pBar').onclick = (e) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const f = (e.clientX - r.left) / r.width;
    if (playing?.kind === 'file' && media?.duration) media.currentTime = f * media.duration;
    if (playing?.kind === 'stems') {
      const d = playing.tracks[0].el.duration;
      if (d) for (const t of playing.tracks) t.el.currentTime = f * d;
    }
  };
}
