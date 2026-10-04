<script lang="ts">
  // Файлы workspace'а (ADR-006, рендеринг Files карточки): каталог с хлебными крошками, предпросмотр текста и картинок,
  // загрузка файлов и фото (кнопкой и перетаскиванием), папка/файл/переименование/удаление — через agentd от пользователя coder.
  import { api, type FsEntry } from './api';
  import { t } from './i18n.svelte';
  let { project, root }: { project: string; root: string } = $props();
  let path = $state(root);
  let entries = $state<FsEntry[]>([]);
  let error = $state('');
  let hidden = $state(false);
  let busy = $state(false);
  let drag = $state(false);
  let preview = $state<{ path: string; kind: 'text' | 'image' | 'other'; text?: string; size: number } | null>(null);
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  async function load(p = path) {
    busy = true;
    try { const l = await api.fsList(project, p); path = l.path; entries = l.entries; error = ''; } catch (e) { error = msg(e); } finally { busy = false; }
  }
  // корень грузим только при реальной смене пути: проп `root` приходит из объекта сессии, который опрос рабочих мест
  // пересоздаёт каждые 5 с, и без этой защиты эффект откатывал открытую папку к корню
  let loadedRoot = '';
  $effect(() => { if (root !== loadedRoot) { loadedRoot = root; load(root); } });
  const shown = $derived(entries.filter(e => hidden || !e.name.startsWith('.')));
  const crumbs = $derived.by(() => { const parts = path.split('/').filter(Boolean); return parts.map((p, i) => ({ name: p, path: '/' + parts.slice(0, i + 1).join('/') })); });
  const join = (name: string) => (path.endsWith('/') ? path : path + '/') + name;
  const up = () => load(path.replace(/\/[^/]+\/?$/, '') || '/');
  const IMG = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'avif'];
  function icon(name: string) { const ext = name.split('.').pop()?.toLowerCase() ?? ''; return IMG.includes(ext) ? '🖼' : ['md', 'txt'].includes(ext) ? '📝' : ['zip', 'gz', 'tar', 'tgz', '7z'].includes(ext) ? '🗜' : ['pdf'].includes(ext) ? '📕' : '📄'; }
  async function open(e: FsEntry) {
    if (e.kind === 'dir') { preview = null; await load(join(e.name)); return; }
    const p = join(e.name); const ext = e.name.split('.').pop()?.toLowerCase() ?? '';
    if (IMG.includes(ext)) { preview = { path: p, kind: 'image', size: e.size }; return; }
    if (e.size <= 512 * 1024) { try { preview = { path: p, kind: 'text', text: await api.fsReadText(project, p), size: e.size }; } catch (er) { error = msg(er); } return; }
    preview = { path: p, kind: 'other', size: e.size };
  }
  async function upload(files: FileList | File[]) {
    busy = true;
    try { for (const f of Array.from(files)) await api.fsWrite(project, join(f.name), f); await load(); } catch (e) { error = msg(e); } finally { busy = false; }
  }
  function onDrop(ev: DragEvent) { ev.preventDefault(); drag = false; if (ev.dataTransfer?.files?.length) upload(ev.dataTransfer.files); }
  async function mkdir() { const n = window.prompt(t('agents.filesNewFolderName'), 'new-folder'); if (!n) return; try { await api.fsMkdir(project, join(n)); await load(); } catch (e) { error = msg(e); } }
  async function newFile() { const n = window.prompt(t('agents.filesNewFileName'), 'notes.md'); if (!n) return; try { await api.fsWrite(project, join(n), ''); await load(); } catch (e) { error = msg(e); } }
  async function rename(e: FsEntry) { const n = window.prompt(t('agents.filesRenameTo'), e.name); if (!n || n === e.name) return; try { await api.fsRename(project, join(e.name), join(n)); await load(); } catch (er) { error = msg(er); } }
  async function del(e: FsEntry) {
    if (!window.confirm(t('agents.filesConfirmDelete', { name: e.name }))) return;
    try { await api.fsDelete(project, join(e.name), e.kind === 'dir'); if (preview?.path === join(e.name)) preview = null; await load(); } catch (er) { error = msg(er); }
  }
  function fmtSize(n: number) { return n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`; }
  function fmtTime(ts: number) { return ts ? new Date(ts * 1000).toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' }) : ''; }
</script>

<div class="files" role="region" aria-label={t('agents.tabFiles')} ondragover={(e) => { e.preventDefault(); drag = true; }} ondragleave={() => (drag = false)} ondrop={onDrop}>
  <div class="bar">
    <div class="crumbs"><button class="crumb" onclick={() => load('/')}>/</button>{#each crumbs as c (c.path)}<span class="sep">›</span><button class="crumb" onclick={() => load(c.path)}>{c.name}</button>{/each}</div>
    <span class="sp"></span>
    <label class="mini up" title={t('agents.filesUploadHint')}>⬆ {t('agents.filesUpload')}<input type="file" multiple onchange={(e) => { const f = (e.target as HTMLInputElement).files; if (f?.length) upload(f); (e.target as HTMLInputElement).value = ''; }} /></label>
    <button class="mini" title={t('agents.filesNewFolder')} onclick={mkdir}>📁+</button>
    <button class="mini" title={t('agents.filesNewFile')} onclick={newFile}>📄+</button>
    <button class="mini" class:active={hidden} title={t('agents.filesHidden')} onclick={() => (hidden = !hidden)}>.*</button>
    <button class="mini" title={t('agents.refresh')} onclick={() => load()}>⟳</button>
  </div>
  {#if error}<div class="err">{error}</div>{/if}
  <div class="split" class:with-preview={!!preview}>
    <div class="list">
      {#if path !== '/'}<button class="row open" onclick={up}><span class="ic">↩</span><span class="nm">..</span></button>{/if}
      {#each shown as e (e.name)}
        <div class="row" class:sel={preview?.path === join(e.name)}>
          <button class="open" onclick={() => open(e)}><span class="ic">{e.kind === 'dir' ? '📁' : icon(e.name)}</span><span class="nm">{e.name}{e.link ? ' ↗' : ''}</span><span class="meta">{e.kind === 'dir' ? '' : fmtSize(e.size)}</span><span class="meta time">{fmtTime(e.mtime)}</span></button>
          <button class="ico" title={t('agents.filesRename')} onclick={() => rename(e)}>✎</button>
          <button class="ico danger" title={t('agents.filesDelete')} onclick={() => del(e)}>🗑</button>
        </div>
      {/each}
      {#if !shown.length && !busy}<div class="muted hint">{t('agents.filesEmpty')}</div>{/if}
    </div>
    {#if preview}
      <div class="preview">
        <div class="ph"><b>{preview.path.split('/').pop()}</b><span class="muted sm">{fmtSize(preview.size)}</span><span class="sp"></span><a class="mini dl" href={api.fsFileUrl(project, preview.path, true)} download>⤓ {t('agents.filesDownload')}</a><button class="ico" onclick={() => (preview = null)}>✕</button></div>
        {#if preview.kind === 'image'}<div class="img"><img src={api.fsFileUrl(project, preview.path)} alt={preview.path} /></div>
        {:else if preview.kind === 'text'}<pre>{preview.text}</pre>
        {:else}<div class="muted hint">{t('agents.filesTooBig')}</div>{/if}
      </div>
    {/if}
  </div>
  {#if drag}<div class="dropzone">⬆ {t('agents.filesDrop')}</div>{/if}
</div>

<style>
  .files { position: relative; display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 6px; font-size: 13px; }
  .bar { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; }
  .crumbs { display: flex; align-items: center; gap: 2px; flex-wrap: wrap; min-width: 0; }
  .crumb { background: transparent; border-color: transparent; padding: 2px 6px; color: var(--muted); font-size: 12.5px; }
  .crumb:hover { color: var(--ink); background: var(--panel-2); }
  .sep { color: var(--muted); font-size: 12px; }
  .sp { flex: 1; }
  .mini { font-size: 12px; padding: 2px 8px; }
  .mini.active { background: rgb(var(--accent-rgb) / .2); border-color: var(--accent); }
  .up { cursor: pointer; border: 1px solid var(--line); border-radius: 8px; background: var(--panel-2); }
  .up input { display: none; }
  .err { color: var(--bad); font-size: 12.5px; }
  .split { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(0, 1fr); gap: 8px; }
  .split.with-preview { grid-template-columns: minmax(0, 1fr) minmax(0, 1.2fr); }
  .list { overflow: auto; min-height: 0; border: 1px solid var(--line); border-radius: 10px; padding: 4px; }
  .row { display: flex; align-items: center; gap: 2px; border-radius: 8px; }
  .row:hover { background: var(--panel-2); }
  .row.sel { background: rgb(var(--accent-rgb) / .14); }
  .row .ico { opacity: 0; }
  .row:hover .ico { opacity: 1; }
  .open { flex: 1; min-width: 0; display: flex; align-items: center; gap: 8px; text-align: left; background: transparent; border-color: transparent; padding: 4px 8px; }
  .ic { width: 20px; text-align: center; flex: none; }
  .nm { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { color: var(--muted); font-size: 11.5px; white-space: nowrap; }
  .meta.time { width: 110px; text-align: right; }
  .ico { width: 24px; height: 24px; padding: 0; font-size: 12px; background: transparent; border-color: transparent; color: var(--muted); flex: none; }
  .ico:hover { color: var(--ink); background: var(--panel-2); }
  .ico.danger:hover { color: var(--bad); }
  .preview { display: flex; flex-direction: column; min-height: 0; border: 1px solid var(--line); border-radius: 10px; padding: 6px 8px; gap: 6px; }
  .ph { display: flex; align-items: center; gap: 8px; }
  .dl { text-decoration: none; }
  .preview pre { flex: 1; min-height: 0; overflow: auto; margin: 0; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 12px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .img { flex: 1; min-height: 0; overflow: auto; display: grid; place-items: center; }
  .img img { max-width: 100%; max-height: 100%; border-radius: 8px; }
  .muted { color: var(--muted); }
  .sm { font-size: 11.5px; }
  .hint { font-size: 12.5px; padding: 8px; }
  .dropzone { position: absolute; inset: 0; display: grid; place-items: center; background: rgb(var(--accent-rgb) / .12); border: 2px dashed var(--accent); border-radius: 10px; font-weight: 600; pointer-events: none; }
  @media (max-width: 700px) { .split.with-preview { grid-template-columns: minmax(0, 1fr); grid-template-rows: minmax(0, 1fr) minmax(0, 1fr); } .meta.time { display: none; } .row .ico { opacity: 1; } }
</style>
