// Playback: real audio through the lt:// protocol, a synth preview for songs that only have a melody,
// the now-playing queue and repeat modes. The analyser feeds both visualizers.
import { api, lt, type Song, type Stem } from './api';
import { applePause, applePosition, appleResume, appleSeek, appleStop, artUrl, playApple } from './apple';
import { BEATS, IDX, SCALE, keyName } from './music/notation';
import { byId, playable, saveSetting, setting, viewName, type Repeat } from './state';
import { $, cssUrl, esc, hueOf, PAUSE, PLAY, toast } from './util';

let ac: AudioContext | null = null;
let master: GainNode | null = null;
export let analyser: AnalyserNode | null = null;
let media: HTMLAudioElement | null = null;

type StemTrack = { name: string; el: HTMLAudioElement; gain: GainNode; src: MediaElementAudioSourceNode };
type Playing =
  | { kind: 'file'; id: string }
  | { kind: 'synth'; id: string; nodes: OscillatorNode[]; tick: number }
  | { kind: 'stems'; id: string; tracks: StemTrack[] }
  | { kind: 'apple'; id: string };
export let playing: Playing | null = null;
let nowSong: Song | null = null;
let lastPlay: (() => void) | null = null;
export let queue: Song[] = [];
export let qi = -1;
let queueFrom = '';
export let repeat: Repeat = 'off';
/** Shuffle: the list plays in a random order; turning it off goes back to the list's own order. */
export let shuffle = false;
let unshuffled: Song[] | null = null;

function shuffled<T>(a: T[]): T[] {
  const b = a.slice();
  for (let i = b.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [b[i], b[j]] = [b[j], b[i]];
  }
  return b;
}

/** Puts the current song first and the rest in a random order (or back in order). */
function reorder() {
  if (shuffle) {
    unshuffled = queue.slice();
    const cur = qi >= 0 ? queue[qi] : undefined;
    queue = cur ? [cur, ...shuffled(queue.filter((_, i) => i !== qi))] : shuffled(queue);
    qi = cur ? 0 : -1;
  } else if (unshuffled) {
    const cur = qi >= 0 ? queue[qi] : undefined;
    // songs added while shuffled stay, after the original list
    queue = [...unshuffled.filter((s) => queue.includes(s)), ...queue.filter((s) => !unshuffled!.includes(s))];
    qi = cur ? queue.indexOf(cur) : -1;
    unshuffled = null;
  }
}

export function setShuffle(on: boolean) {
  if (shuffle === on) return;
  shuffle = on;
  void saveSetting('shuffle', on);
  reorder();
  syncShuffle();
  renderQueue();
  saveNowPlaying();
}

/** Plays a list in a random order from a random song (a playlist, a genre, or the whole library). */
export function playShuffled(list: Song[]) {
  const l = list.filter(playable);
  if (!l.length) return toast('Nothing here can be played.');
  shuffle = true;
  void saveSetting('shuffle', true);
  syncShuffle();
  stop();
  playSong(l[Math.floor(Math.random() * l.length)], l);
}

