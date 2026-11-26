<script lang="ts">
  // Widget layer: between the wallpaper/icons and the windows. Dragging by the title, resize by the corner, ⚙ settings inside
  // the widget, ✕ remove. On the phone — a grid under the icons without dragging (one column on a narrow screen, two on an
  // unfolded foldable). In the umbrel look the layer is the same (widgets by default in a column on the top right), and the
  // app grid makes room for them.
  import { widgets, WIDGETS, type WidgetInst, type WidgetType } from './widgets.svelte';
  import { t } from './i18n.svelte';
  import { wm, deviceClass } from './windows.svelte';
  import Glyph, { type GlyphName } from './Glyph.svelte';
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
  const phone = $derived(mobile || (void wm.vw, deviceClass() === 'phone'));
  const GLYPH: Record<WidgetType, GlyphName> = { monitor: 'chart', news: 'news', clock: 'clock', projects: 'cube', health: 'pulse', events: 'list', notes: 'note', apps: 'rocket', weather: 'cloud' };

  // what is drawn is widgets.fitted — the saved layout fitted into the current screen (widgets.svelte.ts)
  const fitted = $derived(widgets.fitted);

  let drag: { id: string; kind: 'move' | 'resize'; x: number; y: number } | null = null;
  function down(e: PointerEvent, w: WidgetInst, kind: 'move' | 'resize') {
    if (phone || (e.target as HTMLElement).closest('button, a, input, textarea')) return;
    const r = fitted.get(w.id);
    if (r) Object.assign(w, { x: r.x, y: r.y, w: r.w, h: r.h });  // the gesture starts from what is on screen
    drag = { id: w.id, kind, x: e.clientX, y: e.clientY }; widgets.dragging = true; (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) { if (!drag) return; const dx = e.clientX - drag.x, dy = e.clientY - drag.y; drag.x = e.clientX; drag.y = e.clientY; if (drag.kind === 'move') widgets.move(drag.id, dx, dy); else widgets.resize(drag.id, dx, dy); }
  function up() { if (drag) { widgets.save(); drag = null; widgets.dragging = false; } }
  function frame(w: WidgetInst) { const r = fitted.get(w.id) ?? w; return `transform: translate(${r.x}px, ${r.y}px); width:${r.w}px; height:${r.h}px`; }
</script>

<div class="wlayer" class:phone>
  {#if phone}
    <div class="wtop"><span class="muted">{t('widgets.title')}</span><button onclick={() => widgets.picker = true}>{t('widgets.add')}</button></div>
    {#if !widgets.items.length}<button class="empty" onclick={() => widgets.picker = true}>{t('widgets.emptyHint')}</button>{/if}
  {/if}
  {#each widgets.items as w (w.id)}
    <section class="widget" data-id={w.id} data-type={w.type} style={phone ? '' : frame(w)} aria-label={t(WIDGETS[w.type].title)}>
      <header class="whead" role="toolbar" tabindex="-1" aria-label={t('widgets.headAria')} onpointerdown={(e) => down(e, w, 'move')} onpointermove={move} onpointerup={up} onpointercancel={up}>
        <span class="wt"><Glyph name={GLYPH[w.type]} size={14} /> {t(WIDGETS[w.type].title)}</span>
        <span class="spacer"></span>
        <button class="wx" title={t('widgets.remove')} aria-label={t('widgets.remove')} onclick={() => widgets.remove(w.id)}><Glyph name="close" size={14} /></button>
      </header>
      <div class="wbody">
        {#if w.type === 'monitor'}<Monitor config={w.config} w={fitted.get(w.id)?.w ?? w.w} />
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
      {#if !phone}<div class="grip" role="separator" aria-label={t('widgets.resize')} onpointerdown={(e) => down(e, w, 'resize')} onpointermove={move} onpointerup={up} onpointercancel={up}></div>{/if}
    </section>
  {/each}
</div>

{#if widgets.picker}
  <div class="backdrop" role="presentation" onclick={() => widgets.picker = false}></div>
  <div class="picker" role="dialog" aria-label={t('widgets.pickerAria')}>
    <div class="phead"><h3>{t('widgets.pickerTitle')}</h3><button class="wx" title={t('common.closeTitle')} aria-label={t('common.closeTitle')} onclick={() => widgets.picker = false}><Glyph name="close" size={16} /></button></div>
    <div class="pgrid">
      {#each Object.entries(WIDGETS) as [k, m]}
        <button class="pitem" onclick={() => widgets.add(k as any)}><span class="pi"><Glyph name={GLYPH[k as WidgetType]} size={20} /></span><b>{t(m.title)}</b><span class="muted small">{t(m.description)}</span></button>
      {/each}
    </div>
  </div>
{/if}

<style>
  .wlayer { position: fixed; inset: 0 0 var(--shell-b) 0; pointer-events: none; z-index: 1; }
  .wlayer.phone { position: static; inset: auto; display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 300px), 1fr)); gap: 10px; align-items: start; padding: 0; pointer-events: auto; }
  .wtop { grid-column: 1 / -1; display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .wtop button { min-height: 40px; }
  .widget { position: absolute; top: 0; left: 0; display: flex; flex-direction: column; background: var(--widget-bg); backdrop-filter: var(--widget-blur); -webkit-backdrop-filter: var(--widget-blur); border: 1px solid var(--line); border-radius: var(--widget-r); box-shadow: var(--shadow-2); pointer-events: auto; overflow: hidden; container: widget / inline-size; }
  :global(:root[data-look="umbrel"]) .widget { border-width: .5px; }
  .phone .widget { position: static; min-height: 120px; max-height: 420px; }
  .whead { display: flex; align-items: center; gap: 6px; min-height: 30px; padding: 3px 6px 3px 12px; font-size: .76rem; font-weight: 500; color: var(--muted); cursor: grab; user-select: none; -webkit-user-select: none; touch-action: none; flex: none; }
  .phone .whead { cursor: default; touch-action: auto; }
  .wt { display: inline-flex; align-items: center; gap: 6px; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .wx { display: grid; place-items: center; width: 24px; height: 24px; padding: 0; border-radius: 50%; background: transparent; border-color: transparent; color: var(--muted); flex: none; }
  .wx:hover { background: var(--fill-2); color: var(--ink); border-color: transparent; }
  .whead .wx { opacity: 0; }
  .widget:hover .whead .wx, .widget:focus-within .whead .wx, .phone .whead .wx { opacity: 1; }
  @media (pointer: coarse) { .whead .wx { opacity: 1; } .wx { width: 36px; height: 36px; } .whead { min-height: 38px; } }
  .spacer { flex: 1; }
  .wbody { flex: 1; min-height: 0; overflow: auto; padding: 2px 12px 12px; }
  .grip { position: absolute; right: 0; bottom: 0; width: 18px; height: 18px; cursor: nwse-resize; touch-action: none; }
  .grip::after { content: ""; position: absolute; right: 5px; bottom: 5px; width: 7px; height: 7px; border-right: 2px solid color-mix(in oklab, var(--ink) 26%, transparent); border-bottom: 2px solid color-mix(in oklab, var(--ink) 26%, transparent); border-bottom-right-radius: 4px; opacity: 0; transition: opacity var(--dur); }
  .widget:hover .grip::after { opacity: 1; }

  .backdrop { position: fixed; inset: 0; z-index: 2400; background: var(--scrim); }
  .picker { position: fixed; left: 50%; top: 50%; transform: translate(-50%, -50%); width: min(760px, calc(100vw - 24px)); max-height: min(80vh, calc(100dvh - 32px)); z-index: 2500; overflow: auto; background: var(--pop-bg); backdrop-filter: var(--pop-blur); -webkit-backdrop-filter: var(--pop-blur); border: 1px solid var(--line-2); border-radius: var(--r-lg); box-shadow: var(--shadow-3); padding: 14px 16px 16px; transition: opacity .18s var(--ease), transform .18s var(--ease); }
  @starting-style { .picker { opacity: 0; transform: translate(-50%, calc(-50% + 10px)) scale(.98); } }
  .phead { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .phead .wx { width: 32px; height: 32px; }
  .pgrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 210px), 1fr)); gap: 8px; margin-top: 12px; }
  .pitem { display: flex; flex-direction: column; align-items: flex-start; gap: 4px; text-align: left; padding: 12px; border-radius: var(--r); background: var(--card); border-color: var(--card-line); }
  .pitem:hover { background: var(--fill-2); border-color: var(--accent-line); }
  .pi { display: grid; place-items: center; width: 36px; height: 36px; border-radius: 10px; background: var(--accent-soft); color: var(--accent-ink); margin-bottom: 4px; }
  .small { font-size: .78rem; font-weight: 400; }
  .empty { grid-column: 1 / -1; text-align: left; padding: 14px; color: var(--muted); background: var(--glass); }
</style>
