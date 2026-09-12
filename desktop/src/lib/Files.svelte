<script lang="ts">
  // «Файлы»: три корня (данные приложений, Store, home workspace'ов), список или плитки (запоминается), сортировка,
  // редактор текстов, предпросмотр картинок, загрузка/скачивание. Десктоп — боковая панель корней и таблица/плитки;
  // телефон — крупные строки или плитки, меню действий листом снизу, редактор на весь экран.
  import { api } from './api';
  import Dropdown from './Dropdown.svelte';
  import { device, NAV_H } from './device.svelte';
  import { fade, fly } from 'svelte/transition';
  import { t, fmt } from './i18n.svelte';
  let { root: initialRoot = 'apps', path: initialPath = '' }: { root?: string; path?: string } = $props();
  type Entry = { name: string; dir: boolean; size: number; modified?: string | null; text: boolean };
  type SheetItem = { label: string; icon: string; danger?: boolean; run: () => void };
  type View = 'list' | 'grid';
  type Sort = 'name' | 'date' | 'size';
  const pref = (k: string, d: string) => { try { return localStorage.getItem(k) ?? d; } catch { return d; } };
  let roots = $state<{ id: string; title: string; kind: string }[]>([]);
  let root = $state(initialRoot);
  let path = $state(initialPath);
  let entries = $state<Entry[]>([]);
  let error = $state('');
  let busy = $state('');
  let editing = $state<{ name: string; content: string; dirty: boolean } | null>(null);
  let preview = $state<string | null>(null);
  let selected = $state<string | null>(null);
  let sheet = $state<{ title: string; items: SheetItem[]; x?: number; y?: number } | null>(null);
  let view = $state<View>(pref('cloudos.files.view', 'list') as View);
  let sortBy = $state<Sort>(pref('cloudos.files.sort', 'name') as Sort);
  let fileInput: HTMLInputElement | undefined = $state();
  const join = (a: string, b: string) => (a ? `${a}/${b}` : b);
  const crumbs = $derived(path ? path.split('/') : []);
  const isImage = (n: string) => /\.(png|jpe?g|gif|webp|svg|avif)$/i.test(n);
  function glyph(e: Entry) {
    if (e.dir) return '📁';
    const n = e.name.toLowerCase();
    if (isImage(n)) return '🖼';
    if (/\.(mp3|flac|wav|ogg|m4a)$/.test(n)) return '🎵';
    if (/\.(mp4|mkv|webm|mov|avi)$/.test(n)) return '🎬';
    if (/\.(zip|tar|gz|tgz|xz|7z|rar|bz2)$/.test(n)) return '🗜';
    if (/\.pdf$/.test(n)) return '📕';
    if (/\.(ya?ml|json|toml|ini|env|conf|cfg)$/.test(n) || n.startsWith('.env')) return '⚙️';
    if (/\.(sh|py|js|ts|rs|go|css|html|sql)$/.test(n)) return '🧩';
    if (/\.(db|sqlite|sqlite3)$/.test(n)) return '🗄';
    if (/\.(log)$/.test(n)) return '🧾';
    return e.text ? '📄' : '📦';
  }
  const rootIcon = (id: string) => id === 'apps' ? '📦' : id === 'store' ? '🛍️' : '🧩';
  const sorted = $derived([...entries].sort((a, b) => {
    if (a.dir !== b.dir) return a.dir ? -1 : 1;
    if (sortBy === 'size') return b.size - a.size || a.name.localeCompare(b.name);
    if (sortBy === 'date') return (b.modified ?? '').localeCompare(a.modified ?? '') || a.name.localeCompare(b.name);
    return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
  }));
  async function load() {
    try { const l = await api.fileList(root, path); entries = l.entries; error = ''; } catch (e) { error = e instanceof Error ? e.message : String(e); entries = []; }
  }
  $effect(() => { api.fileRoots().then(r => roots = r).catch(() => {}); });
  $effect(() => { root; path; editing = null; preview = null; selected = null; sheet = null; load(); });
  $effect(() => { try { localStorage.setItem('cloudos.files.view', view); localStorage.setItem('cloudos.files.sort', sortBy); } catch {} });
  function when(m?: string | null) { return m ? fmt.dateTime(m, { dateStyle: 'medium', timeStyle: 'short' }) : ''; }
  function up() { path = crumbs.slice(0, -1).join('/'); }
  async function open(e: Entry) {
    if (e.dir) { path = join(path, e.name); return; }
    const full = join(path, e.name);
    if (e.text && e.size < 2 * 1024 * 1024) { try { editing = { name: e.name, content: await api.fileRead(root, full), dirty: false }; preview = null; } catch (err) { error = String(err); } }
    else if (isImage(e.name)) { preview = api.fileReadUrl(root, full); editing = null; }
    else window.open(api.fileReadUrl(root, full), '_blank');
  }
  async function act(label: string, f: () => Promise<unknown>) { busy = label; try { await f(); error = ''; } catch (e) { error = e instanceof Error ? e.message : String(e); } finally { busy = ''; await load(); } }
  async function save() { if (!editing) return; const ed = editing; await act(t('files.busySaving'), async () => { await api.fileWrite(root, join(path, ed.name), ed.content); ed.dirty = false; }); }
  async function upload(files: FileList | null) { if (!files) return; for (const f of Array.from(files)) await act(t('files.busyUploading', { name: f.name }), () => api.fileUpload(root, join(path, f.name), f)); }
  function newFile() { const n = prompt(t('files.promptNewFile')); if (!n) return; act(t('files.busyCreating'), async () => { await api.fileWrite(root, join(path, n), ''); }); }
  function newDir() { const n = prompt(t('files.promptNewDir')); if (n) act(t('files.busyCreating'), () => api.fileMkdir(root, join(path, n))); }
  function rename(e: Entry) { const n = prompt(t('files.promptRename'), e.name); if (n && n !== e.name) act(t('files.busyRenaming'), () => api.fileRename(root, join(path, e.name), join(path, n))); }
  function del(e: Entry) { if (confirm(t(e.dir ? 'files.confirmDeleteDir' : 'files.confirmDeleteFile', { name: e.name }))) act(t('files.busyDeleting'), () => api.fileDelete(root, join(path, e.name))); }
  function download(e: Entry) { const a = document.createElement('a'); a.href = api.fileReadUrl(root, join(path, e.name)); a.download = e.name; a.click(); }
  function onDrop(ev: DragEvent) { ev.preventDefault(); upload(ev.dataTransfer?.files ?? null); }
  // меню действий: лист снизу на телефоне, всплывающее меню у курсора на десктопе
  function folderSheet(ev?: MouseEvent) {
    sheet = { title: path ? `/${path}` : roots.find(r => r.id === root)?.title ?? '/', x: ev?.clientX, y: ev?.clientY, items: [
      { label: t('files.newFile'), icon: '📄', run: newFile },
      { label: t('files.newDir'), icon: '📁', run: newDir },
      { label: t('files.uploadFromDevice'), icon: '⇪', run: () => fileInput?.click() },
      { label: t('common.refresh'), icon: '↻', run: () => { load(); } },
    ] };
  }
  function entrySheet(e: Entry, ev?: MouseEvent) {
    const items: SheetItem[] = [{ label: t(e.dir ? 'common.open' : e.text ? 'files.openInEditor' : isImage(e.name) ? 'files.viewImage' : 'common.open'), icon: glyph(e), run: () => open(e) }];
    if (!e.dir) items.push({ label: t('files.download'), icon: '⤓', run: () => download(e) });
    items.push({ label: t('files.rename'), icon: '✎', run: () => rename(e) });
    items.push({ label: t('common.delete'), icon: '✕', danger: true, run: () => del(e) });
    sheet = { title: e.name, items, x: ev?.clientX, y: ev?.clientY };
  }
  function runItem(it: SheetItem) { sheet = null; setTimeout(it.run, 30); }
  function popStyle(s: { x?: number; y?: number }) {
    if (device.phone || s.x === undefined || s.y === undefined) return `bottom: ${NAV_H}px`;
    const w = 260, h = 240; const x = Math.min(s.x, innerWidth - w - 8), y = Math.min(s.y, innerHeight - h - 8);
    return `left:${x}px; top:${y}px; right:auto; bottom:auto; width:${w}px`;
  }
