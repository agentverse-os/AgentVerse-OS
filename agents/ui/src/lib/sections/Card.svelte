<script lang="ts">
  // Canvas card: the header (status, agent, title, workspace, model, cost, actions), the body — session renderings
  // (Chat / Terminal / Files / Git), a resize corner, dragging by the header.
  import { ROW_UNIT, GRID_GAP, H_DEF, H_MIN, H_MAX, ui, data, actions, key, same, titleOf, agentOf, dotOf, statusText, shortModel, money, live, stopLabel, accountMenu, type Card } from '../state.svelte';
  import { dnd } from '../dnd.svelte';
  import { menu } from '../menu.svelte';
  import { t } from '../i18n';
  import Icon from '../Icon.svelte';
  import SessionPane from '../SessionPane.svelte';
  import type { SessionInfo } from '../api';
  import Mark from '../Mark.svelte';
  let { card }: { card: Card } = $props();
  const info = $derived(data.infoOf(card));
  const s = $derived(card.kind === 'session' ? (info as SessionInfo | null) : null);
  const folded = $derived(ui.folded(card));
  let over = $state(false);
  // free resize by the corner (§4.11): while the pointer is held, the size is computed from the card's top left
  // corner
  let grip: { el: HTMLElement; left: number; top: number; colW: number; cols: number } | null = null;
  let sizing = $state<{ w: number; h: number } | null>(null);
  const px = (h: number) => h * ROW_UNIT + (h - 1) * GRID_GAP;
  function more(el: HTMLElement) {
    const dead = !live(info);
    menu.show(el, [
      { icon: 'max', label: t('sess.menu.expand'), on: () => ui.expand(card) },
      ...(card.kind === 'session' ? [{ icon: 'edit', label: t('sess.menu.rename'), on: () => actions.rename(card) }] : []),
      { icon: 'globe', label: t('browser.menuPreview'), checked: !!data.browserOf(card.project).preview, on: () => actions.togglePreview(card.project) },
      { icon: 'monitor', label: t('abrowser.menu'), checked: !!data.browserOf(card.project).agent, on: () => actions.toggleAgentBrowser(card.project) },
      { icon: 'wide', label: t('card.wider'), checked: card.w >= 2, on: () => ui.resize(card, card.w >= 2 ? 1 : 2) },
      { icon: 'tall', label: t('card.taller'), checked: card.h >= 8, on: () => ui.resize(card, undefined, card.h >= 8 ? H_DEF : 8) },
      ...(card.kind === 'session' && dead ? [{ icon: 'play', label: t('sess.menu.resume'), on: () => actions.resume(card) }] : []),
      { icon: dead ? 'trash' : 'square', label: stopLabel(info, true), danger: dead, on: () => actions.stopOrRemove(card) },
      ...accountMenu(card),
      { sep: true }, { icon: 'x', label: t('sess.menu.remove'), on: () => ui.removeCard(card) },
    ]);
  }
  function drop(e: DragEvent) {
    e.preventDefault(); e.stopPropagation(); over = false; const r = dnd.ref; dnd.ref = null; if (!r || same(r, card)) return;
    if (!ui.onCanvas(r)) ui.addCard(r); ui.reorder(key(r), key(card));
  }
  function gripDown(e: PointerEvent) {
    const el = (e.currentTarget as HTMLElement).closest('.card') as HTMLElement | null; const canvas = el?.closest('.canvas, .group') as HTMLElement | null;
    if (!el || !canvas) return;
    const r = el.getBoundingClientRect();
    const cols = Math.max(1, getComputedStyle(canvas).gridTemplateColumns.split(' ').filter(Boolean).length);
    const colW = (canvas.clientWidth - GRID_GAP * (cols - 1)) / cols;
    grip = { el, left: r.left, top: r.top, colW, cols }; sizing = { w: card.w, h: card.h };
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId); e.preventDefault(); e.stopPropagation();
  }
  function gripMove(e: PointerEvent) {
    if (!grip) return;
    const w = Math.min(grip.cols, Math.max(1, Math.round((e.clientX - grip.left + GRID_GAP / 2) / (grip.colW + GRID_GAP))));
    const h = Math.min(H_MAX, Math.max(H_MIN, Math.round((e.clientY - grip.top + GRID_GAP / 2) / (ROW_UNIT + GRID_GAP))));
    if (w !== card.w || h !== card.h) ui.resize(card, w, h);
    sizing = { w, h };
  }
  function gripUp() { grip = null; sizing = null; }
</script>

