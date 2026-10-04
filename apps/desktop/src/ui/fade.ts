// Visualizer view: while music plays and nothing moves for 10 seconds, everything but the player bar fades
// away so the visualizer fills the window; any mouse move, touch or key brings it straight back. A button on
// the player bar cycles: fade when idle (default), always show, or hide now (stays hidden until the button
// or Escape brings it back).
import { paused, playing } from '../player';
import { saveSetting, state } from '../state';
import { $, toast } from '../util';

type Mode = 'auto' | 'show' | 'hide';
const IDLE_MS = 10_000;
const LABEL: Record<Mode, string> = {
  auto: 'Fade away while music plays (after 10 seconds without moving the mouse)',
  show: 'Always show the library while music plays',
  hide: 'Visualizer only: the library is hidden until you press this again or Escape',
};
const SHORT: Record<Mode, string> = { auto: 'Fades away when idle', show: 'Always shown', hide: 'Visualizer only' };
const ICON: Record<Mode, string> = {
  // a fading eye, an open eye, a closed eye
  auto: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z" stroke-dasharray="3 3"/><circle cx="12" cy="12" r="3"/></svg>',
  show: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/></svg>',
  hide: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 3l18 18"/><path d="M10.6 5.1A10 10 0 0 1 12 5c6.4 0 10 7 10 7a17 17 0 0 1-3.2 3.9M6.6 6.6C3.9 8.4 2 12 2 12s3.6 7 10 7a9.7 9.7 0 0 0 5.4-1.6"/></svg>',
};

let mode: Mode = 'auto';
let last = Date.now();

const live = () => !!playing && !paused;
/** Something the person is in the middle of: a dialog, the assistant, typing. */
function busy() {
  if (document.querySelector('dialog[open]') || document.querySelector('.ctx')) return true;
  const a = document.getElementById('assist');
  if (a && !a.hidden) return true;
  const f = document.activeElement;
  return !!f && (f.matches('input, textarea, select, [contenteditable="true"]') && !f.closest('.player'));
}

function apply() {
  const hide = mode === 'hide' || (mode === 'auto' && live() && !busy() && Date.now() - last >= IDLE_MS);
  document.body.classList.toggle('faded', hide);
}

function wake() {
  last = Date.now();
  if (mode !== 'hide' && document.body.classList.contains('faded')) document.body.classList.remove('faded');
}

function sync() {
  const b = $('#vizMode');
  b.innerHTML = ICON[mode];
  b.setAttribute('aria-pressed', String(mode === 'hide'));
  b.setAttribute('aria-label', LABEL[mode]);
  b.title = LABEL[mode];
}

/** Settings › Visualizer: fade when idle, or always show (the same as the eye button's first two states). */
export function setFadeMode(m: 'auto' | 'show') {
  mode = m;
  void saveSetting('foregroundFade', m);
  last = Date.now();
  sync();
  apply();
}

export function initFade() {
  const saved = state.settings.foregroundFade;
  mode = saved === 'show' ? 'show' : 'auto';
  sync();
  $('#vizMode').onclick = () => {
    mode = mode === 'auto' ? 'hide' : mode === 'hide' ? 'show' : 'auto';
    if (mode !== 'hide') void saveSetting('foregroundFade', mode);
    last = Date.now();
    sync();
    apply();
    toast(SHORT[mode]);
  };
  for (const ev of ['mousemove', 'mousedown', 'wheel', 'touchstart', 'keydown'] as const) {
    window.addEventListener(ev, wake, { passive: true, capture: true });
  }
  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && mode === 'hide') {
      mode = (state.settings.foregroundFade as Mode) === 'show' ? 'show' : 'auto';
      last = Date.now();
      sync();
      apply();
    }
  });
  setInterval(apply, 500);
}
