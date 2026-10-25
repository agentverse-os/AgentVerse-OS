<script lang="ts">
  // Project card: used in the "Projects" list and in the window of a single
  // project.
  import { api, type ProjectView } from './api';
  import { wm } from './windows.svelte';
  import { t, tn } from './i18n.svelte';
  import Dropdown from './Dropdown.svelte';
  let { p, showIps = true, capabilitiesAvailable = [], onchange, compact = false }: { p: ProjectView; showIps?: boolean; capabilitiesAvailable?: string[]; onchange?: () => void; compact?: boolean } = $props();
  let busy = $state('');
  let error = $state('');
  let editing = $state(false);
  let edit = $state({ cpu: 2, memory: 4, description: '' });
  const ws = $derived(p.workspace);
  async function run(label: string, f: () => Promise<unknown>) {
    busy = label;
    try { await f(); error = ''; } catch (e) { error = e instanceof Error ? e.message : String(e); } finally { busy = ''; onchange?.(); }
  }
  function statusClass(s?: string | null) { return s === 'running' ? 'ok' : s === 'stopped' ? '' : s?.startsWith('fail') || s?.startsWith('unknown') ? 'bad' : 'warn'; }
  function startEdit() { editing = true; edit = { cpu: p.spec.workspace.cpu, memory: Number(String(p.spec.workspace.memory).replace(/GiB$/, '')), description: p.spec.description ?? '' }; }
  function terminalUrl() { return ws ? `${ws.url}/terminal` : ''; }
  const caps = $derived([...new Set([...capabilitiesAvailable, ...p.spec.capabilities])].sort());
</script>

