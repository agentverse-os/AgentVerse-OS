<script lang="ts">
  // Workspace git (docs/agents-ux.md §4.3.6): index and commit, branches, fetch/pull/push, worktrees with sessions and
  // merging, a commit message suggestion from headless Claude (haiku through the gate). Refreshes every 15 s and after
  // every action.
  import { onDestroy, tick } from 'svelte';
  import { api, type GitStatus, type GitEntry, type GitCommit, type GitBranch, type GitWorktree } from './api';
  import { t, tn } from './i18n';
  import { menu } from './menu.svelte';
  import { toast } from './toast.svelte';
  import { data, ui } from './state.svelte';
  import Icon from './Icon.svelte';
  let { project, cwd }: { project: string; cwd: string } = $props();
  let st = $state<GitStatus | null>(null);
  let log = $state<GitCommit[]>([]);
  let branches = $state<GitBranch[]>([]);
  let remotes = $state<string[]>([]);
  let wts = $state<GitWorktree[]>([]);
  let diff = $state<{ path: string; text: string; truncated: boolean } | null>(null);
  let diffLoading = $state(false); let diffError = $state('');
  let diffEntry: GitEntry | null = null; let diffOrigin: HTMLButtonElement | null = null;
  let loadSeq = 0; let diffSeq = 0; let alive = true;
  let loading = $state(false);
  let error = $state('');
  let message = $state('');
  let busy = $state('');
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  const entries = $derived(st?.entries ?? []);
  const stagedN = $derived(entries.filter(e => e.staged).length);
  /** Project sessions working in a worktree directory — shows who is
   *  where. */
  const sessionsIn = (path: string) => (data.ws?.projects.find(p => p.name === project)?.sessions ?? []).filter(s => s.cwd === path || s.cwd.startsWith(path + '/'));
  const targetKey = () => `${project}:${cwd}`;
  const current = (key: string) => alive && key === targetKey();
  function resetDiff() { ++diffSeq; diff = null; diffError = ''; diffLoading = false; }
  async function closeDiff() { resetDiff(); await tick(); if (diffOrigin?.isConnected) diffOrigin.focus(); }
  async function load() {
    const seq = ++loadSeq; const owner = project; const dir = cwd; const key = targetKey();
    loading = true;
    try {
      const status = await api.gitStatus(owner, dir);
      if (!current(key) || seq !== loadSeq) return;
      if (status.repo) {
        st = status;
        const [l, b, w] = await Promise.allSettled([
          api.gitLog(owner, dir), api.gitBranches(owner, dir), api.gitWorktrees(owner, dir),
        ] as const);
        if (!current(key) || seq !== loadSeq) return;
        if (l.status === 'fulfilled') log = l.value.entries;
        if (b.status === 'fulfilled') { branches = b.value.branches; remotes = b.value.remotes; }
        if (w.status === 'fulfilled') wts = w.value.worktrees;
        error = [l, b, w].filter(r => r.status === 'rejected').map(r => msg((r as PromiseRejectedResult).reason)).join(' · ');
      } else { st = status; log = []; branches = []; remotes = []; wts = []; error = ''; }
    } catch (e) { if (current(key) && seq === loadSeq) error = msg(e); }
    finally { if (current(key) && seq === loadSeq) loading = false; }
  }
  // Retain the current reader on polling, reset only when the actual workspace/directory changes.
  let loadedCwd = '';
  $effect(() => {
    const key = targetKey();
    if (key !== loadedCwd) { loadedCwd = key; st = null; log = []; branches = []; remotes = []; wts = []; message = ''; busy = ''; resetDiff(); load(); }
  });
  $effect(() => { const i = setInterval(load, 15000); return () => clearInterval(i); });
  onDestroy(() => { alive = false; ++loadSeq; ++diffSeq; });
  async function run(what: string, op: string, body: Record<string, unknown>): Promise<any | null> {
    if (busy) return null;
    const owner = project; const dir = cwd; const key = targetKey();
    busy = what;
    try {
      const r = await api.gitOp(owner, op, { cwd: dir, ...body });
      if (!current(key)) return null;
      error = ''; await load(); return r;
    } catch (e) { if (current(key)) { error = msg(e); toast(msg(e), 'bad'); } return null; }
    finally { if (current(key)) busy = ''; }
  }
  async function show(e: GitEntry, origin?: HTMLButtonElement) {
    const seq = ++diffSeq; const owner = project; const dir = cwd; const key = targetKey();
    if (origin) diffOrigin = origin;
    diffEntry = e; diffError = ''; diffLoading = true;
    diff = { path: e.path, text: '', truncated: false };
    try {
      const d = await api.gitDiff(owner, dir, e.path, e.staged && !e.unstaged, e.kind === 'untracked');
      if (current(key) && seq === diffSeq) diff = { path: e.path, text: d.diff, truncated: d.truncated };
    } catch (er) { if (current(key) && seq === diffSeq) diffError = msg(er); }
    finally { if (current(key) && seq === diffSeq) diffLoading = false; }
  }
  const toggle = (e: GitEntry) => (e.staged && !e.unstaged ? run('unstage', 'unstage', { paths: [e.path] }) : run('stage', 'stage', { paths: [e.path] }));
  async function discard(e: GitEntry) {
    if (!confirm(t(e.kind === 'untracked' ? 'git.deleteConfirm' : 'git.discardConfirm', { path: e.path }))) return;
    if (diff?.path === e.path) resetDiff();
    await run('discard', 'discard', { paths: [e.path] });
  }
  async function commit() {
    const draft = message; const m = draft.trim(); if (busy || !m || !stagedN) return;
    const r = await run('commit', 'commit', { message: m });
    if (r) { if (message === draft) message = ''; toast(t('git.committed', { short: r.short })); }
  }
  async function suggest() {
    if (busy) return;
    const owner = project; const dir = cwd; const key = targetKey(); const draft = message;
    busy = 'suggest';
    try {
      const r = await api.gitOp(owner, 'suggest', { cwd: dir });
      if (current(key)) { if (message === draft) message = r.message; error = ''; }
    } catch (e) { if (current(key)) { error = msg(e); toast(msg(e), 'bad'); } }
    finally { if (current(key)) busy = ''; }
  }
  function branchMenu(el: HTMLElement) {
    menu.show(el, [
      { header: true, label: t('git.branches') },
      ...branches.map(b => ({ label: b.name, checked: b.current, hint: b.upstream ?? undefined, on: () => { if (!b.current) run('checkout', 'checkout', { branch: b.name }); } })),
      { sep: true },
      { icon: 'plus', label: t('git.newBranch'), on: () => { const n = prompt(t('git.newBranchPrompt')); if (n?.trim()) run('checkout', 'checkout', { branch: n.trim(), create: true }); } },
    ]);
  }
  async function push() {
    const remote = remotes[0] ?? 'origin';
    if (!confirm(t('git.pushConfirm', { branch: st?.head ?? '', remote }))) return;
    const r = await run('push', 'push', { remote }); if (r) toast(t('git.pushed'));
  }
  async function addWorktree() {
    const n = prompt(t('git.wtPrompt')); if (!n?.trim()) return;
    const r = await run('wt-add', 'worktrees/add', { name: n.trim() }); if (r) toast(t('git.wtCreated', { name: r.name, path: r.path }));
  }
  async function openSession(w: GitWorktree) {
    if (busy) return;
    const owner = project; const key = targetKey();
    busy = 'wt-session';
    try {
      const s = await api.sessionCreate(owner, { agent: 'claude', cwd: w.path, title: `⎇ ${w.name}` });
      await data.loadWs();
      if (current(key)) { ui.open({ project: owner, kind: 'session', id: s.id }); toast(t('toast.created', { ws: owner })); }
    } catch (e) { if (current(key)) { error = msg(e); toast(msg(e), 'bad'); } }
    finally { if (current(key)) busy = ''; }
  }
  function mergeMenu(el: HTMLElement, w: GitWorktree) {
    const go = async (drop: boolean) => {
      if (!confirm(t('git.wtMergeConfirm', { branch: w.branch ?? w.name }))) return;
      const r = await run('wt-merge', 'worktrees/merge', { name: w.name, squash: true, delete_branch: drop, force: false });
      if (r) toast(t('git.wtMerged', { branch: r.branch, short: r.commit?.short ?? '' }));
    };
    menu.show(el, [{ header: true, label: `⎇ ${w.branch ?? w.name} → ${wts[0]?.branch ?? 'main'}` }, { icon: 'git', label: t('git.wtMergeKeep'), on: () => go(false) }, { icon: 'git', label: t('git.wtMergeDrop'), on: () => go(true) }]);
  }
  function removeMenu(el: HTMLElement, w: GitWorktree) {
    const go = async (dropBranch: boolean) => {
      if (!confirm(t('git.wtRemoveConfirm', { name: w.name, changes: w.changes ? t('git.wtRemoveChanges', { n: w.changes }) : '' }))) return;
      const r = await run('wt-remove', 'worktrees/remove', { name: w.name, force: w.changes > 0, delete_branch: dropBranch });
      if (r) toast(t('git.wtRemoved', { name: w.name }));
    };
    menu.show(el, [{ icon: 'trash', label: t('git.wtRemoveKeep'), on: () => go(false) }, { icon: 'trash', label: t('git.wtRemoveDrop'), on: () => go(true), danger: true }]);
  }
  const letter = (e: GitEntry) => (e.kind === 'untracked' ? '?' : e.kind === 'renamed' ? 'R' : e.kind === 'unmerged' ? 'U' : e.y !== '.' ? e.y : e.x);
  const cls = (l: string) => (l === '?' ? 'q' : l === 'A' ? 'a' : l === 'D' ? 'd' : l === 'R' ? 'r' : 'm');
  function fmtTime(ts: number) { return ts ? new Date(ts * 1000).toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' }) : ''; }
  const rel = (w: GitWorktree) => (wts[0] && w.path.startsWith(wts[0].path + '/') ? w.path.slice(wts[0].path.length + 1) : w.path);
</script>

<div class="git" class:busy={!!busy}>
  {#if error}<div class="err" role="alert">{error} <button class="mini" onclick={load}>{t('wsux.retry')}</button></div>{/if}
  {#if st && !st.repo}
    <div class="muted hint">{t('agents.gitNoRepo')} <code>{st.cwd}</code></div>
  {:else if st}
    <div class="gbar">
      <button class="branch" disabled={!!busy} title={t('git.branches')} onclick={(e) => { e.stopPropagation(); branchMenu(e.currentTarget as HTMLElement); }}>⎇ <b>{st.head || '—'}</b> ▾</button>
      {#if st.upstream}<span class="muted sm">→ {st.upstream}{#if st.ahead || st.behind} · ↑{st.ahead} ↓{st.behind}{/if}</span>{/if}
      <span class="sp"></span>
      {#if remotes.length}
        <button class="mini fetch" disabled={!!busy} title={t('git.fetch')} onclick={() => run('fetch', 'fetch', {}).then(r => r && toast(t('git.fetched')))}>↓ {t('git.fetch')}</button>
        <button class="mini pull" disabled={!!busy} title={t('git.pull')} onclick={() => run('pull', 'pull', {}).then(r => r && toast(t('git.pulled')))}>⇣ {t('git.pull')}</button>
        <button class="mini push" disabled={!!busy} title={t('git.push')} onclick={push}>⇡ {t('git.push')}{#if st.ahead} ({st.ahead}){/if}</button>
      {/if}
      <button class="mini" aria-label={t('agents.refresh')} title={t('agents.refresh')} onclick={load}>⟳</button>
    </div>
    <div class="split" class:with-diff={!!diff}>
      <div class="list">
        <div class="lh muted"><span>{tn('agents.gitChanges', entries.length)}</span><span class="sp"></span>{#if entries.length}<button class="lnk stage-all" disabled={!!busy} onclick={() => run('stage', 'stage', { all: true })}>{t('git.stageAll')}</button>{#if stagedN}<button class="lnk unstage-all" disabled={!!busy} onclick={() => run('unstage', 'unstage', { all: true })}>{t('git.unstageAll')}</button>{/if}{/if}</div>
        {#each entries as e (e.path)}
          <div class="row" class:sel={diff?.path === e.path}>
            <label class="stage-box"><input class="ck" type="checkbox" checked={e.staged && !e.unstaged} disabled={!!busy} aria-label={t(e.staged && !e.unstaged ? 'wsux.unstageFile' : 'wsux.stageFile', { path: e.path })} title={e.staged && !e.unstaged ? t('git.unstage') : t('git.stage')} onchange={() => toggle(e)} /></label>
            <button class="nmb" onclick={(ev) => show(e, ev.currentTarget)}><span class="st {cls(letter(e))}">{letter(e)}</span><span class="nm">{e.path}</span>{#if e.staged && e.unstaged}<span class="badge">{t('agents.gitStaged')}</span>{/if}</button>
            <button class="ico disc" aria-label={t(e.kind === 'untracked' ? 'wsux.deleteFile' : 'wsux.discardFile', { name: e.path, path: e.path })} title={e.kind === 'untracked' ? t('git.delete') : t('git.discard')} disabled={!!busy} onclick={() => discard(e)}>{e.kind === 'untracked' ? '🗑' : '↶'}</button>
          </div>
        {/each}
        {#if !entries.length}<div class="muted hint">{t('agents.gitClean')}</div>{/if}
        {#if entries.length}
          <div class="cm">
            <div class="lh muted"><span>{t('git.message')}</span><span class="sp"></span><button class="lnk suggest" disabled={!!busy} title={t('git.suggestHint')} onclick={suggest}>✨ {busy === 'suggest' ? t('git.working') : t('git.suggest')}</button></div>
            <textarea rows="2" aria-label={t('git.message')} placeholder={t('git.messagePh')} bind:value={message} onkeydown={(e) => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); commit(); } }}></textarea>
            <div class="cmr"><span class="muted sm">{stagedN ? '' : t('git.nothingStaged')}</span><span class="sp"></span><button class="primary commit" disabled={!!busy || !stagedN || !message.trim()} onclick={commit}>{tn('git.commitN', stagedN)}</button></div>
          </div>
        {/if}
        <div class="lh muted"><span>{t('git.worktrees')} · {wts.length}</span><span class="sp"></span><button class="lnk wt-add" disabled={!!busy} onclick={addWorktree}>{t('git.wtAdd')}</button></div>
        <div class="wt">
          {#each wts as w (w.path)}
            {@const ses = sessionsIn(w.path)}
            <div class="wrow" class:cur={w.current} data-name={w.name}>
              <span class="wn">{w.main ? '●' : '⎇'} <b>{w.main ? (w.branch ?? w.name) : w.name}</b>{#if w.current}<span class="badge">{t('git.wtCurrent')}</span>{/if}</span>
              <span class="muted sm wp" title={w.path}>{w.main ? w.path : rel(w)}{#if !w.main && w.branch && w.branch !== w.name} · {w.branch}{/if}{#if w.detached} · detached{/if}</span>
              <span class="muted sm">{tn('git.wtSessions', ses.length)}{#if w.changes} · <span class="warn">{tn('git.wtChanges', w.changes)}</span>{/if}</span>
              <span class="wa">
                <button class="mini act-session" disabled={!!busy} title={t('git.wtSessionHint')} onclick={() => openSession(w)}><Icon name="claude" size="s" /> {t('git.wtSession')}</button>
                {#if !w.main}<button class="mini act-merge" disabled={!!busy || !w.branch} onclick={(e) => { e.stopPropagation(); mergeMenu(e.currentTarget as HTMLElement, w); }}>{t('git.wtMerge')} ▾</button><button class="mini act-remove" disabled={!!busy} onclick={(e) => { e.stopPropagation(); removeMenu(e.currentTarget as HTMLElement, w); }}>{t('git.wtRemove')} ▾</button>{/if}
              </span>
            </div>
          {/each}
        </div>
        <div class="lh muted">{t('agents.gitRecent')}</div>
        {#each log as c (c.hash)}<div class="commit"><code>{c.short}</code><span class="nm">{c.subject}</span><span class="muted sm">{c.author} · {fmtTime(c.ts)}</span></div>{/each}
      </div>
      {#if diff}
        <div class="diff">
          <div class="ph"><button class="mini back" onclick={closeDiff}>← {t('wsux.back')}</button><b title={diff.path}>{diff.path}</b><span class="sp"></span><button class="ico close-preview" aria-label={t('wsux.closePreview')} onclick={closeDiff}>✕</button></div>
          {#if diffLoading}<div class="muted hint" role="status">{t('wsux.loading')}</div>
          {:else if diffError}<div class="err" role="alert">{diffError} <button class="mini" onclick={() => diffEntry && show(diffEntry)}>{t('wsux.retry')}</button></div>
          {:else}<pre>{#each diff.text.split('\n') as line, i (i)}<span class="ln" class:add={line.startsWith('+') && !line.startsWith('+++')} class:del={line.startsWith('-') && !line.startsWith('---')} class:hunk={line.startsWith('@@')}>{line}</span>{'\n'}{/each}</pre>{/if}
          {#if diff.truncated}<div class="muted sm">{t('agents.gitTruncated')}</div>{/if}
        </div>
      {/if}
    </div>
  {:else if loading}
    <div class="muted hint" role="status">{t('wsux.loading')}</div>
  {/if}
</div>

<style>
  .git { --diff-add: var(--ok); --diff-del: var(--bad); container: workspace-git / inline-size; min-width: 0; display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 6px; font-size: 13px; }
  :global(:root[data-theme="light"]) .git { --diff-add: #16733e; --diff-del: #b32f2f; }
  .git.busy { cursor: progress; }
  /* not .bar: in app.css that is a progress strip (height 6px, flex 1) — it stretched into a "pancake" over half the tab */
  .gbar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .branch { max-width: 100%; min-width: 0; background: transparent; border: 1px solid var(--line); border-radius: 8px; padding: 3px 10px; font-size: 13px; }
  .branch b { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .gbar > .muted { min-width: 0; max-width: 100%; overflow-wrap: anywhere; }
  .branch:hover { background: var(--panel-2); }
  .sp { flex: 1; }
  .mini { font-size: 12px; padding: 2px 8px; display: inline-flex; align-items: center; gap: 4px; }
  .lnk { background: transparent; border: 0; color: var(--accent); font-size: 11.5px; padding: 0 4px; text-transform: none; letter-spacing: 0; cursor: pointer; }
  .lnk:disabled { opacity: .5; cursor: default; }
  .err { color: var(--bad); font-size: 12.5px; }
  .split { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(0, 1fr); gap: 8px; }
  .split.with-diff { grid-template-columns: minmax(0, 1fr) minmax(0, 1.4fr); }
  .list { overflow: auto; min-height: 0; border: 1px solid var(--line); border-radius: 10px; padding: 4px; }
  .row { display: flex; align-items: center; gap: 6px; padding: 2px 6px; border-radius: 8px; }
  .row:hover { background: var(--panel-2); }
  .row.sel { background: rgb(var(--accent-rgb) / .14); }
  .ck { margin: 0; accent-color: var(--accent); flex: none; }
  .stage-box { display: inline-flex; align-items: center; justify-content: center; flex: none; }
  .nmb { flex: 1; min-width: 0; display: flex; align-items: center; gap: 8px; text-align: left; background: transparent; border: 0; padding: 3px 4px; color: inherit; }
  .st { width: 18px; text-align: center; font-family: ui-monospace, monospace; font-weight: 700; font-size: 12px; flex: none; }
  .st.m { color: var(--warn); } .st.a { color: var(--ok); } .st.d { color: var(--bad); } .st.q { color: var(--muted); } .st.r { color: var(--accent); }
  .nm { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .badge { font-size: 10.5px; color: var(--ok); border: 1px solid rgb(63 185 80 / .4); border-radius: 999px; padding: 0 6px; margin-left: 6px; }
  .lh { flex-wrap: wrap; font-size: 11px; text-transform: uppercase; letter-spacing: .05em; padding: 10px 8px 4px; display: flex; align-items: center; gap: 8px; }
  .cm { padding: 4px 6px 8px; }
  .cm textarea { width: 100%; box-sizing: border-box; resize: vertical; font: inherit; font-size: 12.5px; background: var(--panel-2); color: inherit; border: 1px solid var(--line); border-radius: 8px; padding: 6px 8px; }
  .cmr { flex-wrap: wrap; display: flex; align-items: center; gap: 8px; margin-top: 6px; }
  .commit code { font-size: 11.5px; color: var(--muted); }
  .commit { display: flex; align-items: center; gap: 8px; padding: 3px 8px; }
  .wt { display: flex; flex-direction: column; gap: 2px; }
  .wrow { display: grid; grid-template-columns: minmax(120px, auto) minmax(0, 1fr) auto auto; align-items: center; gap: 10px; padding: 4px 8px; border-radius: 8px; }
  .wrow:hover { background: var(--panel-2); }
  .wrow.cur { background: rgb(var(--accent-rgb) / .08); }
  .wn { min-width: 0; overflow-wrap: anywhere; }
  .wp { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .wa { display: inline-flex; flex-wrap: wrap; gap: 4px; }
  .warn { color: var(--warn); }
  .diff { display: flex; flex-direction: column; min-height: 0; border: 1px solid var(--line); border-radius: 10px; padding: 6px 8px; gap: 6px; }
  .ph { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .ph b { min-width: 0; overflow-wrap: anywhere; }
  .back { display: none; }
  .ico { width: 24px; height: 24px; padding: 0; font-size: 12px; background: transparent; border-color: transparent; color: var(--muted); flex: none; }
  .ico.disc:hover { color: var(--bad); }
  .diff pre { flex: 1; min-height: 0; overflow: auto; margin: 0; font-family: var(--mono); font-size: 12px; line-height: 1.4; }
  .ln.add { color: var(--diff-add); background: var(--ok-soft); } .ln.del { color: var(--diff-del); background: var(--bad-soft); } .ln.hunk { color: var(--accent); }
  .muted { color: var(--muted); }
  .sm { font-size: 11.5px; }
  .hint { font-size: 12.5px; padding: 8px; }
  @container workspace-git (max-width: 680px) {
    .split.with-diff { grid-template-columns: minmax(0, 1fr); }
    .split.with-diff .list { display: none; }
    .back { display: inline-flex; }
    .close-preview { display: none; }
    .ph b { flex-basis: 100%; }
    .wrow { grid-template-columns: minmax(0, 1fr); gap: 5px; }
    .wrow .wp { overflow-wrap: anywhere; white-space: normal; }
    .commit { flex-wrap: wrap; }
    .commit .nm { flex-basis: calc(100% - 60px); }
  }
  @media (pointer: coarse) {
    .mini, .branch, .lnk, .ico, .nmb, .primary { min-height: 44px; }
    .ico { min-width: 44px; }
    .stage-box { width: 44px; height: 44px; }
    .ck { width: 18px; height: 18px; padding: 0; }
    .cm textarea { font-size: 16px; }
  }
</style>
