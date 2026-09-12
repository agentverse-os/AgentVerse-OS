<script lang="ts">
  // Мастер первого запуска (docs/first-run.md): Tailscale → адрес и DNS → сертификат → первый проект → что дальше.
  // Состояние берётся у ядра (/api/setup) и обновляется каждые 8 с: пока пользователь авторизует узел в другой вкладке,
  // шаги зеленеют сами. Проверка «с этого устройства» делается из браузера: fetch no-cors до адреса ядра.
  // Тексты ядра (state и error Tailscale, health, error edge, ошибки API) показываются как есть; словарь — i18n/messages/setup.ts.
  import { api, type SetupState } from './api';
  import { wm } from './windows.svelte';
  import { t } from './i18n.svelte';
  let st = $state<SetupState | null>(null);
  let error = $state('');
  let busy = $state('');
  let msg = $state('');
  let authUrl = $state<string | null>(null);
  let device = $state<{ ok: boolean; host: string } | null>(null);
  let switching = $state<{ url: string; tries: number } | null>(null);
  let pname = $state('');
  let created = $state('');

  async function load() {
    try { st = await api.setup(); error = ''; if (st.tailscale.auth_url) authUrl = st.tailscale.auth_url; if (st.tailscale.state === 'Running') authUrl = null; }
    catch (e) { error = e instanceof Error ? e.message : String(e); }
  }
  $effect(() => { load(); const timer = setInterval(() => { if (!busy) load(); }, 8000); return () => clearInterval(timer); });

  async function act(label: string, f: () => Promise<unknown>) {
    busy = label; msg = '';
    try { await f(); } catch (e) { msg = (e instanceof Error ? e.message : String(e)); } finally { busy = ''; await load(); }
  }
  const login = () => act('login', async () => { const r = await api.setupLogin(); authUrl = r.auth_url ?? null; if (r.state === 'Running') msg = t('setup.tsConnected'); else if (!r.auth_url) msg = t('setup.noAuthUrl', { state: r.state }); });
  async function checkDevice() {
    if (!st) return;
    const host = st.edge.host;
    // текст результата собирается в разметке по device.ok — переводится вместе со сменой языка
    try { await fetch(`https://${host}/api/status`, { mode: 'no-cors', cache: 'no-store' }); device = { ok: true, host }; }
    catch { device = { ok: false, host }; }
  }
  const switchHost = () => act('switch', async () => {
    if (!st?.suggested_host) return;
    const r = await api.setupSwitchHost(st.suggested_host);
    switching = { url: r.url, tries: 0 };
    // ядро перезапускается; ждём, когда новый адрес ответит с этого устройства, и переходим на него
    const tick = async () => {
      const sw = switching; if (!sw) return;
      const tries = sw.tries + 1; switching = { url: sw.url, tries };
      try { await fetch(`${sw.url}api/status`, { mode: 'no-cors', cache: 'no-store' }); if (location.host !== new URL(sw.url).host) location.href = sw.url; else switching = null; return; } catch {}
      if (tries < 40) setTimeout(tick, 3000); else msg = t('setup.switchTimeout', { url: sw.url });
    };
    setTimeout(tick, 5000);
  });
  const createProject = () => act('project', async () => {
    const name = pname.trim().toLowerCase();
    if (!/^[a-z0-9][a-z0-9-]{1,30}$/.test(name)) throw new Error(t('setup.projectNameRule'));
    await api.applyProject({ schema: 1, project: name, workspace: { runtime: 'incus-nesting', cpu: 2, memory: 4, home: 50 }, agents: ['claude-code', 'codex'], capabilities: [] });
    created = name; pname = '';
  });
  const finish = () => act('done', async () => { await api.setupDone(true); wm.close('sys:setup'); });
  const reopen = () => act('reopen', async () => { await api.setupDone(false); });

  const tsState = $derived.by(() => {
    const ts = st?.tailscale;
    if (!ts) return { icon: '⏳', title: t('setup.tsChecking'), kind: 'wait' as const };
    switch (ts.state) {
      case 'Running': return { icon: '✅', title: t('setup.tsRunning', { name: ts.dns_name ?? t('setup.tsNoName') }), kind: 'ok' as const };
      case 'NeedsLogin': return { icon: '⚠️', title: t('setup.tsNeedsLogin'), kind: 'warn' as const };
      case 'NeedsMachineAuth': return { icon: '⚠️', title: t('setup.tsNeedsMachineAuth'), kind: 'warn' as const };
      case 'Stopped': return { icon: '⚠️', title: t('setup.tsStopped'), kind: 'warn' as const };
      case 'Missing': return { icon: '❌', title: t('setup.tsMissing'), kind: 'bad' as const };
      case 'Starting': case 'NoState': return { icon: '⏳', title: t('setup.tsStarting', { state: ts.state }), kind: 'wait' as const };
      default: return { icon: '❌', title: t('setup.tsOther', { state: ts.state }) + (ts.error ? ' — ' + ts.error : ''), kind: 'bad' as const };
    }
  });
  const fmtIps = (ips: string[]) => ips.filter(i => i.includes('.')).join(', ');
