<script lang="ts">
  // Карточка проекта: используется в списке «Проекты» и в окне отдельного проекта.
  import { api, type ProjectView } from './api';
  import { wm } from './windows.svelte';
  import { t, tn } from './i18n.svelte';
  let { p, showIps = true, capabilitiesAvailable = [], onchange, compact = false }: { p: ProjectView; showIps?: boolean; capabilitiesAvailable?: string[]; onchange?: () => void; compact?: boolean } = $props();
  let busy = $state('');
  let error = $state('');
  let editing = $state(false);
  let edit = $state({ cpu: 2, memory: 4 });
  const ws = $derived(p.workspace);
  async function run(label: string, f: () => Promise<unknown>) {
    busy = label;
    try { await f(); error = ''; } catch (e) { error = e instanceof Error ? e.message : String(e); } finally { busy = ''; onchange?.(); }
  }
  function statusClass(s?: string | null) { return s === 'running' ? 'ok' : s === 'stopped' ? '' : s?.startsWith('fail') || s?.startsWith('unknown') ? 'bad' : 'warn'; }
  function startEdit() { editing = true; edit = { cpu: p.spec.workspace.cpu, memory: Number(String(p.spec.workspace.memory).replace(/GiB$/, '')) }; }
  function terminalUrl() { return ws ? `${ws.url}/terminal` : ''; }
  const caps = $derived([...new Set([...capabilitiesAvailable, ...p.spec.capabilities])].sort());
</script>

<div class="card" class:compact>
  {#if error}<div class="muted" style="color: var(--bad); margin-bottom: 6px;">{error}</div>{/if}
  <div class="row" style="justify-content: space-between;">
    <h3>{compact ? 'Workspace' : p.name}</h3>
    <span class="badge {statusClass(ws?.status)}">{ws?.status ?? t('project.noWorkspace')}</span>
  </div>
  <div class="muted">{p.spec.workspace.runtime} · {p.spec.workspace.cpu} CPU · {p.spec.workspace.memory}{#if showIps} · {p.network} {p.subnet}{/if}</div>
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
    <div class="row" style="margin-top: 8px;">
      <label>CPU <input type="number" min="1" max="32" bind:value={edit.cpu} style="width:5em" /></label>
      <label>RAM GiB <input type="number" min="2" max="128" bind:value={edit.memory} style="width:5em" /></label>
      <button class="primary" disabled={!!busy} onclick={() => run(t('project.applying'), async () => { await api.applyProject({ ...p.spec, workspace: { ...p.spec.workspace, cpu: edit.cpu, memory: edit.memory } }); editing = false; })}>{busy || t('project.apply')}</button>
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
  .caps { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; margin-top: 10px; }
  .caps label { cursor: pointer; }
  .caps input { vertical-align: -1px; }
  .card.compact { background: transparent; border: 0; box-shadow: none; padding: 0; backdrop-filter: none; -webkit-backdrop-filter: none; }
</style>
