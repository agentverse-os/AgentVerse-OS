<!-- Settings → Notifications → "Channels" (docs/agents-ux.md §9.12.1): Telegram, Slack, Discord, a plain webhook. A secret
     (a bot token, a webhook address) is typed here and never shown again — the card keeps the bot's name, the chat, the
     host and a tail. A channel says nothing until it is bound; the bindings are made from a conversation, a workspace or a
     routine and listed here. -->
<script lang="ts">
  import { agents, type NotifyBinding, type NotifyChannel, type NotifyKind, type NotifyLogRow } from './api';
  import { notify, kindName, kindIcon } from './notify.svelte';
  import { data, titleOf } from './state.svelte';
  import { menu, type MenuItem } from './menu.svelte';
  import { toast } from './toast.svelte';
  import { t, fmt } from './i18n';
  import { onMount } from 'svelte';
  import Icon from './Icon.svelte';

  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  onMount(() => { void notify.load(); });

  // ---- a new channel ----
  type Draft = { kind: NotifyKind; name: string; url: string; token: string; slackChannel: string; internal: boolean; signed: boolean; protect: boolean; buttons: boolean;
    /** Telegram: the handshake — the token checked, the code to send, the chat it came from */
    pending: string; bot: string; code: string; chat: string; chatType: string; notFound: boolean };
  const blank = (kind: NotifyKind): Draft => ({ kind, name: '', url: '', token: '', slackChannel: '', internal: false, signed: true, protect: true, buttons: false, pending: '', bot: '', code: '', chat: '', chatType: '', notFound: false });
  let draft = $state<Draft | null>(null);
  let busy = $state(false);
  let err = $state('');
  /** The signing key of a webhook just made: shown once */
  let shownKey = $state<{ name: string; key: string } | null>(null);
  const kindTitle = (k: NotifyKind) => (k === 'slack' ? t('ntf.kSlackHook') : k === 'slack_bot' ? t('ntf.kSlackBot') : k === 'webhook' ? t('ntf.kWebhook') : kindName(k));
  function addMenu(el: HTMLElement) {
    menu.show(el, (['telegram', 'slack', 'slack_bot', 'discord', 'webhook'] as NotifyKind[]).map(k => ({ icon: kindIcon(k), label: kindTitle(k), on: () => { draft = blank(k); err = ''; shownKey = null; } })));
  }
  async function tgCheck() {
    if (!draft || busy) return; busy = true; err = '';
    try { const r = await agents.notifyTgBegin(draft.token.trim()); draft.pending = r.pending; draft.bot = r.bot; draft.code = r.code; draft.chat = ''; draft.notFound = false; }
    catch (e) { err = msg(e); } finally { busy = false; }
  }
  async function tgFind() {
    if (!draft?.pending || busy) return; busy = true; err = '';
    try { const r = await agents.notifyTgFind(draft.pending); draft.notFound = !r.found; if (r.found) { draft.chat = r.chat_title ?? ''; draft.chatType = r.chat_type ?? 'private'; if (!draft.name.trim()) draft.name = draft.chat; } }
    catch (e) { err = msg(e); } finally { busy = false; }
  }
  const ready = $derived(!!draft && (draft.kind === 'telegram' ? !!draft.chat : draft.kind === 'slack_bot' ? !!draft.token.trim() && !!draft.slackChannel.trim() : !!draft.url.trim()));
  async function save() {
    const d = draft; if (!d || !ready || busy) return; busy = true; err = '';
    try {
      const body: Record<string, unknown> = { kind: d.kind, name: d.name.trim() };
      if (d.kind === 'telegram') Object.assign(body, { pending: d.pending, protect: d.protect, buttons: d.buttons });
      else if (d.kind === 'slack_bot') Object.assign(body, { token: d.token.trim(), slack_channel: d.slackChannel.trim() });
      else Object.assign(body, { url: d.url.trim(), ...(d.kind === 'webhook' ? { internal: d.internal, signed: d.signed } : {}) });
      const c = await agents.notifyChannelCreate(body);
      draft = null;
      if (c.signing_key) shownKey = { name: c.name, key: c.signing_key };
      await notify.load();
      await test(c);
    } catch (e) { err = msg(e); } finally { busy = false; }
  }

  // ---- the channels ----
  let testing = $state('');
  async function test(c: NotifyChannel) {
    if (testing) return; testing = c.id;
    try { const r = await agents.notifyChannelTest(c.id); if (r.ok) toast(t('ntf.testOk', { name: c.name }), 'info'); else toast(t('ntf.testFail', { e: r.error ?? '' }), 'bad'); }
    catch (e) { toast(t('ntf.testFail', { e: msg(e) }), 'bad'); }
    finally { testing = ''; void notify.load(); if (logOf === c.id) void openLog(c, true); }
  }
  async function upd(c: NotifyChannel, patch: Record<string, unknown>) {
    try { await agents.notifyChannelUpdate(c.id, patch); await notify.load(); return true; } catch (e) { toast(msg(e), 'bad'); return false; }
  }
  /** What is being edited in place under a channel: its name, or a secret being replaced */
  let edit = $state<{ id: string; what: 'name' | 'secret'; value: string } | null>(null);
  async function saveEdit(c: NotifyChannel) {
    const e = edit; if (!e || !e.value.trim()) return;
    const ok = await upd(c, e.what === 'name' ? { name: e.value.trim() } : c.kind === 'slack_bot' ? { token: e.value.trim() } : { url: e.value.trim() });
    if (ok) { edit = null; toast(t('ntf.saved'), 'info'); }
  }
  const stand = (c: NotifyChannel) => c.bindings.find(b => b.scope === 'all');
  async function toggleStand(c: NotifyChannel) {
    try { const b = stand(c); if (b) await notify.unbind(b.id); else await notify.bind({ channel: c.id, scope: 'all', away_only: false }); } catch (e) { toast(msg(e), 'bad'); }
  }
  async function unbind(b: NotifyBinding) { try { await notify.unbind(b.id); } catch (e) { toast(msg(e), 'bad'); } }
  async function remove(c: NotifyChannel) {
    if (!window.confirm(t('ntf.removeConfirm', { name: c.name }))) return;
    try { await agents.notifyChannelDelete(c.id); toast(t('ntf.removed'), 'info'); if (logOf === c.id) logOf = ''; await notify.load(); } catch (e) { toast(msg(e), 'bad'); }
  }
  function more(el: HTMLElement, c: NotifyChannel) {
    const items: MenuItem[] = [{ icon: 'edit', label: t('ntf.rename'), on: () => (edit = { id: c.id, what: 'name', value: c.name }) }];
    if (c.kind === 'telegram') items.push({ label: t('ntf.protect'), checked: c.protect, on: () => void upd(c, { protect: !c.protect }) }, { label: t('ntf.buttons'), checked: c.buttons, hint: t('ntf.tgButtonsHint'), on: () => void upd(c, { buttons: !c.buttons }) });
    else items.push({ icon: 'key', label: c.kind === 'slack_bot' ? t('ntf.replaceToken') : t('ntf.replace'), on: () => (edit = { id: c.id, what: 'secret', value: '' }) });
    if (c.kind === 'webhook') items.push({ label: t('ntf.internal'), checked: c.internal, on: () => void upd(c, { internal: !c.internal }) });
    items.push({ label: t('ntf.system'), checked: !!stand(c), on: () => void toggleStand(c) });
    if (data.projects.length) items.push({ header: true, label: t('ntf.bindWs') }, ...data.projects.map(p => ({ icon: 'grid', label: p.name, checked: c.bindings.some(b => b.scope === 'workspace' && b.project === p.name), on: () => notify.open({ scope: 'workspace', project: p.name, title: p.name }) })));
    items.push({ sep: true }, { icon: 'trash', label: t('ntf.remove'), danger: true, on: () => void remove(c) });
    menu.show(el, items);
  }
  /** Where a channel sends, in words: never the secret itself */
  const target = (c: NotifyChannel) => (c.kind === 'telegram' ? `${c.bot} → ${c.chat_title || '—'}` : c.kind === 'slack_bot' ? `${c.slack_channel} · …${c.tail}` : `${c.host}${c.tail ? ` · …${c.tail}` : ''}`);
  function bindingWords(b: NotifyBinding): string {
    const what = b.scope === 'session' ? t('ntf.bSession', { title: data.infoOf({ project: b.project, kind: 'session', id: b.session }) ? titleOf({ project: b.project, kind: 'session', id: b.session }) : b.session.slice(0, 8), project: b.project })
      : b.scope === 'workspace' ? t('ntf.bWorkspace', { project: b.project })
      : b.scope === 'routine' ? t('ntf.bRoutine', { title: data.routines.find(r => r.id === b.routine)?.name ?? b.routine })
      : t('ntf.bAll');
    const marks = [...(b.scope === 'session' && b.when === 'once' ? [t('ntf.once')] : []), ...(b.content === 'brief' ? [t('ntf.cBrief')] : b.content === 'full' ? [t('ntf.cFull')] : []), ...(b.reports ? [t('ntf.reports')] : []), ...(b.away_only ? [t('ntf.away')] : [])];
    return marks.length ? `${what} · ${marks.join(' · ')}` : what;
  }
  const when = (ts: number) => fmt.date(ts * 1000, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
  const eventName = (e: string) => t(({ done: 'ntf.evDone', waiting: 'ntf.evWaiting', error: 'ntf.evError', routine: 'ntf.evRoutine', system: 'ntf.evSystem', report: 'ntf.evReport', share: 'ntf.evShare', test: 'ntf.evTest' } as Record<string, string>)[e] ?? 'ntf.evSystem');
  const statusName = (s: string) => (s === 'sent' ? t('ntf.stSent') : s === 'failed' ? t('ntf.stFailed') : t('ntf.stPending'));

  // ---- the delivery journal of a channel ----
  let logOf = $state('');
  let logRows = $state<NotifyLogRow[]>([]);
  let logBusy = $state(false);
  async function openLog(c: NotifyChannel, keep = false) {
    if (!keep && logOf === c.id) { logOf = ''; return; }
    logOf = c.id; logBusy = true;
    try { logRows = (await agents.notifyChannelLog(c.id)).log; } catch (e) { logRows = []; toast(msg(e), 'bad'); } finally { logBusy = false; }
  }
  async function copy(text: string) { try { await navigator.clipboard?.writeText(text); toast(t('ntf.copied'), 'info'); } catch {} }
</script>

<div class="card-s fs ntf-card" style="margin-top: 14px">
  <div class="ntf-top"><div><h3>{t('ntf.title')}</h3><p>{t('ntf.hint')}</p></div><button class="sm ntf-add" aria-haspopup="menu" onclick={(e) => addMenu(e.currentTarget)}><Icon name="plus" size="s" /> {t('ntf.add')}</button></div>
  {#if notify.error}<div class="load-error" role="alert"><span>{notify.error}</span><button class="sm" disabled={notify.loading} onclick={() => notify.load()}>{t('sx.retry')}</button></div>{/if}
  {#if !notify.loaded && notify.loading}<p class="muted sm" role="status">{t('common.loading')}</p>{/if}

  {#if shownKey}
    <div class="ntf-key" role="status"><span>{t('ntf.signingKey')}</span><code>{shownKey.key}</code><button class="xs" onclick={() => copy(shownKey!.key)}><Icon name="copy" size="s" /> {t('set.copy')}</button><button class="xs ghost" aria-label={t('ntf.cancel')} onclick={() => (shownKey = null)}><Icon name="x" size="s" /></button></div>
  {/if}

  {#if draft}
    <form class="ntf-new" data-kind={draft.kind} onsubmit={(e) => { e.preventDefault(); void save(); }}>
      <b>{t('ntf.newTitle', { kind: kindTitle(draft.kind) })}</b>
      {#if draft.kind === 'telegram'}
        <p class="muted sm">{t('ntf.tgStep1')}</p>
        <div class="ntf-line"><input class="sm mono ntf-token" type="password" autocomplete="off" aria-label={t('ntf.tgToken')} placeholder="123456789:AA…" bind:value={draft.token} disabled={busy || !!draft.pending} /><button type="button" class="sm" disabled={busy || !draft.token.trim() || !!draft.pending} onclick={tgCheck}>{t('ntf.tgCheck')}</button>{#if draft.bot}<span class="chip ok ntf-bot">{draft.bot}</span>{/if}</div>
        {#if draft.pending}
          <p class="muted sm">{t('ntf.tgStep2', { bot: draft.bot })}</p>
          <div class="ntf-line"><code class="ntf-code">/start {draft.code}</code><button type="button" class="xs" onclick={() => copy(`/start ${draft!.code}`)}><Icon name="copy" size="s" /> {t('set.copy')}</button><button type="button" class="sm ntf-find" disabled={busy} onclick={tgFind}>{t('ntf.tgFind')}</button></div>
          {#if draft.chat}<p class="sm ntf-found">✓ {t('ntf.tgFound', { title: draft.chat, type: draft.chatType === 'private' ? t('ntf.tgPrivate') : t('ntf.tgGroup') })}</p>{#if draft.chatType !== 'private'}<p class="sm ntf-warn">{t('ntf.tgGroupWarn')}</p>{/if}
          {:else if draft.notFound}<p class="muted sm ntf-notfound">{t('ntf.tgNotFound')}</p>{/if}
        {/if}
        {#if draft.chat}
          <p class="muted sm">{t('ntf.tgStep3')}</p>
          <div class="ntf-line"><input class="sm ntf-name" aria-label={t('ntf.name')} placeholder={t('ntf.name')} maxlength="60" bind:value={draft.name} /></div>
          <label class="ntf-check"><input type="checkbox" bind:checked={draft.protect} /> {t('ntf.protect')}</label>
          <label class="ntf-check"><input type="checkbox" bind:checked={draft.buttons} /> {t('ntf.buttons')}</label>
          {#if draft.buttons}<p class="muted xs">{t('ntf.tgButtonsHint')}</p>{/if}
        {/if}
      {:else if draft.kind === 'slack_bot'}
        <p class="muted sm">{t('ntf.slackBotHint')}</p>
        <div class="ntf-line"><input class="sm mono ntf-token" type="password" autocomplete="off" aria-label={t('ntf.slackToken')} placeholder={t('ntf.slackToken')} bind:value={draft.token} /></div>
        <div class="ntf-line"><input class="sm mono ntf-slack-channel" aria-label={t('ntf.slackChannel')} placeholder={t('ntf.slackChannelPh')} bind:value={draft.slackChannel} /><input class="sm ntf-name" aria-label={t('ntf.name')} placeholder={t('ntf.name')} maxlength="60" bind:value={draft.name} /></div>
      {:else}
        {#if draft.kind === 'slack'}<p class="muted sm">{t('ntf.slackHookHint')}</p>{/if}
        <div class="ntf-line"><input class="sm mono ntf-url" type="password" autocomplete="off" aria-label={t('ntf.url')} placeholder={draft.kind === 'slack' ? 'https://hooks.slack.com/services/…' : draft.kind === 'discord' ? 'https://discord.com/api/webhooks/…' : 'https://…'} bind:value={draft.url} /></div>
        <div class="ntf-line"><input class="sm ntf-name" aria-label={t('ntf.name')} placeholder={t('ntf.name')} maxlength="60" bind:value={draft.name} /></div>
        {#if draft.kind === 'webhook'}
          <label class="ntf-check"><input type="checkbox" bind:checked={draft.internal} /> {t('ntf.internal')}</label>
          <label class="ntf-check"><input type="checkbox" bind:checked={draft.signed} /> {t('ntf.signed')} <span class="muted xs">· {t('ntf.signedHint')}</span></label>
        {/if}
      {/if}
      <p class="muted xs">{t('ntf.secretHint')}</p>
      {#if err}<p class="err ntf-err" role="alert">{err}</p>{/if}
      <div class="ntf-acts"><span class="sp"></span><button type="button" class="sm" disabled={busy} onclick={() => { draft = null; err = ''; }}>{t('ntf.cancel')}</button><button type="submit" class="sm primary ntf-save" disabled={busy || !ready}>{busy ? t('sx.saving') : t('ntf.saveTest')}</button></div>
    </form>
  {/if}

  {#if notify.loaded && !notify.channels.length && !draft}<p class="muted sm ntf-empty">{t('ntf.none')}</p>{/if}
  <div class="ntf-list">
    {#each notify.channels as c (c.id)}
      <div class="ntf-ch" class:off={!c.enabled} data-id={c.id} data-kind={c.kind}>
        <div class="ntf-ch-head">
          <span class="ntf-ch-name"><Icon name={kindIcon(c.kind)} size="s" /> <b>{kindName(c.kind)}</b><span>· {c.name}</span></span>
          <span class="ntf-ch-acts"><button class="switch ntf-switch" class:on={c.enabled} aria-pressed={c.enabled} aria-label={t('ntf.toggle')} title={t('ntf.toggle')} onclick={() => upd(c, { enabled: !c.enabled })}></button><button class="xs ntf-test" disabled={!!testing} onclick={() => test(c)}>{testing === c.id ? t('ntf.testing') : t('ntf.test')}</button><button class="icon xs ghost ntf-more" aria-label={t('ntf.more')} title={t('ntf.more')} aria-haspopup="menu" onclick={(e) => more(e.currentTarget, c)}><Icon name="more" size="s" /></button></span>
        </div>
        <div class="muted sm ntf-ch-target">{target(c)}</div>
        {#if edit?.id === c.id}
          <form class="ntf-line ntf-edit" onsubmit={(e) => { e.preventDefault(); void saveEdit(c); }}>
            <!-- svelte-ignore a11y_autofocus -->
            <input class="sm" class:mono={edit.what === 'secret'} type={edit.what === 'secret' ? 'password' : 'text'} autocomplete="off" autofocus maxlength={edit.what === 'name' ? 60 : 2000} aria-label={edit.what === 'name' ? t('ntf.renamePrompt') : c.kind === 'slack_bot' ? t('ntf.slackToken') : t('ntf.url')} placeholder={edit.what === 'name' ? t('ntf.renamePrompt') : c.kind === 'slack_bot' ? t('ntf.slackToken') : t('ntf.url')} bind:value={edit.value} />
            <button type="submit" class="xs primary" disabled={!edit.value.trim()}>{t('common.save')}</button><button type="button" class="xs" onclick={() => (edit = null)}>{t('ntf.cancel')}</button>
          </form>
          {#if edit.what === 'secret'}<p class="muted xs">{t('ntf.replaceHint')}</p>{/if}
        {/if}
        <div class="sm ntf-ch-binds" class:muted={!c.bindings.length}>{#if c.bindings.length}<span class="muted">{t('ntf.listens')}</span>{#each c.bindings as b (b.id)}<span class="ntf-bind" data-scope={b.scope}><span>{bindingWords(b)}</span><button class="icon xs ghost ntf-unbind" aria-label={t('ntf.unbind')} title={t('ntf.unbind')} onclick={() => unbind(b)}><Icon name="x" size="s" /></button></span>{/each}{:else}{t('ntf.silent')}{/if}</div>
        <div class="muted sm ntf-ch-stat"><span>{t('ntf.sentToday', { n: c.sent_today })}{#if c.last}{' '}· {#if c.last.status === 'failed'}<span class="ntf-bad">{t('ntf.lastFail', { e: c.last.error ?? '' })}</span>{:else}{t('ntf.last', { when: when(c.last.at) })} ✓{/if}{:else}{' '}· {t('ntf.neverSent')}{/if}</span><button class="xs ghost ntf-log-btn" aria-expanded={logOf === c.id} onclick={() => openLog(c)}>{t('ntf.log')} <Icon name="chevD" size="s" /></button></div>
        {#if logOf === c.id}
          <div class="ntf-log" role="list">
            {#if logBusy && !logRows.length}<p class="muted sm" role="status">{t('common.loading')}</p>{:else if !logRows.length}<p class="muted sm">{t('ntf.logEmpty')}</p>{/if}
            {#each logRows as r, i (i)}
              <div class="ntf-log-row" class:failed={r.status === 'failed'} role="listitem"><span class="muted">{when(r.at)}</span><span>{eventName(r.event)}{#if r.files}{' '}· {t('ntf.logFiles', { n: r.files })}{/if}</span><span class="ntf-log-st">{statusName(r.status)}{#if r.tries > 1}{' '}· {t('ntf.logTries', { n: r.tries })}{/if}</span>{#if r.title || r.error}<span class="ntf-log-text">{r.title}{#if r.error}{r.title ? ' — ' : ''}<span class="ntf-bad">{r.error}</span>{/if}</span>{/if}</div>
            {/each}
          </div>
        {/if}
      </div>
    {/each}
  </div>
</div>

<style>
  .ntf-top { display: flex; gap: 12px; align-items: flex-start; justify-content: space-between; }
  .ntf-top > div { display: grid; gap: 4px; min-width: 0; }
  .ntf-top h3 { font-size: 14px; font-weight: 600; }
  .ntf-top p { color: var(--fg-3); font-size: 12.5px; }
  .ntf-add { flex: none; }
  .ntf-list { display: grid; gap: 8px; }
  .ntf-ch, .ntf-new { display: grid; gap: 6px; padding: 10px 12px; border: 1px solid var(--line-1); border-radius: var(--r-2); background: var(--bg-3); min-width: 0; }
  .ntf-ch.off .ntf-ch-name { color: var(--fg-3); }
  .ntf-ch-head { display: flex; gap: 8px; align-items: center; justify-content: space-between; flex-wrap: wrap; }
  .ntf-ch-name { display: inline-flex; gap: 6px; align-items: center; min-width: 0; font-size: 13px; }
  /* the kind of a channel is our own short word and stays whole; the name next to it is the owner's and wraps */
  .ntf-ch-name b, .ntf-ch-name :global(.i) { flex: none; }
  .ntf-ch-name > span { min-width: 0; overflow-wrap: anywhere; }
  .ntf-ch-acts { display: inline-flex; gap: 6px; align-items: center; margin-left: auto; }
  /* names of chats, hosts and the words of a service's refusal are not ours: they wrap wherever they have to */
  .ntf-ch-name, .ntf-ch-target, .ntf-bind > span, .ntf-ch-stat > span, .ntf-log-text, .ntf-err, .ntf-found, .ntf-key code, .ntf-new b { overflow-wrap: anywhere; min-width: 0; }
  .ntf-ch-binds { display: flex; flex-wrap: wrap; gap: 2px 10px; align-items: center; min-width: 0; }
  .ntf-bind { display: inline-flex; align-items: center; gap: 2px; min-width: 0; }
  .ntf-ch-stat { display: flex; gap: 8px; align-items: center; justify-content: space-between; }
  .ntf-bad { color: var(--bad); }
  .ntf-warn { color: var(--warn); }
  .ntf-found { color: var(--ok); }
  .ntf-line { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; min-width: 0; }
  .ntf-line input { flex: 1 1 220px; min-width: 0; }
  .ntf-code { font-family: var(--mono); font-size: 13px; padding: 4px 8px; border-radius: var(--r-1); background: var(--bg-4); user-select: all; }
  .ntf-check { display: inline-flex; gap: 8px; align-items: center; font-size: 13px; cursor: pointer; flex-wrap: wrap; }
  .ntf-acts { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .ntf-key { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; padding: 10px 12px; border: 1px solid rgb(var(--accent-rgb) / .35); border-radius: var(--r-2); font-size: 12.5px; }
  .ntf-key code { font-family: var(--mono); user-select: all; }
  .ntf-log { display: grid; gap: 4px; max-height: 280px; overflow: auto; padding-top: 4px; border-top: 1px solid var(--line-1); }
  .ntf-log-row { display: grid; grid-template-columns: auto auto 1fr; gap: 2px 10px; font-size: 12px; align-items: baseline; }
  .ntf-log-st { justify-self: end; color: var(--fg-3); }
  .ntf-log-row.failed .ntf-log-st { color: var(--bad); }
  .ntf-log-text { grid-column: 1 / -1; color: var(--fg-2); }
  @media (pointer: coarse) { .ntf-line input, .ntf-acts button, .ntf-add { min-height: 40px; } }
</style>
