<script lang="ts">
  // Frame of the "Agents" app (docs/agents-ux.md): the section column on the left, the section panel, the main area,
  // the inspector; on the phone — the bottom section bar and a slide-out panel. Data is polled while a section is open.
  import { onMount } from 'svelte';
  import { ui, data, SECTIONS, hardReload } from './lib/state.svelte';
  import { prefs } from './lib/prefs.svelte';
  import { t, lang } from './lib/i18n';
  import { api, agents } from './lib/api';
  import { toast } from './lib/toast.svelte';
  import { flushOutbox, outboxVersion } from './lib/outbox';
  import Icon from './lib/Icon.svelte';
  import MenuHost from './lib/ui/MenuHost.svelte';
  import Toasts from './lib/ui/Toasts.svelte';
  import Palette from './lib/ui/Palette.svelte';
  import Sheet from './lib/ui/Sheet.svelte';
  import Rename from './lib/ui/Rename.svelte';
  import Work from './lib/sections/Work.svelte';
  import Inbox from './lib/sections/Inbox.svelte';
  import Models from './lib/sections/Models.svelte';
  import Settings from './lib/sections/Settings.svelte';
  import Routines from './lib/sections/Routines.svelte';
  import ChatSection from './lib/sections/ChatSection.svelte';
  let palette = $state(false);
  // Phone: the on-screen keyboard does not shrink 100dvh (iOS) but covers the bottom — the app height follows
  // visualViewport, with the keyboard open the bottom section bar is hidden and the page is held at zero (iOS scrolls it
  // to the focus).
  onMount(() => {
    const vv = window.visualViewport; if (!vv) return;
    const root = document.documentElement;
    const apply = () => {
      // When the page is zoomed, visualViewport shrinks without an on-screen keyboard: do not change the layout and do not
      // interfere with scrolling.
      if (vv.scale > 1.05) { root.classList.remove('kbd'); root.style.removeProperty('--app-h'); return; }
      const h = Math.round(vv.height); const kbd = matchMedia('(max-width: 599px)').matches && h < window.innerHeight - 120;
      root.style.setProperty('--app-h', `${h}px`); root.classList.toggle('kbd', kbd);
      if (kbd && (window.scrollY || vv.offsetTop)) window.scrollTo(0, 0);
    };
    vv.addEventListener('resize', apply); vv.addEventListener('scroll', apply); apply();
    return () => { vv.removeEventListener('resize', apply); vv.removeEventListener('scroll', apply); root.classList.remove('kbd'); root.style.removeProperty('--app-h'); };
  });
  let sheet = $state(false);
  const withPanel = $derived(ui.section === 'work' || ui.section === 'chat');
  const waiting = $derived(data.waiting.length);
  onMount(() => {
    prefs.apply(); data.loadHealth(); ui.loadLayout(); data.loadAccounts();
    // the Desktop language (the core appearance) is the
    // default for "auto"
    api.desktopSettings().then(s => lang.setDesktop(s?.appearance?.language)).catch(() => {});
    const h = setInterval(() => data.loadHealth(), 30000);
    // UI freshness: a long-lived tab (phone) otherwise shows an old build for weeks
    data.checkUiFresh(); const f = setInterval(() => data.checkUiFresh(), 300000);
    data.loadLimits();   // spending limits (§8.3): the composer bar is computed from the hub snapshot
    data.loadBrowser();   // dev server preview (§4.9): per-workspace flags so the tab appears only for enabled ones
    const vis = () => { if (document.visibilityState === 'visible') data.checkUiFresh(); }; document.addEventListener('visibilitychange', vis);
    // install as an app: Chrome gives beforeinstallprompt, we defer it until the button in Settings
    const bip = (e: Event) => { e.preventDefault(); data.installPrompt = e as Event & { prompt: () => Promise<void> }; }; window.addEventListener('beforeinstallprompt', bip);
    const inst = () => { data.installPrompt = null; data.installed = true; }; window.addEventListener('appinstalled', inst);
    // §9.9: persistent storage (the conversation cache and tree snapshot are not purged), outbox without a
    // network, worker messages
    if (data.installed) data.persistStorage();
    const online = () => { flushOutbox((it) => toast(t('outbox.sent', { n: 1 }), 'info')).then(n => { if (n) data.loadWs(); }); }; window.addEventListener('online', online);
    const swMsg = (e: MessageEvent) => {
      const m = e.data as { kind?: string; url?: string; data?: { project?: string; id?: string }; sent?: number } | null; if (!m) return;
      if (m.kind === 'open' && m.data?.project && m.data?.id) { ui.sel = { project: m.data.project, kind: 'session', id: m.data.id }; ui.section = 'work'; ui.view = 'thread'; }
      else if (m.kind === 'open' && m.url) ui.applyUrl(location.origin + m.url);
      else if (m.kind === 'outbox') { if (m.sent) { toast(t('outbox.sent', { n: m.sent }), 'info'); data.loadWs(); } outboxVersion.bump(); }
      else if (m.kind === 'permission-answered') data.loadInbox();
    };
    navigator.serviceWorker?.addEventListener('message', swMsg); try { navigator.serviceWorker?.startMessages(); } catch {}   // without startMessages the worker message queue does not open
    // §9.10: the browser has a push subscription but the server does not know it (re-subscription, database cleanup) —
    // register anew
    navigator.serviceWorker?.ready.then(reg => reg.pushManager.getSubscription()).then(async sub => {
      if (!sub) return; const info = await agents.pushInfo();
      if (!info.subscriptions.some(s => s.endpoint === sub.endpoint)) { const j = sub.toJSON(); await agents.pushSubscribe({ endpoint: sub.endpoint, keys: { p256dh: j.keys?.p256dh ?? '', auth: j.keys?.auth ?? '' }, ua: navigator.userAgent.replace(/\(.*?\)/g, '').replace(/\s+/g, ' ').trim().slice(0, 100) }); }
    }).catch(() => {});
    if (ui.shareId) { const sid = ui.shareId; ui.shareId = null; agents.shareTake(sid).then(sh => { data.pendingShare = { text: [sh.title, sh.text, sh.url].filter(Boolean).join('\n'), files: sh.files ?? [] }; if (!ui.sel || ui.section !== 'work') toast(t('share.pick'), 'info'); else ui.view = 'thread'; }).catch(() => toast(t('share.gone'), 'bad')); }
    return () => { clearInterval(h); clearInterval(f); document.removeEventListener('visibilitychange', vis); window.removeEventListener('beforeinstallprompt', bip); window.removeEventListener('appinstalled', inst); window.removeEventListener('online', online); navigator.serviceWorker?.removeEventListener('message', swMsg); };
  });
  // section data polling
  $effect(() => {
    const s = ui.section; const timers: ReturnType<typeof setInterval>[] = [];
    if (s === 'work' || s === 'inbox' || s === 'chat') {
      // the hub (ADR-007) pushes the tree; polling every 5 s — only while the hub is not
      // connected
      const refresh = () => data.loadWs().then(() => { if (!data.wsError) ui.prune(); });
      data.connectHub(); if (!data.hubLive) refresh(); timers.push(setInterval(() => { if (!data.hubLive && !data.wsLoading) refresh(); }, 5000));
    }
    // "Inbox" and "Chat" (slice 60): with the hub the data arrives by push (inbox / chat / the chats signal), polling every 5 s —
    // only without the hub
    if (s === 'inbox') { if (!data.hubLive || !data.hubInbox) data.loadInbox(); timers.push(setInterval(() => { if (!data.hubLive && !data.inboxLoading) data.loadInbox(); }, 5000)); }
    if (s === 'chat') { if (data.hubLive && data.hubChat) data.loadChats(); else data.loadChat(); timers.push(setInterval(() => { if (!data.hubLive && !data.chatLoading) data.loadChat(); }, 5000)); }
    if (s === 'routines') { data.loadRoutines(); data.loadWs(); data.loadAccounts(); timers.push(setInterval(() => { if (!data.routinesLoading) data.loadRoutines(); }, 10000)); }
    if (s === 'models') { data.loadModels(ui.period); timers.push(setInterval(() => { if (!data.modelsLoading) data.loadModels(ui.period); }, 60000)); }
    if (s === 'settings') { data.loadAccounts(); data.loadWs(); }
    return () => timers.forEach(clearInterval);
  });
  $effect(() => { ui.persist(); ui.saveLayout(); });
  $effect(() => { const m = matchMedia('(max-width: 1099px)'); const f = () => { if (m.matches) ui.inspector = false; }; m.addEventListener('change', f); return () => m.removeEventListener('change', f); });
  function keys(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); sheet = false; palette = !palette; return; }
    if (e.key === 'Escape') { palette = false; sheet = false; if (ui.popen) ui.popen = false; }
  }
