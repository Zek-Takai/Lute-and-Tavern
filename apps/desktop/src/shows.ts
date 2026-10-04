// Visualizer shows (tools/show-analyzer): what happens in a song and exactly when, made once ahead of time.
// While a song with a show plays, the visualizer follows the show at the song's playback position; a song
// without one uses live listening (audiofeatures.ts).
import { invoke } from '@tauri-apps/api/core';
import { byId } from './state';

type Hit = [number, number]; // time, strength 0..1
interface Env { rate: number; v: number[] }
export interface Show {
  uid: string;
  duration: number;
  tempo: number;
  beats: number[];
  downbeats: number[];
  sections: { start: number; end: number; energy: number; group: number }[];
  hits: { kick: Hit[]; snare: Hit[]; hat: Hit[] };
  bass: { notes: Hit[]; env: Env };
  other: { notes: Hit[]; env: Env };
  vocal: { phrases: [number, number][]; env: Env; pitch: { rate: number; midi: number[] } };
  mix: { env: Env };
  key: { tonic: string; mode: string };
  /** Made here when loaded: the strength a hit needs to count in this song (its strongest 70%, at most
   *  0.25), and its strong-hit level, so one huge hit doesn't make the rest of the song look weak. */
  cut?: Record<'kick' | 'snare' | 'hat', { min: number; top: number }>;
}

function prepare(s: Show): Show {
  const q = (xs: number[], p: number) => (xs.length ? [...xs].sort((a, b) => a - b)[Math.floor((xs.length - 1) * p)] : 0);
  const c = (hits: Hit[]) => {
    const xs = hits.map((h) => h[1]);
    return { min: Math.min(0.25, q(xs, 0.3)), top: Math.max(0.05, q(xs, 0.95)) };
  };
  s.cut = { kick: c(s.hits.kick), snare: c(s.hits.snare), hat: c(s.hits.hat) };
  return s;
}

const shows = new Map<string, Show | null>();
const loading = new Set<string>();

/** The show for a song (by library id), if there is one; starts loading it if not loaded yet. */
export function showForSong(songId: number): Show | null {
  const uid = byId(songId)?.uid;
  if (!uid) return null;
  if (shows.has(uid)) return shows.get(uid)!;
  if (!loading.has(uid)) {
    loading.add(uid);
    invoke<string | null>('song_show', { uid })
      .then((text) => shows.set(uid, text ? prepare(JSON.parse(text) as Show) : null))
      .catch(() => shows.set(uid, null))
      .finally(() => loading.delete(uid));
  }
  return null;
}

/** Loads a song's show ahead of time (the next song in the list), so it's ready when the song starts. */
export function preloadShow(songId: number) {
  showForSong(songId);
}

/** A level from an envelope at a time, 0..1, smoothly between samples. */
export function envAt(e: Env, t: number): number {
  const i = t * e.rate, a = Math.floor(i);
  if (a < 0 || a >= e.v.length) return 0;
  const b = Math.min(a + 1, e.v.length - 1), f = i - a;
  return (e.v[a] * (1 - f) + e.v[b] * f) / 255;
}

/** The strongest hit between two times (0 if none); hits are sorted by time. */
export function hitBetween(list: Hit[], from: number, to: number, min = 0): number {
  let lo = 0, hi = list.length;
  while (lo < hi) {
    const m = (lo + hi) >> 1;
    if (list[m][0] <= from) lo = m + 1;
    else hi = m;
  }
  let best = 0;
  for (let i = lo; i < list.length && list[i][0] <= to; i++) if (list[i][1] >= min) best = Math.max(best, list[i][1]);
  return best;
}

/** How many hits fall in a time window. */
export function hitsIn(list: Hit[], from: number, to: number): number {
  let n = 0;
  for (const h of list) {
    if (h[0] > to) break;
    if (h[0] > from) n++;
  }
  return n;
}

export const sectionAt = (s: Show, t: number) => s.sections.find((x) => t >= x.start && t < x.end) ?? null;
export const singingAt = (s: Show, t: number) => s.vocal.phrases.some(([a, b]) => t >= a && t < b);
export function pitchAt(s: Show, t: number): number | null {
  const m = s.vocal.pitch.midi[Math.floor(t * s.vocal.pitch.rate)] ?? 0;
  return m ? Math.max(0, Math.min(1, (m - 45) / 36)) : null;
}
