<script lang="ts">
  // Собственный выпадающий список: нативный <select> в Firefox не принимает клик по пункту внутри transform-контейнера (окна).
  type Opt = { value: string; label: string };
  let { value = $bindable(''), options, placeholder = '', ariaLabel = '', onchange, style = '' }: { value?: string; options: Opt[]; placeholder?: string; ariaLabel?: string; onchange?: (v: string) => void; style?: string } = $props();
  let open = $state(false);
  let hi = $state(-1);
  let btn: HTMLButtonElement | undefined = $state();
  const current = $derived(options.find(o => o.value === value)?.label ?? placeholder);
  function pick(v: string) { value = v; open = false; onchange?.(v); btn?.focus(); }
  // Список живёт в document.body: карточки с backdrop-filter и окна с transform создают свои stacking context, и абсолютный список
  // перекрывался следующей карточкой или обрезался прокруткой окна. Координаты — от кнопки, пересчёт при прокрутке и ресайзе.
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    const place = () => {
      if (!btn) return;
      const r = btn.getBoundingClientRect();
      node.style.left = `${Math.max(4, Math.min(r.left, innerWidth - node.offsetWidth - 4))}px`;
      node.style.minWidth = `${r.width}px`;
      const below = r.bottom + 4;
      node.style.top = `${below + node.offsetHeight > innerHeight - 4 && r.top - 4 - node.offsetHeight > 4 ? r.top - 4 - node.offsetHeight : below}px`;
    };
    place();
    const onDoc = (e: PointerEvent) => { const t = e.target as Node; if (!node.contains(t) && !btn?.contains(t)) open = false; };
    window.addEventListener('scroll', place, true);
    window.addEventListener('resize', place);
    document.addEventListener('pointerdown', onDoc, true);
    return { destroy() { window.removeEventListener('scroll', place, true); window.removeEventListener('resize', place); document.removeEventListener('pointerdown', onDoc, true); node.remove(); } };
  }
  function key(e: KeyboardEvent) {
    if (!open && (e.key === 'ArrowDown' || e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); open = true; hi = Math.max(0, options.findIndex(o => o.value === value)); return; }
    if (!open) return;
    if (e.key === 'Escape') { open = false; }
    else if (e.key === 'ArrowDown') { e.preventDefault(); hi = Math.min(options.length - 1, hi + 1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); hi = Math.max(0, hi - 1); }
    else if (e.key === 'Enter') { e.preventDefault(); if (hi >= 0) pick(options[hi].value); }
    else if (e.key === 'Tab') open = false;
  }
</script>

<div class="dd" {style} onfocusout={(e) => { if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node)) open = false; }}>
  <button bind:this={btn} type="button" class="dd-btn" aria-haspopup="listbox" aria-expanded={open} aria-label={ariaLabel} onclick={() => { open = !open; hi = options.findIndex(o => o.value === value); }} onkeydown={key}>
    <span class="dd-cur">{current}</span><span class="dd-caret">▾</span>
  </button>
  {#if open}
    <ul class="dd-list" role="listbox" tabindex="-1" onkeydown={key} use:portal>
      {#each options as o, i}
        <li role="option" aria-selected={o.value === value} class:hi={i === hi} class:sel={o.value === value}
          onpointerdown={(e) => { e.preventDefault(); pick(o.value); }} onpointerenter={() => hi = i}>{o.label}</li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .dd { position: relative; display: inline-block; min-width: 0; }
  .dd-btn { display: flex; align-items: center; gap: 8px; width: 100%; text-align: left; background: var(--panel-2); border: 1px solid var(--line); border-radius: var(--r-sm); padding: calc(6px * var(--sp)) calc(10px * var(--sp)); }
  .dd-cur { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dd-caret { color: var(--muted); font-size: .8rem; }
  .dd-list { position: fixed; max-width: min(92vw, 520px); max-height: 50vh; overflow: auto; margin: 0; padding: 4px; list-style: none; background: var(--panel); border: 1px solid var(--line); border-radius: var(--r-sm); box-shadow: var(--shadow); z-index: 6000; font: inherit; }
  .dd-list li { padding: 8px 10px; border-radius: 6px; cursor: pointer; white-space: nowrap; }
  .dd-list li.hi { background: rgb(var(--accent-rgb) / .18); }
  .dd-list li.sel { color: var(--accent); font-weight: 600; }
  @media (pointer: coarse) { .dd-list li { padding: 12px 12px; } }
</style>
