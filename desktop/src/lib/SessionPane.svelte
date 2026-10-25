<script lang="ts">
  // Session card renderings (ADR-006): Chat | Shell | Files | Git over one workspace session. Shell is a PTY in the session
  // cwd bound to it (`session` on the PTY, removed with the session); Files and Git — the agentd API; the tab is remembered
  // per session. For a "bare" terminal (kind = pty) the tabs are Shell | Files | Git.
  import { api, type PtyInfo } from './api';
  import Chat from './Chat.svelte';
  import Term from './Term.svelte';
  import WsFiles from './WsFiles.svelte';
  import WsGit from './WsGit.svelte';
  import { t } from './i18n.svelte';
  let { project, id, kind, cwd, onchange, compact = false }: { project: string; id: string; kind: 'session' | 'pty'; cwd: string; onchange?: () => void; compact?: boolean } = $props();
  type Tab = 'chat' | 'shell' | 'files' | 'git';
  const tabs: Tab[] = kind === 'session' ? ['chat', 'shell', 'files', 'git'] : ['shell', 'files', 'git'];
  const LS = `cloudos.agents.tab.${project}:${id}`;
  let tab = $state<Tab>(tabs[0]);
  try { const s = localStorage.getItem(LS) as Tab | null; if (s && tabs.includes(s)) tab = s; } catch {}
  $effect(() => { try { localStorage.setItem(LS, tab); } catch {} });
  const label: Record<Tab, () => string> = { chat: () => '💬 ' + t('agents.tabChat'), shell: () => '⌨ ' + t('agents.tabShell'), files: () => '📁 ' + t('agents.tabFiles'), git: () => '⎇ ' + t('agents.tabGit') };
  // session shell: a live PTY with session == id, otherwise created
  // in its cwd
  let shell = $state<PtyInfo | null>(null);
  let shellErr = $state('');
  async function ensureShell(fresh = false) {
    shellErr = '';
    try {
      if (kind === 'pty') { shell = { id } as PtyInfo; return; }
      if (!fresh) { const mine = (await api.ptys(project)).filter(p => p.session === id && p.alive); if (mine.length) { shell = mine[mine.length - 1]; return; } }
      shell = await api.ptyCreate(project, { cwd, session: id, title: `shell · ${id.slice(0, 8)}` });
      onchange?.();
    } catch (e) { shellErr = e instanceof Error ? e.message : String(e); }
  }
  $effect(() => { if (tab === 'shell' && !shell) ensureShell(); });
  async function restartShell() { if (shell && kind === 'session') { try { await api.ptyKill(project, shell.id); } catch {} } shell = null; await ensureShell(true); }
</script>

<div class="pane" class:compact>
  <div class="tabs">
    {#each tabs as x (x)}<button class:active={tab === x} onclick={() => (tab = x)}>{label[x]()}</button>{/each}
    <span class="sp"></span>
    {#if tab === 'shell' && kind === 'session'}<button class="mini" title={t('agents.shellRestartHint')} onclick={restartShell}>↻ {t('agents.shellRestart')}</button>{/if}
  </div>
  <div class="body">
    {#if tab === 'chat'}
      <Chat {project} {id} {onchange} />
    {:else if tab === 'shell'}
      {#if shellErr}<div class="err">{shellErr}</div>
      {:else if shell}{#key shell.id}<Term {project} id={shell.id} onexit={() => { if (kind === 'session') shell = null; onchange?.(); }} />{/key}
      {:else}<div class="muted hint">{t('agents.connecting')}</div>{/if}
    {:else if tab === 'files'}
      <WsFiles {project} root={cwd} />
    {:else}
      <WsGit {project} {cwd} />
    {/if}
  </div>
</div>

<style>
  .pane { display: flex; flex-direction: column; height: 100%; min-height: 0; min-width: 0; }
  .tabs { display: flex; align-items: center; gap: 2px; padding: 0 0 6px; margin-bottom: 6px; border-bottom: 1px solid var(--line); }
  .tabs button { background: transparent; border-color: transparent; color: var(--muted); font-size: 12.5px; padding: 3px 10px; border-radius: 8px; }
  .tabs button.active { background: var(--panel-2); color: var(--ink); }
  .compact .tabs button { padding: 2px 8px; font-size: 12px; }
  .sp { flex: 1; }
  .mini { font-size: 12px; padding: 2px 8px; }
  .body { flex: 1; min-height: 0; display: flex; }
  .body > :global(*) { flex: 1; min-height: 0; min-width: 0; }
  .err { color: var(--bad); font-size: 12.5px; padding: 8px; }
  .muted { color: var(--muted); }
  .hint { font-size: 12.5px; padding: 8px; }
</style>
