<script lang="ts">
  // Настройки: аккаунты агентов (ядро), маршруты и умолчания (сервис), уведомления (ntfy через ядро), разрешения по умолчанию,
  // внешний вид (браузер), голос (позже), о системе.
  import { ui, data, MODELS_FOR, MODES_FOR, modeLabel } from '../state.svelte';
  import { byProvider, api, agents, type AgentsSettings, type GateAuth, type GateTarget, type GateLogin, type GateStart , type ChatSettings, type McpServer, type McpList } from '../api';
  import { menu } from '../menu.svelte';
  import { prefs } from '../prefs.svelte';
  import { toast } from '../toast.svelte';
  import { t, LANGS, LANG_NAMES, type LangSetting } from '../i18n';
  import Icon from '../Icon.svelte';
  import Term from '../Term.svelte';
  const TABS = $derived<[string, string, string][]>([['accounts', 'key', t('set.accounts')], ['routes', 'globe', t('set.routes')], ['chat', 'zap', t('set.chat')], ['notify', 'bell', t('set.notify')], ['perm', 'shield', t('set.perm')], ['mcp', 'cpu', t('set.mcp')], ['look', 'sun', t('set.look')], ['voice', 'mic', t('set.voice')], ['about', 'info', t('set.about')]] as const);
  let settings = $state<AgentsSettings | null>(null);
  let notify = $state<{ app: string | null; server: string | null; topic: string; user: string; password: string | null } | null>(null);
  let form = $state<'none' | 'import' | 'add' | 'login'>('none');
  // вход OAuth: терминал в workspace с отдельным каталогом конфигурации; ссылку открывают на своём устройстве, код вставляют в терминал
  let login = $state<{ provider: 'claude' | 'codex'; project: string; way: 'link' | 'code' | 'term'; callback?: string; pty?: string; dir?: string; done?: boolean; code?: number | null; imported?: boolean; busy?: boolean; error?: string; url?: string; state?: string; pasted: string; paste: string }>({ provider: 'claude', project: '', way: 'link', pasted: '', paste: '' });
  // вход по ссылке: Claude — ссылка с PKCE и код со страницы (code#state); ChatGPT (Codex) — одноразовый код устройства,
  // ядро опрашивает подтверждение само (device-code, как `codex login --device-auth`), терминал и workspace не нужны
  let codexPoll: ReturnType<typeof setInterval> | undefined;
  let userCode = $state(''); let codeUntil = $state(0); let accountLabel = $state('');
  function stopCodexPoll() { if (codexPoll) clearInterval(codexPoll); codexPoll = undefined; }
  async function getLink() {
    login = { ...login, busy: true, error: '' }; userCode = ''; accountLabel = '';
    try {
      // «по ссылке» — OAuth-ссылка как в панели прокси (вход в браузере, затем адрес localhost:<порт>/… из адресной строки);
      // «по коду» — device-code Codex / код со страницы Claude
      const prov = login.way === 'link' ? (login.provider === 'codex' ? 'codex-oauth' : 'claude-oauth') : login.provider;
      const r = await api.oauthStart(prov); login = { ...login, url: r.url, state: r.state, callback: r.callback ?? '', busy: false };
      if (login.provider === 'codex' && login.way === 'code') {
        userCode = r.user_code ?? ''; codeUntil = Date.now() + (r.expires_in ?? 900) * 1000; stopCodexPoll();
        const st = r.state; codexPoll = setInterval(async () => {
          try {
            const p = await api.oauthStatus(st);
            if (p.status === 'ok') { stopCodexPoll(); accountLabel = p.account?.label ?? ''; login = { ...login, imported: true }; await data.loadAccounts(); toast(t('set.loginImported')); }
            else if (p.status === 'error') { stopCodexPoll(); login = { ...login, error: p.error ?? t('set.loginFailed') }; }
          } catch {}
        }, Math.max(3, r.interval ?? 5) * 1000);
      }
    } catch (e) { login = { ...login, busy: false, error: e instanceof Error ? e.message : String(e) }; }
  }
  async function copyCode() { try { await navigator.clipboard.writeText(userCode); toast(t('set.loginCopied'), 'info'); } catch {} }
  const minutesLeft = () => Math.max(0, Math.round((codeUntil - Date.now()) / 60000));
  async function finishLink() {
    const st = login.state, code = login.pasted.trim(); if (!st || !code) return;
    login = { ...login, busy: true, error: '' };
    try { await api.oauthFinish({ state: st, code }); login = { ...login, imported: true, busy: false }; await data.loadAccounts(); toast(t('set.loginImported')); }
    catch (e) { login = { ...login, busy: false, error: e instanceof Error ? e.message : String(e) }; }
  }
  async function copyLink() { try { await navigator.clipboard.writeText(login.url ?? ''); toast(t('set.loginCopied'), 'info'); } catch {} }
  // вставка кода в терминал входа (на телефоне вставка в xterm ненадёжна): строка + Enter в PTY
  async function pasteToTerm() { if (!login.pty || !login.paste.trim()) return; try { await api.ptyInput(login.project, login.pty, login.paste.trim() + '\r'); login = { ...login, paste: '' }; } catch (e) { login = { ...login, error: e instanceof Error ? e.message : String(e) }; } }
  async function startLogin() {
    const project = login.project || data.running[0]?.name; if (!project) { toast(t('set.loginNoWs'), 'bad'); return; }
    login = { ...login, project, busy: true, error: '', done: false, imported: false };
    try { const r = await api.accountLogin({ provider: login.provider, project }); login = { ...login, pty: r.pty, dir: r.dir, busy: false }; } catch (e) { login = { ...login, busy: false, error: e instanceof Error ? e.message : String(e) }; }
  }
  async function finishLogin(code: number) {
    login = { ...login, done: true, code };
    if (code === 0) await importLogin();
  }
  async function importLogin() {
    if (!login.dir) return;
    try { await api.accountImport({ provider: login.provider, project: login.project, dir: login.dir }); login = { ...login, imported: true, error: '' }; await data.loadAccounts(); toast(t('set.loginImported')); }
    catch (e) { login = { ...login, error: e instanceof Error ? e.message : String(e) }; }
  }
  async function closeLogin() { stopCodexPoll(); if (login.pty && !login.done) { try { await api.ptyKill(login.project, login.pty); } catch {} } login = { provider: login.provider, project: login.project, way: login.way, pasted: '', paste: '' }; userCode = ''; form = 'none'; }
  // провайдеры через gate (docs/agents-ux.md §9.1): входы в прокси провайдера llm через его Management API — device-code (ссылка + код)
  // или callback (ссылка, затем адрес localhost:<порт>/… из адресной строки вставляется обратно)
  let gate = $state<GateAuth | null>(null); let gateErr = $state('');
  let gl = $state<{ app: string; provider: string; title: string; url: string; state: string; flow: 'device' | 'callback' | 'code'; userCode: string; callback: string; paste: string; busy: boolean; error: string; done: boolean; sent: boolean } | null>(null);
  let gatePoll: ReturnType<typeof setInterval> | undefined;
  function stopGatePoll() { if (gatePoll) clearInterval(gatePoll); gatePoll = undefined; }
  async function loadGate() { try { gate = await api.gateAuth(); gateErr = ''; } catch (e) { gateErr = msg(e); } }
  $effect(() => { if (ui.settingsTab === 'accounts' && !gate) loadGate(); });
  function gateLoginMenu(el: HTMLElement, tg: GateTarget) {
    const plug = tg.plugins.filter(p => p.oauth && p.enabled && p.provider);
    menu.show(el, [
      { header: true, label: t('set.gateLoginHdr') },
      ...tg.providers.filter(p => !p.since).map(p => ({ icon: p.core ? 'key' : 'ext', label: p.title, hint: p.core ? t('set.gateCoreHint') : p.models, on: () => gateStart(tg, p.id, p.title) })),
      ...(plug.length ? [{ sep: true }, ...plug.map(p => ({ icon: 'key', label: `${p.name ?? p.id} · ${t('set.gatePlugin')}`, on: () => gateStart(tg, p.provider!, p.name ?? p.id) }))] : []),
      { sep: true },
      ...tg.providers.filter(p => p.since).map(p => ({ icon: 'key', label: `${p.title} · ${p.since}+`, hint: p.models, on: () => gateStart(tg, p.id, p.title) })),
    ]);
  }
  function gatePluginsMenu(el: HTMLElement, tg: GateTarget) {
    const items = tg.store.length ? tg.store.map(p => ({ icon: p.installed ? 'check' : 'download', label: `${p.name ?? p.id}${p.version ? ' · ' + p.version : ''}`, hint: p.installed ? t('set.gateInstalled') : p.description ?? undefined, disabled: p.installed, on: () => gateInstall(tg, p.id) })) : [{ label: t('set.gateStoreEmpty'), disabled: true }];
    menu.show(el, [{ header: true, label: t('set.gatePluginsHdr') }, ...items, { sep: true }, { label: t('set.gateQwenNote'), disabled: true }]);
  }
  async function gateStart(tg: GateTarget, provider: string, title: string) {
    stopGatePoll(); gl = { app: tg.app, provider, title, url: '', state: '', flow: 'callback', userCode: '', callback: '', paste: '', busy: true, error: '', done: false, sent: false };
    try {
      const r: GateStart = await api.gateAuthStart(tg.app, provider);
      gl = { ...gl!, url: r.url, state: r.state, flow: r.flow, userCode: r.user_code ?? '', callback: r.callback ?? '', busy: false };
      const app = tg.app, st = r.state;
      gatePoll = setInterval(async () => {
        try {
          const p = await api.gateAuthStatus(app, st);
          if (p.status === 'ok') { stopGatePoll(); gl = gl ? { ...gl, done: true } : gl; toast(t('set.gateDone')); await loadGate(); }
          else if (p.status === 'error') { stopGatePoll(); gl = gl ? { ...gl, error: p.error ?? t('set.loginFailed') } : gl; }
        } catch {}
      }, 4000);
    } catch (e) { gl = { ...gl!, busy: false, error: msg(e) }; }
  }
  async function gateCallback() {
    if (!gl || !gl.paste.trim()) return; gl = { ...gl, busy: true, error: '' };
    try { const r = await api.gateAuthCallback(gl.app, gl.state, gl.paste.trim()); if (r.done) { stopGatePoll(); gl = { ...gl, busy: false, sent: true, paste: '', done: true }; toast(t('set.gateDone')); await loadGate(); } else gl = { ...gl, busy: false, sent: true, paste: '' }; } catch (e) { gl = { ...gl, busy: false, error: msg(e) }; }
  }
  function gateCancel() { stopGatePoll(); gl = null; }
  async function gateDelete(tg: GateTarget, l: GateLogin) { if (!confirm(t('set.gateDeleteConfirm', { name: l.name }))) return; try { await api.gateLoginDelete(tg.app, l.name); toast(t('set.gateDeleted')); await loadGate(); } catch (e) { toast(msg(e), 'bad'); } }
  async function gateToggle(tg: GateTarget, l: GateLogin) { try { await api.gateLoginDisabled(tg.app, l.name, !l.disabled); await loadGate(); } catch (e) { toast(msg(e), 'bad'); } }
  async function gateInstall(tg: GateTarget, id: string) { if (!confirm(t('set.gateInstallConfirm', { id }))) return; try { toast(t('set.gateInstalling', { id }), 'info'); await api.gatePluginInstall(tg.app, id); toast(t('set.gateInstalledToast', { id })); await loadGate(); } catch (e) { toast(msg(e), 'bad'); } }
  let imp = $state({ provider: 'claude', project: '', label: '' });
  let add = $state({ provider: 'openai', kind: 'api_key', label: '', env: '' });
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  $effect(() => { agents.settings().then(s => (settings = s)).catch(() => {}); });
  let chatCfg = $state<ChatSettings>({ route: '', model: '', search: { kind: 'none', url: '', key: '' } });
  $effect(() => { const c = settings?.chat; if (c) chatCfg = { route: c.route ?? '', model: c.model ?? '', search: { kind: c.search?.kind ?? 'none', url: c.search?.url ?? '', key: c.search?.key ?? '' } }; });
  $effect(() => { if (ui.settingsTab === 'chat' && !data.chatRoutes) data.loadChatRoutes(); });
  const chatRouteGroups = $derived(byProvider(data.chatRoutes?.routes.find(r => r.id === chatCfg.route)?.models ?? []));
  async function save(patch: Partial<AgentsSettings> | Record<string, unknown>) { try { await agents.saveSettings(patch as any); settings = await agents.settings(); toast(t('toast.saved')); } catch (e) { toast(msg(e), 'bad'); } }
  async function doImport() { try { await api.accountImport({ provider: imp.provider, project: imp.project || data.projects[0]?.name, label: imp.label || undefined } as any); form = 'none'; await data.loadAccounts(); toast(t('toast.saved')); } catch (e) { toast(msg(e), 'bad'); } }
  async function doAdd() {
    const env: Record<string, string> = {}; for (const line of add.env.split('\n')) { const i = line.indexOf('='); if (i > 0) env[line.slice(0, i).trim()] = line.slice(i + 1).trim(); }
    try { await api.accountCreate({ provider: add.provider, kind: add.kind, label: add.label || add.provider, env } as any); form = 'none'; await data.loadAccounts(); toast(t('toast.saved')); } catch (e) { toast(msg(e), 'bad'); }
  }
  async function del(id: string, label: string) { if (!window.confirm(t('set.deleteConfirm', { label }))) return; try { await api.accountDelete(id); await data.loadAccounts(); } catch (e) { toast(msg(e), 'bad'); } }
  async function setProjectAccount(project: string, id: string) { try { await api.projectAccount(project, id || null); await data.loadAccounts(); } catch (e) { toast(msg(e), 'bad'); } }
  async function showNotify() { try { notify = await api.agentsNotify(); } catch (e) { toast(msg(e), 'bad'); } }
  const perm = $derived((settings?.defaults as any) ?? {});
  // политика workspace'ов: режим обхода разрешений Claude Code (managed-settings) — решение владельца
  let policy = $state<{ bypass: boolean } | null>(null);
  async function loadPolicy() { try { policy = await api.agentsPolicyGet(); } catch {} }
  async function setPolicy(bypass: boolean) {
    try { const r = await api.agentsPolicyPut(bypass); policy = { bypass: r.bypass }; toast(t('set.policySaved', { n: r.applied.length })); } catch (e) { toast(e instanceof Error ? e.message : String(e), 'bad'); }
  }
  $effect(() => { if (ui.settingsTab === 'perm' && !policy) loadPolicy(); });
  // MCP-серверы workspace'а (срез 36): список agentd, форма добавления, раскладка движкам
  let mcpWs = $state(''); let mcp = $state<McpList | null>(null); let mcpBusy = $state(false); let mcpErr = $state(''); let mcpApplied = $state<Record<string, string> | null>(null); let mcpLoaded = '';
  let mcpForm = $state({ name: '', transport: 'stdio' as 'stdio' | 'http' | 'sse', command: '', url: '', env: '', headers: '' });
  async function loadMcp() { if (!mcpWs) return; mcpLoaded = mcpWs; try { mcp = await api.mcp(mcpWs); mcpErr = ''; } catch (e) { mcp = null; mcpErr = msg(e); } }
  $effect(() => { if (ui.settingsTab !== 'mcp') return; const want = ui.mcpWs || mcpWs || data.running[0]?.name || ''; if (ui.mcpWs) ui.mcpWs = null; if (want && want !== mcpLoaded) { mcpWs = want; loadMcp(); } });
  const parseArgs = (line: string) => (line.match(/"[^"]*"|'[^']*'|\S+/g) || []).map((a) => a.replace(/^["']|["']$/g, ''));
  function kv(text: string, sep: string): Record<string, string> { const o: Record<string, string> = {}; for (const l of text.split('\n')) { const i = l.indexOf(sep); if (i > 0) o[l.slice(0, i).trim()] = l.slice(i + sep.length).trim(); } return o; }
  async function saveMcp(servers: McpServer[]) {
    mcpBusy = true; mcpErr = '';
    try { const r = await api.mcpSave(mcpWs, servers); mcp = { servers: r.servers, engines: r.engines }; mcpApplied = r.applied; toast(t('mcp.saved')); }
    catch (e) { mcpErr = msg(e); } finally { mcpBusy = false; }
  }
  async function addMcp() {
    const f = mcpForm; const parts = parseArgs(f.command);
    const srv: McpServer = { name: f.name.trim(), transport: f.transport, command: parts[0] ?? '', args: parts.slice(1), url: f.url.trim(), env: kv(f.env, '='), headers: kv(f.headers, ':') };
    await saveMcp([...(mcp?.servers ?? []).filter((x) => x.name !== srv.name), srv]);
    if (!mcpErr) mcpForm = { name: '', transport: f.transport, command: '', url: '', env: '', headers: '' };
  }
  const removeMcp = (name: string) => saveMcp((mcp?.servers ?? []).filter((x) => x.name !== name));
  // правила инструментов Claude Code в settings workspace'а (agentd perms.rs): область «пользователь» (~/.claude/settings.json) или «проект» (.claude/settings.local.json дома)
  let rules = $state<{ ws: string; scope: 'user' | 'project'; allow: string; deny: string; ask: string; path?: string; loaded?: string }>({ ws: '', scope: 'user', allow: '', deny: '', ask: '' });
  const lines = (s: string) => s.split('\n').map(x => x.trim()).filter(Boolean);
  const RULE_CHIPS = ['Bash(npm test:*)', 'Bash(git:*)', 'Read', 'Edit', 'WebFetch', 'WebSearch'];
  async function loadRules() {
    const ws = rules.ws || data.running[0]?.name; if (!ws) return;
    try { const r = await api.permissionsGet(ws, rules.scope, '/home/coder'); rules = { ...rules, ws, allow: r.allow.join('\n'), deny: r.deny.join('\n'), ask: r.ask.join('\n'), path: r.path, loaded: `${ws}:${rules.scope}` }; } catch (e) { toast(msg(e), 'bad'); }
  }
  async function saveRules() {
    try { const r = await api.permissionsPut(rules.ws, rules.scope, '/home/coder', { allow: lines(rules.allow), deny: lines(rules.deny), ask: lines(rules.ask) }); rules = { ...rules, path: r.path, allow: r.allow.join('\n'), deny: r.deny.join('\n'), ask: r.ask.join('\n') }; toast(t('perm.saved')); } catch (e) { toast(msg(e), 'bad'); }
  }
  $effect(() => { if (ui.settingsTab === 'perm' && data.running.length && rules.loaded !== `${rules.ws || data.running[0].name}:${rules.scope}`) loadRules(); });