<div class="card" class:sized={(card.w > 1 || card.h !== H_DEF) && !folded} class:sizing={!!sizing} style:grid-column={folded ? undefined : `span ${card.w}`} style:grid-row={folded ? undefined : `span ${card.h}`} class:collapsed={folded} class:active={ui.sel && same(ui.sel, card)} class:drop-before={over} data-card={key(card)} role="listitem"
  onfocusin={() => (ui.sel = card)} ondragover={(e) => { if (dnd.ref) { e.preventDefault(); e.stopPropagation(); over = true; } }} ondragleave={() => (over = false)} ondrop={drop}>
  <div class="c-head" draggable="true" ondragstart={(e) => { dnd.ref = card; e.dataTransfer?.setData('text/plain', key(card)); (e.currentTarget as HTMLElement).closest('.card')?.classList.add('dragging'); }} ondragend={(e) => { dnd.ref = null; (e.currentTarget as HTMLElement).closest('.card')?.classList.remove('dragging'); }} role="presentation">
    <button class="fold" aria-label={folded ? t('card.unfold') : t('card.fold')} aria-expanded={!folded} data-tip={folded ? t('card.unfold') : t('card.fold')} onclick={(e) => { e.stopPropagation(); ui.toggleFold(card); }}><Icon name={folded ? 'chevronRight' : 'chevronDown'} size="s" /></button>
    <Icon name="drag" size="s" /><span class="dot {dotOf(card)}"></span>
    <span class="av {agentOf(card)}"><Icon name={agentOf(card) === 'shell' ? 'terminal' : agentOf(card)} size="s" /></span>
    {#if card.kind === 'session'}<Mark id={card.id} />{/if}
    <button class="ttl card-select ghost" aria-pressed={!!ui.sel && same(ui.sel, card)} title={s?.label ? `${t('insp.autoTitle')}: ${s.title}` : undefined} onclick={() => { ui.sel = card; if (ui.isPhone) ui.toggleFold(card); }}><b>{titleOf(card)}</b><span>{card.project} · {s ? statusText(s) : info?.cwd ?? ''}</span></button>
    <div class="meta">{#if s?.model}<span class="chip mono model">{shortModel(s.model)}</span>{/if}{#if s}<span class="chip num">{money(s.usage?.cost_usd)}</span>{/if}</div>
    <div class="acts">
      <button class="icon xs ghost opt" aria-label={t('card.wider')} data-tip={t('card.wider')} onclick={(e) => { e.stopPropagation(); ui.resize(card, card.w >= 2 ? 1 : 2); }}><Icon name="wide" size="s" /></button>
      <button class="icon xs ghost opt" aria-label={t('card.taller')} data-tip={t('card.taller')} onclick={(e) => { e.stopPropagation(); ui.resize(card, undefined, card.h >= 8 ? H_DEF : 8); }}><Icon name="tall" size="s" /></button>
      <button class="icon xs ghost" aria-label={t('card.expand')} data-tip={t('card.expand')} onclick={(e) => { e.stopPropagation(); ui.expand(card); }}><Icon name="max" size="s" /></button>
      <button class="icon xs ghost" aria-label={t('card.more')} data-tip={t('card.more')} onclick={(e) => { e.stopPropagation(); more(e.currentTarget as HTMLElement); }}><Icon name="more" size="s" /></button>
      <button class="icon xs ghost" aria-label={t('card.remove')} data-tip={t('card.remove')} onclick={(e) => { e.stopPropagation(); ui.removeCard(card); }}><Icon name="x" size="s" /></button>
    </div>
  </div>
  {#if !folded}<div class="c-body">
    {#if !info}<div class="muted sm" style="padding: 8px">{data.project(card.project)?.reachable === false ? t('agents.noAgentd') : t('agents.connecting')}</div>
    {:else}<SessionPane project={card.project} id={card.id} kind={card.kind} cwd={info.cwd} onchange={() => { if (!data.hubLive) data.loadWs(); }} compact />{/if}
  </div>{/if}
  <button class="grip" aria-label={t('card.grip')} title={t('card.grip')} onkeydown={(e) => { if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') { e.preventDefault(); ui.resize(card, card.w + (e.key === 'ArrowRight' ? 1 : -1)); } if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); ui.resize(card, undefined, card.h + (e.key === 'ArrowDown' ? 1 : -1)); } }} onpointerdown={gripDown} onpointermove={gripMove} onpointerup={gripUp} onpointercancel={gripUp}><Icon name="grip" size="s" /></button>
  {#if sizing}<span class="size-hint" aria-live="polite">{sizing.w} × {sizing.h} · {px(sizing.h)} px</span>{/if}
</div>
