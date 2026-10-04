// Settings: the songwriter connection, music folders, genre tags and the sidebar's starting tab.
import { audioDir, join } from '@tauri-apps/api/path';
import { message, open as pickFolder } from '@tauri-apps/plugin-dialog';
import { api, errText, type McpInfo, type PlexServer, type SpeechSettings, type UpdateInfo } from '../api';
import { comboOf, DEFAULT_KEYS, keyLabel } from './assistant';
import { connections, loadConnections } from './sources';
import { applyLineColor, applySkin, LINE_COLORS, SKINS } from '../player';
import { appleStatus, connectApple, disconnectApple, forgetAppleKey, isConnected, musicKit, setAppleKey, type AppleStatus } from '../apple';
import { libraryFolder, reload, rerender, saveSetting, state } from '../state';
import { $, copyText, esc, toast } from '../util';

const ORIGIN: Record<string, string> = { default: 'Built in', user: 'Yours', writer: 'Made by the songwriter', file: 'From your music files' };

export async function addMusicFolder() {
  const path = await pickFolder({ directory: true, multiple: false, title: 'Choose a music folder' });
  if (typeof path !== 'string') return false;
  toast('Reading your music…');
  try {
    const r = await api.addFolder(path);
    await reload();
    state.view = { kind: 'library' };
    state.genre = 'all';
    rerender();
    toast(`Added ${r.added} song${r.added === 1 ? '' : 's'}${r.untagged ? `. ${r.untagged} had no genre tag and wait in Unsorted.` : '.'}`);
    return true;
  } catch (e) {
    toast(errText(e));
    return false;
  }
}

let updates: UpdateInfo = { canUpdate: false, currentVersion: '', defaultUrl: '' };
let speech: SpeechSettings = { baseUrl: '', model: '', language: '', hasKey: false };
const KIND_NAME: Record<string, string> = { plex: 'Plex', jellyfin: 'Jellyfin or Emby', subsonic: 'Subsonic server (Navidrome, Airsonic, Gonic, Ampache…)', webdav: 'WebDAV folder (Nextcloud, NAS file sharing…)' };
const KIND_HELP: Record<string, string> = {
  plex: 'Sign in with your Plex account in the browser; the app finds your Plex servers and you pick one. Nothing to install.',
  jellyfin: 'Your Jellyfin or Emby address and account. The app signs in once and keeps only the access token, not your password.',
  subsonic: 'For servers that speak the Subsonic API. The server only ever receives a one-time token, never your password.',
  webdav: 'A music folder shared over WebDAV, played as it is. Folders in the Lute & Tavern layout show their artists and albums.',
};
let addKind = 'plex';
let mcp: McpInfo = { enabled: false, running: false, url: '', error: null };
let apple: AppleStatus = { ready: false, builtIn: false, keyId: '', teamId: '' };

const reportText = (r: { added: number; untagged: number }) =>
  `${r.added} song${r.added === 1 ? '' : 's'} added${r.untagged ? `. ${r.untagged} had no genre tag and wait in Unsorted.` : '.'}`;

/** Makes `path` the library folder; its files get organized into song folders. */
export async function useAsLibrary(path: string) {
  toast('Organizing your library…');
  try {
    const r = await api.setLibraryFolder(path);
    await reload();
    state.view = { kind: 'all' };
    rerender();
    toast(`Library ready. ${reportText(r)}`);
    return true;
  } catch (e) {
    toast(errText(e));
    return false;
  }
}

/** Sets up the suggested library folder (~/Music/Lute & Tavern) or one the person picks. */
export async function chooseLibrary(suggested: boolean) {
  const path = suggested
    ? await join(await audioDir(), 'Lute & Tavern')
    : await pickFolder({ directory: true, multiple: false, title: 'Choose your library folder' });
  if (typeof path !== 'string') return false;
  return useAsLibrary(path);
}

/** Moves songs into the library folder, where they get sorted into song folders. */
export async function addSongsToLibrary(paths: string[]) {
  if (!libraryFolder()) {
    toast('Choose where your library lives first.');
    await openSettings();
    return;
  }
  toast('Adding to your library…');
  try {
    const r = await api.addToLibrary(paths);
    await reload();
    rerender();
    toast(reportText(r));
  } catch (e) {
    toast(errText(e));
  }
}

