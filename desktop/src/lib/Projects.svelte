<script lang="ts">
  import { api, type ProjectView, type AppView, type Runtime } from './api';
  import { wm } from './windows.svelte';
  import ProjectCard from './ProjectCard.svelte';
  import Dropdown from './Dropdown.svelte';
  import { t } from './i18n.svelte';
  let { showIps = true }: { showIps?: boolean } = $props();
  let projects = $state<ProjectView[]>([]);
  let apps = $state<AppView[]>([]);
  let busy = $state<Record<string, string>>({});
  let error = $state('');
  let creating = $state(false);
  let form = $state({ project: '', runtime: 'incus-nesting' as Runtime, cpu: 2, memory: 4, home: 50, capabilities: [] as string[] });

  async function load() {
    try { [projects, apps] = await Promise.all([api.projects(), api.apps()]); error = ''; } catch (e) { error = e instanceof Error ? e.message : String(e); }
  }
  async function run(key: string, label: string, f: () => Promise<unknown>) {
    busy = { ...busy, [key]: label };
    try { await f(); error = ''; } catch (e) { error = e instanceof Error ? e.message : String(e); } finally { const { [key]: _, ...rest } = busy; busy = rest; await load(); }
  }
  $effect(() => { load(); const t = setInterval(load, 5000); return () => clearInterval(t); });

  const capabilitiesAvailable = $derived([...new Set(apps.filter(a => a.installed).flatMap(a => a.manifest.provides))]);
  function create() {
    run('new', t('project.creating'), async () => {
      await api.applyProject({ schema: 1, project: form.project.trim(), workspace: { runtime: form.runtime, cpu: form.cpu, memory: form.memory, home: form.home }, agents: ['claude-code', 'codex'], capabilities: form.capabilities });
      creating = false; form.project = ''; form.capabilities = [];
    });
  }
</script>

<div class="row" style="justify-content: space-between; margin-bottom: 12px;">
  <h2 style="margin:0">{t('sys.projects')}</h2>
  <button class="primary" onclick={() => creating = !creating}>{creating ? t('common.cancel') : t('project.new')}</button>
</div>
{#if error}<div class="card" style="border-color: var(--bad); margin-bottom: 12px;">{error}</div>{/if}

{#if creating}
  <div class="card" style="margin-bottom: 12px;">
    <div class="row">
      <input placeholder={t('project.namePlaceholder')} bind:value={form.project} />
      <Dropdown bind:value={form.runtime} ariaLabel="runtime" options={[{ value: 'incus-nesting', label: t('project.runtimeNesting') }, { value: 'incus', label: t('project.runtimeIncus') }, { value: 'incus-vm', label: t('project.runtimeVm') }]} />
      <label>CPU <input type="number" min="1" max="32" bind:value={form.cpu} style="width:5em" /></label>
      <label>RAM GiB <input type="number" min="2" max="128" bind:value={form.memory} style="width:5em" /></label>
      <label>Home GiB <input type="number" min="0" max="2000" bind:value={form.home} style="width:6em" /></label>
    </div>
    <div class="row" style="margin-top: 8px;">
      <span class="muted">capabilities:</span>
      {#each capabilitiesAvailable as cap}
        <label class="badge"><input type="checkbox" value={cap} bind:group={form.capabilities} /> {cap}</label>
      {/each}
      {#if !capabilitiesAvailable.length}<span class="muted">{t('project.noProviders')}</span>{/if}
      <span style="flex:1"></span>
      <button class="primary" disabled={!form.project || !!busy['new']} onclick={create}>{busy['new'] ?? t('project.create')}</button>
    </div>
    <p class="muted" style="margin: 8px 0 0;">{t('project.createHintBefore')} <code>projects/{form.project || '<name>'}/project.yaml</code>{t('project.createHintAfter')}</p>
  </div>
{/if}

<div class="grid">
  {#each projects as p (p.name)}
    <div class="stack">
      <button class="open" title={t('project.openWindow')} onclick={() => wm.openSys('project', { name: p.name }, p.name)}>🧩 {p.name} ↗</button>
      <ProjectCard {p} {showIps} {capabilitiesAvailable} onchange={load} />
    </div>
  {/each}
  {#if !projects.length}<div class="card muted">{t('project.none')}</div>{/if}
</div>
<style>
  .stack { position: relative; }
  .stack .open { position: absolute; right: 10px; top: -12px; z-index: 1; font-size: .78rem; padding: 2px 8px; background: var(--panel-2); }
</style>
