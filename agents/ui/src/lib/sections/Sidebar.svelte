<!-- The right panel as a sidebar (docs/agents-ux.md §4.10): the "Inspector" and "Preview" tabs, the width changes only with the
     "wider/narrower" buttons; on narrow screens — a slide-out panel without width buttons (full screen on the phone) -->
<script lang="ts">
  import { ui, data } from '../state.svelte';
  import { t } from '../i18n';
  import Icon from '../Icon.svelte';
  import Inspector from './Inspector.svelte';
  import Preview from '../Preview.svelte';
  import AgentBrowser from '../AgentBrowser.svelte';
  const project = $derived(ui.sel?.project ?? '');
  const hasPreview = $derived(!!project && !!data.browserOf(project).preview);
  const hasBrowser = $derived(!!project && !!data.browserOf(project).agent);
  const tab = $derived(ui.sideTab === 'preview' && hasPreview ? 'preview' : ui.sideTab === 'browser' && hasBrowser ? 'browser' : 'inspector');
  const widthPx = $derived(ui.sideW || ui.sideDefault());
  const canWider = $derived(typeof window !== 'undefined' && widthPx + 1 < Math.max(ui.sideDefault(), Math.floor(window.innerWidth * 0.6)));
</script>

<div class="i-head side-head">
  <div class="seg side-tabs" role="tablist" aria-label={t('side.title')}>
    <button role="tab" class:active={tab === 'inspector'} aria-selected={tab === 'inspector'} onclick={() => (ui.sideTab = 'inspector')}>{t('insp.title')}</button>
    {#if hasPreview}<button role="tab" class="side-tab-preview" class:active={tab === 'preview'} aria-selected={tab === 'preview'} onclick={() => (ui.sideTab = 'preview')}><Icon name="globe" size="s" /> {t('browser.tab')}</button>{/if}
    {#if hasBrowser}<button role="tab" class="side-tab-browser" class:active={tab === 'browser'} aria-selected={tab === 'browser'} onclick={() => (ui.sideTab = 'browser')}><Icon name="monitor" size="s" /> {t('abrowser.tab')}</button>{/if}
  </div>
  <span class="sp"></span>
  {#if !ui.isNarrow}
    <button class="icon sm ghost side-wider" title={t('side.wider')} aria-label={t('side.wider')} disabled={!canWider} onclick={() => ui.sideWider()}><Icon name="chevronLeft" size="s" /></button>
    <button class="icon sm ghost side-narrower" title={t('side.narrower')} aria-label={t('side.narrower')} disabled={!ui.sideW} onclick={() => ui.sideNarrower()}><Icon name="chevronRight" size="s" /></button>
  {/if}
  <button class="icon sm ghost" aria-label={t('common.close')} onclick={() => (ui.inspector = false)}><Icon name="x" /></button>
</div>
{#if tab === 'preview'}
  <div class="side-preview">{#key project}<Preview {project} />{/key}</div>
{:else if tab === 'browser'}
  <div class="side-preview">{#key project}<AgentBrowser {project} />{/key}</div>
{:else}
  <Inspector />
{/if}

<style>
  .side-head { gap: 6px; }
  .side-tabs button { height: 26px; padding: 0 10px; font-size: 12px; display: inline-flex; align-items: center; gap: 5px; }
  .side-preview { flex: 1; min-height: 0; padding: 0 12px 12px; display: flex; flex-direction: column; }
</style>
