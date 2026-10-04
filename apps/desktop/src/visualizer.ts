// The background visualizer: listens to the music once (audiofeatures.ts) and runs the chosen scene
// (src/viz). Settings › Visualizer picks one, a different one for every song, or none.
import * as THREE from 'three';
import { features, listen } from './audiofeatures';
import { playing } from './player';
import { state } from './state';
import { makeAurora } from './viz/aurora';
import { makeHorizon } from './viz/horizon';
import { makeMandala } from './viz/mandala';
import { makeSilk } from './viz/silk';
import type { MakeScene, Scene } from './viz/types';
import { makeVoyage } from './viz/voyage';

/** The scenes, in the order Settings shows them. */
export const VISUALIZERS: { id: string; name: string; about: string; make: MakeScene }[] = [
  { id: 'silk', name: 'Silk and water', about: 'Glowing silk riding a sheet of water; every hit drops in and ripples out.', make: makeSilk },
  { id: 'aurora', name: 'Aurora', about: 'Northern lights over a starry night; the voice raises its own curtain.', make: makeAurora },
  { id: 'voyage', name: 'Voyage', about: 'Flying through stars past a nebula; kicks and snares send out shells of light.', make: makeVoyage },
  { id: 'mandala', name: 'Mandala', about: 'A slowly turning kaleidoscope; hits send rings outward, the voice makes the petals breathe.', make: makeMandala },
  { id: 'horizon', name: 'Horizon', about: 'A retro grid running toward a setting sun, mountains drawn from the music.', make: makeHorizon },
];

let current: { id: string; scene: Scene } | null = null;
let renderer: THREE.WebGLRenderer | null = null;
let canvasEl: HTMLCanvasElement | null = null;
let lastSong = '';

/** What Settings chose: a scene id, 'random' (a different one each song) or 'off'. */
const choice = () => (state.settings.visualizer as string) || 'silk';

function pick(): string {
  const c = choice();
  if (c !== 'random') return c;
  const others = VISUALIZERS.filter((v) => v.id !== current?.id);
  return others[Math.floor(Math.random() * others.length)].id;
}

function show(id: string) {
  if (!renderer || !canvasEl || current?.id === id) return;
  const v = VISUALIZERS.find((x) => x.id === id) ?? VISUALIZERS[0];
  // a gentle fade between scenes, never a cut
  const c = canvasEl;
  c.style.opacity = '0';
  setTimeout(() => {
    current?.scene.dispose();
    current = { id: v.id, scene: v.make(renderer!) };
    c.style.opacity = '1';
  }, current ? 600 : 0);
}

/** Settings changed: show the newly chosen scene (or stop). */
export function applyVisualizer() {
  if (!canvasEl) return;
  const off = choice() === 'off';
  canvasEl.hidden = off;
  if (!off) show(pick());
}

export function startVisualizer(canvas: HTMLCanvasElement) {
  try {
    renderer = new THREE.WebGLRenderer({ canvas, antialias: false, powerPreference: 'high-performance' });
  } catch {
    canvas.hidden = true;
    return;
  }
  canvasEl = canvas;
  canvas.style.transition = 'opacity .6s ease';
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.5));
  renderer.setClearColor(0x0b0d24, 1);
  const calm = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const size = () => renderer!.setSize(innerWidth, innerHeight, false);
  addEventListener('resize', size);
  size();
  applyVisualizer();

  const prevHit = { kick: 0, snare: 0, hat: 0 };
  let last = performance.now(), time = 0, listenFailed = false;
  const frame = (now: number) => {
    requestAnimationFrame(frame);
    if (document.hidden || canvas.hidden) {
      last = now;
      return;
    }
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    time += dt;
    // listening must never stop the picture: on any error it carries on without the music
    try {
      listen(dt);
    } catch (e) {
      if (!listenFailed) console.error('visualizer listening failed', e);
      listenFailed = true;
    }
    // a different scene for each song, when chosen
    const song = playing?.id ?? '';
    if (song && song !== lastSong) {
      if (lastSong && choice() === 'random') show(pick());
      lastSong = song;
    }
    const f = features;
    const rose = (k: 'kick' | 'snare' | 'hat') => {
      const r = f[k] > prevHit[k] + 0.4;
      prevHit[k] = f[k];
      return r;
    };
    const hit = { kick: rose('kick'), snare: rose('snare'), hat: rose('hat') };
    current?.scene.frame({ dt, time, f, hit, calm, width: innerWidth, height: innerHeight });
  };
  requestAnimationFrame(frame);
}
