<script lang="ts" module>
  // One icon renderer for the whole shell: desktop grid, taskbar and dock, start menu, window titles, the phone home screen.
  // System apps and projects — the vector SysIcon set in every look; Store apps — the manifest image on a uniform tile
  // (transparent logos and full-bleed icons look alike); a broken or missing image — a letter on a color from the name.
  import type { SysIconName } from './SysIcon.svelte';
  import type { Win } from './windows.svelte';
  export interface IconRef { sys?: SysIconName; src?: string; emoji?: string }
  const isImg = (i?: string | null) => !!i && (i.startsWith('/') || i.startsWith('http') || i.startsWith('data:'));
  /** Icon from a manifest value: an image URL or an emoji. */
  export function refOf(icon?: string | null): IconRef { return isImg(icon) ? { src: icon! } : icon ? { emoji: icon } : {}; }
  /** Icon of a window: a system component — its own icon, a workspace app — the project icon, a Store app — its image. */
  export function winIcon(w: Win): IconRef {
    if (w.kind === 'component' && w.component) return { sys: w.component };
    if (w.id.startsWith('ws:')) return { sys: 'project' };
    return refOf(w.icon);
  }
</script>

<script lang="ts">
  import SysIcon from './SysIcon.svelte';
  /** size — pixels or any CSS length (e.g. 'var(--icon)': the icon then follows the "icon size" setting). */
  let { icon, size = 48, label = '', status = null }: { icon: IconRef; size?: number | string; label?: string; status?: 'ok' | 'off' | 'warn' | null } = $props();
  const css = $derived(typeof size === 'number' ? `${size}px` : size);
  let broken = $state(false);
  $effect(() => { void icon.src; broken = false; });
  const hue = $derived([...label].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7));
</script>

<span class="aicon" style="--s:{css}">
  {#if icon.sys}<SysIcon name={icon.sys} size={typeof size === 'number' ? size : 48} />
  {:else if icon.src && !broken}<img src={icon.src} alt="" loading="lazy" draggable="false" onerror={() => broken = true} />
  {:else if icon.emoji}<span class="em">{icon.emoji}</span>
  {:else}<span class="letter" style="--h:{hue}">{(label.trim()[0] ?? '?').toUpperCase()}</span>{/if}
  {#if status}<span class="st {status}"></span>{/if}
</span>

<style>
  .aicon { position: relative; display: inline-grid; place-items: center; width: var(--s); height: var(--s); flex: none; line-height: 1; }
  .aicon :global(svg.sysicon) { width: 100%; height: 100%; }
  img, .em, .letter { width: 100%; height: 100%; border-radius: 23.5%; }
  img { display: block; object-fit: cover; background: color-mix(in oklab, var(--ink) 9%, transparent); box-shadow: inset 0 0 0 .5px color-mix(in oklab, var(--ink) 16%, transparent); }
  .em { display: grid; place-items: center; font-size: calc(var(--s) * .58); background: color-mix(in oklab, var(--ink) 10%, transparent); box-shadow: inset 0 0 0 .5px color-mix(in oklab, var(--ink) 18%, transparent), inset 0 1px 0 rgb(255 255 255 / .12); }
  .letter { display: grid; place-items: center; font-size: calc(var(--s) * .46); font-weight: 700; color: #fff; background: linear-gradient(160deg, oklch(0.66 0.15 var(--h)), oklch(0.46 0.14 calc(var(--h) + 34))); box-shadow: inset 0 1px 0 rgb(255 255 255 / .25); }
  .st { position: absolute; right: -4%; bottom: -4%; width: 26%; height: 26%; min-width: 9px; min-height: 9px; border-radius: 50%; background: var(--muted); box-shadow: 0 0 0 2px var(--icon-ring, rgb(10 12 16 / .78)); }
  .st.ok { background: var(--ok); } .st.warn { background: var(--warn); }
</style>