</script>

<main class="main">
  <div class="top"><button class="icon ghost burger" onclick={() => (ui.popen = !ui.popen)}><Icon name="menu" /></button><div class="title"><h1>{t('set.title')}</h1></div></div>
  <div class="content">
    <div class="settings">
      <div class="subnav">{#each TABS as [k, i, l] (k)}<button class:active={ui.settingsTab === k} onclick={() => (ui.settingsTab = k)}><Icon name={i} size="s" /> {l}</button>{/each}</div>
      <div style="min-width: 0">
        {#if ui.settingsTab === 'accounts'}
          <div class="page-h" style="margin-bottom: 12px"><div><h2>{t('set.accountsTitle')}</h2><p>{t('set.accountsHint')}</p></div><span class="sp"></span><button class="sm" onclick={() => (form = form === 'import' ? 'none' : 'import')}><Icon name="download" size="s" /> {t('set.import')}</button><button class="sm" onclick={() => { login = { provider: 'claude', project: data.running[0]?.name ?? '', way: 'link', pasted: '', paste: '' }; form = form === 'login' ? 'none' : 'login'; }}><Icon name="key" size="s" /> {t('set.login')}</button><button class="sm primary" onclick={() => (form = form === 'add' ? 'none' : 'add')}><Icon name="plus" size="s" /> {t('set.add')}</button></div>
          {#if form === 'import'}
            <div class="card-s fs" style="margin-bottom: 14px"><h3>{t('set.importTitle')}</h3><div class="frm"><span>{t('set.importProvider')}</span><select class="sm" bind:value={imp.provider}><option value="claude">Claude (~/.claude)</option><option value="codex">Codex (~/.codex/auth.json)</option><option value="gate">{t('set.gate')}</option></select><span>{t('set.importProject')}</span><select class="sm" bind:value={imp.project}>{#each data.projects as p (p.name)}<option value={p.name}>{p.name}</option>{/each}</select><span>{t('set.importLabel')}</span><input class="sm" bind:value={imp.label} placeholder="Claude Max · charlie" /></div><div class="row"><button class="sm primary" onclick={doImport}>{t('set.importDo')}</button><button class="sm ghost" onclick={() => (form = 'none')}>{t('set.cancel')}</button></div></div>
          {:else if form === 'login'}
            <div class="card-s fs" style="margin-bottom: 14px"><h3>{t('set.loginTitle')}</h3><p>{t('set.loginHint')}</p>
              {#if login.imported && login.way !== 'term'}
                <div class="row" style="gap: 8px; margin-top: 6px; flex-wrap: wrap"><span class="ok-note"><Icon name="check" size="s" /> {t('set.loginImported')}</span><span class="sp"></span><button class="sm ghost" onclick={closeLogin}>{t('common.close')}</button><span class="muted sm" style="flex-basis: 100%">{t('set.loginUseHint')}</span></div>
              {:else if !login.pty && !(login.way !== 'term' && login.url)}
                <div class="frm"><span>{t('set.importProvider')}</span><select class="sm" bind:value={login.provider}><option value="claude">{t('set.optClaudeSub')}</option><option value="codex">{t('set.optCodexSub')}</option></select>
                  <span>{t('set.loginWay')}</span><span class="radios"><label><input type="radio" name="lw" value="link" bind:group={login.way} /> {t('set.loginByLink')}</label><label><input type="radio" name="lw" value="code" bind:group={login.way} /> {t('set.loginByCode')}</label><label><input type="radio" name="lw" value="term" bind:group={login.way} /> {t('set.loginByTerm')}</label></span>
                  {#if login.way === 'term'}<span>{t('set.loginWs')}</span><select class="sm" bind:value={login.project}>{#each data.running as p (p.name)}<option value={p.name}>{p.name}</option>{/each}</select>{/if}</div>
                <div class="row" style="gap: 8px; margin-top: 10px">
                  {#if login.way !== 'term'}<button class="sm primary" disabled={login.busy} onclick={getLink}>{login.busy ? t('common.loading') : t('set.loginGetLink')}</button>
                  {:else}<button class="sm primary" disabled={login.busy || !data.running.length} onclick={startLogin}>{login.busy ? t('common.loading') : t('set.loginStart')}</button>{/if}
                  <button class="sm ghost" onclick={closeLogin}>{t('set.cancel')}</button></div>
                {#if login.error}<div class="err" style="margin-top: 8px">{login.error}</div>{/if}
              {:else if login.way === 'link' && login.url}
                <p class="muted sm">{t('set.gateCallbackHint', { url: login.callback || 'http://localhost:…' })}</p>
                <div class="row" style="gap: 8px; flex-wrap: wrap; margin-top: 6px"><a class="btn sm primary" href={login.url} target="_blank" rel="noopener"><Icon name="ext" size="s" /> {t('set.loginOpen')}</a><button class="sm" onclick={copyLink}><Icon name="copy" size="s" /> {t('set.copy')}</button></div>
                <div class="frm" style="margin-top: 10px"><span>{t('set.loginRedirect')}</span><input class="sm mono cbaddr" placeholder={(login.callback || 'http://localhost:…/…') + '?code=…&state=…'} bind:value={login.pasted} onkeydown={(e) => { if (e.key === 'Enter') finishLink(); }} /></div>
                <div class="row" style="gap: 8px; margin-top: 10px"><button class="sm primary" disabled={login.busy || !login.pasted.trim()} onclick={finishLink}>{login.busy ? t('common.loading') : t('set.loginFinish')}</button><button class="sm ghost" onclick={closeLogin}>{t('set.cancel')}</button></div>
                {#if login.error}<div class="err" style="margin-top: 8px">{login.error}</div>{/if}
              {:else if login.way === 'code' && login.url && login.provider === 'codex'}
                <p class="muted sm">{t('set.loginCodexHint')}</p>
                <div class="row" style="gap: 8px; flex-wrap: wrap; margin-top: 6px"><a class="btn sm primary" href={login.url} target="_blank" rel="noopener"><Icon name="ext" size="s" /> {t('set.loginOpen')}</a><button class="sm" onclick={copyLink}><Icon name="copy" size="s" /> {t('set.copy')}</button></div>
                <div class="devcode" style="margin-top: 10px"><span class="muted sm">{t('set.loginDeviceCode')}</span><code class="code">{userCode}</code><button class="sm" onclick={copyCode}><Icon name="copy" size="s" /> {t('set.copy')}</button><span class="muted xs">{t('set.loginExpires', { m: minutesLeft() })}</span></div>
                <div class="row" style="gap: 8px; margin-top: 10px; align-items: center">
                  {#if login.imported}<span class="ok-note"><Icon name="check" size="s" /> {t('set.loginImported')}{accountLabel ? ` · ${accountLabel}` : ''}</span><span class="muted sm" style="flex-basis: 100%">{t('set.loginUseHint')}</span>{:else if !login.error}<span class="muted sm waiting">⏳ {t('set.loginDeviceWait')}</span>{/if}
                  <span class="sp"></span><button class="sm ghost" onclick={closeLogin}>{login.imported ? t('common.close') : t('set.cancel')}</button></div>
                {#if login.error}<div class="err" style="margin-top: 8px">{login.error}</div>{/if}
              {:else if login.way === 'code' && login.url}
                <p class="muted sm">{t('set.loginLinkHint')}</p>
                <div class="row" style="gap: 8px; flex-wrap: wrap; margin-top: 6px"><a class="btn sm primary" href={login.url} target="_blank" rel="noopener"><Icon name="ext" size="s" /> {t('set.loginOpen')}</a><button class="sm" onclick={copyLink}><Icon name="copy" size="s" /> {t('set.copy')}</button></div>
                <div class="frm" style="margin-top: 10px"><span>{t('set.loginCode')}</span><input class="sm mono" placeholder="xxxxxxxx#yyyyyyyy" bind:value={login.pasted} onkeydown={(e) => { if (e.key === 'Enter') finishLink(); }} /></div>
                <div class="row" style="gap: 8px; margin-top: 10px"><button class="sm primary" disabled={login.busy || !login.pasted.trim()} onclick={finishLink}>{login.busy ? t('common.loading') : t('set.loginFinish')}</button><button class="sm ghost" onclick={closeLogin}>{t('set.cancel')}</button></div>
                {#if login.error}<div class="err" style="margin-top: 8px">{login.error}</div>{/if}
              {:else}
                <div class="login-term">{#key login.pty}<Term project={login.project} id={login.pty ?? ""} onexit={finishLogin} />{/key}</div>
                {#if !login.done}<div class="row" style="gap: 8px; margin-top: 8px"><input class="sm mono" style="flex: 1" placeholder={t('set.loginPaste')} bind:value={login.paste} onkeydown={(e) => { if (e.key === 'Enter') pasteToTerm(); }} /><button class="sm" disabled={!login.paste.trim()} onclick={pasteToTerm}>{t('set.loginPasteDo')}</button></div>{/if}
                <div class="row" style="gap: 8px; margin-top: 10px; flex-wrap: wrap; align-items: center">
                  {#if login.imported}<span class="ok-note"><Icon name="check" size="s" /> {t('set.loginImported')}</span>
                  {:else if login.done}<span class="muted sm">{t('set.loginExited', { code: login.code ?? 0 })}</span><button class="sm primary" onclick={importLogin}>{t('set.loginImport')}</button>
                  {:else}<span class="muted sm">{t('set.loginWaiting')}</span><button class="sm" onclick={importLogin}>{t('set.loginImport')}</button>{/if}
                  <span class="sp"></span><button class="sm ghost" onclick={closeLogin}>{login.imported ? t('common.close') : t('set.cancel')}</button>
                </div>
                {#if login.error}<div class="err" style="margin-top: 8px">{login.error}</div>{/if}
              {/if}
            </div>
          {:else if form === 'add'}
            <div class="card-s fs" style="margin-bottom: 14px"><h3>{t('set.addTitle')}</h3><div class="frm"><span>{t('set.importProvider')}</span><select class="sm" bind:value={add.provider}><option value="anthropic">Anthropic</option><option value="openai">OpenAI</option><option value="openrouter">OpenRouter</option><option value="claude">{t('set.optClaudeProxy')}</option><option value="codex">{t('set.optCodexProxy')}</option></select><span>{t('set.addKind')}</span><select class="sm" bind:value={add.kind}><option value="api_key">{t('set.optKey')}</option><option value="proxy">{t('set.optProxy')}</option></select><span>{t('set.addLabel')}</span><input class="sm" bind:value={add.label} /><span>{t('set.addEnv')}</span><textarea rows="3" class="mono" bind:value={add.env} placeholder="OPENAI_API_KEY=…&#10;OPENAI_BASE_URL=https://openrouter.ai/api/v1"></textarea></div><div class="row"><button class="sm primary" onclick={doAdd}>{t('set.add')}</button><button class="sm ghost" onclick={() => (form = 'none')}>{t('set.cancel')}</button></div></div>
          {/if}
          <div class="card-s" style="overflow: hidden">
            {#each data.accounts as a (a.id)}
              <div class="acct"><span class="av {a.provider === 'codex' ? 'codex' : a.provider === 'claude' || a.provider === 'anthropic' ? 'claude' : 'shell'}"><Icon name={a.provider === 'codex' ? 'codex' : a.provider === 'claude' || a.provider === 'anthropic' ? 'claude' : 'key'} size="s" /></span><div style="min-width: 0"><b class="ell" style="display: block">{a.label}</b><span class="muted sm">{a.provider}{a.imported_from ? ' · ' + t('set.fromWs', { ws: a.imported_from }) : ''}</span></div><span class="chip" style="justify-self: start">{a.kind === 'subscription' ? t('set.kindSubscription') : a.kind === 'proxy' ? t('set.kindProxy') : t('set.kindKey')}</span><code class="muted ell">{[...a.env_keys, ...a.files].join(', ')}</code><span class="muted sm">{t('set.defaultFor')}: {Object.entries(data.projectAccounts).filter(([, id]) => id === a.id).map(([p]) => p).join(', ') || '—'}</span><span class="row"><button class="icon xs ghost danger" data-tip={t('set.delete')} onclick={() => del(a.id, a.label)}><Icon name="trash" size="s" /></button></span></div>
            {/each}
            {#if !data.accounts.length}<div class="muted sm" style="padding: 14px 16px">{t('set.noAccounts')}</div>{/if}
          </div>
          <div class="card-s fs gate" style="margin-top: 14px"><h3>{t('set.gateTitle')}</h3><p>{t('set.gateProvHint')}</p>
            {#if gateErr}<div class="err">{gateErr}</div>{/if}
            {#if gate && !gate.targets.length}<p class="muted sm">{t('set.gateNone')}</p>{/if}
            {#each gate?.targets ?? [] as tg (tg.app)}
              <div class="gtarget" data-app={tg.app}>
                <div class="row" style="gap: 8px; flex-wrap: wrap; align-items: center"><b>{tg.title}</b>{#if !tg.managed}<span class="muted sm">{t('set.gateNoKey')}</span>{:else if tg.reachable}<span class="dot ok"></span>{:else}<span class="err sm">{tg.error ?? t('set.gateUnreachable')}</span>{/if}<span class="sp"></span>
                  {#if tg.managed && tg.reachable}<button class="sm primary glogin" onclick={(e) => { e.stopPropagation(); gateLoginMenu(e.currentTarget as HTMLElement, tg); }}><Icon name="key" size="s" /> {t('set.gateLogin')} ▾</button><button class="sm gplugins" onclick={(e) => { e.stopPropagation(); gatePluginsMenu(e.currentTarget as HTMLElement, tg); }}><Icon name="download" size="s" /> {t('set.gatePlugins')} ▾</button>{/if}
                  <button class="sm ghost" title={t('agents.refresh')} onclick={loadGate}>⟳</button></div>
                {#each tg.logins as l (l.name)}
                  <div class="glogin-row" class:off={l.disabled || l.unavailable}><span class="av {l.provider === 'codex' ? 'codex' : l.provider === 'claude' || l.provider === 'anthropic' ? 'claude' : 'shell'}"><Icon name={l.provider === 'codex' ? 'codex' : l.provider === 'claude' || l.provider === 'anthropic' ? 'claude' : 'key'} size="s" /></span>
                    <span class="gp">{l.provider}</span><span class="ell">{l.label ?? l.email ?? l.name}{#if l.plan} <span class="muted xs">· {l.plan}</span>{/if}</span><span class="muted xs">{l.disabled ? t('set.gateOff') : l.unavailable ? (l.status_message ?? l.status ?? '') : (l.status ?? 'ok')}</span>
                    <button class="xs ghost" onclick={() => gateToggle(tg, l)}>{l.disabled ? t('set.gateEnable') : t('set.gateDisable')}</button><button class="xs ghost" title={t('set.gateDelete')} onclick={() => gateDelete(tg, l)}><Icon name="trash" size="s" /></button></div>
                {/each}
                {#if tg.managed && tg.reachable && !tg.logins.length}<div class="muted sm">{t('set.gateNoLogins')}</div>{/if}
                {#if gl && gl.app === tg.app}
                  <div class="gflow">
                    <div class="row" style="gap: 8px; align-items: center"><b>{gl.title}</b><span class="sp"></span><button class="xs ghost" onclick={gateCancel}>{gl.done ? t('common.close') : t('set.cancel')}</button></div>
                    {#if gl.busy && !gl.url}<div class="muted sm">{t('common.loading')}</div>
                    {:else if gl.url}
                      <div class="row" style="gap: 8px; flex-wrap: wrap; margin-top: 6px"><a class="btn sm primary" href={gl.url} target="_blank" rel="noopener"><Icon name="ext" size="s" /> {t('set.loginOpen')}</a><button class="sm" onclick={async () => { try { await navigator.clipboard.writeText(gl?.url ?? ''); toast(t('set.loginCopied'), 'info'); } catch {} }}><Icon name="copy" size="s" /> {t('set.copy')}</button></div>
                      {#if gl.flow === 'device'}
                        {#if gl.userCode}<div class="devcode" style="margin-top: 8px"><span class="muted sm">{t('set.loginDeviceCode')}</span><code class="code">{gl.userCode}</code><button class="sm" onclick={async () => { try { await navigator.clipboard.writeText(gl?.userCode ?? ''); toast(t('set.loginCopied'), 'info'); } catch {} }}><Icon name="copy" size="s" /> {t('set.copy')}</button></div>{/if}
                      {:else if gl.flow === 'code'}
                        <p class="muted sm" style="margin: 8px 0 4px">{t('set.loginLinkHint')}</p>
                        <div class="row" style="gap: 8px"><input class="sm mono" style="flex: 1" placeholder="xxxxxxxx#yyyyyyyy" bind:value={gl.paste} onkeydown={(e) => { if (e.key === 'Enter') gateCallback(); }} /><button class="sm primary" disabled={gl.busy || !gl.paste.trim()} onclick={gateCallback}>{t('set.loginFinish')}</button></div>
                      {:else}
                        <p class="muted sm" style="margin: 8px 0 4px">{t('set.gateCallbackHint', { url: gl.callback || 'http://localhost:…' })}</p>
                        <div class="row" style="gap: 8px"><input class="sm mono" style="flex: 1" placeholder={gl.callback ? gl.callback + '?code=…&state=…' : 'http://localhost:…/…?code=…&state=…'} bind:value={gl.paste} onkeydown={(e) => { if (e.key === 'Enter') gateCallback(); }} /><button class="sm primary" disabled={gl.busy || !gl.paste.trim()} onclick={gateCallback}>{t('set.gateFinish')}</button></div>
                      {/if}
                      <div class="row" style="gap: 8px; margin-top: 8px; align-items: center">{#if gl.done}<span class="ok-note"><Icon name="check" size="s" /> {t('set.gateDone')}</span>{:else if gl.error}<span class="err sm">{gl.error}</span>{:else}<span class="muted sm waiting">⏳ {gl.sent ? t('set.gateSentWait') : gl.flow === 'device' ? t('set.loginDeviceWait') : gl.flow === 'code' ? t('set.gateCodeWait') : t('set.gateCallbackWait')}</span>{/if}</div>
                    {:else if gl.error}<div class="err" style="margin-top: 6px">{gl.error}</div>{/if}
                  </div>
                {/if}
              </div>
            {/each}
          </div>
          {#if data.accounts.length && data.runningAll.length}
            <div class="card-s fs" style="margin-top: 14px"><h3>{t('set.projectAccount')}</h3><p>{t('set.projectAccountHint')}</p><div class="frm">{#each data.runningAll as p (p.name)}<span>{p.hidden ? `${t('chat.title')} (${p.name})` : p.name}</span><select class="sm" value={data.projectAccounts[p.name] ?? ''} onchange={(e) => setProjectAccount(p.name, (e.target as HTMLSelectElement).value)}><option value="">{t('set.gate')}</option>{#each data.accounts as a (a.id)}<option value={a.id}>{a.label}</option>{/each}</select>{/each}</div></div>
          {/if}
        {:else if ui.settingsTab === 'chat'}
          <div class="card-s fs" style="max-width: 760px">
            <h3>{t('set.chatDefaults')}</h3>
            <div class="frm">
              <span>{t('chat.route')}</span><select class="sm" bind:value={chatCfg.route} onchange={() => (chatCfg.model = '')}><option value="">{t('set.chatAuto')}</option>{#each data.chatRoutes?.routes ?? [] as r (r.id)}<option value={r.id}>{r.title}{r.error ? ' · ' + r.error : ''}</option>{/each}</select>
              <span>{t('chat.model')}</span><select class="sm" bind:value={chatCfg.model}><option value="">{t('set.chatAuto')}</option>{#each chatRouteGroups as g (g.key)}<optgroup label={g.title}>{#each g.models as m, i (m.id + i)}<option value={m.id}>{m.name && m.name !== m.id ? `${m.name} · ${m.id}` : m.id}</option>{/each}</optgroup>{/each}</select>
              <span>{t('set.chatSearch')}<div class="hint">{t('set.chatSearchHint')}</div></span>
              <span class="stack" style="gap: 6px">
                <select class="sm search-kind" bind:value={chatCfg.search.kind}><option value="none">{t('set.chatSearchNone')}</option><option value="searxng">SearXNG</option><option value="brave">Brave Search</option><option value="tavily">Tavily</option></select>
                {#if chatCfg.search.kind === 'searxng'}<input class="sm mono search-url" bind:value={chatCfg.search.url} placeholder="http://127.0.0.1:8450" />{/if}
                {#if chatCfg.search.kind === 'brave' || chatCfg.search.kind === 'tavily'}<input class="sm mono" type="password" bind:value={chatCfg.search.key} placeholder={t('set.chatSearchKey')} autocomplete="off" />{/if}
              </span>
            </div>
            <div class="acts"><button class="sm primary" onclick={() => save({ chat: chatCfg }).then(() => data.loadChatRoutes())}>{t('common.save')}</button></div>
            {#if !data.chatRoutes?.routes.length}<p class="muted sm">{t('chat.noRoutes')}</p>{/if}
          </div>
        {:else if ui.settingsTab === 'routes'}
          <div class="card-s fs"><h3>{t('set.routesTitle')}</h3><p>{t('set.routesHint')}</p>
            {#if settings}<div class="frm"><span>Claude</span><select class="sm" value={settings.defaults.claude ?? ''} onchange={(e) => save({ defaults: { ...settings!.defaults, claude: (e.target as HTMLSelectElement).value } })}><option value="">{t('agents.modelDefault')}</option>{#each MODELS_FOR.claude as x (x)}<option value={x}>{x}</option>{/each}</select><span>Codex</span><select class="sm" value={settings.defaults.codex ?? ''} onchange={(e) => save({ defaults: { ...settings!.defaults, codex: (e.target as HTMLSelectElement).value } })}><option value="">{t('agents.modelDefault')}</option>{#each MODELS_FOR.codex as x (x)}<option value={x}>{x}</option>{/each}</select><span>gate</span><span class="muted sm">{t('set.gateHint')}</span></div>{/if}
          </div>
          {#if settings}<div class="card-s fs" style="margin-top: 14px"><h3>{t('set.prices')}</h3><p>{t('set.pricesHint')}</p><div class="tbl"><div class="tw"><table><thead><tr><th>{t('models.model')}</th><th class="num">{t('set.colIn')}</th><th class="num">{t('set.colOut')}</th><th class="num">{t('set.colCacheR')}</th><th class="num">{t('set.colCacheW')}</th></tr></thead><tbody>{#each Object.entries(settings.prices) as [id, p] (id)}<tr><td class="mono">{id}</td><td class="num">{p.note ?? '$' + p.input}</td><td class="num">{p.note ? '' : '$' + p.output}</td><td class="num">{p.note ? '' : '$' + p.cache_read}</td><td class="num">{p.note ? '' : '$' + p.cache_write}</td></tr>{/each}</tbody></table></div></div></div>{/if}
        {:else if ui.settingsTab === 'notify'}
          <div class="card-s fs"><h3>{t('set.notifyTitle')}</h3><p>{t('set.notifyHint')}</p>
            {#if !notify}<button class="sm" style="justify-self: start" onclick={showNotify}><Icon name="bell" size="s" /> {t('set.notifyShow')}</button>
            {:else if notify.app}<div class="frm"><span>{t('set.notifyServer')}</span><code>{notify.server ?? '—'}</code><span>{t('set.notifyTopic')}</span><code>{notify.topic}</code><span>{t('set.notifyUser')}</span><code>{notify.user}</code><span>{t('set.notifyPassword')}</span><span class="row"><code>{notify.password ?? '—'}</code>{#if notify.password}<button class="icon xs ghost" data-tip={t('set.copy')} onclick={() => navigator.clipboard?.writeText(notify!.password!)}><Icon name="copy" size="s" /></button>{/if}</span></div><p class="muted sm">{t('agents.notifyHow')}</p>
            {:else}<p>{t('set.notifyNo')}</p>{/if}
          </div>
        {:else if ui.settingsTab === 'mcp'}
          <div class="card-s fs mcp"><h3>{t('mcp.title')}</h3><p>{t('mcp.hint')}</p>
            <div class="frm"><span>{t('rt.workspace')}</span><select class="sm" value={mcpWs} onchange={(e) => { mcpWs = (e.target as HTMLSelectElement).value; loadMcp(); }}>{#each data.running as p (p.name)}<option value={p.name}>{p.name}</option>{/each}</select></div>
            {#if mcp}
              <div class="mcp-list">
                {#each mcp.servers as s_ (s_.name)}<div class="mcp-row" data-mcp={s_.name}><span class="mono"><b>{s_.name}</b> <span class="muted">· {s_.transport}</span></span><span class="muted ell mono">{s_.transport === 'stdio' ? [s_.command, ...s_.args].join(' ') : s_.url}</span><button class="xs ghost" disabled={mcpBusy} onclick={() => removeMcp(s_.name)}>{t('mcp.remove')}</button></div>{:else}<div class="muted sm">{t('mcp.empty')}</div>{/each}
              </div>
              <div class="frm mcp-form">
                <span>{t('mcp.name')}</span><input class="sm mono name" bind:value={mcpForm.name} placeholder="context7" />
                <span>{t('mcp.transport')}</span><select class="sm transport" bind:value={mcpForm.transport}><option value="stdio">stdio</option><option value="http">http</option><option value="sse">sse</option></select>
                {#if mcpForm.transport === 'stdio'}
                  <span>{t('mcp.command')}</span><input class="sm mono command" bind:value={mcpForm.command} placeholder="npx -y @modelcontextprotocol/server-everything" />
                  <span>{t('mcp.env')}</span><textarea class="mono env" rows="2" bind:value={mcpForm.env} placeholder="API_KEY=…"></textarea>
                {:else}
                  <span>{t('mcp.url')}</span><input class="sm mono url" bind:value={mcpForm.url} placeholder="https://mcp.example.com/mcp" />
                  <span>{t('mcp.headers')}</span><textarea class="mono headers" rows="2" bind:value={mcpForm.headers} placeholder="Authorization: Bearer …"></textarea>
                {/if}
                <span></span><span class="inl"><button class="sm primary add" disabled={mcpBusy || !mcpForm.name.trim()} onclick={addMcp}>{mcpBusy ? '…' : t('mcp.add')}</button>{#if mcpErr}<span class="bad sm">{mcpErr}</span>{/if}</span>
              </div>
              <p class="muted sm engines">{t('mcp.engines', { c: mcp.engines.claude ?? '—', x: mcp.engines.codex ?? '—', g: mcp.engines.gemini ?? '—' })}{#if mcpApplied} · {Object.entries(mcpApplied).map(([k, v]) => `${k}: ${v}`).join(' · ')}{/if}</p>
              <p class="muted xs">{t('mcp.newSessions')}</p>
            {:else if mcpErr}<p class="bad sm">{mcpErr}</p>{/if}
          </div>
        {:else if ui.settingsTab === 'perm'}
          <div class="card-s fs"><h3>{t('set.policyTitle')}</h3><p>{t('set.policyHint')}</p>
            <div class="frm"><span>{t('set.policyBypass')}</span><span class="row" style="gap: 10px"><button class="switch" class:on={policy?.bypass} aria-pressed={!!policy?.bypass} disabled={!policy} onclick={() => setPolicy(!policy?.bypass)}></button><span class="muted sm">{policy ? (policy.bypass ? t('set.policyOn') : t('set.policyOff')) : t('common.loading')}</span></span></div>
          </div>
          <div class="card-s fs rules"><h3>{t('perm.rulesTitle')}</h3><p>{t('perm.rulesHint')}</p>
            <div class="frm">
              <span>{t('rt.workspace')}</span><select class="sm" bind:value={rules.ws} onchange={loadRules}>{#each data.running as p (p.name)}<option value={p.name}>{p.name}</option>{/each}</select>
              <span>{t('perm.scope')}</span><span class="seg"><button class:active={rules.scope === 'user'} onclick={() => { rules.scope = 'user'; loadRules(); }}>{t('perm.scopeUserShort')}</button><button class:active={rules.scope === 'project'} onclick={() => { rules.scope = 'project'; loadRules(); }}>{t('perm.scopeProjectShort')}</button></span>
              <span>{t('perm.allow')}</span><textarea class="mono" rows="4" bind:value={rules.allow} placeholder="Bash(npm test:*)&#10;Read"></textarea>
              <span>{t('perm.deny')}</span><textarea class="mono" rows="3" bind:value={rules.deny} placeholder="Bash(rm -rf:*)"></textarea>
              <span>{t('perm.ask')}</span><textarea class="mono" rows="2" bind:value={rules.ask}></textarea>
              <span></span><span class="opts">{#each RULE_CHIPS as c (c)}<button class="xs" onclick={() => (rules.allow = (rules.allow.trim() ? rules.allow.trimEnd() + '\n' : '') + c)}>+ {c}</button>{/each}</span>
            </div>
            <div class="row" style="gap: 10px; flex-wrap: wrap"><button class="sm primary" disabled={!rules.ws && !data.running.length} onclick={saveRules}>{t('common.save')}</button><span class="muted xs mono ell">{rules.path ?? ''}</span></div>
            <p class="muted sm" style="margin-top: 0">{t('perm.codexNote')}</p>
          </div>
          <div class="card-s fs"><h3>{t('set.permTitle')}</h3><p>{t('set.permHint')}</p>
            {#if settings}<div class="frm"><span>{t('set.cacheClaude')}</span><select class="sm" value={perm.cache_claude ?? 'auto'} onchange={(e) => save({ defaults: { ...settings!.defaults, cache_claude: (e.target as HTMLSelectElement).value } })}><option value="auto">auto</option><option value="5m">5m</option><option value="1h">1h</option></select><span class="muted xs" style="grid-column: 1 / -1">{t('set.cacheClaudeHint')}</span></div>
            <div class="frm"><span>{t('set.permClaude')}</span><select class="sm" value={perm.perm_claude ?? 'default'} onchange={(e) => save({ defaults: { ...settings!.defaults, perm_claude: (e.target as HTMLSelectElement).value } })}>{#each MODES_FOR.claude as x (x)}<option value={x}>{modeLabel(x)}</option>{/each}</select><span>{t('set.permCodex')}</span><select class="sm" value={perm.perm_codex ?? 'on-request'} onchange={(e) => save({ defaults: { ...settings!.defaults, perm_codex: (e.target as HTMLSelectElement).value } })}>{#each MODES_FOR.codex as x (x)}<option value={x}>{modeLabel(x)}</option>{/each}</select><span>{t('set.permGemini')}</span><select class="sm" value={perm.perm_gemini ?? 'default'} onchange={(e) => save({ defaults: { ...settings!.defaults, perm_gemini: (e.target as HTMLSelectElement).value } })}>{#each MODES_FOR.gemini as x (x)}<option value={x}>{modeLabel(x)}</option>{/each}</select></div>{/if}
          </div>
        {:else if ui.settingsTab === 'look'}
          <div class="card-s fs"><h3>{t('set.lookTitle')}</h3><p>{t('set.lookHint')}</p>
            <div class="frm">
              <span>{t('set.theme')}</span><span class="radios">{#each [['dark', t('set.dark')], ['light', t('set.light')], ['system', t('set.system')]] as [v, l] (v)}<label><input type="radio" name="th" checked={prefs.theme === v} onchange={() => prefs.set('theme', v as any)} /> {l}</label>{/each}</span>
              <span>{t('set.lang')}</span><span class="radios">{#each [['auto', t('set.langAuto')], ...LANGS.map(l => [l, LANG_NAMES[l]])] as [v, l] (v)}<label><input type="radio" name="lg" checked={prefs.lang === v} onchange={() => prefs.setLang(v as LangSetting)} /> {l}</label>{/each}</span>
              <span>{t('set.accent')}</span><span class="swatches">{#each [['blue', '#6d9cff'], ['violet', '#8b7cff'], ['green', '#3ecf8e'], ['orange', '#f2a04b']] as [k, c] (k)}<button class="swatch" class:on={prefs.accent === k} style="--sw: {c}" data-tip={k} aria-label={k} onclick={() => prefs.set('accent', k as any)}></button>{/each}</span>
              <span>{t('set.density')}</span><span class="radios">{#each [['normal', t('set.normal')], ['compact', t('set.compact')]] as [v, l] (v)}<label><input type="radio" name="dn" checked={prefs.density === v} onchange={() => prefs.set('density', v as any)} /> {l}</label>{/each}</span>
            </div>
          </div>
        {:else if ui.settingsTab === 'voice'}
          <div class="card-s fs"><h3>{t('set.voiceTitle')}</h3><p>{t('set.voiceHint')}</p></div>
        {:else}
          <div class="card-s fs"><h3>{t('set.aboutTitle')}</h3><div class="frm"><span>{t('set.service')}</span><span>cloudos-agents {data.health?.version ?? '—'}</span><span>{t('set.core')}</span><span>cloudd {data.health?.cloudd_version ?? (data.health?.cloudd ? 'ok' : t('api.offline'))}</span><span>agentd</span><span>{data.running.filter(p => p.reachable).length} {t('set.of')} {data.running.length} {t('set.aboutWs')}</span><span>{t('set.docs')}</span><span><a href="https://github.com/" target="_blank" rel="noopener">docs/agents-ux.md</a> · ADR-006</span></div></div>
        {/if}
      </div>
    </div>
  </div>
</main>

<style>
  .devcode { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .devcode .code { font-size: 22px; letter-spacing: .12em; padding: 6px 14px; border: 1px dashed var(--line-1); border-radius: var(--r-2); background: var(--bg-3); }
  .gate .gtarget { border-top: 1px solid var(--line-2); padding: 10px 0 4px; }
  .gate .gtarget:first-of-type { border-top: 0; }
  .glogin-row { display: grid; grid-template-columns: 24px 90px minmax(0, 1fr) auto auto auto; align-items: center; gap: 8px; padding: 4px 0; font-size: 13px; }
  .glogin-row.off { opacity: .55; }
  .glogin-row .gp { font-family: var(--mono); font-size: 12px; color: var(--fg-2); }
  .gflow { margin-top: 8px; padding: 10px 12px; border: 1px solid var(--line-1); border-radius: var(--r-2); background: var(--bg-2); }
  .login-term { height: 360px; margin-top: 10px; }
  .ok-note { color: var(--ok); display: inline-flex; align-items: center; gap: 6px; font-size: 12.5px; }
  .err { color: var(--bad); font-size: 12.5px; }
</style>