</script>

<div class="setup">
  <div class="row" style="justify-content: space-between; align-items: baseline;">
    <h2>{t('sys.setup')}</h2>
    {#if st}<span class="muted">AgentVerse OS {st.version}{#if st.done}{' · '}<span class="badge ok">{t('setup.doneBadge')}</span>{/if}</span>{/if}
  </div>
  <p class="muted">{t('setup.intro')}</p>
  {#if error}<div class="card err">{error}</div>{/if}
  {#if msg}<div class="card note">{msg}</div>{/if}

  {#if st}
    <!-- 1. Tailscale -->
    <section class="card step {tsState.kind}">
      <div class="head"><span class="ico">{tsState.icon}</span><div><b>{t('setup.step1')}</b><div class="muted">{tsState.title}</div></div></div>
      {#if st.tailscale.state === 'Running'}
        <div class="muted">{t('setup.ips')} <code>{fmtIps(st.tailscale.ips) || '—'}</code>{#if st.tailscale.tailnet} · tailnet <code>{st.tailscale.tailnet}</code>{/if}{#if st.tailscale.version} · Tailscale {st.tailscale.version}{/if}</div>
        <ul class="checks">
          <li>{st.tailscale.magic_dns ? '✅' : '⚠️'} MagicDNS {st.tailscale.magic_dns ? t('setup.magicDnsOn') : t('setup.magicDnsOff')}{#if !st.tailscale.magic_dns} — <a href={st.tailscale_admin_url} target="_blank" rel="noopener">{t('setup.magicDnsEnable')}</a>{/if}</li>
          <li>{st.tailscale.https_certs ? '✅' : '⚠️'} HTTPS Certificates {st.tailscale.https_certs ? t('setup.httpsOn', { domains: st.tailscale.cert_domains.join(', ') }) : t('setup.httpsOff')}{#if !st.tailscale.https_certs} — <a href={st.tailscale_admin_url} target="_blank" rel="noopener">{t('setup.httpsEnable')}</a>{/if}</li>
          {#each st.tailscale.health as h}<li>⚠️ {h}</li>{/each}
        </ul>
      {:else if st.tailscale.state === 'NeedsLogin'}
        <div class="row">
          <button class="primary" disabled={busy === 'login'} onclick={login}>{busy === 'login' ? t('setup.requestingLink') : authUrl ? t('setup.requestAgain') : t('setup.getLink')}</button>
          {#if authUrl}<a class="auth" href={authUrl} target="_blank" rel="noopener">{t('setup.openLogin')}</a>{/if}
        </div>
        <div class="muted">{t('setup.loginHint')} <code>sudo tailscale up --auth-key=…</code>.</div>
      {:else if st.tailscale.state === 'NeedsMachineAuth'}
        <div class="muted">{t('setup.machineAuth1')} <a href="https://login.tailscale.com/admin/machines" target="_blank" rel="noopener">{t('setup.machineAuthLink')}</a> {t('setup.machineAuth2')}</div>
      {:else if st.tailscale.state === 'Stopped'}
        <div class="row"><button class="primary" disabled={busy === 'login'} onclick={login}>{busy === 'login' ? t('setup.enabling') : t('setup.enableTs')}</button></div>
      {:else if st.tailscale.state === 'Missing'}
        <div class="muted">{t('setup.missing1')} <code>sudo bootstrap/install.sh tailscale</code> ({t('setup.or')} <code>curl -fsSL https://tailscale.com/install.sh | sh</code>) {t('setup.missing2')}</div>
      {/if}
    </section>

    <!-- 2. Адрес и DNS -->
    <section class="card step {st.edge.matches_tailscale && st.edge.reachable ? 'ok' : st.suggested_host ? 'warn' : st.edge.reachable ? 'ok' : 'warn'}">
      <div class="head"><span class="ico">{st.edge.matches_tailscale && st.edge.reachable ? '✅' : st.suggested_host || !st.edge.reachable ? '⚠️' : '✅'}</span>
        <div><b>{t('setup.step2')}</b><div class="muted">{t('setup.edgeAnswers')} <code>https://{st.edge.host}</code>{#if st.edge.alt_hosts.length} · {t('setup.altHosts')} {#each st.edge.alt_hosts as h, i}{i ? ', ' : ''}<a href="https://{h}/">{h}</a>{/each}{/if}</div></div></div>
      <ul class="checks">
        <li>{st.edge.matches_tailscale ? '✅ ' + t('setup.hostMatches') : st.suggested_host ? '⚠️ ' + t('setup.hostMismatch', { suggested: st.suggested_host, host: st.edge.host }) : st.tailscale.state === 'Running' ? '⏳ ' + t('setup.hostPending') : '⏳ ' + t('setup.hostAfterTs')}</li>
        <li>{st.edge.dns_ok === true ? '✅' : st.edge.dns_ok === false ? '❌' : '⏳'} {t('setup.dnsFromServer')} {st.edge.dns_ok === true ? t('setup.dnsOk', { host: st.edge.host }) : st.edge.dns_ok === false ? t('setup.dnsFail', { host: st.edge.host }) : t('setup.notChecked')}</li>
        <li>{device ? (device.ok ? '✅' : '❌') : '·'} {t('setup.fromDevice')} {device ? t(device.ok ? 'setup.deviceOk' : 'setup.deviceFail', { host: device.host }) : t('setup.notChecked')} <button class="mini" onclick={checkDevice}>{t('setup.checkDevice')}</button></li>
      </ul>
      {#if st.suggested_host}
        <div class="row" style="margin-top: 6px;">
          <button class="primary" disabled={busy === 'switch' || !st.tailscale.https_certs || !!switching} onclick={switchHost}>{busy === 'switch' ? t('setup.switching') : t('setup.switchTo', { host: st.suggested_host })}</button>
          <span class="muted small">{t('setup.switchHint')}{#if !st.tailscale.https_certs}; {t('setup.switchNeedsHttps')}{/if}</span>
        </div>
      {/if}
      {#if switching}<div class="note card">{t('setup.switchWait1')} <a href={switching.url}>{switching.url}</a> {t('setup.switchWait2', { tries: switching.tries })}</div>{/if}
    </section>

    <!-- 3. Сертификат -->
    <section class="card step {st.edge.cert_ok === true && !st.edge.tls_internal ? 'ok' : st.edge.cert_ok === false ? 'bad' : 'warn'}">
      <div class="head"><span class="ico">{st.edge.cert_ok === true && !st.edge.tls_internal ? '✅' : st.edge.cert_ok === false ? '❌' : '⚠️'}</span>
        <div><b>{t('setup.step3')}</b>
          <div class="muted">
            {#if st.edge.tls_internal}{t('setup.certInternal')} <a href="/api/ca.crt">{t('setup.certDownload')}</a> {t('setup.certInternal2')}
            {:else if st.edge.cert_ok === true}{t('setup.certOk1')} <code>{st.edge.host}</code> {t('setup.certOk2')}
            {:else if st.edge.cert_ok === false}{t('setup.certFail', { error: st.edge.error ?? '' })}
            {:else}{t('setup.certUnknown', { error: st.edge.error ?? t('setup.noEdgeResponse') })}{/if}
          </div></div></div>
    </section>

    <!-- 4. Первый проект -->
    <section class="card step {st.projects ? 'ok' : 'warn'}">
      <div class="head"><span class="ico">{st.projects ? '✅' : '·'}</span>
        <div><b>{t('setup.step4')}</b><div class="muted">{st.projects ? t('setup.projectsCount', { n: st.projects }) : t('setup.projectIntro')}</div></div></div>
      {#if !st.coder_ok}<div class="muted">⚠️ {t('setup.coderDown')}</div>{/if}
      {#if st.projects}
        <div class="row"><button onclick={() => wm.openSys('projects')}>{t('setup.openProjects')}</button>{#if created}<span class="muted">{t('setup.projectCreated')} <code>{created}</code></span>{/if}</div>
      {:else}
        <div class="row">
          <input placeholder={t('setup.projectPlaceholder')} bind:value={pname} onkeydown={(e) => { if (e.key === 'Enter') createProject(); }} style="min-width: 220px;" />
          <button class="primary" disabled={busy === 'project' || !pname.trim()} onclick={createProject}>{busy === 'project' ? t('setup.creating') : t('setup.createProject')}</button>
          <span class="muted small">{t('setup.projectDefaults')}</span>
        </div>
      {/if}
    </section>

    <!-- 5. Дальше -->
    <section class="card step">
      <div class="head"><span class="ico">→</span><div><b>{t('setup.next')}</b><div class="muted">{t('setup.nextHint')}</div></div></div>
      <ul class="checks">
        <li>{st.apps_installed ? '✅' : '·'} {t('setup.storeApps')} {st.apps_installed ? t('setup.appsInstalled', { n: st.apps_installed }) : t('setup.none')} <button class="mini" onclick={() => wm.openSys('store')}>{t('setup.openStore')}</button></li>
        <li>{st.backups_enabled ? '✅' : '·'} {t('setup.backups')} {st.backups_enabled ? t('setup.backupsOn') : t('setup.backupsOff')} <button class="mini" onclick={() => wm.openSys('settings')}>{t('setup.openBackups')}</button></li>
        <li>· {t('setup.updatesLine')} <button class="mini" onclick={() => wm.openSys('updates')}>{t('setup.openUpdates')}</button></li>
      </ul>
    </section>

    <div class="row" style="justify-content: flex-end; gap: 10px;">
      {#if st.done}<span class="muted">{t('setup.doneNote')}</span><button onclick={reopen} disabled={busy === 'reopen'}>{t('setup.showAgain')}</button>
      {:else}<span class="muted">{t('setup.laterNote')}</span><button class="primary" onclick={finish} disabled={busy === 'done'}>{t('common.done')}</button>{/if}
    </div>
  {:else if !error}
    <div class="muted">{t('setup.gathering')}</div>
  {/if}
</div>

<style>
  .setup { display: flex; flex-direction: column; gap: 10px; }
  .step { display: flex; flex-direction: column; gap: 8px; border-left: 3px solid var(--line); }
  .step.ok { border-left-color: var(--ok); }
  .step.warn { border-left-color: var(--warn); }
  .step.bad { border-left-color: var(--bad); }
  .head { display: flex; gap: 12px; align-items: flex-start; }
  .ico { font-size: 1.3rem; line-height: 1.3; width: 28px; text-align: center; flex: none; }
  .checks { margin: 0; padding-left: 40px; display: flex; flex-direction: column; gap: 4px; list-style: none; }
  .checks li { font-size: .9rem; }
  .mini { padding: 1px 8px; font-size: .8rem; margin-left: 6px; }
  .auth { font-weight: 600; }
  .err { border-color: var(--bad); }
  .note { border-color: var(--accent); }
  .small { font-size: .8rem; }
  input { min-width: 0; }
</style>
