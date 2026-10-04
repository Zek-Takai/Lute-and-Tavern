// Listening for the visualizer: what is happening in the music, not just how loud it is. Every number is
// measured against the song itself (a quiet ballad and a loud rock song both use the full range):
//   kick, snare, hat   beats, found as sudden rises (onsets) in their part of the spectrum
//   bass, body, air    how much low end, middle and sparkle there is right now
//   vocal              a voice is singing: tonal sound in the voice range, centered in the stereo mix
//   pitch              roughly how high the voice (or lead) is, 0..1
//   intensity          the section: quiet verse low, big chorus high (loudness over a few seconds vs the song)
//   rise               the music is building up
// When a song plays as stems, the vocals, drums and bass parts are heard separately, so those are exact.
import { audioGraph, playing, stemTaps } from './player';

export interface Features { kick: number; snare: number; hat: number; bass: number; body: number; air: number; vocal: number; pitch: number; intensity: number; rise: number; level: number }
export const features: Features = { kick: 0, snare: 0, hat: 0, bass: 0, body: 0, air: 0, vocal: 0, pitch: 0.5, intensity: 0.5, rise: 0, level: 0 };

const FFT = 2048;
let built: AudioContext | null = null;
let broken: AudioContext | null = null;
let aMid: AnalyserNode, aSide: AnalyserNode;
let mid: Float32Array<ArrayBuffer>, side: Float32Array<ArrayBuffer>, prev: Float32Array<ArrayBuffer>, wave: Float32Array<ArrayBuffer>;
let hz = 23.4;
const stemAn = new Map<AudioNode, { name: string; an: AnalyserNode; buf: Float32Array<ArrayBuffer>; prev: Float32Array<ArrayBuffer> }>();

function analyserOn(ac: AudioContext, node: AudioNode, sink: AudioNode) {
  const an = ac.createAnalyser();
  an.fftSize = FFT;
  an.smoothingTimeConstant = 0.2;
  an.minDecibels = -100;
  an.maxDecibels = -10;
  node.connect(an);
  an.connect(sink);
  return an;
}

/** Mid (left + right: what's centered, where a lead vocal sits) and side (left − right: the wide parts). */
function build() {
  const g = audioGraph();
  if (!g || broken === g.ac) return false;
  if (built === g.ac) return true;
  // set up once per sound graph; if it can't be, don't retry every frame
  broken = g.ac;
  const { ac, master } = g;
  const sink = ac.createGain();
  sink.gain.value = 0; // analysers must be pulled by the graph to run; this keeps them silent
  sink.connect(ac.destination);
  // A splitter's channel handling can't be changed, so a two-channel stage in front of it turns a mono song
  // into left = right (all mid).
  const stereo = ac.createGain();
  stereo.channelCount = 2;
  stereo.channelCountMode = 'explicit';
  stereo.channelInterpretation = 'speakers';
  const split = ac.createChannelSplitter(2);
  master.connect(stereo);
  stereo.connect(split);
  const m = ac.createGain(), sd = ac.createGain();
  const add = (ch: number, to: GainNode, k: number) => {
    const x = ac.createGain();
    x.gain.value = k;
    split.connect(x, ch);
    x.connect(to);
  };
  add(0, m, 0.5); add(1, m, 0.5); add(0, sd, 0.5); add(1, sd, -0.5);
  aMid = analyserOn(ac, m, sink);
  aSide = analyserOn(ac, sd, sink);
  mid = new Float32Array(FFT / 2); side = new Float32Array(FFT / 2); prev = new Float32Array(FFT / 2); wave = new Float32Array(FFT);
  hz = ac.sampleRate / FFT;
  built = ac;
  broken = null;
  return true;
}

const bin = (f: number) => Math.max(1, Math.min(FFT / 2 - 1, Math.round(f / hz)));
const lin = (db: number) => (db <= -100 ? 0 : Math.pow(10, db / 20));

