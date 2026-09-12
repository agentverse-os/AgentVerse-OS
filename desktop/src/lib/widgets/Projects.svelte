<script lang="ts">
  import { api, type ProjectView } from '../api';
  import { wm } from '../windows.svelte';
  import { t } from '../i18n.svelte';
  let projects = $state<ProjectView[]>([]);
  let busy = $state('');
  async function load() { try { projects = await api.projects(); } catch {} }
  $effect(() => { load(); const t = setInterval(load, 7000); return () => clearInterval(t); });
  async function act(name: string, f: () => Promise<unknown>) { busy = name; try { await f(); } catch {} finally { busy = ''; load(); } }
</script>
<div class="list">
  {#each projects as p (p.name)}
    {@const ws = p.workspace}
    <div class="p">
      <button class="name" onclick={() => wm.openSys('project', { name: p.name }, p.name)}>🧩 {p.name}</button>
      <span class="badge {ws?.status === 'running' ? 'ok' : ws?.status === 'stopped' ? '' : 'warn'}">{ws?.status ?? '—'}</span>
      {#if ws?.status === 'running'}
        {#each ws.apps.slice(0, 1) as a}<button class="mini" onclick={() => wm.open(`ws:${p.name}:${a.slug}`, `${p.name} · ${a.display_name}`, a.url, '🧩')}>VS Code</button>{/each}
        <button class="mini" disabled={busy === p.name} onclick={() => act(p.name, () => api.wsStop(p.name))}>{t('widgets.projects.stop')}</button>
      {:else if ws?.status === 'stopped' || ws?.status === 'failed'}
        <button class="mini primary" disabled={busy === p.name} onclick={() => act(p.name, () => api.wsStart(p.name))}>{t('widgets.projects.start')}</button>
      {/if}
    </div>
  {/each}
  {#if !projects.length}<div class="muted">{t('widgets.projects.none')}</div>{/if}
  <button class="mini" style="align-self:flex-start" onclick={() => wm.openSys('projects')}>{t('widgets.projects.new')}</button>
</div>
<style>
  .list { display: flex; flex-direction: column; gap: 6px; font-size: .9rem; }
  .p { display: flex; gap: 6px; align-items: center; }
  .name { background: transparent; border-color: transparent; padding: 2px 4px; flex: 1; text-align: left; font-weight: 600; }
  .mini { padding: 1px 8px; font-size: .8rem; }
</style>
