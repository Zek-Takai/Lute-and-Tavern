// The assistant overlay: a chat with the person's own AI that can run the app (search, playlists, playback,
// songwriting). Opens with a shortcut the person picks; a second shortcut talks instead of typing, through
// any Whisper server set in Settings. luteandtavern://ask?text=... links only fill in the box; the person sends.
import { listen } from '@tauri-apps/api/event';
import { api, errText, type ChatLine } from '../api';
import { state, writerName } from '../state';
import { $, esc, toast } from '../util';

export const DEFAULT_KEYS = { assistantKey: 'Mod+J', voiceKey: 'Mod+Shift+J' };
const isMac = navigator.platform.toLowerCase().includes('mac');

/** "Mod+Shift+J" -> "⌘⇧J" on a Mac, "Ctrl+Shift+J" elsewhere. */
export function keyLabel(combo: string) {
  if (!combo) return 'Not set';
  const parts = combo.split('+');
  if (isMac) return parts.map((p) => ({ Mod: '⌘', Ctrl: '⌃', Alt: '⌥', Shift: '⇧' })[p] ?? p).join('');
  return parts.map((p) => (p === 'Mod' ? 'Ctrl' : p)).join('+');
}

/** The combo a key press makes, or null for a bare modifier. */
export function comboOf(e: KeyboardEvent) {
  if (['Meta', 'Control', 'Shift', 'Alt'].includes(e.key)) return null;
  const mod = isMac ? e.metaKey : e.ctrlKey;
  const key = e.code.startsWith('Key') ? e.code.slice(3) : e.code.startsWith('Digit') ? e.code.slice(5) : e.key.length === 1 ? e.key.toUpperCase() : e.key;
  return [mod && 'Mod', isMac && e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', key].filter(Boolean).join('+');
}

const keyFor = (name: keyof typeof DEFAULT_KEYS) => (state.settings[name] as string | undefined) ?? DEFAULT_KEYS[name];

let busy = false;
let lines: ChatLine[] = [];
let recorder: MediaRecorder | null = null;
/** True while asking for the mic, before `recorder` exists; `cancelStart` asks that start to give up. */
let starting = false;
let cancelStart = false;

function render(extra = '') {
  $('#aName').textContent = writerName();
  const open = keyLabel(keyFor('assistantKey')), talk = keyLabel(keyFor('voiceKey'));
  $('#aKeys').textContent = `${open} to open, ${talk} to talk`;
  const log = $('#aLog');
  log.innerHTML =
    (lines.length ? '' : `<p class="muted small">Ask ${esc(writerName())} to play something, make a playlist ("an hour of calm songs for studying"), tidy genres, or write a song.</p>`) +
    lines.map((l) => `<div class="m ${l.role}">${esc(l.text)}</div>`).join('') +
    (busy ? `<div class="m action">${esc(writerName())} is working…</div>` : '') +
    extra;
  log.scrollTop = log.scrollHeight;
}

async function refresh(extra = '') {
  lines = await api.assistantHistory().catch(() => lines);
  render(extra);
}

export async function openAssistant() {
  const a = $('#assist');
  if (a.hidden) {
    a.hidden = false;
    await refresh();
  }
  $<HTMLTextAreaElement>('#aText').focus();
}

function closeAssistant() {
  stopRecording(false);
  $('#assist').hidden = true;
}

export async function ask(text: string) {
  text = text.trim();
  if (!text || busy) return;
  await openAssistant();
  if (!state.writer.baseUrl || !state.writer.model) {
    render(`<div class="m assistant err">Set up your AI first in Settings › Your AI (an address and a model).</div>`);
    return;
  }
  busy = true;
  lines.push({ id: 0, role: 'user', text, createdAt: 0 });
  render();
  try {
    await api.assistantSend(text);
  } catch (e) {
    busy = false;
    await refresh(`<div class="m assistant err">${esc(errText(e))}</div>`);
  }
}

// ---------- voice ----------

async function startRecording() {
  if (recorder || starting) return;
  starting = true;
  cancelStart = false;
  const sp = await api.speechSettings().catch(() => null);
  if (cancelStart) return void (starting = false);
  if (!sp?.baseUrl) {
    starting = false;
    await openAssistant();
    render(`<div class="m assistant err">To talk instead of type, add a Whisper server in Settings › Voice.</div>`);
    return;
  }
  let stream: MediaStream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  } catch (e) {
    starting = false;
    if (!cancelStart) toast(`The microphone is not available: ${errText(e)}`);
    return;
  }
  starting = false;
  // Stopped or closed while the mic permission was being asked: let the mic go straight away.
  if (cancelStart) return stream.getTracks().forEach((t) => t.stop());
  const chunks: Blob[] = [];
  const rec = new MediaRecorder(stream);
  recorder = rec;
  rec.ondataavailable = (e) => e.data.size && chunks.push(e.data);
  rec.onstop = async () => {
    stream.getTracks().forEach((t) => t.stop());
    $('#aMic').setAttribute('aria-pressed', 'false');
    if (!(rec as MediaRecorder & { send?: boolean }).send || !chunks.length) return;
    const blob = new Blob(chunks, { type: rec.mimeType || 'audio/mp4' });
    render(`<div class="m action">Listening back…</div>`);
    try {
      const text = await api.transcribe(Array.from(new Uint8Array(await blob.arrayBuffer())), blob.type);
      if (text) await ask(text);
      else render(`<div class="m assistant err">I didn't catch anything. Try again a little closer to the mic.</div>`);
    } catch (e) {
      render(`<div class="m assistant err">${esc(errText(e))}</div>`);
    }
  };
  rec.start();
  $('#aMic').setAttribute('aria-pressed', 'true');
  await openAssistant();
  render(`<div class="m action">Listening… press ${esc(keyLabel(keyFor('voiceKey')))} or the mic again to send.</div>`);
}

