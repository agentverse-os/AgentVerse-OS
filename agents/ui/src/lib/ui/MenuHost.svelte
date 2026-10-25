<script lang="ts">
  import { menu, type MenuItem } from '../menu.svelte';
  import Icon from '../Icon.svelte';
  let host = $state<HTMLDivElement | undefined>(undefined);
  let active = $state(-1);
  function activate(it: MenuItem, restoreFocus: boolean) { menu.close(restoreFocus); it.on?.(); }
  function outsideClick(e: MouseEvent) {
    const target = e.target as Node | null;
    if (menu.open && target && !host?.contains(target) && !menu.anchor?.contains(target)) menu.close();
  }
  function escape(e: KeyboardEvent) {
    if (menu.open && e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); menu.close(true); }
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') { escape(e); return; }
    // The focus returns to the anchor before the browser's standard Tab navigation; the menu does not
    // trap the keyboard.
    if (e.key === 'Tab') { e.stopPropagation(); menu.close(true); return; }
    const inSearch = e.target instanceof HTMLInputElement;
    if (inSearch && e.key === 'Enter') {
      const first = menu.visible.find(it => !it.header && !it.sub && !it.sep && !it.disabled);
      if (first) { e.preventDefault(); e.stopPropagation(); activate(first, true); }
      return;
    }
    // Home/End in the search stay editing commands; from menu items they lead to the list edges.
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key) || (inSearch && ['Home', 'End'].includes(e.key))) return;
    e.preventDefault(); e.stopPropagation();
    const items = Array.from(host?.querySelectorAll<HTMLButtonElement>('.mi:not(:disabled)') ?? []);
    if (!items.length) return;
    const at = items.indexOf(e.target as HTMLButtonElement);
    const next = e.key === 'Home' ? 0 : e.key === 'End' ? items.length - 1 : e.key === 'ArrowDown' ? (at + 1) % items.length : at < 0 ? items.length - 1 : (at - 1 + items.length) % items.length;
    items[next].focus();
  }
</script>

<svelte:window onclick={outsideClick} onkeydown={escape} />
{#if menu.open}
  <div bind:this={host} class="menu" class:searchable={menu.search} style="left:{menu.x}px; top:{menu.y}px" role="menu" aria-label={menu.label || undefined} tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={key}>
    {#if menu.search}<input class="ms" type="search" aria-label={menu.placeholder || menu.label} bind:value={menu.query} placeholder={menu.placeholder} autocomplete="off" spellcheck="false" />{/if}
    {#each menu.visible as it, i (i)}
      {#if it.sep}<hr />
      {:else if it.header}<div class="mh">{it.label}</div>
      {:else if it.sub}<div class="mh sub">{it.label}</div>
      {:else}<button class="mi" class:on={it.checked} class:danger={it.danger} class:dis={it.disabled} role={typeof it.checked === 'boolean' ? 'menuitemcheckbox' : 'menuitem'} aria-checked={typeof it.checked === 'boolean' ? it.checked : undefined} aria-disabled={it.disabled || undefined} tabindex={!it.disabled && active === i ? 0 : -1} disabled={it.disabled} onfocus={() => (active = i)} onclick={(e) => activate(it, e.detail === 0)}>{#if it.avatar}<span class="av {it.avatar}"><Icon name={it.avatar === 'shell' ? 'terminal' : it.avatar} size="s" /></span>{:else if it.icon}<Icon name={it.icon} />{/if}{it.label}{#if it.hint}<span class="k">{it.hint}</span>{/if}</button>{/if}
    {/each}
    {#if menu.search && menu.query.trim() && !menu.visible.length}<div class="mh">∅</div>{/if}
  </div>
{/if}
