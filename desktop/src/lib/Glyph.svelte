<script lang="ts" module>
  // Interface glyphs: one stroke set (24×24, round caps) for the shell — the tray, window and navigation buttons, widget
  // titles. Emoji are not used for controls: they differ between systems and are missing on some.
  const G = {
    search: '<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    close: '<path d="m6.5 6.5 11 11M17.5 6.5l-11 11"/>',
    minus: '<path d="M5.5 12h13"/>',
    back: '<path d="m14.5 5.5-6.5 6.5 6.5 6.5"/>',
    forward: '<path d="m9.5 5.5 6.5 6.5-6.5 6.5"/>',
    down: '<path d="m5.5 9.5 6.5 6.5 6.5-6.5"/>',
    home: '<path d="M4.5 11 12 4.5l7.5 6.5v7.5a1.5 1.5 0 0 1-1.5 1.5h-3.5v-5.5h-5V20H6a1.5 1.5 0 0 1-1.5-1.5Z"/>',
    recents: '<rect x="4" y="8" width="12" height="12" rx="2.5"/><path d="M8.5 4.5H17a3 3 0 0 1 3 3V16"/>',
    sun: '<circle cx="12" cy="12" r="3.8"/><path d="M12 3v2.2M12 18.8V21M3 12h2.2M18.8 12H21M5.6 5.6l1.6 1.6M16.8 16.8l1.6 1.6M18.4 5.6l-1.6 1.6M7.2 16.8l-1.6 1.6"/>',
    moon: '<path d="M19.5 14.2A8 8 0 0 1 9.8 4.5a8 8 0 1 0 9.7 9.7Z"/>',
    update: '<circle cx="12" cy="12" r="8.5"/><path d="M12 16V8.2M8.6 11.4 12 8l3.4 3.4"/>',
    external: '<path d="M14 5h5v5M19 5l-8.5 8.5M11 6.5H6.5A1.5 1.5 0 0 0 5 8v9.5A1.5 1.5 0 0 0 6.5 19H16a1.5 1.5 0 0 0 1.5-1.5V13"/>',
    key: '<circle cx="8" cy="15.5" r="3.6"/><path d="m10.7 12.8 8.3-8.3M16 7.5l2.6 2.6M13.6 9.9l1.9 1.9"/>',
    apps: '<rect x="4.5" y="4.5" width="6" height="6" rx="1.6"/><rect x="13.5" y="4.5" width="6" height="6" rx="1.6"/><rect x="4.5" y="13.5" width="6" height="6" rx="1.6"/><rect x="13.5" y="13.5" width="6" height="6" rx="1.6"/>',
    widget: '<rect x="4" y="4.5" width="16" height="15" rx="2.5"/><path d="M4 10h16M10 10v9.5"/>',
    reload: '<path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3"/><path d="M19.8 4.5v4.8H15"/>',
    pulse: '<path d="M3 12.5h4l2.3-6 4.2 12 2.4-6H21"/>',
    chart: '<path d="M4.5 19.5h15"/><path d="M7.5 16.5V10M12 16.5v-10M16.5 16.5V12.5"/>',
    clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.2V12l3.2 2"/>',
    news: '<rect x="4" y="5" width="16" height="14" rx="2.5"/><path d="M8 9.5h8M8 13h8M8 16h5"/>',
    cube: '<path d="m12 3.8 7.5 4.2v8L12 20.2 4.5 16V8Z"/><path d="M4.8 8.2 12 12.2l7.2-4M12 12.2V20"/>',
    note: '<path d="M6.5 4h8l3.5 3.5V19a1 1 0 0 1-1 1h-10.5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1Z"/><path d="M9 12h6M9 15.5h6"/>',
    rocket: '<path d="M13.5 15.5 8.5 10.5c1-3.6 3.8-6.2 8-6.5.4 0 .9.5.9.9-.3 4.2-2.9 7-6.5 8"/><path d="M8.5 10.5 5 11l2.2 2.2M13.5 15.5 13 19l-2.2-2.2M6.5 17.5l-1.5 1.5"/><circle cx="14.6" cy="9.4" r="1.2"/>',
    cloud: '<path d="M7.5 18.5a4 4 0 0 1-.6-8 5.2 5.2 0 0 1 10 1.2 3.4 3.4 0 0 1 .1 6.8Z"/>',
    list: '<path d="M9 7h10.5M9 12h10.5M9 17h10.5"/><circle cx="5" cy="7" r=".9"/><circle cx="5" cy="12" r=".9"/><circle cx="5" cy="17" r=".9"/>',
    gear: '<circle cx="12" cy="12" r="2.8"/><path d="M12 3.5v2.3M12 18.2v2.3M3.5 12h2.3M18.2 12h2.3M6 6l1.6 1.6M16.4 16.4 18 18M18 6l-1.6 1.6M7.6 16.4 6 18"/>',
    trash: '<path d="M5 7h14M9.5 7V5h5v2M7 7l.8 12h8.4L17 7"/>',
    shield: '<path d="M12 3.5 5 6v5.5c0 4.2 2.9 7.6 7 9 4.1-1.4 7-4.8 7-9V6Z"/>',
    maximize: '<rect x="5.5" y="5.5" width="13" height="13" rx="2"/>',
    restore: '<rect x="5" y="8.5" width="10.5" height="10.5" rx="2"/><path d="M9 5.5h8a2 2 0 0 1 2 2v8"/>',
    dots: '<circle cx="6" cy="12" r="1.2"/><circle cx="12" cy="12" r="1.2"/><circle cx="18" cy="12" r="1.2"/>',
    check: '<path d="m5.5 12.5 4.2 4.2 8.8-9.2"/>',
    download: '<path d="M12 4.5v10.5M7.8 11.2 12 15.4l4.2-4.2M5 19.5h14"/>',
  } as const;
  export type GlyphName = keyof typeof G;
</script>

<script lang="ts">
  let { name, size = 18, stroke = 1.75 }: { name: GlyphName; size?: number; stroke?: number } = $props();
</script>

<svg class="glyph" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={stroke} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html G[name]}</svg>

<style>
  .glyph { display: block; flex: none; }
</style>
