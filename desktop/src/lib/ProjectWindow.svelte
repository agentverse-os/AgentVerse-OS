<script lang="ts">
  import { api, type ProjectView, type AppView } from './api';
  import ProjectCard from './ProjectCard.svelte';
  import { t } from './i18n.svelte';
  let { name, showIps = true }: { name: string; showIps?: boolean } = $props();
  let p = $state<ProjectView | null>(null);
  let caps = $state<string[]>([]);
  let error = $state('');
  async function load() {
    try { const [proj, apps] = await Promise.all([api.projects(), api.apps()]); p = proj.find(x => x.name === name) ?? null; caps = [...new Set(apps.filter(a => a.installed).flatMap(a => a.manifest.provides))]; error = p ? '' : t('project.notFound', { name }); }
    catch (e) { error = e instanceof Error ? e.message : String(e); }
  }
  $effect(() => { load(); const t = setInterval(load, 5000); return () => clearInterval(t); });
</script>

{#if error}<div class="card" style="border-color: var(--bad);">{error}</div>{/if}
{#if p}
  <div class="row" style="align-items: baseline; gap: 12px; margin-bottom: 10px;">
    <h2 style="margin:0; font-size: 1.3rem;">🧩 {p.name}</h2>
    <span class="muted">{t('project.createdAs', { runtime: p.spec.workspace.runtime, agents: p.spec.agents.join(', ') || '—' })}</span>
  </div>
  <ProjectCard {p} {showIps} capabilitiesAvailable={caps} onchange={load} compact />
  <h3 style="margin-top: 16px;">{t('project.howTo')}</h3>
  <ul class="muted" style="margin: 6px 0 0; padding-left: 18px;">
    <li>{@html t('project.help1')}</li>
    <li>{@html t('project.help2')}</li>
    <li>{@html t('project.help3')}</li>
  </ul>
{/if}
