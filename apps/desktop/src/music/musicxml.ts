// MusicXML import and export for the melody editor (single melody line, 4/4, chord symbols per measure).
import type { Note } from '../api';
import { toMeasures } from './notation';

const KIND: Record<string, string> = {
  '': 'major', m: 'minor', '7': 'dominant', maj7: 'major-seventh', m7: 'minor-seventh',
  dim: 'diminished', aug: 'augmented', sus4: 'suspended-fourth', sus2: 'suspended-second',
};
const xmlEsc = (s: string) => s.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c]!);

export function toMusicXML(title: string, notes: Note[], chords: string[], bpm: number) {
  const L = 'CDEFGAB', dv = { w: 8, h: 4, q: 2, e: 1 }, ty = { w: 'whole', h: 'half', q: 'quarter', e: 'eighth' };
  let x = `<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE score-partwise PUBLIC "-//Recordare//DTD MusicXML 4.0 Partwise//EN" "http://www.musicxml.org/dtds/partwise.dtd">\n<score-partwise version="4.0">\n<work><work-title>${xmlEsc(title)}</work-title></work>\n<part-list><score-part id="P1"><part-name>Melody</part-name></score-part></part-list>\n<part id="P1">\n`;
  toMeasures(notes).forEach((m, mi) => {
    x += `<measure number="${mi + 1}">`;
    if (mi === 0)
      x += `<attributes><divisions>2</divisions><key><fifths>0</fifths></key><time><beats>4</beats><beat-type>4</beat-type></time><clef><sign>G</sign><line>2</line></clef></attributes>` +
        (bpm ? `<sound tempo="${bpm}"/>` : '');
    const c = chords.length ? chords[mi % chords.length] : null;
    const r = c?.match(/^([A-G])([#b]?)(.*)$/);
    if (r)
      x += `<harmony><root><root-step>${r[1]}</root-step>${r[2] ? `<root-alter>${r[2] === '#' ? 1 : -1}</root-alter>` : ''}</root><kind>${KIND[r[3]] ?? 'major'}</kind></harmony>`;
    for (const nt of m) {
      const i = nt.s + 30;
      x += `<note><pitch><step>${L[((i % 7) + 7) % 7]}</step><octave>${Math.floor(i / 7)}</octave></pitch><duration>${dv[nt.d]}</duration><type>${ty[nt.d]}</type>${nt.w ? `<lyric><text>${xmlEsc(nt.w.replace(/-$/, ''))}</text></lyric>` : ''}</note>`;
    }
    x += '</measure>\n';
  });
  return x + '</part>\n</score-partwise>\n';
}

export function fromMusicXML(text: string) {
  const d = new DOMParser().parseFromString(text, 'application/xml');
  if (d.querySelector('parsererror')) return null;
  const part = d.querySelector('part');
  if (!part) return null;
  const L = 'CDEFGAB';
  const tm: Record<string, Note['d']> = { whole: 'w', half: 'h', quarter: 'q', eighth: 'e', '16th': 'e', '32nd': 'e' };
  const back = Object.fromEntries(Object.entries(KIND).map(([k, v]) => [v, k]));
  const notes: Note[] = [], chords: string[] = [];
  part.querySelectorAll('measure').forEach((m) => {
    const h = m.querySelector('harmony');
    if (h) {
      const st = h.querySelector('root-step')?.textContent?.trim() || 'C';
      const al = Number(h.querySelector('root-alter')?.textContent || 0);
      chords.push(st + (al > 0 ? '#' : al < 0 ? 'b' : '') + (back[h.querySelector('kind')?.textContent?.trim() ?? ''] ?? ''));
    }
    m.querySelectorAll('note').forEach((n) => {
      if (n.querySelector('rest, chord, grace')) return;
      const p = n.querySelector('pitch');
      if (!p) return;
      const i = Number(p.querySelector('octave')?.textContent) * 7 + L.indexOf(p.querySelector('step')?.textContent?.trim() ?? 'C');
      notes.push({
        s: Math.max(-8, Math.min(16, i - 30)),
        d: tm[n.querySelector('type')?.textContent?.trim() ?? ''] ?? 'q',
        w: n.querySelector('lyric text')?.textContent?.trim() ?? '',
      });
    });
  });
  const tempo = d.querySelector('sound[tempo]')?.getAttribute('tempo');
  return { notes, chords, bpm: tempo ? Math.round(Number(tempo)) : 0 };
}