function syncShuffle() {
  const b = document.getElementById('sMode');
  if (!b) return;
  b.innerHTML = SHUFFLE_ICON;
  b.setAttribute('aria-pressed', String(shuffle));
  const label = shuffle ? 'Shuffle is on' : 'Shuffle is off';
  b.setAttribute('aria-label', label);
  b.title = label;
}
const SHUFFLE_ICON = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 3h5v5"/><path d="M4 20 21 3"/><path d="M21 16v5h-5"/><path d="m15 15 6 6"/><path d="M4 4l5 5"/></svg>';
/** Paused keeps the song and its place; Stop clears the place. */
export let paused = false;
/** Where the next start of the current song begins (a restored position after relaunch). */
let resumeAt = 0;
let lastSaved = 0;
/** Bumped by every stop and every new start; a start that awaited something checks it is still the latest. */
let playGen = 0;

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
  playGen++;
  if (playing) {
    if (playing.kind === 'synth') {
      playing.nodes.forEach((n) => { try { n.stop(); } catch { /* already stopped */ } });
      clearInterval(playing.tick);
      if (nowSong) onMelodyNote(nowSong, -1);
    } else if (playing.kind === 'apple') {
      void appleStop();
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
  paused = false;
  resumeAt = 0;
  paintScrub(0, nowSong?.duration ?? 0);
  saveNowPlaying();
  syncIcons();
}

function mediaEls(): HTMLMediaElement[] {
  if (playing?.kind === 'file') return media ? [media] : [];
  if (playing?.kind === 'stems') return playing.tracks.map((t) => t.el);
  return [];
}

/** Current place and length of what is playing, or of the paused/restored song. */
export function position() {
  if (playing?.kind === 'apple') {
    const p = applePosition();
    return { cur: p.cur, dur: p.dur || nowSong?.duration || 0 };
  }
  const e = mediaEls()[0];
  const fallback = nowSong?.duration ?? 0;
  if (!e) return { cur: resumeAt, dur: fallback };
  return { cur: e.currentTime, dur: Number.isFinite(e.duration) && e.duration > 0 ? e.duration : fallback };
}

/** Pause or resume without losing the place. A melody preview simply stops. */
export function togglePause() {
  if (!playing) return;
  if (playing.kind === 'synth') return stop();
  if (playing.kind === 'apple') {
    if (paused) void appleResume();
    else void applePause();
    paused = !paused;
    saveNowPlaying();
    return syncIcons();
  }
  const els = mediaEls();
  if (paused) {
    void ac?.resume();
    const t = els[0]?.currentTime ?? 0;
    for (const e of els) {
      if (els.length > 1) e.currentTime = t;
      void e.play();
    }
    paused = false;
  } else {
    for (const e of els) e.pause();
    paused = true;
  }
  saveNowPlaying();
  syncIcons();
}

/** Moves to a fraction of the song. Works on a paused or restored song too. */
export function seekTo(fraction: number) {
  const f = Math.max(0, Math.min(1, fraction));
  const { dur } = position();
  const els = mediaEls();
  if (playing?.kind === 'apple' && dur) void appleSeek(f * dur);
  else if (els.length && dur) for (const e of els) e.currentTime = f * dur;
  else resumeAt = f * dur;
  paintScrub(f * dur, dur);
}

const fmt = (sec: number) => {
  if (!Number.isFinite(sec) || sec < 0) sec = 0;
  const m = Math.floor(sec / 60), s = Math.floor(sec % 60);
  return `${m}:${String(s).padStart(2, '0')}`;
};
let dragging = false;
function paintScrub(cur: number, dur: number) {
  const pct = dur ? Math.min(100, (cur / dur) * 100) : 0;
  $('#pProg').style.width = pct + '%';
  $('#pHead').style.left = pct + '%';
  $('#pCur').textContent = fmt(cur);
  $('#pDur').textContent = nowSong?.provider === 'radio' ? 'Live' : dur ? fmt(dur) : '0:00';
}

/** Remembers the song, its list and the place in it, so the app can pick up where it left off. */
export function saveNowPlaying(force = true) {
  if (!nowSong) return;
  const now = Date.now();
  if (!force && now - lastSaved < 10000) return;
  lastSaved = now;
  void saveSetting('nowPlaying', { songId: nowSong.id, queue: queue.map((s) => s.id), qi, from: queueFrom, pos: Math.round(position().cur * 10) / 10, paused: !playing || paused });
}

/** On launch: shows the last song paused at its place; Play continues from there. */
export function restoreNowPlaying() {
  const np = setting<{ songId: number; queue: number[]; qi: number; from: string; pos: number } | null>('nowPlaying', null);
  const s = np && byId(np.songId);
  if (!np || !s) return;
  queue = np.queue.map((id) => byId(id)).filter((x): x is Song => !!x && playable(x));
  qi = queue.findIndex((x) => x.id === s.id);
  queueFrom = np.from;
  showNow(s, [s.artist, s.album].filter(Boolean).join(', '));
  resumeAt = np.pos || 0;
  lastPlay = () => playSong(s);
  paintScrub(resumeAt, s.duration ?? 0);
}

/** Where streamed songs get their audio address, by provider (radio, connected servers). Set by ui/sources.ts. */
export const streamSources: Record<string, (s: Song) => Promise<string>> = {};

/** Shows a station's current song under its name. */
export function setNowSub(songId: number, text: string) {
  if (nowSong?.id === songId) $('#pSub').textContent = text;
}

export function playSong(s: Song, list?: Song[]) {
  if (list) {
    queue = list.filter(playable);
    qi = queue.findIndex((x) => x.id === s.id);
    queueFrom = viewName();
    unshuffled = null;
    if (shuffle) reorder();
  }
  if (playing && playing.id === String(s.id)) return togglePause();
  const startAt = nowSong?.id === s.id ? resumeAt : 0;
  if (s.provider === 'apple') return void startApple(s, startAt);
  const stream = streamSources[s.provider];
  if (stream) {
    // A newer pick (or Stop) while the address loads wins.
    const gen = ++playGen;
    stream(s)
      .then((url) => gen === playGen && playFile(url, s, s.artist || s.album, String(s.id), s.provider === 'radio' ? 0 : startAt))
      .catch((e) => gen === playGen && toast(`Could not play "${s.title}": ${e}`));
    return;
  }
  if (s.hasFile) playFile(lt('audio', s.id), s, [s.artist, s.album].filter(Boolean).join(', '), String(s.id), startAt);
  else if (s.stems > 0) void playStems(s, startAt);
  else if (s.notes.length) playMelody(s);
  else toast('This song has no audio or melody yet.');
}

export function playTake(s: Song, takeId: number, label: string) {
  const key = `${s.id}:t${takeId}`;
  if (playing && playing.id === key) return togglePause();
  playFile(lt('take', takeId), s, label, key);
}

function playFile(url: string, s: Song, sub: string, id: string, startAt = 0) {
  stop();
  if (!ensureAudio()) return;
  void ac!.resume();
  if (!media) {
    media = new Audio();
    media.crossOrigin = 'anonymous';
    ac!.createMediaElementSource(media).connect(master!);
    media.onended = onEnded;
    media.onerror = () => {
      const e = media!.error;
      void api.logEvent('error', `audio element failed: code=${e?.code} message=${e?.message ?? ''} src=${media!.currentSrc}`);
    };
  }
  const gen = playGen;
  media.src = url;
  if (startAt > 0) media.addEventListener('loadedmetadata', () => { if (gen === playGen) media!.currentTime = startAt; }, { once: true });
  media.play().catch((err) => {
    // A later song or Stop cut this start short (AbortError): not a failure, and must not stop the new song.
    if (gen !== playGen || err?.name === 'AbortError') return;
    void api.logEvent('error', `play() rejected for ${url}: ${err?.name ?? ''} ${err?.message ?? err}`);
    stop();
    toast('This file could not be played. The format may not be supported.');
  });
  playing = { kind: 'file', id };
  paused = false;
  resumeAt = 0;
  showNow(s, sub);
  lastPlay = () => playFile(url, s, sub, id, takeResume());
  saveNowPlaying();
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

/** Apple Music songs play in Apple's own player; the app's queue and controls still drive them. */
async function startApple(s: Song, startAt: number) {
  stop();
  const gen = playGen;
  const id = String(s.id);
  playing = { kind: 'apple', id };
  paused = false;
  resumeAt = 0;
  showNow(s, [s.artist, s.album].filter(Boolean).join(', ') + ' · Apple Music');
  lastPlay = () => void startApple(s, takeResume());
  syncIcons();
  try {
    await playApple(s, startAt, () => {
      if (gen === playGen) onEnded();
    }, () => gen === playGen);
  } catch (e) {
    if (gen !== playGen) return;
    playing = null;
    syncIcons();
    toast(`Apple Music: ${e instanceof Error ? e.message : String(e)}`);
  }
}

const takeResume = () => {
  const r = resumeAt;
  resumeAt = 0;
  return r;
};

export async function playStems(s: Song, startAt = 0) {
  stop();
  const gen = playGen;
  if (!ensureAudio()) return;
  void ac!.resume();
  const detail = await api.songDetail(s.id);
  // Pressed twice, or another song started while the details loaded: only the latest start plays.
  if (gen !== playGen) return;
  const files = pickStems(detail.stemFiles);
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
  paused = false;
  resumeAt = 0;
  applyMix(s.id);
  showNow(s, `${tracks.length} stems${[s.artist].filter(Boolean).map((a) => ', ' + a).join('')}`);
  lastPlay = () => void playStems(s, takeResume());
  saveNowPlaying();
  syncIcons();
  // Start every part together once each can play, so they stay in step.
  await Promise.all(tracks.map((t) => new Promise<void>((res) => {
    if (t.el.readyState >= 3) return res();
    t.el.addEventListener('canplay', () => res(), { once: true });
    t.el.addEventListener('error', () => res(), { once: true });
  })));
  if (gen !== playGen || playing?.kind !== 'stems' || playing.id !== id) return;
  const lead = tracks[0].el;
  lead.onended = onEnded;
  if (startAt > 0) for (const t of tracks) t.el.currentTime = startAt;
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
      paintScrub(Math.min(el, total), total);
      const i = times.findIndex((x, k) => el >= x && (k === times.length - 1 || el < times[k + 1]));
      if (i !== lastNote) onMelodyNote(song, (lastNote = i));
      if (el > total + 0.1) onEnded();
    }, 60),
  };
  paused = false;
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

