<script lang="ts">
  // Слой виджетов: между обоями/иконками и окнами. Перетаскивание за заголовок (появляется при наведении), размер за угол,
  // ⚙ настройки внутри виджета, ✕ удалить. На телефоне — столбец под иконками без перетаскивания.
  import { widgets, WIDGETS, type WidgetInst } from './widgets.svelte';
  import { t } from './i18n.svelte';
  import { deviceClass, TASKBAR_H } from './windows.svelte';
  import Monitor from './widgets/Monitor.svelte';
  import News from './widgets/News.svelte';
  import Clock from './widgets/Clock.svelte';
  import Projects from './widgets/Projects.svelte';
  import Health from './widgets/Health.svelte';
  import Events from './widgets/Events.svelte';
  import Notes from './widgets/Notes.svelte';
  import Apps from './widgets/Apps.svelte';
  import Weather from './widgets/Weather.svelte';
  let { mobile = false }: { mobile?: boolean } = $props();
  const phone = $derived(mobile || deviceClass() === 'phone');
  let drag: { id: string; kind: 'move' | 'resize'; x: number; y: number } | null = null;
  function down(e: PointerEvent, w: WidgetInst, kind: 'move' | 'resize') {
    if (phone || (e.target as HTMLElement).closest('button, a, input, textarea')) return;
    drag = { id: w.id, kind, x: e.clientX, y: e.clientY }; (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) { if (!drag) return; const dx = e.clientX - drag.x, dy = e.clientY - drag.y; drag.x = e.clientX; drag.y = e.clientY; if (drag.kind === 'move') widgets.move(drag.id, dx, dy); else widgets.resize(drag.id, dx, dy); }
  function up() { if (drag) { widgets.save(); drag = null; } }
</script>

<div class="wlayer" class:phone style="--tb:{TASKBAR_H}px">
  {#if phone}
    <div class="row" style="justify-content: space-between;"><span class="muted">{t('widgets.title')}</span><button onclick={() => widgets.picker = true}>{t('widgets.add')}</button></div>
    {#if !widgets.items.length}<button class="empty" onclick={() => widgets.picker = true}>{t('widgets.emptyHint')}</button>{/if}
  {/if}
  {#each widgets.items as w (w.id)}
    <section class="widget" style={phone ? '' : `transform: translate(${w.x}px, ${w.y}px); width:${w.w}px; height:${w.h}px`} aria-label={t(WIDGETS[w.type].title)}>
      <header class="whead" role="toolbar" tabindex="-1" aria-label={t('widgets.headAria')} onpointerdown={(e) => down(e, w, 'move')} onpointermove={move} onpointerup={up}>
        <span class="wt">{WIDGETS[w.type].icon} {t(WIDGETS[w.type].title)}</span>
        <span class="spacer"></span>
        <button title={t('widgets.remove')} onclick={() => widgets.remove(w.id)}>✕</button>
      </header>
      <div class="wbody">
        {#if w.type === 'monitor'}<Monitor config={w.config} w={w.w} />
        {:else if w.type === 'news'}<News config={w.config} onconfig={(p) => widgets.setConfig(w.id, p)} />
        {:else if w.type === 'clock'}<Clock config={w.config} onconfig={(p) => widgets.setConfig(w.id, p)} />
        {:else if w.type === 'projects'}<Projects />
        {:else if w.type === 'health'}<Health />
        {:else if w.type === 'events'}<Events config={w.config} />
        {:else if w.type === 'notes'}<Notes config={w.config} onconfig={(p) => widgets.setConfig(w.id, p)} />
        {:else if w.type === 'apps'}<Apps />
        {:else if w.type === 'weather'}<Weather config={w.config} onconfig={(p) => widgets.setConfig(w.id, p)} />
        {/if}
      </div>
      {#if !phone}<div class="grip" role="separator" aria-label={t('widgets.resize')} onpointerdown={(e) => down(e, w, 'resize')} onpointermove={move} onpointerup={up}></div>{/if}
    </section>
  {/each}
</div>

{#if widgets.picker}
  <div class="backdrop" role="presentation" onclick={() => widgets.picker = false}></div>
  <div class="picker" role="dialog" aria-label={t('widgets.pickerAria')}>
    <div class="row" style="justify-content: space-between;"><h3>{t('widgets.pickerTitle')}</h3><button onclick={() => widgets.picker = false}>✕</button></div>
    <div class="pgrid">
      {#each Object.entries(WIDGETS) as [k, m]}
        <button class="pitem" onclick={() => widgets.add(k as any)}><span class="pi">{m.icon}</span><b>{t(m.title)}</b><span class="muted small">{t(m.description)}</span></button>
      {/each}
    </div>
  </div>
{/if}

<style>
  .wlayer { position: fixed; inset: 0 0 var(--tb) 0; pointer-events: none; z-index: 1; }
  .wlayer.phone { position: static; inset: auto; display: flex; flex-direction: column; gap: 10px; padding: 0; pointer-events: auto; }
  .widget { position: absolute; top: 0; left: 0; display: flex; flex-direction: column; background: var(--glass); backdrop-filter: blur(14px); -webkit-backdrop-filter: blur(14px); border: 1px solid var(--line); border-radius: var(--r); box-shadow: var(--shadow); overflow: hidden; pointer-events: auto; }
  .phone .widget { position: static; transform: none !important; width: auto !important; height: auto !important; min-height: 120px; }
  .whead { display: flex; align-items: center; gap: 6px; padding: 4px 8px; font-size: .78rem; color: var(--muted); cursor: grab; user-select: none; touch-action: none; border-bottom: 1px solid transparent; }
  .widget:hover .whead { border-bottom-color: var(--line); background: color-mix(in srgb, var(--panel-2) 60%, transparent); }
  .whead button { opacity: 0; padding: 0 6px; font-size: .75rem; }
  .widget:hover .whead button, .phone .whead button { opacity: 1; }
  .spacer { flex: 1; }
  .wbody { flex: 1; min-height: 0; overflow: auto; padding: 6px 10px 10px; }
  .grip { position: absolute; right: 0; bottom: 0; width: 16px; height: 16px; cursor: nwse-resize; background: linear-gradient(135deg, transparent 50%, var(--line) 50%); touch-action: none; }
  .backdrop { position: fixed; inset: 0; z-index: 2400; }
  .picker { position: fixed; left: 50%; top: 50%; transform: translate(-50%, -50%); width: min(720px, calc(100vw - 24px)); max-height: 80vh; z-index: 2500; overflow: auto; background: var(--glass); backdrop-filter: blur(20px); -webkit-backdrop-filter: blur(20px); border: 1px solid var(--line); border-radius: var(--r); box-shadow: var(--shadow); padding: 14px; }
  .pgrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 8px; margin-top: 10px; }
  .pitem { display: flex; flex-direction: column; align-items: flex-start; gap: 4px; text-align: left; padding: 10px; }
  .pi { font-size: 22px; }
  .small { font-size: .78rem; }
  .empty { text-align: left; padding: 14px; color: var(--muted); background: var(--glass); }
</style>
