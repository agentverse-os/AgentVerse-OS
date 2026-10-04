<script lang="ts">
  // Каркас приложения «Агенты» (docs/agents-ux.md): колонка разделов слева, панель раздела, главная область, инспектор;
  // на телефоне — нижняя панель разделов и выдвижная панель. Данные опрашиваются, пока раздел открыт.
  import { onMount } from 'svelte';
  import { ui, data, SECTIONS } from './lib/state.svelte';
  import { prefs } from './lib/prefs.svelte';
  import { t, lang } from './lib/i18n';
  import { api } from './lib/api';
  import Icon from './lib/Icon.svelte';
  import MenuHost from './lib/ui/MenuHost.svelte';
  import Toasts from './lib/ui/Toasts.svelte';
  import Palette from './lib/ui/Palette.svelte';
  import Sheet from './lib/ui/Sheet.svelte';
  import Work from './lib/sections/Work.svelte';
  import Inbox from './lib/sections/Inbox.svelte';
  import Models from './lib/sections/Models.svelte';
  import Settings from './lib/sections/Settings.svelte';
  import Routines from './lib/sections/Routines.svelte';
  import ChatSection from './lib/sections/ChatSection.svelte';
  let palette = $state(false);
  // Телефон: экранная клавиатура не уменьшает 100dvh (iOS), а накрывает низ — высоту приложения ведём по visualViewport,
  // при открытой клавиатуре прячем нижнюю панель разделов и держим страницу на нуле (iOS уводит её к фокусу).
  onMount(() => {
    const vv = window.visualViewport; if (!vv) return;
    const root = document.documentElement;
    const apply = () => {
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
    // язык Desktop (оформление ядра) — умолчание для «auto»
    api.desktopSettings().then(s => lang.setDesktop(s?.appearance?.language)).catch(() => {});
    const h = setInterval(() => data.loadHealth(), 30000);
    return () => clearInterval(h);
  });
  // опрос данных раздела
  $effect(() => {
    const s = ui.section; const timers: ReturnType<typeof setInterval>[] = [];
    if (s === 'work' || s === 'inbox' || s === 'chat') { data.loadWs().then(() => ui.prune()); timers.push(setInterval(() => data.loadWs().then(() => ui.prune()), 5000)); }
    if (s === 'inbox') { data.loadInbox(); timers.push(setInterval(() => data.loadInbox(), 5000)); }
    if (s === 'chat') { data.loadChat(); timers.push(setInterval(() => data.loadChat(), 5000)); }
    if (s === 'routines') { data.loadRoutines(); data.loadWs(); data.loadAccounts(); timers.push(setInterval(() => data.loadRoutines(), 10000)); }
    if (s === 'models') { data.loadModels(ui.period); timers.push(setInterval(() => data.loadModels(ui.period), 60000)); }
    if (s === 'settings') data.loadAccounts();
    return () => timers.forEach(clearInterval);
  });
  $effect(() => { ui.persist(); ui.saveLayout(); });
  $effect(() => { const m = matchMedia('(max-width: 1099px)'); const f = () => { if (m.matches) ui.inspector = false; }; m.addEventListener('change', f); return () => m.removeEventListener('change', f); });
  function keys(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); palette = !palette; return; }
    if (e.key === 'Escape') { palette = false; sheet = false; if (ui.popen) ui.popen = false; }
  }
</script>

<svelte:window onkeydown={keys} />

<div class="app" class:insp={ui.inspector && ui.section === 'work'} class:nopanel={!withPanel} class:popen={ui.popen}>
  <nav class="rail" aria-label={t('sec.work')}>
    <div class="logo" title="AgentVerse OS · {t('sec.work')}"></div>
    {#each SECTIONS as s (s.id)}
      <button class="nav" class:active={ui.section === s.id} data-tip={s.name} onclick={() => { ui.section = s.id; ui.popen = false; }}><Icon name={s.icon} size="l" />{#if s.id === 'inbox' && waiting}<span class="count warn">{waiting}</span>{/if}</button>
    {/each}
    <div class="bottom">
      <button class="nav" data-tip={prefs.effective === 'dark' ? t('set.light') : t('set.dark')} onclick={() => prefs.toggleTheme()}><Icon name={prefs.effective === 'dark' ? 'sun' : 'moon'} size="l" /></button>
      <button class="nav" data-tip={t('top.palette')} onclick={() => (palette = true)}><Icon name="search" size="l" /></button>
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
  <nav class="tabbar" aria-label={t('sec.more')}>
    {#each SECTIONS.filter(s => !['routines', 'settings'].includes(s.id)) as s (s.id)}
      <button class="tb" class:active={ui.section === s.id} onclick={() => { ui.section = s.id; ui.popen = false; }}><Icon name={s.icon} size="l" /><span>{s.name}</span>{#if s.id === 'inbox' && waiting}<span class="count warn">{waiting}</span>{/if}</button>
    {/each}
    <button class="tb" class:active={['routines', 'settings'].includes(ui.section)} onclick={() => (sheet = true)}><Icon name="more" size="l" /><span>{t('sec.more')}</span></button>
  </nav>
</div>
<MenuHost />
<Toasts />
{#if palette}<Palette onclose={() => (palette = false)} />{/if}
{#if sheet}<Sheet onclose={() => (sheet = false)} onpalette={() => (palette = true)} />{/if}