</script>

<div class="files" class:phone={device.phone} class:withside={!device.phone} role="region" aria-label={t('files.region')} ondragover={(e) => e.preventDefault()} ondrop={onDrop}>
  {#if !device.phone}
    <aside class="side">
      <div class="muted h">{t('files.roots')}</div>
      {#each roots as r (r.id)}
        <button class="sroot" class:active={root === r.id} onclick={() => { root = r.id; path = ''; }} title={r.title}><span class="emoji">{rootIcon(r.id)}</span><span class="stitle2">{r.title.replace(/\s*\(.*\)$/, '').replace(/ · .*$/, '')}</span></button>
      {/each}
    </aside>
  {/if}
  <div class="main">
  {#if device.phone}
    <div class="bar mbar">
      <Dropdown bind:value={root} options={roots.map(r => ({ value: r.id, label: r.title }))} ariaLabel={t('files.rootAria')} onchange={() => path = ''} style="flex: 1; min-width: 0;" />
      <button class="more" title={t('files.folderActions')} aria-label={t('files.folderActions')} onclick={() => folderSheet()}>⋯</button>
    </div>
    <div class="bar mnav">
      <button class="up" disabled={!path} title={t('files.up')} onclick={up}>‹</button>
      <nav class="crumbs">
        <button onclick={() => path = ''}>/</button>
        {#each crumbs as c, i}<span class="muted">›</span><button onclick={() => path = crumbs.slice(0, i + 1).join('/')}>{c}</button>{/each}
      </nav>
      <div class="view" role="group" aria-label={t('files.viewAria')}><button class:primary={view === 'list'} title={t('files.list')} onclick={() => view = 'list'}>☰</button><button class:primary={view === 'grid'} title={t('files.grid')} onclick={() => view = 'grid'}>▦</button></div>
      {#if busy}<span class="muted small nowrap">{busy}…</span>{/if}
    </div>
  {:else}
    <div class="bar">
      <button class="up" disabled={!path} title={t('files.up')} onclick={up}>‹</button>
      <nav class="crumbs">
        <button onclick={() => path = ''}>{rootIcon(root)} {roots.find(r => r.id === root)?.title.replace(/\s*\(.*\)$/, '').replace(/ · .*$/, '') ?? '/'}</button>
        {#each crumbs as c, i}<span class="muted">›</span><button onclick={() => path = crumbs.slice(0, i + 1).join('/')}>{c}</button>{/each}
      </nav>
      <span style="flex:1"></span>
      {#if busy}<span class="muted small nowrap">{busy}…</span>{/if}
      <button onclick={newFile} title={t('files.newFileTitle')}>{t('files.newFile')}</button>
      <button onclick={newDir} title={t('files.newDirTitle')}>{t('files.newDir')}</button>
      <button onclick={() => fileInput?.click()} title={t('files.uploadTitle')}>{t('files.uploadBtn')}</button>
      <div class="view" role="group" aria-label={t('files.viewAria')}><button class:primary={view === 'list'} title={t('files.list')} onclick={() => view = 'list'}>☰</button><button class:primary={view === 'grid'} title={t('files.grid')} onclick={() => view = 'grid'}>▦</button></div>
      <Dropdown bind:value={sortBy} ariaLabel={t('files.sortAria')} options={[{ value: 'name', label: t('files.sortName') }, { value: 'date', label: t('files.sortDate') }, { value: 'size', label: t('files.sortSize') }]} style="min-width: 9em;" />
    </div>
  {/if}
  <input bind:this={fileInput} type="file" multiple hidden onchange={(e) => upload((e.currentTarget as HTMLInputElement).files)} />
  {#if error}<div class="card err">{error}</div>{/if}

  <div class="split" class:with-editor={!device.phone && (!!editing || !!preview)}>
    {#if view === 'grid'}
      <div class="grid" role="list">
        {#each sorted as e (e.name)}
          <div class="row-entry tile" class:sel={selected === e.name} role="listitem">
            <button class="link topen" onclick={() => open(e)} title={e.name}>
              <span class="ticon">{#if e.dir}<span class="fold"></span>{:else}<span class="fglyph">{glyph(e)}</span>{/if}</span>
              <span class="tname">{e.name}</span>
              <span class="muted small">{e.dir ? t('files.folder') : fmt.bytes(e.size)}</span>
            </button>
            <button class="rowmenu tmenu" title={t('files.actions')} aria-label={t('files.actionsWith', { name: e.name })} onclick={(ev) => entrySheet(e, ev)}>⋯</button>
          </div>
        {/each}
        {#if !sorted.length && !error}<div class="muted" style="padding: 12px 6px;">{t('files.empty')}</div>{/if}
      </div>
    {:else if device.phone}
      <div class="mlist" role="list">
        {#each sorted as e (e.name)}
          <div class="row-entry mrow" role="listitem">
            <button class="link mopen" onclick={() => open(e)}>
              <span class="mi">{glyph(e)}</span>
              <span class="mtext"><span class="mname">{e.name}</span><span class="muted small">{e.dir ? t('files.folder') : fmt.bytes(e.size)}{#if e.modified}{' · '}{when(e.modified)}{/if}</span></span>
            </button>
            <button class="rowmenu" title={t('files.actions')} aria-label={t('files.actionsWith', { name: e.name })} onclick={() => entrySheet(e)}>⋯</button>
          </div>
        {/each}
        {#if !sorted.length && !error}<div class="muted" style="padding: 12px 6px;">{t('files.empty')}</div>{/if}
      </div>
    {:else}
      <table class="list">
        <thead><tr><th>{t('files.colName')}</th><th class="num">{t('files.colSize')}</th><th>{t('files.colModified')}</th><th></th></tr></thead>
        <tbody>
          {#if path}<tr class="row-entry" ondblclick={up}><td colspan="4"><button class="link" onclick={up}>📁 ..</button></td></tr>{/if}
          {#each sorted as e (e.name)}
            <tr class="row-entry" class:sel={selected === e.name} onclick={() => selected = e.name} ondblclick={() => open(e)}>
              <td><button class="link" onclick={() => open(e)}>{glyph(e)} {e.name}</button></td>
              <td class="num muted">{e.dir ? '—' : fmt.bytes(e.size)}</td>
              <td class="muted">{when(e.modified)}</td>
              <td class="ops">
                {#if !e.dir}<a href={api.fileReadUrl(root, join(path, e.name))} download={e.name} title={t('files.downloadTitle')}><button>⤓</button></a>{/if}
                <button title={t('files.renameTitle')} onclick={() => rename(e)}>✎</button>
                <button title={t('files.deleteTitle')} class="danger" onclick={() => del(e)}>✕</button>
              </td>
            </tr>
          {/each}
          {#if !sorted.length && !error}<tr><td colspan="4" class="muted">{t('files.empty')}</td></tr>{/if}
        </tbody>
      </table>
    {/if}
    {#if (editing || preview) && !device.phone}
      <div class="editor">
        {#if editing}
          <div class="row" style="justify-content: space-between;">
            <b>{editing.name}</b>
            <span class="row">
              <span class="muted">{editing.dirty ? t('files.unsaved') : t('common.saved')}</span>
              <button class="primary" disabled={!editing.dirty || !!busy} onclick={save}>{t('common.save')}</button>
              <button onclick={() => editing = null}>{t('common.close')}</button>
            </span>
          </div>
          <textarea spellcheck="false" bind:value={editing.content} oninput={() => { if (editing) editing.dirty = true; }} onkeydown={(e) => { if ((e.ctrlKey || e.metaKey) && e.key === 's') { e.preventDefault(); save(); } }}></textarea>
        {:else if preview}
          <div class="row" style="justify-content: space-between;"><b>{t('files.preview')}</b><button onclick={() => preview = null}>{t('common.close')}</button></div><img src={preview} alt="" style="max-width: 100%; border-radius: var(--r-sm);" />
        {/if}
      </div>
    {/if}
  </div>
  {#if (editing || preview) && device.phone}
    <div class="editor mfull" transition:fly={{ y: 24, duration: 160 }}>
      {#if editing}
        <div class="row mebar">
          <b class="mname">{editing.name}</b>
          <span class="muted small">{editing.dirty ? t('files.unsaved') : t('common.saved')}</span>
          <button class="primary" disabled={!editing.dirty || !!busy} onclick={save}>{t('common.save')}</button>
          <button onclick={() => editing = null}>{t('common.close')}</button>
        </div>
        <textarea spellcheck="false" autocapitalize="off" bind:value={editing.content} oninput={() => { if (editing) editing.dirty = true; }} onkeydown={(e) => { if ((e.ctrlKey || e.metaKey) && e.key === 's') { e.preventDefault(); save(); } }}></textarea>
      {:else if preview}
        <div class="row mebar"><b>{t('files.preview')}</b><span style="flex:1"></span><button onclick={() => preview = null}>{t('common.close')}</button></div>
        <img src={preview} alt="" class="mprev" />
      {/if}
    </div>
  {/if}
  </div>
  {#if sheet}
    <div class="backdrop" class:clear={!device.phone} role="presentation" transition:fade={{ duration: 100 }} onclick={() => sheet = null}></div>
    <div class="sheet" class:pop={!device.phone} role="menu" aria-label={sheet.title} style={popStyle(sheet)} transition:fly={{ y: device.phone ? 200 : 6, duration: 160 }}>
      <div class="stitle muted">{sheet.title}</div>
      {#each sheet.items as it}<button class="sitem" class:danger={it.danger} role="menuitem" onclick={() => runItem(it)}><span class="si">{it.icon}</span>{it.label}</button>{/each}
      {#if device.phone}<button class="sitem cancel" onclick={() => sheet = null}>{t('common.cancel')}</button>{/if}
    </div>
  {/if}
</div>

<style>
  .files { display: flex; flex-direction: column; gap: 8px; height: 100%; position: relative; container-type: inline-size; }
  .files.withside { display: grid; grid-template-columns: 200px 1fr; gap: 12px; }
  .main { display: flex; flex-direction: column; gap: 8px; min-width: 0; min-height: 0; }
  .side { display: flex; flex-direction: column; gap: 2px; border-right: 1px solid var(--line); padding-right: 8px; min-height: 0; overflow: auto; }
  .side .h { font-size: .75rem; letter-spacing: .06em; text-transform: uppercase; padding: 4px 8px; }
  .sroot { display: flex; align-items: center; gap: 8px; justify-content: flex-start; background: transparent; border-color: transparent; text-align: left; padding: 7px 10px; width: 100%; }
  .sroot.active { background: rgb(var(--accent-rgb) / .14); border-color: rgb(var(--accent-rgb) / .35); }
  .stitle2 { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  @container (max-width: 720px) { .files.withside { grid-template-columns: 1fr; } .side { display: none; } }
  .bar { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
  .crumbs { display: flex; gap: 2px; align-items: center; overflow-x: auto; }
  .crumbs button, .link { background: transparent; border-color: transparent; padding: 2px 6px; }
  .link { text-align: left; }
  .err { border-color: var(--bad); }
  .small { font-size: .8rem; }
  .nowrap { white-space: nowrap; }
  .view { display: inline-flex; gap: 0; } .view button { padding: 0 10px; min-height: 32px; } .view button:first-child { border-radius: var(--r-sm) 0 0 var(--r-sm); } .view button:last-child { border-radius: 0 var(--r-sm) var(--r-sm) 0; }
  .up { width: 34px; height: 34px; padding: 0; font-size: 1.2rem; line-height: 1; flex: none; }
  /* десктоп: таблица/плитки и редактор рядом */
  .split { display: grid; grid-template-columns: 1fr; gap: 10px; flex: 1; min-height: 0; }
  .split.with-editor { grid-template-columns: minmax(280px, 1fr) 2fr; }
  .list { width: 100%; border-collapse: collapse; font-size: .9rem; align-self: start; }
  .list th { text-align: left; color: var(--muted); font-weight: 500; font-size: .8rem; padding: 4px 6px; border-bottom: 1px solid var(--line); }
  .list td { padding: 2px 6px; border-bottom: 1px solid color-mix(in srgb, var(--line) 50%, transparent); white-space: nowrap; }
  .list td:first-child { white-space: normal; overflow-wrap: anywhere; }
  .num { text-align: right; }
  .ops { text-align: right; } .ops button { padding: 0 6px; font-size: .8rem; }
  .row-entry.sel { background: rgb(var(--accent-rgb) / .1); }
  .editor { display: flex; flex-direction: column; gap: 6px; min-height: 0; }
  .editor textarea { flex: 1; min-height: 50vh; width: 100%; font: .85rem/1.45 ui-monospace, Menlo, Consolas, monospace; background: var(--panel-2); color: var(--ink); border: 1px solid var(--line); border-radius: var(--r-sm); padding: 8px; resize: vertical; }
  .split.with-editor .list th:nth-child(3), .split.with-editor .list td:nth-child(3) { display: none; } /* рядом с редактором таблица узкая — без столбца даты */
  .split.with-editor .list td { white-space: normal; }
  @media (max-width: 900px) { .split.with-editor { grid-template-columns: 1fr; } .list th:nth-child(3), .list td:nth-child(3) { display: none; } }
  /* плитки */
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(112px, 1fr)); gap: 6px; align-content: start; overflow: auto; }
  .tile { position: relative; border-radius: var(--r); }
  .tile.sel, .tile:hover { background: rgb(var(--accent-rgb) / .08); }
  .topen { display: flex; flex-direction: column; align-items: center; gap: 4px; width: 100%; padding: 10px 6px 8px; text-align: center; }
  .ticon { height: 56px; display: grid; place-items: center; }
  .fglyph { font-size: 42px; line-height: 1; }
  .fold { position: relative; width: 58px; height: 44px; border-radius: 6px 10px 8px 8px; background: linear-gradient(180deg, color-mix(in srgb, var(--accent) 80%, #fff) 0%, var(--accent) 100%); box-shadow: inset 0 -10px 0 rgb(0 0 0 / .12); margin-top: 8px; }
  .fold::before { content: ""; position: absolute; left: 0; top: -7px; width: 24px; height: 10px; border-radius: 4px 6px 0 0; background: color-mix(in srgb, var(--accent) 85%, #000); }
  .tname { font-size: .85rem; line-height: 1.25; overflow-wrap: anywhere; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .tmenu { position: absolute; top: 2px; right: 2px; width: 28px; height: 28px; padding: 0; font-size: 1rem; background: transparent; border-color: transparent; opacity: 0; }
  .tile:hover .tmenu, .tile.sel .tmenu, .phone .tmenu { opacity: 1; }
  .phone .grid { grid-template-columns: repeat(3, 1fr); }
  .phone .tmenu { width: 36px; height: 36px; }
  /* телефон */
  .mbar { flex-wrap: nowrap; }
  .mnav { flex-wrap: nowrap; }
  .mnav .crumbs { flex: 1; min-width: 0; white-space: nowrap; scrollbar-width: none; }
  .mnav .crumbs button { white-space: nowrap; }
  .phone .up { width: 40px; height: 40px; font-size: 1.3rem; }
  .more { width: 44px; height: 44px; padding: 0; font-size: 1.3rem; flex: none; }
  .mlist { display: flex; flex-direction: column; flex: 1; min-height: 0; overflow: auto; }
  .mrow { display: flex; align-items: center; gap: 4px; border-bottom: 1px solid color-mix(in srgb, var(--line) 60%, transparent); }
  .mopen { flex: 1; min-width: 0; display: flex; align-items: center; gap: 10px; padding: 10px 6px; min-height: 52px; }
  .mi { font-size: 1.4rem; flex: none; }
  .mtext { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .mname { overflow-wrap: anywhere; }
  .rowmenu { width: 44px; height: 44px; padding: 0; font-size: 1.3rem; flex: none; background: transparent; border-color: transparent; }
  .mfull { position: absolute; inset: 0; background: var(--bg); z-index: 3; padding-top: 4px; }
  .mebar { flex-wrap: wrap; gap: 8px; }
  .mebar .mname { flex: 1 1 100%; }
  .mfull textarea { min-height: 0; font-size: 16px; line-height: 1.4; } /* 16px — иначе iOS увеличивает страницу при фокусе */
  .mprev { max-width: 100%; max-height: 80%; object-fit: contain; border-radius: var(--r-sm); }
  .backdrop { position: fixed; inset: 0; background: rgb(0 0 0 / .45); z-index: 4000; }
  .backdrop.clear { background: transparent; }
  .sheet { position: fixed; left: 8px; right: 8px; z-index: 4001; background: var(--panel); border: 1px solid var(--line); border-radius: var(--r); box-shadow: var(--shadow); padding: 6px; display: flex; flex-direction: column; gap: 2px; max-height: 70vh; overflow: auto; }
  .sheet.pop { padding: 4px; }
  .stitle { padding: 8px 12px 6px; font-size: .85rem; overflow-wrap: anywhere; }
  .pop .stitle { padding: 4px 10px; font-size: .75rem; }
  .sitem { display: flex; align-items: center; gap: 12px; justify-content: flex-start; width: 100%; min-height: 48px; padding: 0 14px; background: transparent; border-color: transparent; font-size: 1rem; text-align: left; }
  .pop .sitem { min-height: 34px; font-size: .9rem; padding: 0 10px; }
  .sitem:hover { background: var(--panel-2); }
  .si { width: 24px; text-align: center; }
  .cancel { justify-content: center; border-top: 1px solid var(--line); border-radius: 0; margin-top: 4px; color: var(--muted); }
</style>
