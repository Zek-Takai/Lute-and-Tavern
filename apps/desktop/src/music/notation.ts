// Staff drawing and music helpers. Step 0 is the bottom staff line (E4); 8 is the top line (F5).
import type { Note } from '../api';
import { esc } from '../util';

export const NAMES = ['C', 'C#', 'D', 'Eb', 'E', 'F', 'F#', 'G', 'Ab', 'A', 'Bb', 'B'];
export const IDX: Record<string, number> = { C: 0, 'C#': 1, Db: 1, D: 2, 'D#': 3, Eb: 3, E: 4, F: 5, 'F#': 6, Gb: 6, G: 7, 'G#': 8, Ab: 8, A: 9, 'A#': 10, Bb: 10, B: 11 };
export const BEATS = { w: 4, h: 2, q: 1, e: 0.5 } as const;
export const DUR_NAME = { w: 'Whole', h: 'Half', q: 'Quarter', e: 'Eighth' } as const;
export const SCALE = [60, 62, 64, 65, 67, 69, 71, 72, 74, 76, 77, 79, 81, 83, 84];

export function tChord(c: string, n: number) {
  const m = String(c).match(/^([A-G][#b]?)(.*)$/);
  if (!m || IDX[m[1]] === undefined) return c;
  return NAMES[(IDX[m[1]] + n + 120) % 12] + m[2];
}
export function keyName(k: string, n = 0) {
  if (!k) return '';
  const t = tChord(k, n);
  return t.endsWith('m') ? t.slice(0, -1) + ' minor' : t + ' major';
}

export type Placed = Note & { n: number };

/** Splits notes into 4/4 measures; a note that would cross the bar line starts the next measure. */
export function toMeasures(notes: Note[]): Placed[][] {
  const ms: Placed[][] = [[]];
  let beat = 0;
  notes.forEach((nt, n) => {
    const b = BEATS[nt.d] ?? 1;
    if (beat + b > 4 + 1e-6) {
      ms.push([]);
      beat = 0;
    }
    ms[ms.length - 1].push({ ...nt, n });
    beat += b;
  });
  return ms;
}

export interface StaffOpts {
  g?: number;
  W?: number;
  chords?: string[] | null;
  chordAt?: number;
  lyrics?: boolean;
  timesig?: boolean;
  semis?: number;
  edit?: boolean;
  sel?: number;
  title?: string;
}

export function staffSVG(ms: Placed[][], o: StaffOpts = {}) {
  const { g = 9, W = 760, chords = null, chordAt = 0, lyrics = true, timesig = true, semis = 0, edit = false, sel = -1, title = '' } = o;
  const shift = Math.round((semis * 7) / 12);
  const top = (chords && chords.length ? 3.4 : 2.2) * g;
  const y = (s: number) => top + 4 * g - (s * g) / 2;
  const H = top + 4 * g + (lyrics ? 4.6 : 3) * g;
  const clefW = 3.4 * g, tsW = timesig ? 2.4 * g : 0, x0 = clefW + tsW + 6, mw = (W - x0 - 4) / Math.max(1, ms.length);
  const rx = g * 0.64, ry = g * 0.46;
  let out = `<svg viewBox="0 0 ${W} ${H}" class="${edit ? 'edit' : ''}" data-top="${top}" data-g="${g}" data-shift="${shift}" role="img" aria-label="${esc(title)} melody">`;
  out += `<rect x="0" y="0" width="${W}" height="${H}" fill="transparent"/>`;
  for (let i = 0; i < 5; i++) out += `<line x1="2" x2="${W - 2}" y1="${top + i * g}" y2="${top + i * g}" stroke="currentColor" stroke-opacity=".55" stroke-width="${g / 9}"/>`;
  out += `<text x="4" y="${top + 3.1 * g}" font-family="Noto Music, serif" font-size="${g * 4.6}" fill="currentColor">𝄞</text>`;
  if (timesig)
    out += `<text x="${clefW + g * 0.8}" y="${top + 1.9 * g}" font-family="Bricolage Grotesque, sans-serif" font-weight="700" font-size="${g * 2.1}" fill="currentColor" text-anchor="middle">4</text><text x="${clefW + g * 0.8}" y="${top + 3.9 * g}" font-family="Bricolage Grotesque, sans-serif" font-weight="700" font-size="${g * 2.1}" fill="currentColor" text-anchor="middle">4</text>`;
  ms.forEach((m, mi) => {
    const mx = x0 + mi * mw;
    let beat = 0;
    const ch = chords && chords.length ? chords[(chordAt + mi) % chords.length] : null;
    if (ch) out += `<text x="${mx + mw * 0.08}" y="${top - 1.3 * g}" font-family="Instrument Sans, sans-serif" font-weight="600" font-size="${g * 1.45}" style="fill:var(--accent)">${esc(tChord(ch, semis))}</text>`;
    for (const nt of m) {
      const s = nt.s + shift, x = mx + mw * 0.14 + (beat / 4) * mw * 0.8, yy = y(s), hollow = nt.d === 'h' || nt.d === 'w';
      const cls = edit && nt.n === sel ? 'nh sel' : 'nh';
      for (let l = -2; l >= s; l -= 2) out += `<line x1="${x - rx * 1.6}" x2="${x + rx * 1.6}" y1="${y(l)}" y2="${y(l)}" stroke="currentColor" stroke-width="${g / 8}"/>`;
      for (let l = 10; l <= s; l += 2) out += `<line x1="${x - rx * 1.6}" x2="${x + rx * 1.6}" y1="${y(l)}" y2="${y(l)}" stroke="currentColor" stroke-width="${g / 8}"/>`;
      const k = edit ? 1.25 : 1;
      out += `<ellipse class="${cls}" data-n="${nt.n}" cx="${x}" cy="${yy}" rx="${rx * k}" ry="${ry * k}" transform="rotate(-20 ${x} ${yy})" style="fill:${hollow ? 'var(--paper)' : 'currentColor'}" stroke="currentColor" stroke-width="${hollow ? g / 6 : 0}"/>`;
      if (nt.d !== 'w') {
        const up = s < 4, sx = up ? x + rx * 0.9 : x - rx * 0.9, ey = up ? yy - 3.4 * g : yy + 3.4 * g;
        out += `<line x1="${sx}" x2="${sx}" y1="${yy}" y2="${ey}" stroke="currentColor" stroke-width="${g / 7}"/>`;
        if (nt.d === 'e')
          out += up
            ? `<path d="M${sx} ${ey} q${g * 0.9} ${g * 1.1} ${g * 0.8} ${g * 2.2}" fill="none" stroke="currentColor" stroke-width="${g / 6}"/>`
            : `<path d="M${sx} ${ey} q${g * 0.9} ${-g * 1.1} ${g * 0.8} ${-g * 2.2}" fill="none" stroke="currentColor" stroke-width="${g / 6}"/>`;
      }
      if (lyrics && nt.w) out += `<text x="${x}" y="${top + 4 * g + 3.8 * g}" font-family="Instrument Sans, sans-serif" font-size="${g * 1.35}" fill="currentColor" text-anchor="middle">${esc(nt.w)}</text>`;
      beat += BEATS[nt.d] ?? 1;
    }
    out += `<line x1="${mx + mw}" x2="${mx + mw}" y1="${top}" y2="${top + 4 * g}" stroke="currentColor" stroke-width="${g / 8}"/>`;
  });
  return out + '</svg>';
}
