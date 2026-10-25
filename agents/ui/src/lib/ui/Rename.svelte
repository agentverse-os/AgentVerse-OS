<!-- Own session name (docs/agents-ux.md §4.8): a small dialog above everything, shared by desktop and phone -->
<script lang="ts">
  import { ui, data, actions, titleOf, autoTitleOf, type Ref } from '../state.svelte';
  import type { SessionInfo } from '../api';
  import { t } from '../i18n';
  import { untrack } from 'svelte';
  import { dialog } from './dialog';
  import Mark from '../Mark.svelte';
  let { r, onclose }: { r: Ref; onclose: () => void } = $props();
  const info = $derived(data.infoOf(r) as SessionInfo | null);
  // the initial value is deliberately taken once: the dialog lives for one
  // session
  let value = $state(untrack(() => titleOf(r)));
  let busy = $state(false);
  let input = $state<HTMLInputElement | null>(null);
  $effect(() => { const el = input; if (el) queueMicrotask(() => { el.focus(); el.select(); }); });
  async function save(label: string) {
    if (busy) return; busy = true;
    try { await actions.setLabel(r, label); onclose(); } finally { busy = false; }
  }
</script>

<div class="rename-scrim" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="rename-dlg" use:dialog={onclose} role="dialog" aria-modal="true" aria-labelledby="rename-title" tabindex="-1">
  <form onsubmit={(e) => { e.preventDefault(); save(value); }}>
    <h3 id="rename-title"><Mark id={r.id} /> {t('rename.title')}</h3>
    <input bind:this={input} class="rename-input" type="text" maxlength="80" bind:value aria-label={t('rename.title')} placeholder={autoTitleOf(r)} autocomplete="off" disabled={busy} />
    <p class="muted sm rename-auto">{t('rename.auto', { title: autoTitleOf(r) })}</p>
    <p class="muted sm">{t('rename.hint')}</p>
    <div class="rename-acts">
      {#if info?.label}<button type="button" class="sm ghost rename-reset" disabled={busy} onclick={() => save('')}>{t('rename.reset')}</button>{/if}
      <span class="sp"></span>
      <button type="button" class="sm" disabled={busy} onclick={onclose}>{t('rename.cancel')}</button>
      <button type="submit" class="sm primary rename-save" disabled={busy}>{busy ? t('sx.saving') : t('common.save')}</button>
    </div>
  </form>
  </div>
</div>