/** Average level (linear) between two frequencies. */
function band(a: Float32Array, lo: number, hi: number) {
  let s = 0;
  const i0 = bin(lo), i1 = bin(hi);
  for (let i = i0; i < i1; i++) s += lin(a[i]);
  return s / Math.max(1, i1 - i0);
}

/** Sudden rise in a range (spectral flux): the start of a hit. */
function flux(a: Float32Array, p: Float32Array, lo: number, hi: number) {
  let s = 0;
  const i0 = bin(lo), i1 = bin(hi);
  for (let i = i0; i < i1; i++) {
    const d = lin(a[i]) - lin(p[i]);
    if (d > 0) s += d;
  }
  return s / Math.max(1, i1 - i0);
}

/** Flatness: 1 = noise (cymbals, hiss), near 0 = clear tones (a voice, a melody). */
function flatness(a: Float32Array, lo: number, hi: number) {
  let lg = 0, ar = 0, n = 0;
  for (let i = bin(lo); i < bin(hi); i++) {
    const v = lin(a[i]) + 1e-9;
    lg += Math.log(v); ar += v; n++;
  }
  return n ? Math.exp(lg / n) / (ar / n) : 1;
}

/** Strongest frequency in a range, as 0..1 on a log scale. */
function peakPitch(a: Float32Array, lo: number, hi: number) {
  let best = -Infinity, at = bin(lo);
  for (let i = bin(lo); i < bin(hi); i++) if (a[i] > best) { best = a[i]; at = i; }
  return Math.log(at * hz / lo) / Math.log(hi / lo);
}

/** A level measured against its own recent range, so every song uses 0..1. */
class Norm {
  lo = 0; hi = 1e-6;
  get(v: number, dt: number) {
    this.hi = Math.max(v, this.hi - (this.hi - this.lo) * dt * 0.08); // the ceiling settles over ~12 s
    this.lo = Math.min(v, this.lo + (v - this.lo) * dt * 0.05);
    return Math.max(0, Math.min(1, (v - this.lo) / Math.max(1e-6, this.hi - this.lo)));
  }
  reset() { this.lo = 0; this.hi = 1e-6; }
}

/** Onsets: a rise well above its recent typical size, not too soon after the last one. */
class Onset {
  mean = 0; dev = 0; since = 1;
  constructor(public gap: number, public k: number) {}
  hit(v: number, dt: number) {
    this.since += dt;
    const hit = v > this.mean + this.k * this.dev && v > 1e-4 && this.since > this.gap;
    this.mean += (v - this.mean) * Math.min(1, dt * 3);
    this.dev += (Math.abs(v - this.mean) - this.dev) * Math.min(1, dt * 3);
    if (hit) this.since = 0;
    return hit;
  }
}

const N = { bass: new Norm(), body: new Norm(), air: new Norm(), vocal: new Norm(), level: new Norm() };
const O = { kick: new Onset(0.11, 1.6), snare: new Onset(0.12, 1.8), hat: new Onset(0.06, 1.5) };
let short = 0, long = 0, song = 0, longPrev = 0, lastId = '';

function stemFor(re: RegExp) {
  for (const s of stemAn.values()) if (re.test(s.name)) return s;
  return null;
}

/** Hears the stems separately when they play (vocals, drums, bass). */
function syncStems(ac: AudioContext) {
  const taps = stemTaps();
  for (const [node] of stemAn) if (!taps.some((t) => t.node === node)) stemAn.delete(node);
  for (const t of taps) {
    if (stemAn.has(t.node)) continue;
    const sink = ac.createGain();
    sink.gain.value = 0;
    sink.connect(ac.destination);
    stemAn.set(t.node, { name: t.name.toLowerCase(), an: analyserOn(ac, t.node, sink), buf: new Float32Array(FFT / 2), prev: new Float32Array(FFT / 2) });
  }
}

