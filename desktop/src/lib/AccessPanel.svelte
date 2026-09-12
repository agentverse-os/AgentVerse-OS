<script lang="ts">
  // Блок «Данные для входа»: адрес, логин, пароль с показом и копированием — в карточке, в окне «Готово», по 🔑 в заголовке окна
  // и в сводном разделе «Пароли и доступы». manage — с кнопками «сменить» (сгенерированные секреты) и заметкой.
  import { api, type AppView } from './api';
  import { accessOf, modeText, type AccessField } from './access';
  import { t } from './i18n.svelte';
  let { app, compact = false, manage = false, onchange }: { app: AppView; compact?: boolean; manage?: boolean; onchange?: (v: AppView) => void } = $props();
  let acc = $derived(accessOf(app));
  let shown = $state<Record<string, string>>({});
  let visible = $state<Record<string, boolean>>({});
  let copied = $state('');
  let err = $state('');
  let rotating = $state('');
  let noteOpen = $state(false);
  let noteText = $state('');
  let noteLoaded = $state(false);
  let noteSaving = $state(false);
  const key = (f: AccessField) => f.env ?? f.label;
  const generated = (f: AccessField) => !!f.env && typeof (app.manifest.env ?? {})[f.env] === 'object';
  async function value(f: AccessField): Promise<string> {
    if (!f.secret) return f.value ?? '';
    if (f.env) { if (shown[f.env] === undefined) shown[f.env] = (await api.reveal(app.name, f.env)).value; return shown[f.env]; }
    return f.value ?? '';
  }
  async function toggle(f: AccessField) { try { await value(f); visible[key(f)] = !visible[key(f)]; err = ''; } catch (e) { err = String(e); } }
  async function copy(f: AccessField) { try { await navigator.clipboard?.writeText(await value(f)); copied = f.label; setTimeout(() => copied = '', 1500); } catch (e) { err = String(e); } }
  async function copyAll() {
    try {
      const parts = [app.manifest.title ?? app.name];
      if (acc.url) parts.push(`${t('access.url')}: ${acc.url}`);
      for (const f of [acc.user, acc.password]) if (f) parts.push(`${f.label}: ${await value(f)}`);
      await navigator.clipboard?.writeText(parts.join('\n')); copied = t('access.copiedAll'); setTimeout(() => copied = '', 1500);
    } catch (e) { err = String(e); }
  }
  async function rotate(f: AccessField) {
    if (!f.env) return;
    if (!confirm(t('access.rotateConfirm', { field: f.label, title: app.manifest.title ?? app.name }))) return;
    rotating = f.env; err = '';
    try { const v = await api.rotate(app.name, f.env); delete shown[f.env]; visible[key(f)] = false; onchange?.(v); copied = t('access.rotated', { field: f.label }); setTimeout(() => copied = '', 2500); }
    catch (e) { err = e instanceof Error ? e.message : String(e); } finally { rotating = ''; }
  }
  async function openNote() {
    if (!noteLoaded) { try { noteText = (await api.note(app.name)).text; noteLoaded = true; } catch (e) { err = String(e); return; } }
    noteOpen = !noteOpen;
  }
  async function saveNote() {
    noteSaving = true; err = '';
    try { await api.setNote(app.name, noteText); noteLoaded = true; onchange?.({ ...app, has_note: noteText.trim().length > 0 }); copied = t('access.noteSaved'); setTimeout(() => copied = '', 1500); if (!noteText.trim()) noteOpen = false; }
    catch (e) { err = String(e); } finally { noteSaving = false; }
  }
  function display(f: AccessField): string {
    if (!f.secret) return f.value ?? '—';
    return visible[key(f)] && shown[f.env ?? ''] !== undefined ? (shown[f.env ?? ''] || t('access.emptyValue')) : '••••••••';
  }
  let unset = $derived(!!acc.password?.env && app.settings_values?.[acc.password.env] === '');
</script>

<div class="access" class:compact>
  {#if acc.url}<div class="row"><span class="k">{t('access.url')}</span><a class="v url" href={acc.url} target="_blank" rel="noopener">{acc.url.replace(/^https?:\/\//, '')}</a><button title={t('access.copyUrl')} onclick={() => { navigator.clipboard?.writeText(acc.url ?? ''); copied = t('access.url'); setTimeout(() => copied = '', 1500); }}>⧉</button></div>{/if}
  {#each [acc.user, acc.password] as f}
    {#if f}
      <div class="row field" data-field={f.label}>
        <span class="k">{f.label}</span><code class="v">{display(f)}</code>
        {#if f.secret}<button title={visible[key(f)] ? t('access.hide') : t('access.show')} onclick={() => toggle(f)}>{visible[key(f)] ? '🙈' : '👁'}</button>{/if}
        <button title={t('access.copyField', { field: f.label })} onclick={() => copy(f)}>⧉</button>
        {#if manage && generated(f)}<button class="rotate" title={t('access.rotateTitle')} disabled={!!rotating} onclick={() => rotate(f)}>{rotating === f.env ? t('access.rotating') : t('access.rotate')}</button>{/if}
      </div>
    {/if}
  {/each}
  {#if unset}<div class="small warn">{t('access.unset')}</div>{/if}
  <div class="small muted">{acc.note ?? modeText(acc.mode)}</div>
  {#if manage || app.has_note}
    <div class="row">
      <button class="notebtn" onclick={openNote}>{app.has_note ? t('access.note') : t('access.addNote')}</button>
      {#if !app.has_note && manage}<span class="small muted">{t('access.noteHint')}</span>{/if}
    </div>
    {#if noteOpen}
      <div class="note">
        <textarea rows="3" bind:value={noteText} readonly={!manage} placeholder={t('access.notePlaceholder')}></textarea>
        {#if manage}<div class="row"><button class="primary" disabled={noteSaving} onclick={saveNote}>{noteSaving ? t('access.saving') : t('access.saveNote')}</button><span class="small muted">{t('access.noteStored')}</span></div>{/if}
      </div>
    {/if}
  {/if}
  {#if acc.user || acc.password}
    <div class="row"><button onclick={copyAll}>{t('access.copyAll')}</button>{#if copied}<span class="small muted">{copied}</span>{/if}</div>
  {:else if copied}<span class="small muted">{copied}</span>{/if}
  {#if err}<div class="small" style="color: var(--bad)">{err}</div>{/if}
</div>

<style>
  .access { display: flex; flex-direction: column; gap: 6px; }
  .k { color: var(--muted); min-width: 52px; font-size: .85rem; }
  .v { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 1rem; background: var(--panel-2); padding: 3px 8px; border-radius: var(--r-sm); word-break: break-all; }
  .v.url { font-size: .85rem; }
  .compact .v { font-size: .9rem; }
  .access button { padding: 0 8px; line-height: 1.7; }
  .rotate { font-size: .8rem; }
  .note { display: flex; flex-direction: column; gap: 6px; }
  .note textarea { width: 100%; font: inherit; font-size: .9rem; background: var(--panel-2); color: var(--ink); border: 1px solid var(--line); border-radius: var(--r-sm); padding: 6px 8px; resize: vertical; }
  .small { font-size: .8rem; }
  .warn { color: var(--warn, #e0a030); }
</style>
