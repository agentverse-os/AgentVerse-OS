<script lang="ts">
  // Git workspace'а (ADR-006, рендеринг Git карточки; в этом срезе только чтение): ветка и upstream, изменения индекса,
  // рабочего дерева и неотслеживаемые с дифом по клику, последние коммиты. Обновляется раз в 15 с.
  import { api, type GitStatus, type GitEntry, type GitCommit } from './api';
  import { t, tn } from './i18n.svelte';
  let { project, cwd }: { project: string; cwd: string } = $props();
  let st = $state<GitStatus | null>(null);
  let log = $state<GitCommit[]>([]);
  let diff = $state<{ path: string; text: string; truncated: boolean } | null>(null);
  let error = $state('');
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  async function load() {
    try { st = await api.gitStatus(project, cwd); error = ''; if (st.repo) log = (await api.gitLog(project, cwd)).entries; } catch (e) { error = msg(e); }
  }
  // статус — при реальной смене каталога и раз в 15 с; сам эффект не должен зависеть от пропа `cwd`, который опрос
  // рабочих мест пересоздаёт каждые 5 с (иначе перезагрузка и сброс интервала при каждом опросе)
  let loadedCwd = '';
  $effect(() => { if (cwd !== loadedCwd) { loadedCwd = cwd; load(); } });
  $effect(() => { const i = setInterval(load, 15000); return () => clearInterval(i); });
  async function show(e: GitEntry) {
    try { const d = await api.gitDiff(project, cwd, e.path, e.staged && !e.unstaged, e.kind === 'untracked'); diff = { path: e.path, text: d.diff, truncated: d.truncated }; } catch (er) { error = msg(er); }
  }
  const letter = (e: GitEntry) => (e.kind === 'untracked' ? '?' : e.kind === 'renamed' ? 'R' : e.kind === 'unmerged' ? 'U' : e.y !== '.' ? e.y : e.x);
  const cls = (l: string) => (l === '?' ? 'q' : l === 'A' ? 'a' : l === 'D' ? 'd' : l === 'R' ? 'r' : 'm');
  function fmtTime(ts: number) { return ts ? new Date(ts * 1000).toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' }) : ''; }
</script>

<div class="git">
  {#if error}<div class="err">{error}</div>{/if}
  {#if st && !st.repo}
    <div class="muted hint">{t('agents.gitNoRepo')} <code>{st.cwd}</code></div>
  {:else if st}
    <div class="bar">
      <b>⎇ {st.head || '—'}</b>
      {#if st.upstream}<span class="muted sm">→ {st.upstream}{#if st.ahead || st.behind} · ↑{st.ahead} ↓{st.behind}{/if}</span>{/if}
      <span class="sp"></span>
      <span class="muted sm">{tn('agents.gitChanges', st.entries?.length ?? 0)}</span>
      <button class="mini" title={t('agents.refresh')} onclick={load}>⟳</button>
    </div>
    <div class="split" class:with-diff={!!diff}>
      <div class="list">
        {#each st.entries ?? [] as e (e.path)}
          <button class="row" class:sel={diff?.path === e.path} onclick={() => show(e)}><span class="st {cls(letter(e))}">{letter(e)}</span><span class="nm">{e.path}</span>{#if e.staged}<span class="badge">{t('agents.gitStaged')}</span>{/if}</button>
        {/each}
        {#if !(st.entries ?? []).length}<div class="muted hint">{t('agents.gitClean')}</div>{/if}
        <div class="lh muted">{t('agents.gitRecent')}</div>
        {#each log as c (c.hash)}<div class="commit"><code>{c.short}</code><span class="nm">{c.subject}</span><span class="muted sm">{c.author} · {fmtTime(c.ts)}</span></div>{/each}
      </div>
      {#if diff}
        <div class="diff">
          <div class="ph"><b>{diff.path}</b><span class="sp"></span><button class="ico" onclick={() => (diff = null)}>✕</button></div>
          <pre>{#each diff.text.split('\n') as line, i (i)}<span class="ln" class:add={line.startsWith('+') && !line.startsWith('+++')} class:del={line.startsWith('-') && !line.startsWith('---')} class:hunk={line.startsWith('@@')}>{line + '\n'}</span>{/each}</pre>
          {#if diff.truncated}<div class="muted sm">{t('agents.gitTruncated')}</div>{/if}
        </div>
      {/if}
    </div>
  {:else}
    <div class="muted hint">{t('agents.connecting')}</div>
  {/if}
</div>

<style>
  .git { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 6px; font-size: 13px; }
  .bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .sp { flex: 1; }
  .mini { font-size: 12px; padding: 2px 8px; }
  .err { color: var(--bad); font-size: 12.5px; }
  .split { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(0, 1fr); gap: 8px; }
  .split.with-diff { grid-template-columns: minmax(0, 1fr) minmax(0, 1.4fr); }
  .list { overflow: auto; min-height: 0; border: 1px solid var(--line); border-radius: 10px; padding: 4px; }
  .row { width: 100%; display: flex; align-items: center; gap: 8px; text-align: left; background: transparent; border-color: transparent; padding: 4px 8px; border-radius: 8px; }
  .row:hover { background: var(--panel-2); }
  .row.sel { background: rgb(var(--accent-rgb) / .14); }
  .st { width: 18px; text-align: center; font-family: ui-monospace, monospace; font-weight: 700; font-size: 12px; flex: none; }
  .st.m { color: var(--warn); } .st.a { color: var(--ok); } .st.d { color: var(--bad); } .st.q { color: var(--muted); } .st.r { color: var(--accent); }
  .nm { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .badge { font-size: 10.5px; color: var(--ok); border: 1px solid rgb(63 185 80 / .4); border-radius: 999px; padding: 0 6px; }
  .lh { font-size: 11px; text-transform: uppercase; letter-spacing: .05em; padding: 10px 8px 4px; }
  .commit { display: flex; align-items: center; gap: 8px; padding: 3px 8px; }
  .commit code { font-size: 11.5px; color: var(--muted); }
  .diff { display: flex; flex-direction: column; min-height: 0; border: 1px solid var(--line); border-radius: 10px; padding: 6px 8px; gap: 6px; }
  .ph { display: flex; align-items: center; gap: 8px; }
  .ico { width: 24px; height: 24px; padding: 0; font-size: 12px; background: transparent; border-color: transparent; color: var(--muted); }
  .diff pre { flex: 1; min-height: 0; overflow: auto; margin: 0; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 12px; line-height: 1.4; }
  .ln.add { color: #7ee787; background: rgb(63 185 80 / .12); } .ln.del { color: #ffa198; background: rgb(248 81 73 / .12); } .ln.hunk { color: var(--accent); }
  .muted { color: var(--muted); }
  .sm { font-size: 11.5px; }
  .hint { font-size: 12.5px; padding: 8px; }
  @media (max-width: 700px) { .split.with-diff { grid-template-columns: minmax(0, 1fr); grid-template-rows: minmax(0, 1fr) minmax(0, 1fr); } }
</style>
