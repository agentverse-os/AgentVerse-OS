<script lang="ts">
  // Удаление приложения с выбором: оставить данные и пароли (переустановка вернёт всё) или удалить полностью.
  import { api, type AppView } from './api';
  import { titleOf } from './store-utils';
  import { fade } from 'svelte/transition';
  import { toast } from './toast.svelte';
  import { t } from './i18n.svelte';
  let { app, onclose, ondone, onstart }: { app: AppView; onclose: () => void; ondone: (a: AppView) => void; onstart?: (a: AppView, label: string) => void } = $props();
  let stage = $state('');
  let mode = $state<'keep' | 'purge'>('keep');
  let confirmName = $state('');
  let busy = $state(false);
  let err = $state('');
  async function go() {
    busy = true; err = '';
    onstart?.(app, t('store.removing')); stage = t('store.removing');
    try {
      await toast.track(t('store.removingTitle', { name: titleOf(app) }), progress => api.removeWait(app.name, mode === 'purge', st => { stage = st; progress(st); }), () => ({ text: mode === 'purge' ? t('store.removedPurged') : t('store.removedKept') }));
      ondone(app);
    } catch (e) { err = e instanceof Error ? e.message : String(e); } finally { busy = false; }
  }
</script>

<div class="overlay" role="dialog" aria-label={t('store.removeDialog')} tabindex="-1" transition:fade={{ duration: 120 }} onkeydown={(e) => e.key === 'Escape' && onclose()}>
  <div class="dlg">
    <h3>{t('store.removeTitle', { name: titleOf(app) })}</h3>
    <label class="opt" class:sel={mode === 'keep'}><input type="radio" bind:group={mode} value="keep" />
      <span><b>{t('store.keepData')}</b><span class="muted small">{t('store.keepDataHint')}</span></span></label>
    <label class="opt" class:sel={mode === 'purge'}><input type="radio" bind:group={mode} value="purge" />
      <span><b>{t('store.purge')}</b><span class="muted small">{t('store.purgeHint')}</span></span></label>
    {#if mode === 'purge'}<input class="confirm" placeholder={t('store.confirmPlaceholder')} autocomplete="off" bind:value={confirmName} />{/if}
    {#if err}<div class="small" style="color: var(--bad)">{err}</div>{/if}
    <div class="row">
      <button class="danger" disabled={busy || (mode === 'purge' && confirmName.trim().toLowerCase() !== 'delete')} onclick={go}>{#if busy}<span class="spin"></span> {stage || t('store.removing')}{:else if mode === 'purge'}{t('store.purge')}{:else}{t('store.remove')}{/if}</button>
      <button onclick={onclose} disabled={busy}>{t('common.cancel')}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 6; background: rgb(0 0 0 / .45); display: grid; place-items: center; padding: 12px; }
  .dlg { background: var(--panel); border: 1px solid var(--line); border-radius: var(--r); box-shadow: var(--shadow); padding: 16px; width: min(520px, 100%); display: flex; flex-direction: column; gap: 10px; }
  .opt { display: flex; gap: 10px; align-items: flex-start; padding: 10px; border: 1px solid var(--line); border-radius: var(--r-sm); cursor: pointer; }
  .opt.sel { border-color: var(--accent); }
  .opt > span { display: flex; flex-direction: column; gap: 4px; }
  .small { font-size: .8rem; }
  .confirm { width: 100%; }
  h3 { margin: 0; }
</style>