/** Adds songs to the queue, right after the current song or at the end. Nothing starts playing by itself. */
export function enqueue(songs: Song[], next: boolean) {
  const add = songs.filter(playable);
  if (next) queue.splice(qi + 1, 0, ...add);
  else queue.push(...add);
  renderQueue();
  saveNowPlaying();
}

/** Carries out a request from a connected AI app (Settings › AI apps › Full control). */
/** Extra AI actions other modules handle (e.g. play_station in ui/sources.ts). */
export const aiHooks: Record<string, (cmd: Record<string, unknown>) => void> = {};

export function aiControl(cmd: { action: string; songIds?: number[]; shuffle?: boolean; next?: boolean }) {
  const hook = aiHooks[cmd.action];
  if (hook) return hook(cmd as Record<string, unknown>);
  const songs = (cmd.songIds ?? []).map((id) => byId(id)).filter((s): s is Song => !!s && playable(s));
  switch (cmd.action) {
    case 'play': {
      if (cmd.shuffle) for (let i = songs.length - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [songs[i], songs[j]] = [songs[j], songs[i]]; }
      if (!songs.length) return;
      stop();
      playSong(songs[0], songs);
      queueFrom = 'Picked by your AI';
      renderQueue();
      return;
    }
    case 'queue': return enqueue(songs, !!cmd.next);
    case 'pause': if (playing && !paused) togglePause(); return;
    case 'resume':
      if (playing && paused) togglePause();
      else if (!playing) togglePlay([]);
      return;
    case 'next': return step(1);
    case 'previous': return step(-1);
    case 'stop': return stop();
  }
}