<div class="card" class:compact>
  {#if error}<div class="muted" style="color: var(--bad); margin-bottom: 6px;">{error}</div>{/if}
  <div class="row" style="justify-content: space-between;">
    <h3>{compact ? 'Workspace' : p.name}</h3>
    <span class="badge {statusClass(ws?.status)}">{ws?.status ?? t('project.noWorkspace')}</span>
  </div>
  {#if p.spec.description}<div class="desc" title={p.spec.description}>{p.spec.description}</div>{/if}
  <div class="muted">{p.spec.workspace.runtime} · {p.spec.workspace.cpu} CPU · {p.spec.workspace.memory}{#if showIps} · {p.network} {p.subnet}{/if}</div>
  {#if p.agents?.total}<div class="muted agents"><button class="link" onclick={() => wm.openSys('agents')}>🤖 {t('project.agentsLine', { total: p.agents.total, working: p.agents.working, waiting: p.agents.waiting })}</button>{#if p.agents.waiting}<span class="badge warn">✋ {p.agents.waiting}</span>{/if}</div>{/if}
  <div class="muted">gate {p.gate.name}: <span class="badge {p.gate.status==='running'?'ok':'bad'}">{p.gate.status}</span>{#if showIps} {p.gate.detail}{/if} · {t('project.agent')}: {ws?.agent_status ?? '—'}{#if ws?.ip && showIps} · {ws.ip}{/if}</div>
  <div class="row" style="margin-top: 10px;">
    {#if ws}
      {#if ws.status === 'running'}
        {#each ws.apps as a}<button class="primary" onclick={() => wm.open(`ws:${p.name}:${a.slug}`, `${p.name} · ${a.display_name}`, a.url, '🧩')}>{a.display_name}{a.health !== 'healthy' ? ` (${a.health})` : ''}</button>{/each}
        <button onclick={() => wm.open(`ws:${p.name}:terminal`, t('project.terminalTitle', { name: p.name }), terminalUrl(), '⌨️', { w: 900, h: 560 })}>{t('project.terminal')}</button>
        <button onclick={() => wm.openSys('files', { root: `ws:${p.name}` }, t('project.filesTitle', { name: p.name }))}>{t('sys.files')}</button>
        <a href={ws.url} target="_blank" rel="noopener"><button>Coder ↗</button></a>
        <button disabled={!!busy} onclick={() => run(t('project.stopping'), () => api.wsStop(p.name))}>{busy || t('project.stop')}</button>
      {:else if ws.status === 'stopped' || ws.status === 'failed'}
        <button class="primary" disabled={!!busy} onclick={() => run(t('project.starting'), () => api.wsStart(p.name))}>{busy || t('project.start')}</button>
        <a href={ws.url} target="_blank" rel="noopener"><button>Coder ↗</button></a>
      {:else}
        <span class="muted">{ws.status}…</span>
      {/if}
    {/if}
    <span style="flex:1"></span>
    <button disabled={!!busy} onclick={() => editing ? editing = false : startEdit()}>{editing ? t('common.cancel') : t('project.edit')}</button>
    <button class="danger" disabled={!!busy} onclick={() => { if (confirm(t('project.confirmDelete', { name: p.name }))) run(t('project.deleting'), () => api.deleteProject(p.name)); }}>{t('common.delete')}</button>
  </div>
  {#if editing}
    <textarea class="desc-edit" rows="3" aria-label={t('project.description')} placeholder={t('project.descriptionPlaceholder')} bind:value={edit.description}></textarea>
    <div class="row" style="margin-top: 8px;">
      <label>CPU <input type="number" min="1" max="32" bind:value={edit.cpu} style="width:5em" /></label>
      <label>RAM GiB <input type="number" min="2" max="128" bind:value={edit.memory} style="width:5em" /></label>
      <button class="primary" disabled={!!busy} onclick={() => run(t('project.applying'), async () => { await api.applyProject({ ...p.spec, description: edit.description.trim() || null, workspace: { ...p.spec.workspace, cpu: edit.cpu, memory: edit.memory } }); editing = false; })}>{busy || t('project.apply')}</button>
      <span class="muted">{t('project.editHint')}</span>
    </div>
  {/if}
  <div class="caps">
    <span class="muted">capabilities:</span>
    {#each caps as c}
      {@const g = p.grants.find(g => g.capability === c)}
      {@const missing = p.missing_capabilities.includes(c)}
      <label class="badge {g ? 'ok' : missing ? 'warn' : ''}" title={g ? `http://${c}.gate → ${g.app}` : missing ? t('project.noProvider') : t('project.grantHint')}>
        <input type="checkbox" checked={!!g || missing} disabled={!!busy || missing}
          onchange={(e) => { const on = (e.currentTarget as HTMLInputElement).checked; run(on ? t('project.granting') : t('project.revoking'), () => on ? api.grant(p.name, c) : api.revoke(p.name, c)); }} />
        {c}{#if g} → {g.app}{/if}{#if missing}{' '}{t('project.noProviderBadge')}{/if}
      </label>
      {#if g && (p.providers?.[c]?.length ?? 0) > 1}<Dropdown value={g.app} ariaLabel={t('project.provider')} options={(p.providers?.[c] ?? []).map(a => ({ value: a, label: a }))} onchange={(v) => { if (v !== g.app) run(t('project.granting'), () => api.grant(p.name, c, v)); }} style="min-width: 9em" />{/if}
    {/each}
    {#if !caps.length}<span class="muted">{t('project.installProvider')}</span>{/if}
  </div>
  {#if Object.keys(p.grant_env).length}
    <details class="muted" style="margin-top: 6px;">
      <summary>{t('project.envIn')} ({tn('project.vars', Object.values(p.grant_env).flat().length)}, <code>/etc/cloudos/env.sh</code>)</summary>
      {#each Object.entries(p.grant_env) as [cap, keys]}<div><b>{cap}</b>: {keys.map(k => `$${k}`).join(', ')}</div>{/each}
    </details>
  {/if}
</div>

<style>
  /* the owner's note: in the list — up to three lines (the full text in the tooltip), in the project window — whole */
  .desc { margin-top: 4px; white-space: pre-wrap; overflow-wrap: anywhere; display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .card.compact .desc { display: block; }
  .desc-edit { display: block; width: 100%; box-sizing: border-box; margin-top: 8px; resize: vertical; }
  .caps { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; margin-top: 10px; }
  .caps label { cursor: pointer; }
  .caps input { vertical-align: -1px; }
  .agents { display: flex; gap: 6px; align-items: center; }
  .agents .link { background: none; border: 0; padding: 0; color: inherit; cursor: pointer; font: inherit; text-decoration: underline dotted; }
  .card.compact { background: transparent; border: 0; box-shadow: none; padding: 0; backdrop-filter: none; -webkit-backdrop-filter: none; }
</style>