</script>

<svelte:window onkeydown={keys} />

<div class="app" class:insp={ui.inspector && ui.section === 'work'} class:nopanel={!withPanel} class:popen={ui.popen} style={ui.sideW && !ui.isNarrow ? `--insp-w: ${ui.sideW}px` : undefined}>
  <nav class="rail" aria-label={t('nav.sections')}>
    <img class="logo" src="/agents/icon.svg" alt="AgentVerse OS" title="AgentVerse OS · {t('sec.work')}" width="32" height="32" decoding="async" />
    {#each SECTIONS as s (s.id)}
      <button class="nav" class:active={ui.section === s.id} data-tip={s.name} title={s.name} aria-label={s.name} aria-current={ui.section === s.id ? 'page' : undefined} onclick={() => { ui.section = s.id; ui.popen = false; }}><Icon name={s.icon} size="l" />{#if s.id === 'inbox' && waiting}<span class="count warn">{waiting}</span>{/if}</button>
    {/each}
    <div class="bottom">
      <button class="nav" data-tip={prefs.effective === 'dark' ? t('set.light') : t('set.dark')} title={prefs.effective === 'dark' ? t('set.light') : t('set.dark')} aria-label={prefs.effective === 'dark' ? t('set.light') : t('set.dark')} onclick={() => prefs.toggleTheme()}><Icon name={prefs.effective === 'dark' ? 'sun' : 'moon'} size="l" /></button>
      <button class="nav" data-tip={t('top.palette')} title={t('top.palette')} aria-label={t('top.palette')} onclick={() => (palette = true)}><Icon name="search" size="l" /></button>
      <span class="health" class:bad={data.health ? !data.health.cloudd : false} title={data.health?.cloudd ? `${t('panel.core')} ok` : `${t('panel.core')}: ${t('api.offline')}`}></span>
    </div>
  </nav>

  {#if ui.section === 'work'}<Work onpalette={() => (palette = true)} />
  {:else if ui.section === 'inbox'}<Inbox />
  {:else if ui.section === 'models'}<Models />
  {:else if ui.section === 'settings'}<Settings />
  {:else if ui.section === 'routines'}<Routines />
  {:else}<ChatSection />{/if}

  <div class="scrim" onclick={() => (ui.popen = false)} role="presentation"></div>
  <nav class="tabbar" aria-label={t('nav.sections')}>
    {#each SECTIONS.filter(s => !['routines', 'settings'].includes(s.id)) as s (s.id)}
      <button class="tb" class:active={ui.section === s.id} aria-current={ui.section === s.id ? 'page' : undefined} onclick={() => { ui.section = s.id; ui.popen = false; }}><Icon name={s.icon} size="l" /><span>{s.name}</span>{#if s.id === 'inbox' && waiting}<span class="count warn">{waiting}</span>{/if}</button>
    {/each}
    <button class="tb" class:active={['routines', 'settings'].includes(ui.section)} aria-label={t('sec.more')} aria-haspopup="dialog" aria-expanded={sheet} onclick={() => (sheet = true)}><Icon name="more" size="l" /><span>{t('sec.more')}</span></button>
  </nav>
</div>
{#if data.uiStale}<div class="upd-banner" role="status"><span>{t('upd.banner')}</span><button class="sm primary" onclick={hardReload}>{t('upd.reload')}</button><button class="icon xs ghost" aria-label={t('common.close')} onclick={() => (data.uiStale = false)}>×</button></div>{/if}
<MenuHost />
<Toasts />
{#if palette}<Palette onclose={() => (palette = false)} />{/if}
{#if sheet}<Sheet onclose={() => (sheet = false)} onpalette={() => (palette = true)} />{/if}
{#if ui.rename}<Rename r={ui.rename} onclose={() => (ui.rename = null)} />{/if}