export function togglePlay(fallback: Song[]) {
  if (playing) return togglePause();
  if (lastPlay) return lastPlay();
  const l = fallback.filter(playable);
  if (l.length) playSong(l[0], l);
}

const REPEAT_ICON = (one: boolean) =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 2l3 3-3 3"/><path d="M4 11V9a4 4 0 0 1 4-4h12"/><path d="M7 22l-3-3 3-3"/><path d="M20 13v2a4 4 0 0 1-4 4H4"/>${one ? '<text x="12" y="15.5" text-anchor="middle" font-size="8.5" font-weight="700" fill="currentColor" stroke="none" font-family="Instrument Sans, sans-serif">1</text>' : ''}</svg>`;
const REPEAT_LABEL = { off: 'Repeat is off', all: 'Repeating the list', one: 'Repeating this song' };

/** One button: off, then repeat the list, then repeat this song, then off. */
export function cycleRepeat() {
  repeat = repeat === 'off' ? 'all' : repeat === 'all' ? 'one' : 'off';
  void saveSetting('repeat', repeat);
  syncRepeat();
  toast(REPEAT_LABEL[repeat]);
}
function syncRepeat() {
  const b = $('#rMode');
  b.innerHTML = REPEAT_ICON(repeat === 'one');
  b.setAttribute('aria-pressed', String(repeat !== 'off'));
  b.setAttribute('aria-label', REPEAT_LABEL[repeat]);
  b.title = REPEAT_LABEL[repeat];
}

export function initPlayer() {
  repeat = setting<Repeat>('repeat', 'off');
  syncRepeat();
  shuffle = setting<boolean>('shuffle', false);
  syncShuffle();
  $('#sMode').onclick = () => {
    setShuffle(!shuffle);
    toast(shuffle ? 'Shuffle is on' : 'Shuffle is off: back to the list order');
  };
  applySkin(setting<string>('playerSkin', 'classic'));
  applyLineColor(setting<string>('lineColor', 'theme'));
  $('#pBtn').innerHTML = PLAY;
  // The scrubber: click anywhere to jump, or drag the handle.
  const bar = $('#pBar');
  const frac = (e: PointerEvent) => {
    const r = bar.getBoundingClientRect();
    return (e.clientX - r.left) / r.width;
  };
  bar.addEventListener('pointerdown', (e) => {
    if (!nowSong) return;
    dragging = true;
    bar.classList.add('drag');
    bar.setPointerCapture(e.pointerId);
    const { dur } = position();
    paintScrub(Math.max(0, Math.min(1, frac(e))) * dur, dur);
  });
  bar.addEventListener('pointermove', (e) => {
    if (!dragging) return;
    const { dur } = position();
    paintScrub(Math.max(0, Math.min(1, frac(e))) * dur, dur);
  });
  const end = (e: PointerEvent) => {
    if (!dragging) return;
    dragging = false;
    bar.classList.remove('drag');
    seekTo(frac(e));
    saveNowPlaying();
  };
  bar.addEventListener('pointerup', end);
  bar.addEventListener('pointercancel', end);
  addEventListener('beforeunload', () => saveNowPlaying());
  requestAnimationFrame(tick);
}

