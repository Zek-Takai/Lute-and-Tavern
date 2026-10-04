export const $ = <T extends HTMLElement = HTMLElement>(sel: string) => document.querySelector(sel) as T;
export const esc = (s: unknown) =>
  String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);

/** A CSS url(...) for an image address. Quotes, brackets, backslashes and spaces are percent-encoded so an
 *  address from outside (a radio station's icon) can't break out of the style. Still esc() it inside HTML. */
export const cssUrl = (u: string) => `url("${u.replace(/["'()\\\s]/g, (c) => '%' + c.charCodeAt(0).toString(16).padStart(2, '0').toUpperCase())}")`;

export const PLAY = '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M7 4v16l13-8z"/></svg>';
export const PAUSE = '<svg viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="5" width="4" height="14" rx="1"/><rect x="14" y="5" width="4" height="14" rx="1"/></svg>';
export const STOP = '<svg viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="1.5"/></svg>';

let toastTimer = 0;
export function toast(msg: string) {
  const t = $('#toast');
  t.textContent = msg;
  t.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => t.classList.remove('show'), 3200);
}

export function hueOf(str: string) {
  let h = 0;
  for (const c of str) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return h % 360;
}

export const date = (secs: number) => new Date(secs * 1000).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });

export async function copyText(text: string, label: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast(`${label} copied`);
  } catch {
    toast('Copy did not work here. Select the text and copy it by hand.');
  }
}

/** A small right-click menu at the pointer. Closes on the next click, scroll or Escape. */
export function contextMenu(e: MouseEvent, items: [string, () => void][]) {
  e.preventDefault();
  document.querySelector('.ctx')?.remove();
  const m = document.createElement('div');
  m.className = 'ctx';
  m.setAttribute('role', 'menu');
  items.forEach(([label, fn]) => {
    const b = document.createElement('button');
    b.setAttribute('role', 'menuitem');
    b.textContent = label;
    b.onclick = () => { close(); fn(); };
    m.append(b);
  });
  document.body.append(m);
  const x = Math.min(e.clientX, innerWidth - m.offsetWidth - 8), y = Math.min(e.clientY, innerHeight - m.offsetHeight - 8);
  m.style.left = `${Math.max(8, x)}px`;
  m.style.top = `${Math.max(8, y)}px`;
  m.querySelector('button')?.focus();
  const away = (ev: Event) => { if (!(ev.target instanceof Node && m.contains(ev.target))) close(); };
  const key = (ev: KeyboardEvent) => { if (ev.key === 'Escape') close(); };
  function close() {
    m.remove();
    document.removeEventListener('mousedown', away, true);
    document.removeEventListener('keydown', key, true);
    document.removeEventListener('scroll', close, true);
  }
  document.addEventListener('mousedown', away, true);
  document.addEventListener('keydown', key, true);
  document.addEventListener('scroll', close, true);
}
