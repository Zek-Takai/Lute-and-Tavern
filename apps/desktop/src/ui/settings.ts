// Settings: the songwriter connection, music folders, genre tags and the sidebar's starting tab.
import { open as pickFolder } from '@tauri-apps/plugin-dialog';
import { api, errText, type UpdateInfo } from '../api';
import { reload, rerender, saveSetting, state } from '../state';
import { $, esc, toast } from '../util';

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

export async function openSettings(section?: 'writer') {
  state.writer = await api.writerConfig();
  updates = await api.updateInfo();
  render();
  const d = $<HTMLDialogElement>('#set');
  if (!d.open) d.showModal();
  if (section === 'writer') $('#wUrl')?.focus();
}

function render() {
  const w = state.writer;
  const side = (state.settings.sidebarDefault as string) || 'library';
  $('#setBody').innerHTML = `
    <div class="row" style="justify-content:space-between"><h2 id="setTitle">Settings</h2><button class="btn" id="setClose">Done</button></div>

    <section class="set-sec">
      <h3>Songwriter</h3>
      <p class="muted small">Songs are written by any AI that speaks the OpenAI chat format: a model on your own computer or network (LM Studio, Ollama, vLLM) or a hosted service. Your key is kept in the system keychain.</p>
      <div class="form">
        <label for="wName">Name</label><input id="wName" value="${esc(w.name)}" placeholder="Bard">
        <label for="wUrl">Address</label><input id="wUrl" value="${esc(w.baseUrl)}" placeholder="http://localhost:1234/v1">
        <label for="wModel">Model</label><input id="wModel" value="${esc(w.model)}" list="wModels" placeholder="Check the connection to list models">
        <label for="wKey">API key</label><input id="wKey" type="password" autocomplete="off" placeholder="${w.hasKey ? 'Saved in your keychain. Type to replace it.' : 'Only needed for hosted services'}">
      </div>
      <datalist id="wModels"></datalist>
      <div class="row"><button class="btn" id="wCheck">Check connection</button><button class="btn primary" id="wSave">Save songwriter</button>${w.hasKey ? '<button class="btn small" id="wForget">Remove saved key</button>' : ''}<span class="muted small" id="wStatus"></span></div>
    </section>

    <section class="set-sec">
      <h3>Music folders</h3>
      <p class="muted small">Lute &amp; Tavern watches these folders. Songs you add, change or delete in them update here on their own.</p>
      <ul class="set-list">${state.folders.map((f) => `<li><span class="path">${esc(f.path)}</span><button class="btn small" data-rm-folder="${f.id}">Stop watching</button></li>`).join('') || '<li class="muted">No folders yet.</li>'}</ul>
      <div class="row"><button class="btn" id="fAdd">Add a music folder</button>${state.folders.length ? '<button class="btn" id="fScan">Check folders now</button>' : ''}</div>
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
      <h3>Updates</h3>
      <p class="muted small">Version ${esc(updates.currentVersion)}. ${updates.canUpdate
        ? 'When this is on, Lute &amp; Tavern checks for a new release each time it starts and asks before installing.'
        : 'This copy was built on this Mac for everyday use and cannot update itself. Official releases can.'}</p>
      <label class="row"><input type="checkbox" id="uAuto" ${state.settings.autoUpdate ? 'checked' : ''} ${updates.canUpdate ? '' : 'disabled'}> Check for updates when the app starts</label>
      <div class="form"><label for="uUrl">Address</label><input id="uUrl" value="${esc((state.settings.updateUrl as string) || '')}" placeholder="${esc(updates.defaultUrl || 'https://example.com/lute-and-tavern/latest.json')}" ${updates.canUpdate ? '' : 'disabled'}></div>
      <div class="row"><button class="btn" id="uCheck" ${updates.canUpdate ? '' : 'disabled'}>Check now</button><span class="muted small" id="uStatus"></span></div>
    </section>

    <section class="set-sec">
      <fieldset class="choice" id="setSide">
        <legend>Sidebar opens on</legend>
        ${[['library', 'Library'], ['playlists', 'Playlists'], ['genres', 'Genres']].map(([v, l]) => `<label><input type="radio" name="sideDefault" value="${v}" ${side === v ? 'checked' : ''}> ${l}</label>`).join('')}
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
