<script lang="ts">
  // Обновления (docs/updates.md): система (канал или загруженный пакет, установка с автооткатом), приложения из каталога
  // (новая версия манифеста, изменившийся compose, плавающие теги образов), компоненты (edge, Coder, Komodo — docker compose pull).
  // Тексты ядра (check_error, notes, note, last_applied, состояния контейнеров, ошибки API) показываются как есть; словарь — i18n/messages/updates.ts.
  import { api, type UpdatesView, type AppUpdateInfo } from './api';
  import { t, tn, fmt } from './i18n.svelte';
  let v = $state<UpdatesView | null>(null);
  let error = $state('');
  let busy = $state('');
  let msg = $state('');
  let channel = $state('');
  let log = $state('');
  let restarting = $state<string | null>(null);
  let fileInput = $state<HTMLInputElement | null>(null);

  async function load() {
    try { v = await api.updates(); if (!busy) channel = v.system.channel_url ?? ''; error = ''; }
    catch (e) { error = e instanceof Error ? e.message : String(e); }
  }
  $effect(() => { load(); const timer = setInterval(() => { if (!busy) load(); }, 15000); return () => clearInterval(timer); });

  async function act(label: string, f: () => Promise<string | void>) {
    busy = label; msg = ''; log = '';
    try { const r = await f(); if (typeof r === 'string') msg = r; }
    catch (e) { msg = t('updates.error', { msg: e instanceof Error ? e.message : String(e) }); }
    finally { busy = ''; await load(); }
  }
  const check = () => act('check', async () => { const s = await api.updatesCheck(); return s.latest ? t(s.available ? 'updates.checkAvailable' : 'updates.checkUpToDate', { version: s.latest.version }) : t('updates.checkNoVersion'); });
  const saveChannel = () => act('channel', async () => { const s = await api.updatesChannel(channel.trim()); return s.channel_url ? (s.check_error ? t('updates.channelSavedError', { error: s.check_error }) : t('updates.channelSaved')) : t('updates.channelOff'); });
  const download = () => act('download', async () => { const p = await api.updatesDownload(); return t('updates.downloaded', { file: p.file, size: fmt.bytes(p.size) }); });
  const upload = () => act('upload', async () => { const f = fileInput?.files?.[0]; if (!f) throw new Error(t('updates.chooseFile')); const p = await api.updatesUpload(f); if (fileInput) fileInput.value = ''; return t('updates.uploaded', { file: p.file, version: p.version }); });
  const apply = (file: string, version: string) => act(`apply:${file}`, async () => {
    if (!confirm(t('updates.applyConfirm', { version }))) return;
    const r = await api.updatesApply(file); restarting = r.to; return t('updates.applied', { version: r.to, dur: fmt.duration(r.restart_in_s) });
  });
  const rollback = () => act('rollback', async () => { if (!confirm(t('updates.rollbackConfirm'))) return; await api.updatesRollback(); restarting = v?.system.prev_version ?? '…'; return t('updates.rollbackStarted'); });
  const delPkg = (file: string) => act(`del:${file}`, async () => { await api.updatesDeletePackage(file); return t('updates.pkgDeleted', { file }); });
  const refreshCatalog = () => act('catalog', async () => { const r = await api.updatesCatalog('all'); return t(r.started ? 'updates.catalogStarted' : 'updates.catalogBusy'); });
  const updateApp = (a: AppUpdateInfo) => act(`app:${a.name}`, async () => { await api.updateApp(a.name); return t('updates.appUpdated', { title: a.title }); });
  const updateAll = () => act('apps', async () => { const list = v?.apps.filter(a => a.has_update) ?? []; const done: string[] = []; for (const a of list) { busy = `app:${a.name}`; await api.updateApp(a.name); done.push(a.title); } return done.length ? t('updates.appsUpdated', { list: done.join(', ') }) : t('updates.nothingToUpdate'); });
  const updateComponent = (project: string, title: string) => act(`comp:${project}`, async () => {
    if (title.startsWith('Edge') && !confirm(t('updates.edgeConfirm'))) return;
    const r = await api.updateComponent(project); log = r.log; return r.changed.length ? t('updates.compUpdated', { title, list: r.changed.join(', ') }) : t('updates.compCurrent', { title });
  });
  // Даты ядра — RFC 3339, показываем по локали Desktop; нечитаемое значение — как есть, без секунд и зоны.
  const when = (s?: string | null) => { if (!s) return '—'; const d = new Date(s); return Number.isNaN(d.getTime()) ? s.slice(0, 16).replace('T', ' ') : fmt.dateTime(d, { dateStyle: 'short', timeStyle: 'short' }); };
  const rel = (r: string) => t(r === 'newer' ? 'updates.relNewer' : r === 'same' ? 'updates.relSame' : 'updates.relOlder');
  const toUpdate = $derived(v?.apps.filter(a => a.has_update).length ?? 0);
  const lastImport = $derived(v?.catalog.last_import as { at?: string; ok?: boolean; imported?: number; error?: string } | null | undefined);
