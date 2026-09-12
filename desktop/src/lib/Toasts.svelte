<script lang="ts">
  import { fly } from 'svelte/transition';
  import { toast } from './toast.svelte';
  import { device } from './device.svelte';
  import { t as tr } from './i18n.svelte';
</script>

<div class="toasts" class:phone={device.phone} aria-live="polite">
  {#each toast.list as t (t.id)}
    <div class="toast {t.kind}" transition:fly={{ y: 16, duration: 180 }} role="status">
      <div class="ic">{#if t.kind === 'progress'}<span class="spin"></span>{:else if t.kind === 'ok'}✓{:else if t.kind === 'error'}✕{:else}i{/if}</div>
      <div class="body">
        <div class="title">{t.title}</div>
        {#if t.text}<div class="text">{t.text}</div>{/if}
        {#if t.action}<button class="primary act" onclick={() => { t.action?.run(); toast.dismiss(t.id); }}>{t.action.label}</button>{/if}
      </div>
      <button class="x" title={tr('common.closeTitle')} onclick={() => toast.dismiss(t.id)}>✕</button>
    </div>
  {/each}
</div>

<style>
  .toasts { position: fixed; right: 14px; bottom: 60px; z-index: 5000; display: flex; flex-direction: column; gap: 8px; width: min(380px, calc(100vw - 28px)); pointer-events: none; }
  .toasts.phone { right: 10px; left: 10px; bottom: 70px; width: auto; }
  .toast { pointer-events: auto; display: flex; gap: 10px; align-items: flex-start; background: var(--panel); border: 1px solid var(--line); border-left: 4px solid var(--accent); border-radius: var(--r-sm); box-shadow: var(--shadow); padding: 10px 12px; }
  .toast.ok { border-left-color: var(--ok); }
  .toast.error { border-left-color: var(--bad); }
  .ic { width: 20px; text-align: center; font-weight: 700; flex: 0 0 auto; margin-top: 1px; }
  .ok .ic { color: var(--ok); } .error .ic { color: var(--bad); }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .title { font-weight: 600; }
  .text { font-size: .85rem; color: var(--muted); word-break: break-word; }
  .act { align-self: flex-start; padding: 2px 10px; }
  .x { padding: 0 6px; line-height: 1.5; border: 0; background: transparent; color: var(--muted); }
</style>