function stopRecording(send: boolean) {
  if (starting) cancelStart = true;
  if (!recorder) return;
  (recorder as MediaRecorder & { send?: boolean }).send = send;
  if (recorder.state !== 'inactive') recorder.stop();
  recorder = null;
}

const toggleVoice = () => (recorder || starting ? stopRecording(true) : void startRecording());

// ---------- wiring ----------

export async function initAssistant() {
  $('#assistBtn').onclick = () => ($('#assist').hidden ? void openAssistant() : closeAssistant());
  $('#aClose').onclick = closeAssistant;
  $('#aClear').onclick = async () => {
    await api.assistantClear();
    lines = [];
    render();
  };
  $('#aMic').onclick = toggleVoice;
  $('#aForm').addEventListener('submit', (e) => {
    e.preventDefault();
    const t = $<HTMLTextAreaElement>('#aText');
    const text = t.value;
    t.value = '';
    void ask(text);
  });
  $('#aText').addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      $<HTMLFormElement>('#aForm').requestSubmit();
    }
    if (e.key === 'Escape') closeAssistant();
  });
  // Shortcuts work anywhere in the app, even while typing, since they use a modifier key.
  document.addEventListener('keydown', (e) => {
    if ((e.target as HTMLElement).closest?.('[data-keycap]')) return;
    const c = comboOf(e);
    if (!c) return;
    if (c === keyFor('assistantKey')) {
      e.preventDefault();
      if ($('#assist').hidden) void openAssistant();
      else closeAssistant();
    } else if (c === keyFor('voiceKey')) {
      e.preventDefault();
      toggleVoice();
    }
  });
  await listen<{ ok: boolean; text: string }>('assistant-answer', async (e) => {
    busy = false;
    await refresh(e.payload.ok ? '' : `<div class="m assistant err">${esc(e.payload.text)}</div>`);
    if (!e.payload.ok) toast(e.payload.text);
  });
  // A link can come from any web page or message, so its words are only put in the box. Nothing runs until
  // the person reads them and presses Send.
  const fromLink = async () => {
    const text = (await api.takePendingAsk().catch(() => null))?.trim();
    if (!text) return;
    await openAssistant();
    const box = $<HTMLTextAreaElement>('#aText');
    box.value = text;
    box.focus();
    box.setSelectionRange(text.length, text.length);
    render(`<div class="m action">A link outside the app asked for this. Check it in the box below and press Send if you want ${esc(writerName())} to do it.</div>`);
  };
  await listen('assistant-link', () => void fromLink());
  await fromLink();
}