</script>

<div class="upd">
  <div class="row" style="justify-content: space-between; align-items: baseline;">
    <h2>{t('sys.updates')}</h2>
    {#if v}<span class="muted">AgentVerse OS {v.system.version}{#if v.system.build !== 'dev'}{' · '}{t('updates.build', { build: v.system.build })}{/if}{' · '}{v.system.arch}</span>{/if}
  </div>
  {#if error}<div class="card err">{error}</div>{/if}
  {#if msg}<div class="card note">{msg}</div>{/if}
  {#if restarting}<div class="card note">{t('updates.restarting', { version: restarting })}</div>{/if}

  {#if v}
    <!-- система -->
    <section class="card">
      <h3>{t('sys.system')}</h3>
      <div class="muted">{t('updates.systemHint1')} <code>stable.json</code>{t('updates.systemHint2')} <code>.prev</code>{t('updates.systemHint3')}</div>
      <div class="row" style="margin-top: 8px;">
        <input class="chan" placeholder={t('updates.channelPlaceholder')} bind:value={channel} />
        <button onclick={saveChannel} disabled={!!busy}>{busy === 'channel' ? t('updates.saving') : t('updates.saveChannel')}</button>
        <button onclick={check} disabled={!!busy || !v.system.channel_url}>{busy === 'check' ? t('updates.checking') : t('updates.check')}</button>
      </div>
      {#if v.system.channel_url}
        <div class="muted" style="margin-top: 6px;">
          {#if v.system.latest}{t('updates.inChannel')} <b>{v.system.latest.version}</b>{#if v.system.latest.published} {t('updates.publishedOn', { date: when(v.system.latest.published) })}{/if}
            {#if v.system.available}<span class="badge ok" style="margin-left: 6px;">{t('updates.available')}</span> <button class="mini" onclick={download} disabled={!!busy || !v.system.latest.url}>{busy === 'download' ? t('updates.downloading') : t('updates.download')}</button>{:else}<span class="badge" style="margin-left: 6px;">{t('updates.current')}</span>{/if}
            {#if v.system.latest.notes}<div class="notes">{v.system.latest.notes}</div>{/if}
          {:else if v.system.check_error}<span class="badge bad">{t('updates.channelUnreachable')}</span> {v.system.check_error}
          {:else}{t('updates.notChecked')}{/if}
          {#if v.system.checked_at}<span class="small"> · {t('updates.checkedAt', { date: when(v.system.checked_at) })}</span>{/if}
        </div>
      {/if}
      <div class="row" style="margin-top: 10px;">
        <input type="file" accept=".tar.gz,.tgz,application/gzip" bind:this={fileInput} />
        <button onclick={upload} disabled={!!busy}>{busy === 'upload' ? t('updates.uploading') : t('updates.upload')}</button>
        <span class="muted small">{t('updates.pkgNameHint', { arch: v.system.arch })}</span>
      </div>
      {#if v.system.packages.length}
        <table class="tbl" style="margin-top: 8px;">
          <thead><tr><th>{t('updates.thPackage')}</th><th>{t('updates.thVersion')}</th><th>{t('updates.thSize')}</th><th>{t('updates.thStoreManifests')}</th><th></th></tr></thead>
          <tbody>
            {#each v.system.packages as p (p.file)}
              <tr>
                <td><code>{p.file}</code><div class="small muted">{t('updates.addedAt', { date: when(p.added_at) })}{#if p.notes} · {p.notes}{/if}</div></td>
                <td>{p.version}{#if p.build} <span class="muted small">{p.build}</span>{/if}<div><span class="badge {p.relation === 'newer' ? 'ok' : p.relation === 'older' ? 'warn' : ''}">{rel(p.relation)}</span>{#if !p.arch_ok}<span class="badge bad">{t('updates.wrongArch', { arch: p.arch ?? '' })}</span>{/if}</div></td>
                <td>{fmt.bytes(p.size)}</td>
                <td>{p.store_apps.length ? p.store_apps.join(', ') : '—'}</td>
                <td class="acts"><button class="primary" disabled={!!busy || !p.arch_ok} onclick={() => apply(p.file, p.version)}>{busy === `apply:${p.file}` ? t('updates.installing') : p.relation === 'newer' ? t('common.install') : t('updates.reinstall')}</button> <button class="danger" disabled={!!busy} onclick={() => delPkg(p.file)}>{t('common.delete')}</button></td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
      <div class="row" style="margin-top: 8px;">
        {#if v.system.can_rollback}<button onclick={rollback} disabled={!!busy}>{t('updates.rollback')}{v.system.prev_version ? ` (${v.system.prev_version})` : ''}</button>{/if}
        {#if v.system.last_applied}<span class="muted small">{t('updates.lastApplied', { info: JSON.stringify(v.system.last_applied) })}</span>{/if}
      </div>
    </section>

    <!-- приложения -->
    <section class="card">
      <div class="row" style="justify-content: space-between;">
        <h3>{t('updates.apps')} {#if toUpdate}<span class="badge ok">{toUpdate}</span>{/if}</h3>
        <div class="row">
          <button onclick={refreshCatalog} disabled={!!busy || v.catalog.importing}>{v.catalog.importing ? t('updates.catalogUpdating') : t('updates.refreshCatalog')}</button>
          <button class="primary" onclick={updateAll} disabled={!!busy || !toUpdate}>{t('updates.updateAll', { n: toUpdate })}</button>
        </div>
      </div>
      <div class="muted">{tn('updates.catalogCount', v.catalog.manifests)}{#if lastImport} · {t('updates.lastImport', { date: when(lastImport.at) })} {lastImport.ok ? t('updates.imported', { n: lastImport.imported ?? 0 }) : t('updates.error', { msg: lastImport.error ?? '' })}{/if}. {t('updates.appsHint')}</div>
      {#if v.apps.length}
        <table class="tbl" style="margin-top: 8px;">
          <thead><tr><th>{t('updates.thApp')}</th><th>{t('updates.thInstalled')}</th><th>{t('updates.thInCatalog')}</th><th>{t('updates.thChanged')}</th><th></th></tr></thead>
          <tbody>
            {#each v.apps as a (a.name)}
              <tr class:hl={a.has_update}>
                <td><b>{a.title}</b> <span class="muted small">{a.name} · {a.origin}</span>{#if a.state && a.state !== 'running'}<div><span class="badge warn">{a.state}</span></div>{/if}</td>
                <td>{a.installed_version ?? '—'}<div class="small muted">{when(a.updated_at ?? a.installed_at)}</div></td>
                <td>{a.in_catalog ? (a.catalog_version ?? '—') : '—'}</td>
                <td class="small">
                  {#if !a.in_catalog}<span class="badge warn">{t('updates.notInCatalog')}</span>
                  {:else}
                    {#if a.version_changed}<span class="badge ok">{t('updates.newVersion')}</span>{/if}
                    {#if a.compose_changed}<span class="badge ok">{t('updates.composeChanged')}</span>{/if}
                    {#if !a.has_update && a.floating_tags.length}<span class="muted">{t('updates.floatingTags', { tags: a.floating_tags.join(', ') })}</span>{/if}
                    {#if !a.has_update && !a.floating_tags.length}<span class="muted">{t('updates.upToDate')}</span>{/if}
                  {/if}
                </td>
                <td class="acts"><button class={a.has_update ? 'primary' : ''} disabled={!!busy || !a.in_catalog} onclick={() => updateApp(a)}>{busy === `app:${a.name}` ? t('updates.updating') : t('updates.update')}</button></td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}<div class="muted" style="margin-top: 8px;">{t('updates.noApps')}</div>{/if}
    </section>

    <!-- компоненты -->
    <section class="card">
      <h3>{t('updates.components')}</h3>
      <div class="muted">{t('updates.componentsHint1')} <code>docker compose pull && up -d</code> {t('updates.componentsHint2')}</div>
      <table class="tbl" style="margin-top: 8px;">
        <thead><tr><th>{t('updates.thComponent')}</th><th>{t('updates.thContainers')}</th><th></th></tr></thead>
        <tbody>
          {#each v.components as c (c.project)}
            <tr>
              <td><b>{c.title}</b><div class="small muted">{c.project}{#if c.note} · {c.note}{/if}</div></td>
              <td class="small">{#each c.containers as k}<div><code>{k.name}</code> {k.image} <span class="muted">{k.state}</span></div>{/each}</td>
              <td class="acts"><button disabled={!!busy || !c.updatable} onclick={() => updateComponent(c.project, c.title)}>{busy === `comp:${c.project}` ? t('updates.updating') : t('updates.pullImages')}</button></td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if log}<pre class="log">{log}</pre>{/if}
    </section>
  {:else if !error}
    <div class="muted">{t('updates.gathering')}</div>
  {/if}
</div>

<style>
  .upd { display: flex; flex-direction: column; gap: 10px; }
  .chan { flex: 1; min-width: 260px; }
  .tbl { width: 100%; border-collapse: collapse; font-size: .9rem; }
  .tbl th { text-align: left; font-weight: 600; color: var(--muted); font-size: .8rem; padding: 4px 8px; border-bottom: 1px solid var(--line); }
  .tbl td { padding: 6px 8px; border-bottom: 1px solid color-mix(in srgb, var(--line) 50%, transparent); vertical-align: top; }
  .tbl tr.hl td { background: color-mix(in srgb, var(--ok) 8%, transparent); }
  .acts { white-space: nowrap; text-align: right; }
  .mini { padding: 1px 8px; font-size: .8rem; }
  .small { font-size: .8rem; }
  .err { border-color: var(--bad); }
  .note { border-color: var(--accent); }
  .notes { white-space: pre-wrap; margin-top: 4px; }
  .log { max-height: 220px; overflow: auto; font-size: .75rem; background: var(--panel-2); padding: 8px; border-radius: var(--r-sm); white-space: pre-wrap; }
</style>
