<script lang="ts">
  // Workspace files (ADR-006, the Files rendering of the card): a directory with breadcrumbs, text and image preview, file and
  // photo upload (by button and drag-and-drop), folder/file/rename/delete — via agentd as the coder user.
  import { onDestroy, tick } from 'svelte';
  import { api, type FsEntry } from './api';
  import { t } from './i18n';
  let { project, root }: { project: string; root: string } = $props();
  let path = $state('');
  let entries = $state<FsEntry[]>([]);
  let error = $state('');
  let hidden = $state(false);
  let listing = $state(false);
  let mutating = $state(false);
  let drag = $state(false);
  let uploadInput = $state<HTMLInputElement | null>(null);
  let preview = $state<{ path: string; kind: 'text' | 'image' | 'other'; text?: string; size: number } | null>(null);
  let previewLoading = $state(false);
  let previewError = $state('');
  let previewEntry: FsEntry | null = null;
  let previewOrigin: HTMLButtonElement | null = null;
  let listSeq = 0; let previewSeq = 0; let alive = true;
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  const at = (dir: string, name: string) => (dir.endsWith('/') ? dir : dir + '/') + name;
  const join = (name: string) => at(path, name);
  const current = (owner: string) => alive && owner === project;
  function resetPreview() { ++previewSeq; preview = null; previewError = ''; previewLoading = false; }
  async function closePreview() { resetPreview(); await tick(); if (previewOrigin?.isConnected) previewOrigin.focus(); }
  async function load(p = path) {
    const seq = ++listSeq; const owner = project;
    if (p !== path) { resetPreview(); path = p; entries = []; }
    listing = true;
    try {
      const l = await api.fsList(owner, p);
      if (!current(owner) || seq !== listSeq) return;
      path = l.path; entries = l.entries; error = '';
    } catch (e) { if (current(owner) && seq === listSeq) error = msg(e); }
    finally { if (current(owner) && seq === listSeq) listing = false; }
  }
  // Workspace polling must not reset the reader's directory. Only a real root/project change does.
  let loadedRoot = '';
  $effect(() => {
    const key = `${project}:${root}`;
    if (key !== loadedRoot) { loadedRoot = key; entries = []; mutating = false; resetPreview(); load(root); }
  });
  onDestroy(() => { alive = false; ++listSeq; ++previewSeq; });
  const shown = $derived(entries.filter(e => hidden || !e.name.startsWith('.')));
  const crumbs = $derived.by(() => { const parts = path.split('/').filter(Boolean); return parts.map((p, i) => ({ name: p, path: '/' + parts.slice(0, i + 1).join('/') })); });
  const up = () => load(path.replace(/\/[^/]+\/?$/, '') || '/');
  const IMG = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'avif'];
  function icon(name: string) { const ext = name.split('.').pop()?.toLowerCase() ?? ''; return IMG.includes(ext) ? '🖼' : ['md', 'txt'].includes(ext) ? '📝' : ['zip', 'gz', 'tar', 'tgz', '7z'].includes(ext) ? '🗜' : ['pdf'].includes(ext) ? '📕' : '📄'; }
  async function open(e: FsEntry, origin?: HTMLButtonElement) {
    if (listing) return;
    if (e.kind === 'dir') { resetPreview(); await load(join(e.name)); return; }
    const seq = ++previewSeq; const owner = project; const p = join(e.name);
    if (origin) previewOrigin = origin;
    previewEntry = e; previewError = ''; previewLoading = false;
    const ext = e.name.split('.').pop()?.toLowerCase() ?? '';
    if (IMG.includes(ext)) { preview = { path: p, kind: 'image', size: e.size }; return; }
    if (e.size > 512 * 1024) { preview = { path: p, kind: 'other', size: e.size }; return; }
    preview = { path: p, kind: 'text', text: '', size: e.size }; previewLoading = true;
    try {
      const text = await api.fsReadText(owner, p);
      if (current(owner) && seq === previewSeq) preview = { path: p, kind: 'text', text, size: e.size };
    } catch (er) { if (current(owner) && seq === previewSeq) previewError = msg(er); }
    finally { if (current(owner) && seq === previewSeq) previewLoading = false; }
  }
  async function change(op: (owner: string, dir: string) => Promise<unknown>) {
    if (mutating || listing) return;
    const owner = project; const dir = path;
    mutating = true;
    try {
      await op(owner, dir);
      if (current(owner) && path === dir) await load(dir);
    } catch (e) { if (current(owner)) error = msg(e); }
    finally { if (current(owner)) mutating = false; }
  }
  async function upload(files: FileList | File[]) {
    const items = Array.from(files);
    if (!items.length) return;
    // Capture the destination once: navigating during a multi-file upload must not split it across folders.
    await change(async (owner, dir) => { for (const f of items) await api.fsWrite(owner, at(dir, f.name), f); });
  }
  function onDrop(ev: DragEvent) { ev.preventDefault(); drag = false; if (ev.dataTransfer?.files?.length) upload(ev.dataTransfer.files); }
  async function mkdir() { const n = window.prompt(t('agents.filesNewFolderName'), 'new-folder'); if (n) await change((owner, dir) => api.fsMkdir(owner, at(dir, n))); }
  async function newFile() { const n = window.prompt(t('agents.filesNewFileName'), 'notes.md'); if (n) await change((owner, dir) => api.fsWrite(owner, at(dir, n), '')); }
  async function rename(e: FsEntry) { const n = window.prompt(t('agents.filesRenameTo'), e.name); if (n && n !== e.name) await change((owner, dir) => api.fsRename(owner, at(dir, e.name), at(dir, n))); }
  async function del(e: FsEntry) {
    if (!window.confirm(t('agents.filesConfirmDelete', { name: e.name }))) return;
    await change(async (owner, dir) => {
      await api.fsDelete(owner, at(dir, e.name), e.kind === 'dir');
      if (current(owner) && preview?.path === at(dir, e.name)) resetPreview();
    });
  }
  function fmtSize(n: number) { return n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`; }
  function fmtTime(ts: number) { return ts ? new Date(ts * 1000).toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' }) : ''; }
</script>

<div class="files" role="region" aria-label={t('agents.tabFiles')} ondragover={(e) => { e.preventDefault(); drag = true; }} ondragleave={() => (drag = false)} ondrop={onDrop}>
  <div class="filebar">
    <div class="crumbs"><button class="crumb" onclick={() => load('/')}>/</button>{#each crumbs as c (c.path)}<span class="sep">›</span><button class="crumb" onclick={() => load(c.path)}>{c.name}</button>{/each}</div>
    <span class="sp"></span>
    <input type="file" multiple hidden bind:this={uploadInput} onchange={(e) => { const f = Array.from((e.target as HTMLInputElement).files ?? []); (e.target as HTMLInputElement).value = ''; if (f.length) upload(f); }} />
    <button class="mini up" disabled={mutating || listing} title={t('agents.filesUploadHint')} onclick={() => uploadInput?.click()}>⬆ {t('agents.filesUpload')}</button>
    <button class="mini" disabled={mutating || listing} aria-label={t('agents.filesNewFolder')} title={t('agents.filesNewFolder')} onclick={mkdir}>📁+</button>
    <button class="mini" disabled={mutating || listing} aria-label={t('agents.filesNewFile')} title={t('agents.filesNewFile')} onclick={newFile}>📄+</button>
    <button class="mini" class:active={hidden} aria-pressed={hidden} aria-label={t('agents.filesHidden')} title={t('agents.filesHidden')} onclick={() => (hidden = !hidden)}>.*</button>
    <button class="mini" aria-label={t('agents.refresh')} title={t('agents.refresh')} onclick={() => load()}>⟳</button>
  </div>
  {#if error}<div class="err" role="alert">{error} <button class="mini" onclick={() => load()}>{t('wsux.retry')}</button></div>{/if}
  <div class="split" class:with-preview={!!preview}>
    <div class="list" aria-busy={listing}>
      {#if listing}<div class="muted hint" role="status">{t('wsux.loading')}</div>{/if}
      {#if path !== '/'}<button class="row open" onclick={up}><span class="ic">↩</span><span class="nm">..</span></button>{/if}
      {#each shown as e (e.name)}
        <div class="row" class:sel={preview?.path === join(e.name)}>
          <button class="open" disabled={listing} onclick={(ev) => open(e, ev.currentTarget)}><span class="ic">{e.kind === 'dir' ? '📁' : icon(e.name)}</span><span class="nm">{e.name}{e.link ? ' ↗' : ''}</span><span class="meta">{e.kind === 'dir' ? '' : fmtSize(e.size)}</span><span class="meta time">{fmtTime(e.mtime)}</span></button>
          <button class="ico" disabled={mutating || listing} aria-label={t('wsux.renameFile', { name: e.name })} title={t('agents.filesRename')} onclick={() => rename(e)}>✎</button>
          <button class="ico danger" disabled={mutating || listing} aria-label={t('wsux.deleteFile', { name: e.name })} title={t('agents.filesDelete')} onclick={() => del(e)}>🗑</button>
        </div>
      {/each}
      {#if !shown.length && !listing && !error}<div class="muted hint">{t('agents.filesEmpty')}</div>{/if}
    </div>
    {#if preview}
      <div class="preview">
        <div class="ph"><button class="mini back" onclick={closePreview}>← {t('wsux.back')}</button><b title={preview.path}>{preview.path.split('/').pop()}</b><span class="muted sm">{fmtSize(preview.size)}</span><span class="sp"></span><a class="mini dl" href={api.fsFileUrl(project, preview.path, true)} download>⤓ {t('agents.filesDownload')}</a><button class="ico close-preview" aria-label={t('wsux.closePreview')} onclick={closePreview}>✕</button></div>
        {#if previewLoading}<div class="muted hint" role="status">{t('wsux.loading')}</div>
        {:else if previewError}<div class="err" role="alert">{previewError} <button class="mini" onclick={() => previewEntry && open(previewEntry)}>{t('wsux.retry')}</button></div>
        {:else if preview.kind === 'image'}<div class="img"><img src={api.fsFileUrl(project, preview.path)} alt={preview.path} /></div>
        {:else if preview.kind === 'text'}<pre>{preview.text}</pre>
        {:else}<div class="muted hint">{t('agents.filesTooBig')}</div>{/if}
      </div>
    {/if}
  </div>
  {#if drag}<div class="dropzone">⬆ {t('agents.filesDrop')}</div>{/if}
</div>

<style>
  .files { container: workspace-files / inline-size; min-width: 0; position: relative; display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 6px; font-size: 13px; }
  .filebar { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; }
  .crumbs { display: flex; align-items: center; gap: 2px; flex-wrap: wrap; min-width: 0; }
  .crumb { background: transparent; border-color: transparent; padding: 2px 6px; color: var(--muted); font-size: 12.5px; }
  .crumb:hover { color: var(--ink); background: var(--panel-2); }
  .sep { color: var(--muted); font-size: 12px; }
  .sp { flex: 1; }
  .mini { font-size: 12px; padding: 2px 8px; }
  .mini.active { background: rgb(var(--accent-rgb) / .2); border-color: var(--accent); }
  .up { cursor: pointer; border: 1px solid var(--line); border-radius: 8px; background: var(--panel-2); }
  .err { color: var(--bad); font-size: 12.5px; }
  .split { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(0, 1fr); gap: 8px; }
  .split.with-preview { grid-template-columns: minmax(0, 1fr) minmax(0, 1.2fr); }
  .list { overflow: auto; min-height: 0; border: 1px solid var(--line); border-radius: 10px; padding: 4px; }
  .row { display: flex; align-items: center; gap: 2px; border-radius: 8px; }
  .row:hover { background: var(--panel-2); }
  .row.sel { background: rgb(var(--accent-rgb) / .14); }
  .row .ico { opacity: 0; }
  .row:hover .ico, .row:focus-within .ico { opacity: 1; }
  .open { flex: 1; min-width: 0; display: flex; align-items: center; gap: 8px; text-align: left; background: transparent; border-color: transparent; padding: 4px 8px; }
  .ic { width: 20px; text-align: center; flex: none; }
  .nm { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { color: var(--muted); font-size: 11.5px; white-space: nowrap; }
  .meta.time { width: 110px; text-align: right; }
  .ico { width: 24px; height: 24px; padding: 0; font-size: 12px; background: transparent; border-color: transparent; color: var(--muted); flex: none; }
  .ico:hover { color: var(--ink); background: var(--panel-2); }
  .ico.danger:hover { color: var(--bad); }
  .preview { display: flex; flex-direction: column; min-height: 0; border: 1px solid var(--line); border-radius: 10px; padding: 6px 8px; gap: 6px; }
  .ph { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .ph b { min-width: 0; overflow-wrap: anywhere; }
  .back { display: none; }
  .dl { text-decoration: none; display: inline-flex; align-items: center; min-height: 32px; }
  .preview pre { flex: 1; min-height: 0; overflow: auto; margin: 0; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 12px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .img { flex: 1; min-height: 0; overflow: auto; display: grid; place-items: center; }
  .img img { max-width: 100%; max-height: 100%; border-radius: 8px; }
  .muted { color: var(--muted); }
  .sm { font-size: 11.5px; }
  .hint { font-size: 12.5px; padding: 8px; }
  .dropzone { position: absolute; inset: 0; display: grid; place-items: center; background: rgb(var(--accent-rgb) / .12); border: 2px dashed var(--accent); border-radius: 10px; font-weight: 600; pointer-events: none; }
  @container workspace-files (max-width: 680px) {
    .split.with-preview { grid-template-columns: minmax(0, 1fr); }
    .split.with-preview .list { display: none; }
    .back { display: inline-flex; }
    .close-preview { display: none; }
    .ph b { order: 3; flex: 1; flex-basis: calc(100% - 55px); }
    .ph .sm { order: 4; }
    .ph .sp { order: 1; }
    .ph .dl { order: 2; }
    .meta.time { display: none; }
    .row .ico { opacity: 1; }
    .crumbs { width: 100%; }
  }
  @media (pointer: coarse) {
    .mini, .crumb, .ico, .open, .dl { min-height: 44px; }
    .ico { min-width: 44px; }
    .row .ico { opacity: 1; }
  }
</style>