/** Updates `features` for this frame. Returns false when nothing plays (the visualizer then rests). */
export function listen(dt: number): boolean {
  const live = !!playing && playing.kind !== 'apple' && build();
  const ease = (k: keyof Features, v: number, up: number, down: number) => { features[k] += (v - features[k]) * Math.min(1, dt * (v > features[k] ? up : down)); };
  const decay = (k: 'kick' | 'snare' | 'hat', rate: number) => { features[k] *= Math.exp(-dt * rate); };
  decay('kick', 5); decay('snare', 6); decay('hat', 10);
  if (!live) {
    for (const k of ['bass', 'body', 'air', 'vocal', 'level', 'rise'] as const) ease(k, 0, 1, 1.5);
    ease('intensity', 0.3, 1, 0.5);
    return false;
  }
  const ac = audioGraph()!.ac;
  if (playing!.id !== lastId) {
    // a new song: forget the last one's range quickly
    lastId = playing!.id;
    for (const n of Object.values(N)) n.reset();
    song = long = short;
  }
  prev.set(mid);
  aMid.getFloatFrequencyData(mid);
  aSide.getFloatFrequencyData(side);
  aMid.getFloatTimeDomainData(wave);
  syncStems(ac);
  let rms = 0;
  for (let i = 0; i < wave.length; i++) rms += wave[i] * wave[i];
  rms = Math.sqrt(rms / wave.length);

  // the drums: from the drums stem when there is one, otherwise from the whole mix
  const drums = stemFor(/drum|perc|beat/);
  const dSrc = drums ? (drums.prev.set(drums.buf), drums.an.getFloatFrequencyData(drums.buf), drums) : { buf: mid, prev };
  if (O.kick.hit(flux(dSrc.buf, dSrc.prev, 35, 130), dt)) features.kick = 1;
  if (O.snare.hit(flux(dSrc.buf, dSrc.prev, 1200, 5000) + flux(dSrc.buf, dSrc.prev, 150, 280) * 0.5, dt)) features.snare = 1;
  if (O.hat.hit(flux(dSrc.buf, dSrc.prev, 7000, 15000), dt)) features.hat = Math.max(features.hat, 0.8);

  const bassStem = stemFor(/bass/);
  if (bassStem) bassStem.an.getFloatFrequencyData(bassStem.buf);
  ease('bass', N.bass.get(band(bassStem ? bassStem.buf : mid, 40, 220), dt), 12, 3);
  ease('body', N.body.get(band(mid, 250, 2500) + band(side, 250, 2500), dt), 8, 2.5);
  ease('air', N.air.get(band(mid, 5000, 14000) + band(side, 5000, 14000), dt), 12, 4);

  // the voice: tonal, centered, in the voice range (or straight from the vocals stem)
  const vox = stemFor(/vocal|voice|vox|sing/);
  let v: number, pitchSrc: Float32Array;
  if (vox) {
    vox.an.getFloatFrequencyData(vox.buf);
    v = band(vox.buf, 150, 4000);
    pitchSrc = vox.buf;
  } else {
    const m = band(mid, 250, 3500), sd = band(side, 250, 3500);
    const centered = Math.max(0, (m - sd) / (m + sd + 1e-9));
    const tonal = 1 - Math.min(1, flatness(mid, 250, 3500) * 2.2);
    v = m * centered * tonal;
    pitchSrc = mid;
  }
  ease('vocal', N.vocal.get(v, dt), 6, 1.6);
  if (features.vocal > 0.3) ease('pitch', Math.max(0, Math.min(1, peakPitch(pitchSrc, 110, 1100))), 4, 4);

  // the section: loudness over ~3 s against the song so far
  short += (rms - short) * Math.min(1, dt * 8);
  long += (rms - long) * Math.min(1, dt * 0.35);
  song += (rms - song) * Math.min(1, dt * 0.03);
  const db = (x: number) => 20 * Math.log10(x + 1e-6);
  ease('intensity', Math.max(0, Math.min(1, 0.5 + (db(long) - db(song)) / 8)), 1.2, 0.8);
  ease('rise', Math.max(0, Math.min(1, (long - longPrev) / Math.max(1e-6, dt) / (song + 1e-6) * 2)), 2, 0.6);
  longPrev = long;
  ease('level', N.level.get(short, dt), 14, 4);
  return true;
}
