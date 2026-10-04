<script lang="ts">
  // Рендеринги карточки сессии (ADR-006): Chat | CLI | Shell | Files | Git над одной сессией workspace'а. CLI — терминальный
  // режим (как в CloudCLI): родной TUI агента на той же беседе в PTY сессии; пока он подключён, headless-процесс остановлен,
  // отправка из чата закрывает TUI и возвращает сессию в чат (правило «один живой процесс» держит agentd). Shell — PTY в cwd
  // сессии, привязанный к ней (`session` у PTY, убирается вместе с сессией); Files и Git — API agentd; вкладка запоминается
  // на сессию. Для «голого» терминала (kind = pty) вкладки Shell | Files | Git.
  import { api, type PtyInfo, type SessionInfo } from './api';
  import { data, msg, engineIcon, engineTitle } from './state.svelte';
  import { toast } from './toast.svelte';
  import Chat from './Chat.svelte';
  import Term from './Term.svelte';
  import WsFiles from './WsFiles.svelte';
  import WsGit from './WsGit.svelte';
  import { t } from './i18n';
  import Icon from './Icon.svelte';
  let { project, id, kind, cwd, onchange, compact = false, hide = [] }: { project: string; id: string; kind: 'session' | 'pty'; cwd: string; onchange?: () => void; compact?: boolean; hide?: string[] } = $props();
  type Tab = 'chat' | 'cli' | 'shell' | 'files' | 'git';
  const tabs: Tab[] = (kind === 'session' ? ['chat', 'cli', 'shell', 'files', 'git'] : ['shell', 'files', 'git']).filter(x => !hide.includes(x)) as Tab[];
  const LS = `cloudos.agents.tab.${project}:${id}`;
  let tab = $state<Tab>(tabs[0]);
  try { const s = localStorage.getItem(LS) as Tab | null; if (s && tabs.includes(s)) tab = s; } catch {}
  $effect(() => { try { localStorage.setItem(LS, tab); } catch {} });
  const info = $derived(kind === 'session' ? (data.infoOf({ project, kind: 'session', id }) as SessionInfo | null) : null);
  const agent = $derived(info?.agent ?? 'claude');
  const label: Record<Tab, () => string> = { chat: () => t('agents.tabChat'), cli: () => t('cli.tab'), shell: () => t('agents.tabShell'), files: () => t('agents.tabFiles'), git: () => t('agents.tabGit') };
  const icon = (x: Tab) => (x === 'chat' ? 'chat' : x === 'cli' ? engineIcon(agent) : x === 'shell' ? 'terminal' : x === 'files' ? 'folder' : 'git');
  // shell сессии: живой PTY с session == id (не TUI), иначе создаём в её cwd
  let shell = $state<PtyInfo | null>(null);
  let shellErr = $state('');
  async function ensureShell(fresh = false) {
    shellErr = '';
    try {
      if (kind === 'pty') { shell = { id } as PtyInfo; return; }
      if (!fresh) { const mine = (await api.ptys(project)).filter(p => p.session === id && !p.tui && p.alive); if (mine.length) { shell = mine[mine.length - 1]; return; } }
      shell = await api.ptyCreate(project, { cwd, session: id, title: `shell · ${id.slice(0, 8)}` });
      onchange?.();
    } catch (e) { shellErr = msg(e); }
  }
  $effect(() => { if (tab === 'shell' && !shell) ensureShell(); });
  async function restartShell() { if (shell && kind === 'session') { try { await api.ptyKill(project, shell.id); } catch {} } shell = null; await ensureShell(true); }
  // терминальный режим: PTY с tui == true у этой сессии; «Обход» запоминается на сессию в браузере (как в CloudCLI)
  const LSB = `cloudos.agents.bypass.${project}:${id}`;
  let bypass = $state(false);
  try { bypass = localStorage.getItem(LSB) === '1'; } catch {}
  let cli = $state<PtyInfo | null>(null);
  let termRef = $state<{ attach: (files: File[]) => Promise<void> } | null>(null);
  function attachFiles(e: Event) { const inp = e.currentTarget as HTMLInputElement; const files = Array.from(inp.files ?? []); inp.value = ''; if (files.length) termRef?.attach(files); }
  let cliErr = $state('');
  let cliBusy = $state(false);
  let cliExit = $state<number | null>(null);
  // разрешён ли режим обхода политикой workspace'а (caps сессии из agentd; у Codex всегда)
  let bypassAllowed = $state<boolean | null>(null);
  async function loadCaps() { try { const r = await api.sessionGet(project, id); bypassAllowed = r.caps?.bypass_allowed ?? true; } catch {} }
  async function findCli() { try { const mine = (await api.ptys(project)).filter(p => p.session === id && p.tui && p.alive); cli = mine.length ? mine[mine.length - 1] : null; } catch {} }
  async function connectCli() {
    cliBusy = true; cliErr = '';
    try { cli = await api.sessionTui(project, id, { bypass }); cliExit = null; toast(t('toast.cliOn'), 'info'); onchange?.(); } catch (e) { cliErr = msg(e); } finally { cliBusy = false; }
  }
  // закрытые TUI: опрос ядра отстаёт на несколько секунд, и его `tui` не должен вернуть мёртвый PTY на экран
  const dead = new Set<string>();
  async function disconnectCli() {
    cliBusy = true; if (cli) dead.add(cli.id);
    try { await api.sessionTuiStop(project, id); toast(t('toast.cliOff'), 'info'); } catch {}
    cli = null; cliBusy = false; onchange?.();
  }
  async function toggleBypass() { bypass = !bypass; try { localStorage.setItem(LSB, bypass ? '1' : '0'); } catch {} if (cli) await connectCli(); }
  // клавиши для TUI с телефона (у мобильной клавиатуры их нет): стрелки, Enter, Esc — прямо в PTY, фокус у терминала не отбираем
  const KEYS: [string, string, string][] = [['↑', '\x1b[A', 'Up'], ['↓', '\x1b[B', 'Down'], ['⏎', '\r', 'Enter'], ['Esc', '\x1b', 'Escape']];
  async function sendKey(seq: string) { if (!cli) return; try { await api.ptyInput(project, cli.id, seq); } catch (e) { cliErr = msg(e); } }
  // при каждом открытии вкладки сверяемся с agentd: TUI могли закрыть отправкой из чата (PTY убран) или подключить с другого устройства
  $effect(() => { if (tab === 'cli') { findCli(); if (bypassAllowed === null) loadCaps(); } });
  $effect(() => { const other = info?.tui; if (tab === 'cli' && !cli && other && !dead.has(other)) cli = { id: other } as PtyInfo; });
  $effect(() => { if (info?.alive && cli) cli = null; });
