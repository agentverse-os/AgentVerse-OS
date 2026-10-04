<script lang="ts">
  // Карточка холста: шапка (статус, агент, название, workspace, модель, стоимость, действия), тело — рендеринги сессии
  // (Чат / Терминал / Файлы / Git), угол для ресайза, перетаскивание за шапку.
  import { ui, data, actions, key, same, titleOf, agentOf, dotOf, statusText, shortModel, money, live, stopLabel, accountMenu, type Card } from '../state.svelte';
  import { dnd } from '../dnd.svelte';
  import { menu } from '../menu.svelte';
  import { t } from '../i18n';
  import Icon from '../Icon.svelte';
  import SessionPane from '../SessionPane.svelte';
  import type { SessionInfo } from '../api';
  let { card }: { card: Card } = $props();
  const info = $derived(data.infoOf(card));
  const s = $derived(card.kind === 'session' ? (info as SessionInfo | null) : null);
  let over = $state(false);
  let grip: { x: number; y: number } | null = null;
  function more(el: HTMLElement) {
    const dead = !live(info);
    menu.show(el, [
      { icon: 'max', label: t('sess.menu.expand'), on: () => ui.expand(card) },
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
  function gripDown(e: PointerEvent) { grip = { x: e.clientX, y: e.clientY }; (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId); e.preventDefault(); e.stopPropagation(); }
  function gripUp(e: PointerEvent) {
    if (!grip) return; const dx = e.clientX - grip.x, dy = e.clientY - grip.y; grip = null;
    const w = dx > 80 ? 2 : dx < -80 ? 1 : undefined, h = dy > 80 ? 2 : dy < -80 ? 1 : undefined;
    if (w || h) ui.resize(card, w as 1 | 2 | undefined, h as 1 | 2 | undefined);
  }
</script>

<div class="card" class:w2={card.w === 2} class:h2={card.h === 2} class:active={ui.sel && same(ui.sel, card)} class:drop-before={over} data-card={key(card)} role="listitem"
  onclick={() => (ui.sel = card)} ondragover={(e) => { if (dnd.ref) { e.preventDefault(); e.stopPropagation(); over = true; } }} ondragleave={() => (over = false)} ondrop={drop}>
  <div class="c-head" draggable="true" ondragstart={(e) => { dnd.ref = card; e.dataTransfer?.setData('text/plain', key(card)); (e.currentTarget as HTMLElement).closest('.card')?.classList.add('dragging'); }} ondragend={(e) => { dnd.ref = null; (e.currentTarget as HTMLElement).closest('.card')?.classList.remove('dragging'); }} role="presentation">
    <Icon name="drag" size="s" /><span class="dot {dotOf(card)}"></span>
    <span class="av {agentOf(card)}"><Icon name={agentOf(card) === 'shell' ? 'terminal' : agentOf(card)} size="s" /></span>
    <div class="ttl"><b>{titleOf(card)}</b><span>{card.project} · {s ? statusText(s) : info?.cwd ?? ''}</span></div>
    <div class="meta">{#if s?.model}<span class="chip mono model">{shortModel(s.model)}</span>{/if}{#if s}<span class="chip num">{money(s.usage?.cost_usd)}</span>{/if}</div>
    <div class="acts">
      <button class="icon xs ghost opt" data-tip={t('card.wider')} onclick={(e) => { e.stopPropagation(); ui.resize(card, card.w === 2 ? 1 : 2); }}><Icon name="wide" size="s" /></button>
      <button class="icon xs ghost opt" data-tip={t('card.taller')} onclick={(e) => { e.stopPropagation(); ui.resize(card, undefined, card.h === 2 ? 1 : 2); }}><Icon name="tall" size="s" /></button>
      <button class="icon xs ghost" data-tip={t('card.expand')} onclick={(e) => { e.stopPropagation(); ui.expand(card); }}><Icon name="max" size="s" /></button>
      <button class="icon xs ghost" data-tip={t('card.more')} onclick={(e) => { e.stopPropagation(); more(e.currentTarget as HTMLElement); }}><Icon name="more" size="s" /></button>
      <button class="icon xs ghost" data-tip={t('card.remove')} onclick={(e) => { e.stopPropagation(); ui.removeCard(card); }}><Icon name="x" size="s" /></button>
    </div>
  </div>
  <div class="c-body">
    {#if !info}<div class="muted sm" style="padding: 8px">{data.project(card.project)?.reachable === false ? t('agents.noAgentd') : t('agents.connecting')}</div>
    {:else}<SessionPane project={card.project} id={card.id} kind={card.kind} cwd={info.cwd} onchange={() => data.loadWs()} compact />{/if}
  </div>
  <span class="grip" title={t('card.grip')} onpointerdown={gripDown} onpointerup={gripUp}><Icon name="grip" size="s" /></span>
</div>
