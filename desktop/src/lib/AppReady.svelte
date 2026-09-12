<script lang="ts">
  // Экран «Готово, данные для входа» после установки (docs/userflow-apps.md, шаг 2.3).
  import { api, type AppView } from './api';
  import { wm } from './windows.svelte';
  import { iconOf, isImg, titleOf, sourceOf, openUrl } from './store-utils';
  import AccessPanel from './AccessPanel.svelte';
  import { t } from './i18n.svelte';
  let { name }: { name: string } = $props();
  let app = $state<AppView | null>(null);
  let error = $state('');
  $effect(() => { api.app(name).then(a => app = a).catch(e => error = String(e)); });
  function open() { if (!app?.url) return; if (app.manifest.route.open === 'iframe') wm.open(`app:${app.name}`, titleOf(app), openUrl(app), iconOf(app)); else window.open(openUrl(app), '_blank', 'noopener'); }
</script>

<div class="ready">
  {#if error}<div class="card" style="border-color: var(--bad)">{error}</div>{/if}
  {#if app}
    {@const ic = iconOf(app)}
    <div class="hero">
      {#if isImg(ic)}<img src={ic} alt="" />{:else}<span class="emoji">{ic ?? '📦'}</span>{/if}
      <div>
        <h2>{t('app.readyTitle', { title: titleOf(app) })}</h2>
        <div class="muted">{sourceOf(app).label} · {app.state ?? t('app.startingState')}{#if app.port} · {t('app.port', { port: app.port })}{/if}</div>
      </div>
    </div>
    <h3>{t('app.loginDetails')}</h3>
    <AccessPanel {app} />
    <div class="row actions">
      {#if app.url}<button class="primary" onclick={open}>{t('common.open')}{app.manifest.route.open === 'newtab' ? ' ↗' : ''}</button>{/if}
      <button onclick={() => wm.openSys('appdetail', { name }, titleOf(app!))}>{t('app.card')}</button>
      {#if app.manifest.settings.length}<button onclick={() => wm.openSys('appdetail', { name, tab: 'settings' }, titleOf(app!))}>{t('app.tabSettings', { n: app.manifest.settings.length })}</button>{/if}
    </div>
    <p class="muted small">{t('app.readyHint')}</p>
  {/if}
</div>

<style>
  .ready { display: flex; flex-direction: column; gap: 12px; }
  .hero { display: flex; gap: 14px; align-items: center; }
  .hero img, .hero .emoji { width: 56px; height: 56px; border-radius: 14px; object-fit: cover; font-size: 40px; display: grid; place-items: center; background: var(--panel-2); }
  h2, h3 { margin: 0; }
  .small { font-size: .8rem; }
</style>
