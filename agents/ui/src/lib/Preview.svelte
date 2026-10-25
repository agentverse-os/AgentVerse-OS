<!-- Dev server preview (docs/agents-ux.md §4.9, step 1): a port inside the workspace → an edge port route → an iframe with width
     presets. Optional: the tab exists only for workspaces with the preview enabled (Settings → Browser or the "⋯" menu). -->
<script lang="ts">
  import { api, type PreviewRoute } from './api';
  import { data, msg } from './state.svelte';
  import { toast } from './toast.svelte';
  import { t } from './i18n';
  import Icon from './Icon.svelte';
  let { project }: { project: string } = $props();
  const prefs = $derived(data.browserOf(project));
  let port = $state(0);
  let route = $state<PreviewRoute | null>(null);
  let busy = $state(false);
  let error = $state('');
  let size = $state<'phone' | 'tablet' | 'desktop' | 'full'>('full');
  let nonce = $state(0);
  const WIDTHS = { phone: 390, tablet: 768, desktop: 1024, full: 0 } as const;
  const frameUrl = $derived(route ? `${route.url}${nonce ? `?__pv=${nonce}` : ''}` : '');
  $effect(() => { const p = prefs.port || 3000; if (!port) port = p; });
  async function publish() {
    if (busy || !port) return; busy = true; error = '';
    try {
      route = await api.previewAdd(project, port);
      if (prefs.port !== port) await data.setBrowser(project, { port }).catch(() => {});
    } catch (e) { error = msg(e); route = null; }
    finally { busy = false; }
  }
  $effect(() => { if (port && !route && !busy && !error) publish(); });
  async function off() {
    if (busy) return; busy = true;
    try { if (route) await api.previewRemove(project, route.port).catch(() => {}); await data.setBrowser(project, { preview: false }); toast(t('browser.disabledToast', { project })); }
    catch (e) { toast(msg(e), 'bad'); } finally { busy = false; }
  }
</script>

<div class="preview" data-size={size}>
  <div class="bar pv-bar">
    <label class="pv-port"><span class="muted sm">{t('browser.port')}</span><input class="sm mono" type="number" min="1" max="65535" bind:value={port} aria-label={t('browser.port')} onkeydown={(e) => { if (e.key === 'Enter') { route = null; error = ''; publish(); } }} onchange={() => { route = null; error = ''; publish(); }} /></label>
    <button class="xs ghost" title={t('browser.reload')} aria-label={t('browser.reload')} disabled={!route} onclick={() => (nonce = Date.now())}><Icon name="refresh" size="s" /></button>
    <span class="seg pv-sizes" role="group" aria-label={t('browser.sizeFull')}>
      <button class:active={size === 'phone'} onclick={() => (size = 'phone')}>{t('browser.sizePhone')}</button>
      <button class:active={size === 'tablet'} onclick={() => (size = 'tablet')}>{t('browser.sizeTablet')}</button>
      <button class:active={size === 'desktop'} onclick={() => (size = 'desktop')}>{t('browser.sizeDesktop')}</button>
      <button class:active={size === 'full'} onclick={() => (size = 'full')}>{t('browser.sizeFull')}</button>
    </span>
    <span class="sp"></span>
    {#if route}<a class="btn xs ghost pv-open" href={route.url} target="_blank" rel="noopener" title={t('browser.open')}><Icon name="ext" size="s" /> <span class="hide-p">{t('browser.open')}</span></a>{/if}
    <button class="xs ghost pv-off" disabled={busy} onclick={off}><Icon name="x" size="s" /> <span class="hide-p">{t('browser.off')}</span></button>
  </div>
  {#if error}<div class="composer-error pv-error">{t('browser.error', { error })} <button class="xs" onclick={() => { error = ''; publish(); }}>{t('browser.reload')}</button></div>
  {:else if !route}<div class="muted sm pv-note" role="status">{t('browser.publishing', { port })}</div>
  {:else}
    <div class="pv-note muted xs" title={t('browser.hintFrame')}><span class="mono ell">{route.url}</span><span class="pv-bind hide-p"> · {t('browser.hintBind')}</span></div>
    <div class="pv-frame"><iframe src={frameUrl} title="{t('browser.tab')} :{route.port}" style={WIDTHS[size] ? `width: ${WIDTHS[size]}px` : ''} allow="clipboard-read; clipboard-write; microphone; camera" sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-modals allow-downloads"></iframe></div>
  {/if}
</div>

<style>
  .preview { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 6px; }
  .pv-bar { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .pv-port { display: inline-flex; align-items: center; gap: 6px; }
  .pv-port input { width: 86px; }
  .pv-sizes button { height: 24px; padding: 0 8px; font-size: 12px; }
  .pv-note { padding: 0 2px; min-width: 0; display: flex; gap: 4px; flex-wrap: wrap; }
  .pv-bind { white-space: normal; }
  .pv-frame { flex: 1; min-height: 0; overflow: auto; background: var(--bg-3); border: 1px solid var(--line-1); border-radius: var(--r-2); display: flex; justify-content: center; }
  .pv-frame iframe { width: 100%; height: 100%; min-height: 100%; border: 0; background: #fff; flex: none; max-width: 100%; }
  .pv-error { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  @media (max-width: 599px) { .pv-sizes { display: none; } .pv-frame iframe { width: 100% !important; } }
</style>