const spec = new Uint8Array(512);
function tick() {
  requestAnimationFrame(tick);
  if (playing && playing.kind !== 'synth' && !dragging) {
    const { cur, dur } = position();
    paintScrub(cur, dur);
    if (!paused) saveNowPlaying(false);
  }
  drawSpectrum();
}

/** Player bar looks; "classic" is the original. */
export const SKINS = [
  ['classic', 'Classic', 'The original bar across the bottom.'],
  ['float', 'Floating glass', 'A rounded glass bar that floats above the bottom edge, with a bold play button.'],
  ['aurora', 'Aurora', 'A glowing gradient scrubber with the music’s spectrum rising behind the bar.'],
  ['minimal', 'Minimal', 'No panel at all: slim controls sitting over the background.'],
] as const;
export function applySkin(skin: string) {
  $('.player').dataset.skin = SKINS.some(([k]) => k === skin) ? skin : 'classic';
}

/** Color of the progress line; the skin's own look when 'theme'. */
export const LINE_COLORS = [['theme', 'Match the bar style'], ['rainbow', 'Rainbow'], ['white', 'White'], ['gold', 'Gold']] as const;
export function applyLineColor(c: string) {
  $('.player').dataset.line = LINE_COLORS.some(([k]) => k === c) ? c : 'theme';
}
function drawSpectrum() {
  const c = document.getElementById('pSpec') as HTMLCanvasElement | null;
  if (!c || $('.player').dataset.skin !== 'aurora') return;
  const w = (c.width = c.clientWidth), h = (c.height = c.clientHeight);
  const x = c.getContext('2d')!;
  x.clearRect(0, 0, w, h);
  if (!analyser || !playing || paused) return;
  analyser.getByteFrequencyData(spec);
  const n = 64, bw = w / n;
  const g = x.createLinearGradient(0, 0, w, 0);
  g.addColorStop(0, 'rgba(154,166,255,.55)');
  g.addColorStop(0.5, 'rgba(94,231,223,.5)');
  g.addColorStop(1, 'rgba(255,126,182,.55)');
  x.fillStyle = g;
  for (let i = 0; i < n; i++) {
    const v = spec[Math.floor(Math.pow(i / n, 1.6) * 300) + 1] / 255;
    const bh = Math.max(1, v * h * 0.95);
    x.fillRect(i * bw + 1, h - bh, bw - 2, bh);
  }
}

function showNow(s: Song, sub: string) {
  nowSong = s;
  $('#pTitle').textContent = s.title;
  $('#pSub').textContent = sub;
  const a = $('#pArt');
  const img = s.art ? lt('art', s.art) : artUrl(s.artUrl, 120);
  a.hidden = !img;
  if (img) a.style.backgroundImage = cssUrl(img);
}

/** After the library reloads, keep queue entries pointing at fresh song objects. */
export function refreshQueue() {
  // Streamed songs (radio, servers, unsaved Apple Music) aren't in the library; keep them as they are.
  queue = queue.map((s) => (s.id < 0 ? s : byId(s.id))).filter((s): s is Song => !!s);
  if (nowSong) nowSong = byId(nowSong.id) ?? nowSong;
  qi = nowSong ? queue.findIndex((s) => s.id === nowSong!.id) : -1;
}

export function syncIcons() {
  document.querySelectorAll<HTMLElement>('[data-play]').forEach((b) => {
    const on = !!playing && playing.id === b.dataset.play && !paused;
    b.innerHTML = on ? PAUSE : PLAY;
    b.setAttribute('aria-label', on ? 'Pause' : 'Play');
  });
  const live = !!playing && !paused;
  $('#pBtn').innerHTML = live ? PAUSE : PLAY;
  $('#pBtn').setAttribute('aria-label', live ? 'Pause' : 'Play');
  renderQueue();
}

// ---------- now playing list ----------

export function thumb(s: Song) {
  if (s.art) return `<span class="th" style="background-image:${esc(cssUrl(lt('art', s.art)))}"></span>`;
  if (s.artUrl) return `<span class="th" style="background-image:${esc(cssUrl(artUrl(s.artUrl, 80)!))}"></span>`;
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
        <span class="nm"><b>${esc(s.title)}</b><span>${esc([s.artist, s.provider === 'apple' ? 'Apple Music' : s.genre].filter(Boolean).join(', '))}</span></span>
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
