<script lang="ts">
  import { menu } from '../menu.svelte';
  import Icon from '../Icon.svelte';
</script>

<svelte:window onclick={() => menu.close()} onkeydown={(e) => { if (e.key === 'Escape') menu.close(); }} />
{#if menu.open}
  <div class="menu" class:searchable={menu.search} style="left:{menu.x}px; top:{menu.y}px" role="menu" onclick={(e) => e.stopPropagation()}>
    {#if menu.search}<input class="ms" type="search" bind:value={menu.query} placeholder={menu.placeholder} autocomplete="off" spellcheck="false" onkeydown={(e) => { if (e.key === 'Enter') { const first = menu.visible.find(x => !x.header && !x.sub && !x.sep && !x.disabled); if (first) { menu.close(); first.on?.(); } } }} />{/if}
    {#each menu.visible as it, i (i)}
      {#if it.sep}<hr />
      {:else if it.header}<div class="mh">{it.label}</div>
      {:else if it.sub}<div class="mh sub">{it.label}</div>
      {:else}<button class="mi" class:on={it.checked} class:danger={it.danger} class:dis={it.disabled} disabled={it.disabled} onclick={() => { menu.close(); it.on?.(); }}>{#if it.avatar}<span class="av {it.avatar}"><Icon name={it.avatar === 'shell' ? 'terminal' : it.avatar} size="s" /></span>{:else if it.icon}<Icon name={it.icon} />{/if}{it.label}{#if it.hint}<span class="k">{it.hint}</span>{/if}</button>{/if}
    {/each}
    {#if menu.search && menu.query.trim() && !menu.visible.length}<div class="mh">∅</div>{/if}
  </div>
{/if}