</script>

<div class="pane" class:compact>
  <div class="tabs">
    {#each tabs as x (x)}<button class="tab" class:active={tab === x} onclick={() => (tab = x)}><Icon name={icon(x)} size="s" /><span>{label[x]()}</span></button>{/each}
    <span class="sp"></span>
    {#if tab === 'shell' && kind === 'session'}<button class="xs ghost" title={t('agents.shellRestartHint')} onclick={restartShell}><Icon name="refresh" size="s" /> {t('agents.shellRestart')}</button>{/if}
    {#if tab === 'cli' && cli}
      <span class="keys" role="group" aria-label="keys">{#each KEYS as [label, seq, name] (name)}<button class="key" title={name} aria-label={name} onpointerdown={(e) => { e.preventDefault(); sendKey(seq); }}>{label}</button>{/each}</span>
      <label class="clipbtn" title={t('cli.attach')} aria-label={t('cli.attach')}><Icon name="clip" size="s" /><input type="file" accept="image/*,.pdf,.txt,.md,.json,.csv,.log" multiple hidden onchange={attachFiles} /></label>
      <span class="cstate hide-c"><span class="dot ok"></span> {t('cli.connected')}</span>
      <button class="xs ghost" class:on={bypass && bypassAllowed !== false} title={bypassAllowed === false ? t('cmp.bypassBlocked') : t('cli.bypassHint')} onclick={toggleBypass} disabled={cliBusy || bypassAllowed === false}><Icon name="shield" size="s" /> {t('cli.bypass')}</button>
      <button class="xs ghost" title={t('cli.restart')} onclick={connectCli} disabled={cliBusy}><Icon name="refresh" size="s" /> <span class="hide-c">{t('cli.restart')}</span></button>
      <button class="xs ghost" title={t('cli.disconnect')} onclick={disconnectCli} disabled={cliBusy}><Icon name="x" size="s" /> <span class="hide-c">{t('cli.disconnect')}</span></button>
    {/if}
  </div>
  <div class="body">
    {#if tab === 'chat'}
      <Chat {project} {id} {onchange} />
    {:else if tab === 'cli'}
      {#if cli}{#key cli.id}<Term bind:this={termRef} {project} id={cli.id} onexit={(code) => { if (cli) dead.add(cli.id); cli = null; cliExit = code; onchange?.(); }} />{/key}
      {:else}
        <div class="start">
          <span class="av {agent} lg"><Icon name={engineIcon(agent)} /></span>
          <b>{agent === 'codex' ? 'Codex CLI' : engineTitle(agent)} · {t('cli.inTerminal')}</b>
          <p class="muted sm">{t('cli.hint')}</p>
          <label class="opt" class:dis={bypassAllowed === false}><button class="switch" class:on={bypass && bypassAllowed !== false} aria-pressed={bypass} aria-label={t('cli.bypassLong')} disabled={bypassAllowed === false} onclick={toggleBypass}></button><span><b class="sm">{t('cli.bypassLong')}</b><span class="muted xs">{bypassAllowed === false ? t('cmp.bypassBlocked') : t('cli.bypassHint')}</span></span></label>
          <div class="row">
            <button class="primary" onclick={connectCli} disabled={cliBusy}><Icon name="play" size="s" /> {cliBusy ? t('cli.connecting') : t('cli.connect')}</button>
            {#if cliExit != null}<span class="muted xs">{t('cli.exitedNote', { code: cliExit })}</span>{/if}
          </div>
          {#if cliErr}<div class="err">{cliErr}</div>{/if}
        </div>
      {/if}
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
  .pane { display: flex; flex-direction: column; height: 100%; min-height: 0; min-width: 0; container: pane / inline-size; }
  .tabs { display: flex; align-items: center; gap: 1px; padding: 0 0 3px; margin-bottom: 4px; border-bottom: 1px solid var(--line-1); }
  .tabs .tab { display: inline-flex; align-items: center; gap: 5px; height: 24px; padding: 0 8px; border-radius: var(--r-1); border: 0; background: transparent; color: var(--fg-3); font-size: 12px; font-weight: 500; }
  .tabs .tab:hover { background: var(--bg-3); color: var(--fg-1); }
  .tabs .tab.active { background: var(--bg-3); color: var(--fg-1); box-shadow: inset 0 -2px 0 var(--accent); border-radius: var(--r-1) var(--r-1) 0 0; }
  .tabs button.on { background: var(--accent-soft); color: var(--accent); }
  .compact .tabs { padding-bottom: 2px; margin-bottom: 3px; }
  .compact .tabs .tab { padding: 0 7px; font-size: 11.5px; height: 22px; }
  @media (max-width: 599px) { .tabs { padding-bottom: 2px; margin-bottom: 3px; } .tabs .tab { height: 22px; padding: 0 7px; } }
  .cstate { display: inline-flex; align-items: center; gap: 6px; font-size: 11.5px; color: var(--fg-3); margin-right: 4px; }
  .keys { display: inline-flex; gap: 2px; margin-right: 4px; }
  .keys .key { height: 22px; min-width: 24px; padding: 0 5px; border-radius: 5px; border: 1px solid var(--line-2); background: var(--bg-3); color: var(--fg-2); font-family: var(--mono); font-size: 11px; line-height: 1; touch-action: manipulation; }
  .clipbtn { display: inline-grid; place-items: center; height: 22px; min-width: 24px; margin-right: 4px; border-radius: 5px; border: 1px solid var(--line-2); background: var(--bg-3); color: var(--fg-2); cursor: pointer; }
  .clipbtn:hover { color: var(--fg-1); border-color: var(--line-1); }
  .keys .key:active { background: var(--accent-soft); color: var(--accent); }
  /* узкая карточка или узкий Thread (открыт инспектор): у кнопок CLI остаются иконки, у вкладок — только иконки */
  @container pane (max-width: 1000px) { .hide-c { display: none; } }
  @container pane (max-width: 640px) { .tabs .tab span { display: none; } .tabs .tab { padding: 0 9px; } }
  @container card (max-width: 460px) { .tabs .tab span { display: none; } .tabs .tab { padding: 0 9px; } .hide-c { display: none; } }
  .sp { flex: 1; }
  .body { flex: 1; min-height: 0; display: flex; }
  .body > :global(*) { flex: 1; min-height: 0; min-width: 0; }
  .start { display: flex; flex-direction: column; align-items: flex-start; gap: 10px; max-width: 560px; margin: 8px auto; padding: 18px 20px; border: 1px solid var(--line-1); border-radius: var(--r-2); background: var(--bg-2); align-self: flex-start; }
  .start p { margin: 0; line-height: 1.45; }
  .opt { display: flex; align-items: flex-start; gap: 10px; cursor: pointer; }
  .opt.dis { opacity: .55; cursor: not-allowed; }
  .opt > span { display: flex; flex-direction: column; gap: 2px; }
  .start .row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .err { color: var(--bad); font-size: 12.5px; padding: 8px; }
  .muted { color: var(--fg-3); }
  .hint { font-size: 12.5px; padding: 8px; }
</style>
