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

/** A menu entry: a label and what it does, a label and a sub-menu, or null for a divider. */
export type MenuItem = [string, () => void] | [string, MenuItem[]] | null;

/** A small right-click menu at the pointer. Closes on the next click, scroll or Escape. An entry with a
 *  sub-menu (shown with ›) opens it in place, with a way back. */
export function contextMenu(e: MouseEvent, items: MenuItem[]) {
  e.preventDefault();
  document.querySelector('.ctx')?.remove();
  const m = document.createElement('div');
  m.className = 'ctx';
  m.setAttribute('role', 'menu');
  const show = (list: MenuItem[], back?: MenuItem[]) => {
    m.replaceChildren();
    if (back) list = [['‹ Back', () => show(back)], null, ...list];
    for (const it of list) {
      if (!it) {
        const hr = document.createElement('div');
        hr.className = 'sep';
        hr.setAttribute('role', 'separator');
        m.append(hr);
        continue;
      }
      const [label, act] = it;
      const b = document.createElement('button');
      b.setAttribute('role', 'menuitem');
      b.textContent = Array.isArray(act) ? `${label} ›` : label;
      if (Array.isArray(act)) b.setAttribute('aria-haspopup', 'menu');
      b.onclick = () => {
        if (Array.isArray(act)) return show(act, list === items ? items : undefined);
        if (label === '‹ Back') return act();
        close();
        act();
      };
      m.append(b);
    }
    place();
    m.querySelector('button')?.focus();
  };
  const place = () => {
    const x = Math.min(e.clientX, innerWidth - m.offsetWidth - 8), y = Math.min(e.clientY, innerHeight - m.offsetHeight - 8);
    m.style.left = `${Math.max(8, x)}px`;
    m.style.top = `${Math.max(8, y)}px`;
  };
  document.body.append(m);
  show(items);
  const away = (ev: Event) => { if (!(ev.target instanceof Node && m.contains(ev.target))) close(); };
  const key = (ev: KeyboardEvent) => {
    if (ev.key === 'Escape') close();
    if (ev.key === 'ArrowDown' || ev.key === 'ArrowUp') {
      ev.preventDefault();
      const bs = [...m.querySelectorAll('button')];
      const i = bs.indexOf(document.activeElement as HTMLButtonElement);
      bs[(i + (ev.key === 'ArrowDown' ? 1 : -1) + bs.length) % bs.length]?.focus();
    }
  };
  // the page scrolling closes it; scrolling a long menu itself doesn't
  const scrolled = (ev: Event) => { if (!(ev.target instanceof Node && m.contains(ev.target))) close(); };
  function close() {
    m.remove();
    document.removeEventListener('mousedown', away, true);
    document.removeEventListener('keydown', key, true);
    document.removeEventListener('scroll', scrolled, true);
  }
  document.addEventListener('mousedown', away, true);
  document.addEventListener('keydown', key, true);
  document.addEventListener('scroll', scrolled, true);
}