export async function openSettings(section?: 'writer' | 'servers') {
  state.writer = await api.writerConfig();
  updates = await api.updateInfo();
  mcp = await api.mcpInfo().catch(() => mcp);
  speech = await api.speechSettings().catch(() => speech);
  apple = await appleStatus().catch(() => apple);
  if (apple.ready) await musicKit().catch(() => {});
  render();
  const d = $<HTMLDialogElement>('#set');
  if (!d.open) d.showModal();
  if (section === 'writer') $('#wUrl')?.focus();
  if (section === 'servers') {
    $('#setServers')?.scrollIntoView({ block: 'start' });
    $('#cName')?.focus();
  }
}

function render() {
  const w = state.writer;
  const side = (state.settings.sidebarDefault as string) || 'library';
  $('#setBody').innerHTML = `
    <div class="row" style="justify-content:space-between"><h2 id="setTitle">Settings</h2><button class="btn" id="setClose">Done</button></div>

    <section class="set-sec">
      <h3>Your AI</h3>
      <p class="muted small">Writes your songs and runs the assistant. Any AI that speaks the OpenAI chat format works: a model on your own computer or network (LM Studio, Ollama, vLLM) or a hosted service. For the assistant, pick a model that supports tool calling. Your key is kept in the system keychain.</p>
      <div class="form">
        <label for="wName">Name</label><input id="wName" value="${esc(w.name)}" placeholder="Bard">
        <label for="wUrl">Address</label><input id="wUrl" value="${esc(w.baseUrl)}" placeholder="http://localhost:1234/v1">
        <label for="wModel">Model</label><input id="wModel" value="${esc(w.model)}" list="wModels" placeholder="Check the connection to list models">
        <label for="wKey">API key</label><input id="wKey" type="password" autocomplete="off" placeholder="${w.hasKey ? 'Saved in your keychain. Type to replace it.' : 'Only needed for hosted services'}">
      </div>
      <label class="row small"><input type="checkbox" id="wExclude" ${state.settings.writerExclude ? 'checked' : ''}> Also write Suno's Exclude styles (under More Options in Suno)</label>
      <datalist id="wModels"></datalist>
      <div class="row"><button class="btn" id="wCheck">Check connection</button><button class="btn primary" id="wSave">Save songwriter</button>${w.hasKey ? '<button class="btn small" id="wForget">Remove saved key</button>' : ''}<span class="muted small" id="wStatus"></span></div>
    </section>

    <section class="set-sec">
      <h3>Assistant</h3>
      <p class="muted small">A chat with your AI that can run the app for you: find and play music, build playlists to your taste, tidy genres and song details, write songs. Open it with the Assistant button or your shortcut. Click a shortcut, then press the keys you want.</p>
      <div class="form">
        <label>Open the assistant</label><button class="btn keycap" data-keycap="assistantKey">${esc(keyLabel((state.settings.assistantKey as string) ?? DEFAULT_KEYS.assistantKey))}</button>
        <label>Talk to it</label><button class="btn keycap" data-keycap="voiceKey">${esc(keyLabel((state.settings.voiceKey as string) ?? DEFAULT_KEYS.voiceKey))}</button>
      </div>
      <p class="muted small">Other apps and voice assistants can ask it too, and open Lute &amp; Tavern if it is closed, with a link such as <code>luteandtavern://ask?text=play my road trip playlist</code> (for example from Apple Shortcuts or Siri).</p>
    </section>

    <section class="set-sec">
      <h3>Voice</h3>
      <p class="muted small">To talk instead of type, point this at a Whisper speech-to-text server: one on your own computer (faster-whisper-server, Speaches, whisper.cpp's server and others) or a hosted one (OpenAI, Groq). Anything that answers the Whisper transcription API works. The key is kept in the system keychain.</p>
      <div class="form">
        <label for="vUrl">Address</label><input id="vUrl" value="${esc(speech.baseUrl)}" placeholder="http://localhost:8000/v1">
        <label for="vModel">Model</label><input id="vModel" value="${esc(speech.model)}" placeholder="whisper-1">
        <label for="vLang">Language</label><input id="vLang" value="${esc(speech.language)}" placeholder="Leave empty to detect it (or en, es, de…)">
        <label for="vKey">API key</label><input id="vKey" type="password" autocomplete="off" placeholder="${speech.hasKey ? 'Saved in your keychain. Type to replace it.' : 'Only needed for hosted services'}">
      </div>
      <div class="row"><button class="btn primary" id="vSave">Save voice</button>${speech.hasKey ? '<button class="btn small" id="vForget">Remove saved key</button>' : ''}<span class="muted small" id="vStatus"></span></div>
    </section>

    <section class="set-sec">
      <h3>Library folder</h3>
      <p class="muted small">Songs you add go here and get sorted: each song gets its own folder with its stems, takes, sheet music, cover and a song.json of everything the app knows. Other artists' albums are kept together as Albums/Artist/Album.</p>
      ${libraryFolder()
        ? `<ul class="set-list"><li><span class="path">${esc(libraryFolder()!.path)}</span><button class="btn small" id="libChange">Change…</button></li></ul>`
        : `<div class="row"><button class="btn primary" id="libSuggested">Use Music › Lute &amp; Tavern</button><button class="btn" id="libChoose">Choose a folder…</button></div>`}
    </section>

    <section class="set-sec">
      <h3>Other music folders</h3>
      <p class="muted small">Watched and played, never moved or renamed. Songs you add, change or delete in them update here on their own.</p>
      <ul class="set-list">${state.folders.filter((f) => !f.managed).map((f) => `<li><span class="path">${esc(f.path)}</span><button class="btn small" data-make-lib="${esc(f.path)}">Make this my library</button><button class="btn small" data-rm-folder="${f.id}">Stop watching</button></li>`).join('') || '<li class="muted">None.</li>'}</ul>
      <div class="row"><button class="btn" id="fAdd">Watch a music folder</button>${state.folders.length ? '<button class="btn" id="fScan">Check folders now</button>' : ''}</div>
    </section>

    <section class="set-sec">
      <h3>Genre tags</h3>
      <p class="muted small">Every song carries one tag. Rename a tag and all its songs follow; delete one and its songs go back to Unsorted.</p>
      <ul class="set-list tags-list">${state.genres
        .map((g) => `<li><input class="inline-input" data-rename="${g.id}" value="${esc(g.name)}" aria-label="Tag name"><span class="muted small">${g.songs} song${g.songs === 1 ? '' : 's'}, ${ORIGIN[g.origin] ?? g.origin}</span><button class="btn small" data-del-tag="${g.id}">Delete</button></li>`)
        .join('')}</ul>
      <form class="row" id="tagForm"><input class="inline-input" id="tagNew" placeholder="New tag, e.g. Sea Shanty" style="flex:1"><button class="btn">Add tag</button></form>
    </section>

    <section class="set-sec">
      <h3>Apple Music</h3>
      <p class="muted small">Play Apple Music inside Lute &amp; Tavern with your Apple Music subscription. Your Apple ID sign-in and every key stay in your Mac's Keychain, never in the app's files.</p>
      <ol class="steps small">
        ${apple.builtIn ? '' : `<li><b>MusicKit key.</b> ${apple.ready ? `Saved in your Keychain (Key ID ${esc(apple.keyId)}). <button class="btn small" id="amForget">Remove key</button>` : `Choose the AuthKey_….p8 file from developer.apple.com, check the Team ID, then save.
          <div class="row" style="margin-top:6px"><button class="btn small" id="amKey">Choose the .p8 file…</button><span class="muted" id="amFile">No file chosen</span></div>
          <div class="form" style="margin-top:6px"><label for="amKeyId">Key ID</label><input id="amKeyId" value="${esc(apple.keyId)}" placeholder="Filled in from the file name">
            <label for="amTeam">Team ID</label><input id="amTeam" value="${esc(apple.teamId)}" placeholder="10 characters, under Membership"></div>
          <div class="row" style="margin-top:6px"><button class="btn primary small" id="amSave" disabled>Save key</button><span class="muted" id="amStatus"></span></div>`}</li>`}
        <li><b>Sign in.</b> <button class="btn ${isConnected() ? '' : 'primary'} small" id="amOn" ${apple.ready && !isConnected() ? '' : 'disabled'}>${isConnected() ? 'Connected' : 'Connect Apple Music'}</button>
          ${isConnected() ? '<button class="btn small" id="amOff">Disconnect</button>' : apple.ready ? '' : '<span class="muted">Available once the key is saved.</span>'}</li>
      </ol>
      <label class="row small"><input type="checkbox" id="amShow" ${state.settings.showAppleInLibrary === false ? '' : 'checked'}> Show saved Apple Music songs in my library (they always show in playlists and the Apple Music tab)</label>
    </section>

    <section class="set-sec">
      <h3>Stems</h3>
      <p class="muted small">When you attach WAV stems, they can be stored as FLAC instead: exactly the same sound in about half the space. Keep WAV if you open stems in an app that can't read FLAC, such as Logic Pro or GarageBand.</p>
      <fieldset class="choice">
        <legend>WAV stems</legend>
        ${[['ask', 'Ask each time'], ['always', 'Always store WAV stems as FLAC'], ['never', 'Always keep WAV']].map(([v, l]) => `<label><input type="radio" name="stemsFlac" value="${v}" ${((state.settings.stemsFlac as string) || 'ask') === v ? 'checked' : ''}> ${l}</label>`).join('')}
      </fieldset>
      <div class="row"><button class="btn" id="flacAll">Convert WAV stems already in my library</button><span class="muted small" id="flacStatus"></span></div>
    </section>

    <section class="set-sec">
      <h3>AI apps (MCP)</h3>
      <p class="muted small">Lets AI apps on this computer that support MCP (Claude, Hermes and others) write Suno songs straight into your library. They get a Suno songwriting guide and tools to list, create and revise written songs; turn on Full control to let them run the rest of the app too. Only works while Lute &amp; Tavern is open.</p>
      <label class="row"><input type="checkbox" id="mcpOn" ${mcp.enabled ? 'checked' : ''}> Let AI apps write songs here</label>
      ${mcp.error ? `<p class="small" style="color:var(--danger, #c44)">${esc(mcp.error)}</p>` : ''}
      ${mcp.enabled ? `<label class="row small"><input type="checkbox" id="mcpFull" ${state.settings.mcpFullControl ? 'checked' : ''}> Full control: they can also search your library, play music, make and change playlists, edit song details and genre tags</label>
      <div class="form"><label>Address</label><input readonly value="${esc(mcp.url)}"></div>
      <div class="row"><button class="btn small" id="mcpCopyUrl">Copy address</button><button class="btn small" id="mcpCopyToken">Copy token</button><button class="btn small" id="mcpCopyCfg">Copy setup for your AI app</button><button class="btn small" id="mcpNew">New token</button></div>
      <p class="muted small">The token is kept in your system keychain. Give it only to apps you trust; New token cuts off every app using the old one.</p>` : ''}
    </section>

    <section class="set-sec" id="setServers">
      <h3>Your music servers</h3>
      <p class="muted small">Play music you keep elsewhere: a server on a device you own (a NAS or home computer) or a cloud server you run. It's just an address, so it works at home, over the internet or through a VPN. Sign-ins are kept in the system keychain.</p>
      <ul class="set-list">${connections.map((c) => `<li><span><b>${esc(c.name)}</b> <span class="muted small">${esc(KIND_NAME[c.kind] ?? c.kind)}, ${esc(c.baseUrl)}</span></span><button class="btn small" data-rm-conn="${c.id}">Remove</button></li>`).join('') || '<li class="muted">None yet.</li>'}</ul>
      <div class="form">
        <label for="cKind">Kind</label><select id="cKind">${Object.entries(KIND_NAME).map(([k, l]) => `<option value="${k}" ${k === addKind ? 'selected' : ''}>${l}</option>`).join('')}</select>
      </div>
      <p class="muted small">${KIND_HELP[addKind]}</p>
      ${addKind === 'plex'
        ? `<div class="row"><button class="btn primary" id="cPlex">Sign in with Plex</button><span class="muted small" id="cStatus"></span></div>
           <div id="cPlexServers"></div>`
        : `<div class="form">
        <label for="cName">Name</label><input id="cName" placeholder="Home NAS">
        <label for="cUrl">Address</label><input id="cUrl" placeholder="${addKind === 'webdav' ? 'https://nas.local:5006/music' : addKind === 'jellyfin' ? 'http://192.168.1.20:8096' : 'http://192.168.1.20:4533'}">
        <label for="cUser">Username</label><input id="cUser" autocomplete="off">
        <label for="cPass">Password</label><input id="cPass" type="password" autocomplete="off">
      </div>
      <div class="row"><button class="btn primary" id="cAdd">Connect</button><span class="muted small" id="cStatus"></span></div>`}
    </section>

    <section class="set-sec">
      <h3>Updates</h3>
      <p class="muted small">Version ${esc(updates.currentVersion)}. ${updates.canUpdate
        ? 'When this is on, Lute &amp; Tavern checks for a new release each time it starts and asks before installing.'
        : 'This copy was built on this Mac for everyday use and cannot update itself. Official releases can.'}</p>
      <label class="row"><input type="checkbox" id="uAuto" ${state.settings.autoUpdate ? 'checked' : ''} ${updates.canUpdate ? '' : 'disabled'}> Check for updates when the app starts</label>
      <div class="form"><label for="uUrl">Address</label><input id="uUrl" value="${esc((state.settings.updateUrl as string) || '')}" placeholder="${esc(updates.defaultUrl || 'https://example.com/lute-and-tavern/latest.json')}" ${updates.canUpdate ? '' : 'disabled'}></div>
      <div class="row"><button class="btn" id="uCheck" ${updates.canUpdate ? '' : 'disabled'}>Check now</button><span class="muted small" id="uStatus"></span></div>
    </section>

    <section class="set-sec">
      <fieldset class="choice" id="setSkin">
        <legend>Player bar</legend>
        ${SKINS.map(([k, l, d]) => `<label><input type="radio" name="playerSkin" value="${k}" ${((state.settings.playerSkin as string) || 'classic') === k ? 'checked' : ''}> <span><b>${l}</b><br><span class="muted small">${d}</span></span></label>`).join('')}
      </fieldset>
      <label class="row small" for="lineColor">Progress line color
        <select id="lineColor" class="btn small">${LINE_COLORS.map(([k, l]) => `<option value="${k}" ${((state.settings.lineColor as string) || 'theme') === k ? 'selected' : ''}>${l}</option>`).join('')}</select></label>
    </section>

    <section class="set-sec">
      <fieldset class="choice" id="setSide">
        <legend>Sidebar opens on</legend>
        ${[['library', 'Library'], ['albums', 'Albums'], ['playlists', 'Playlists'], ['genres', 'Genres']].map(([v, l]) => `<label><input type="radio" name="sideDefault" value="${v}" ${side === v ? 'checked' : ''}> ${l}</label>`).join('')}
      </fieldset>
    </section>`;

  $('#setClose').onclick = () => $<HTMLDialogElement>('#set').close();
  const status = (t: string) => ($('#wStatus').textContent = t);
  $('#wCheck').onclick = async () => {
    status('Checking…');
    try {
      const key = $<HTMLInputElement>('#wKey').value || null;
      const models = await api.writerModels($<HTMLInputElement>('#wUrl').value, key);
      $('#wModels').innerHTML = models.map((m) => `<option value="${esc(m)}">`).join('');
      if (models.length && !$<HTMLInputElement>('#wModel').value) $<HTMLInputElement>('#wModel').value = models[0];
      status(models.length ? `Connected. ${models.length} model${models.length === 1 ? '' : 's'} available.` : 'Connected, but the server listed no models. Type the model name.');
    } catch (e) {
      status(errText(e));
    }
  };
  $('#wSave').onclick = async () => {
    const key = $<HTMLInputElement>('#wKey').value;
    try {
      await api.setWriterConfig($<HTMLInputElement>('#wUrl').value, $<HTMLInputElement>('#wModel').value, $<HTMLInputElement>('#wName').value, key ? key : null);
      state.writer = await api.writerConfig();
      await reload();
      rerender();
      render();
      toast('Songwriter saved');
    } catch (e) {
      status(errText(e));
    }
  };
  $('#wForget')?.addEventListener('click', async () => {
    await api.setWriterConfig(state.writer.baseUrl, state.writer.model, state.writer.name, '').catch((e) => toast(errText(e)));
    state.writer = await api.writerConfig();
    render();
    toast('Key removed from your keychain');
  });
  $('#fAdd').onclick = async () => { if (await addMusicFolder()) render(); };
  const reRender = async (ok: boolean) => { if (ok) { state.writer = await api.writerConfig(); render(); } };
  document.getElementById('libSuggested')?.addEventListener('click', async () => reRender(await chooseLibrary(true)));
  document.getElementById('libChoose')?.addEventListener('click', async () => reRender(await chooseLibrary(false)));
  document.getElementById('libChange')?.addEventListener('click', async () => reRender(await chooseLibrary(false)));
  document.querySelectorAll<HTMLElement>('[data-make-lib]').forEach((b) => (b.onclick = async () => {
    if (b.dataset.armed !== '1') {
      b.dataset.armed = '1';
      b.textContent = 'Yes, organize it';
      toast('This moves the files in that folder into song folders.');
      return;
    }
    await reRender(await useAsLibrary(b.dataset.makeLib!));
  }));
  $('#fScan')?.addEventListener('click', async () => {
    const r = await api.rescan().catch((e) => { toast(errText(e)); return null; });
    if (!r) return;
    await reload();
    rerender();
    toast(r.added + r.updated + r.removed ? `${r.added} added, ${r.updated} updated, ${r.removed} removed` : 'Everything is up to date');
  });
  document.querySelectorAll<HTMLElement>('[data-rm-folder]').forEach((b) => (b.onclick = async () => {
    await api.removeFolder(Number(b.dataset.rmFolder)).catch((e) => toast(errText(e)));
    await reload();
    rerender();
    render();
    toast('Stopped watching that folder. Its songs left the library; the files are untouched.');
  }));
  document.querySelectorAll<HTMLInputElement>('[data-rename]').forEach((inp) => (inp.onchange = async () => {
    await api.renameGenre(Number(inp.dataset.rename), inp.value).catch((e) => toast(errText(e)));
    await reload();
    rerender();
    render();
  }));
  document.querySelectorAll<HTMLElement>('[data-del-tag]').forEach((b) => (b.onclick = async () => {
    if (b.dataset.armed !== '1') {
      b.dataset.armed = '1';
      b.textContent = 'Yes, delete';
      return;
    }
    await api.deleteGenre(Number(b.dataset.delTag)).catch((e) => toast(errText(e)));
    await reload();
    rerender();
    render();
  }));
  $<HTMLFormElement>('#tagForm').onsubmit = async (e) => {
    e.preventDefault();
    try {
      const g = await api.createGenre($<HTMLInputElement>('#tagNew').value);
      await reload();
      rerender();
      render();
      toast(`Added the tag ${g.name}`);
    } catch (err) {
      toast(errText(err));
    }
  };
  const amStatus = (t: string) => { const el = document.getElementById('amStatus'); if (el) el.textContent = t; };
  document.getElementById('amOn')?.addEventListener('click', async () => {
    try { await connectApple(); toast('Apple Music connected'); } catch (e) { toast(`Apple Music: ${errText(e)}`); }
    render();
  });
  document.getElementById('amOff')?.addEventListener('click', async () => { await disconnectApple().catch(() => {}); render(); rerender(); });
  $('#amShow').onchange = async (e) => { await saveSetting('showAppleInLibrary', (e.target as HTMLInputElement).checked); rerender(); };
  let keyPath = '';
  const validId = (v: string) => /^[A-Z0-9]{10}$/i.test(v.trim());
  const checkSave = () => {
    const btn = document.getElementById('amSave') as HTMLButtonElement | null;
    if (!btn) return;
    const kid = ($('#amKeyId') as HTMLInputElement).value, team = ($('#amTeam') as HTMLInputElement).value;
    btn.disabled = !(keyPath && validId(kid) && validId(team));
    amStatus(!keyPath ? '' : !validId(kid) ? 'The Key ID should be 10 letters and numbers.' : !validId(team) ? 'The Team ID should be 10 letters and numbers.' : 'Ready to save.');
  };
  document.getElementById('amKeyId')?.addEventListener('input', checkSave);
  document.getElementById('amTeam')?.addEventListener('input', checkSave);
  document.getElementById('amKey')?.addEventListener('click', async () => {
    const path = await pickFolder({ multiple: false, directory: false, filters: [{ name: 'MusicKit key', extensions: ['p8'] }] });
    if (typeof path !== 'string') return;
    keyPath = path;
    const name = path.split(/[\\/]/).pop() ?? path;
    $('#amFile').textContent = name;
    const m = name.match(/^AuthKey_([A-Z0-9]{10})\.p8$/i);
    if (m && !($('#amKeyId') as HTMLInputElement).value.trim()) ($('#amKeyId') as HTMLInputElement).value = m[1];
    checkSave();
  });
  document.getElementById('amSave')?.addEventListener('click', async () => {
    try {
      await setAppleKey(keyPath, ($('#amKeyId') as HTMLInputElement).value, ($('#amTeam') as HTMLInputElement).value);
      apple = await appleStatus();
      toast('MusicKit key saved in your Keychain. Now connect Apple Music.');
      render();
      rerender();
    } catch (e) {
      amStatus(errText(e));
      toast(`MusicKit key not saved: ${errText(e)}`);
      void api.logEvent('error', `musickit key: ${errText(e)}`);
    }
  });
  document.getElementById('amForget')?.addEventListener('click', async () => { await forgetAppleKey(); apple = await appleStatus(); render(); });
  document.querySelectorAll<HTMLInputElement>('input[name=stemsFlac]').forEach((r) => (r.onchange = () => void saveSetting('stemsFlac', r.value)));
  $('#flacAll').onclick = async () => {
    const st = (t: string) => ($('#flacStatus').textContent = t);
    const btn = $<HTMLButtonElement>('#flacAll');
    btn.disabled = true;
    st('Converting… this can take a minute for big stems.');
    try {
      const r = await api.convertStemsToFlac(null);
      const mb = (b: number) => `${Math.round(b / 1048576)} MB`;
      const kept = r.keptWav.length ? ` ${r.keptWav.length} stayed WAV.` : '';
      st(r.converted ? `Converted ${r.converted} stem${r.converted === 1 ? '' : 's'}: ${mb(r.bytesBefore)} became ${mb(r.bytesAfter)}.${kept}` : r.keptWav.length ? `Nothing converted.${kept}` : 'No WAV stems to convert.');
      await reload();
      if (r.keptWav.length)
        await message(`These WAV stems were kept because they could not be converted exactly:\n\n${r.keptWav.join('\n')}`, { title: 'Some stems stayed WAV', kind: 'warning' });
    } catch (e) {
      st(errText(e));
    }
    btn.disabled = false;
  };
  document.querySelectorAll<HTMLButtonElement>('[data-keycap]').forEach((b) => {
    b.onclick = () => {
      b.textContent = 'Press the keys…';
      const done = async (e: KeyboardEvent) => {
        e.preventDefault();
        e.stopPropagation();
        if (e.key === 'Escape') return finish();
        const c = comboOf(e);
        if (!c) return;
        if (!c.includes('Mod') && !c.includes('Ctrl') && !c.includes('Alt')) {
          b.textContent = 'Use a modifier key too';
          return;
        }
        await saveSetting(b.dataset.keycap!, c);
        finish();
      };
      const finish = () => {
        b.removeEventListener('keydown', done, true);
        render();
      };
      b.addEventListener('keydown', done, true);
      b.addEventListener('blur', finish, { once: true });
    };
  });
  const vStatus = (t: string) => ($('#vStatus').textContent = t);
  $('#vSave').onclick = async () => {
    const key = $<HTMLInputElement>('#vKey').value;
    try {
      await api.setSpeechSettings($<HTMLInputElement>('#vUrl').value, $<HTMLInputElement>('#vModel').value, $<HTMLInputElement>('#vLang').value, key || null);
      speech = await api.speechSettings();
      render();
      toast('Voice saved');
    } catch (e) {
      vStatus(errText(e));
    }
  };
  document.getElementById('vForget')?.addEventListener('click', async () => {
    await api.setSpeechSettings(speech.baseUrl, speech.model, speech.language, '').catch((e) => toast(errText(e)));
    speech = await api.speechSettings();
    render();
    toast('Key removed from your keychain');
  });
  $<HTMLSelectElement>('#cKind').onchange = (e) => {
    addKind = (e.target as HTMLSelectElement).value;
    render();
    $('#cKind').focus();
  };
  document.getElementById('cPlex')?.addEventListener('click', async () => {
    const st = (t: string) => ($('#cStatus').textContent = t);
    try {
      const pin = await api.plexLoginStart();
      st('Finish signing in to Plex in your browser…');
      let servers: PlexServer[] | null = null;
      for (let i = 0; i < 150 && !servers; i++) {
        await new Promise((r) => setTimeout(r, 2000));
        if (!document.getElementById('cStatus')) return; // Settings closed
        servers = await api.plexLoginCheck(pin);
      }
      if (!servers) return st('Plex sign-in timed out. Try again.');
      if (!servers.length) return st('No Plex servers on that account.');
      const connect = async (machine: string) => {
        st('Connecting…');
        try {
          toast(await api.plexConnect(machine));
          await loadConnections();
          render();
        } catch (e) {
          st(errText(e));
        }
      };
      if (servers.length === 1) return void connect(servers[0].machine);
      st('Pick your server:');
      $('#cPlexServers').innerHTML = `<ul class="set-list">${servers.map((sv) => `<li><span><b>${esc(sv.name)}</b> <span class="muted small">${sv.owned ? 'yours' : 'shared with you'}</span></span><button class="btn small" data-plex="${esc(sv.machine)}">Connect</button></li>`).join('')}</ul>`;
      $('#cPlexServers').querySelectorAll<HTMLElement>('[data-plex]').forEach((b) => (b.onclick = () => void connect(b.dataset.plex!)));
    } catch (e) {
      st(errText(e));
    }
  });
  document.getElementById('cAdd')?.addEventListener('click', async () => {
    const v = (id: string) => $<HTMLInputElement>(id).value;
    const st = (t: string) => ($('#cStatus').textContent = t);
    st('Checking the server…');
    try {
      const msg = await api.connectionAdd(v('#cKind'), v('#cName'), v('#cUrl'), v('#cUser'), v('#cPass'));
      await loadConnections();
      render();
      toast(msg);
    } catch (e) {
      st(errText(e));
    }
  });
  document.querySelectorAll<HTMLElement>('[data-rm-conn]').forEach((b) => (b.onclick = async () => {
    await api.connectionRemove(Number(b.dataset.rmConn)).catch((e) => toast(errText(e)));
    await loadConnections();
    render();
  }));
  $('#wExclude').onchange = (e) => void saveSetting('writerExclude', (e.target as HTMLInputElement).checked);
  $('#mcpOn').onchange = async (e) => {
    mcp = await api.mcpSetEnabled((e.target as HTMLInputElement).checked).catch((err) => ({ ...mcp, error: errText(err) }));
    await reload();
    render();
  };
  document.getElementById('mcpFull')?.addEventListener('change', (e) => void saveSetting('mcpFullControl', (e.target as HTMLInputElement).checked));
  document.getElementById('mcpCopyUrl')?.addEventListener('click', () => copyText(mcp.url, 'Address'));
  document.getElementById('mcpCopyToken')?.addEventListener('click', async () => copyText(await api.mcpToken(), 'Token'));
  document.getElementById('mcpCopyCfg')?.addEventListener('click', async () => {
    const cfg = { mcpServers: { 'lute-and-tavern': { type: 'http', url: mcp.url, headers: { Authorization: `Bearer ${await api.mcpToken()}` } } } };
    copyText(JSON.stringify(cfg, null, 2), 'Setup');
  });
  document.getElementById('mcpNew')?.addEventListener('click', async () => {
    mcp = await api.mcpNewToken().catch((err) => ({ ...mcp, error: errText(err) }));
    render();
    toast('New token made. Update it in your AI apps.');
  });
  $('#uAuto').onchange = (e) => void saveSetting('autoUpdate', (e.target as HTMLInputElement).checked);
  $('#uUrl').onchange = (e) => void saveSetting('updateUrl', (e.target as HTMLInputElement).value.trim());
  $('#uCheck').onclick = async () => {
    const st = (t: string) => ($('#uStatus').textContent = t);
    st('Checking…');
    try {
      const a = await api.checkUpdate();
      if (!a) return st('You have the newest version.');
      st(`Version ${a.version} is ready.`);
      offerUpdate(a.version);
    } catch (e) {
      st(errText(e));
    }
  };
  $('#setSkin').onchange = (e) => {
    const v = (e.target as HTMLInputElement).value;
    applySkin(v);
    void saveSetting('playerSkin', v);
  };
  $('#lineColor').onchange = (e) => {
    const v = (e.target as HTMLSelectElement).value;
    applyLineColor(v);
    void saveSetting('lineColor', v);
  };
  $('#setSide').onchange = async (e) => {
    const v = (e.target as HTMLInputElement).value;
    await saveSetting('sidebarDefault', v);
    state.sideTab = v as typeof state.sideTab;
    rerender();
  };
}

/** Shows a small prompt to install an update; nothing installs without a click. */
export function offerUpdate(version: string) {
  document.getElementById('updateBar')?.remove();
  const bar = document.createElement('div');
  bar.id = 'updateBar';
  bar.className = 'update-bar glass';
  bar.innerHTML = `<span>Lute &amp; Tavern ${esc(version)} is ready.</span><button class="btn primary small" id="uInstall">Update and restart</button><button class="btn small" id="uLater">Later</button>`;
  document.body.appendChild(bar);
  bar.querySelector<HTMLElement>('#uLater')!.onclick = () => bar.remove();
  bar.querySelector<HTMLElement>('#uInstall')!.onclick = async () => {
    bar.querySelector('span')!.textContent = 'Downloading the update…';
    try {
      await api.installUpdate();
    } catch (e) {
      bar.querySelector('span')!.textContent = errText(e);
    }
  };
}

/** Called at startup: only checks when the person turned updates on. */
export async function checkForUpdatesAtStart() {
  if (!state.settings.autoUpdate) return;
  try {
    const info = await api.updateInfo();
    if (!info.canUpdate) return;
    const a = await api.checkUpdate();
    if (a) offerUpdate(a.version);
  } catch {
    /* offline or no address: stay quiet at startup */
  }
}
