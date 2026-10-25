import { defineConfig, type Plugin } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { VitePWA } from 'vite-plugin-pwa';

// The "Agents" app lives under /agents/ behind the edge; the statics are served by the cloudos-agents service
// (rust-embed from ui/dist). In dev the API is proxied to the service (AGENTS_URL), which talks to the core
// itself.
/** @xterm/xterm 6.0.0: in InputHandler.requestMode (DECRQM — `CSI ? Ps $ p`, that is how bubbletea/agy, ratatui and others
 *  ask for mode support)
 * there is a dead enum IIFE `let r; (P => …)(r ||= {})`. On minification esbuild drops `let r` but keeps the assignment —
 * `void 0 || (r = {})` → a ReferenceError in strict mode, the xterm parser crashes mid-chunk, and the terminal write queue
 * freezes forever (the very first `?2026$p` request from agy crashed the card: the TUI did not render, input did not get
 * through). The dead statement is removed before minification; if the pattern disappears after an xterm update — a build
 * warning. */
function xtermDecrqmFix(): Plugin {
  return {
    name: 'xterm-decrqm-fix', enforce: 'pre',
    transform(code, id) {
      if (!/@xterm\/xterm\/lib\/xterm\.mjs$/.test(id)) return;
      const re = /let ([A-Za-z_$][\w$]*);\(([A-Za-z_$][\w$]*)=>\(\2\[\2\.NOT_RECOGNIZED=0\]=[^;]*?\)\)\(\1\|\|=\{\}\);/;
      if (!re.test(code)) { this.warn('xterm-decrqm-fix: шаблон не найден — проверьте requestMode в сборке после обновления @xterm/xterm'); return; }
      return { code: code.replace(re, ''), map: null };
    },
  };
}

export default defineConfig({
  base: '/agents/',
  plugins: [
    xtermDecrqmFix(),
    svelte(),
    // Own service worker with scope /agents/ (docs/agents-ux.md §9.8): the installed app opens offline — the shell from
    // the cache, conversations from IndexedDB (slice 71). The rule — the network first, the cache only as a fallback (the
    // lesson of slice 46): only hashed assets and icons go into the precache; index.html and /agents/api/ do not,
    // navigations — NetworkFirst.
    VitePWA({
      registerType: 'autoUpdate',
      injectRegister: null,              // registration — in main.ts (virtual:pwa-register)
      manifest: false,                   // own manifest: public/manifest.webmanifest, linked from index.html
      workbox: {
        // only entry chunks and icons go into the precache (~1.6 MB); lazy chunks (mermaid, elk, katex — another ~6 MB) are cached on
        // access
        globPatterns: ['assets/index-*.{js,css}', '*.{png,svg}'],
        globIgnores: ['**/node_modules/**/*'],
        navigateFallback: null,
        runtimeCaching: [
          { urlPattern: ({ request, url }) => request.mode === 'navigate' && url.pathname.startsWith('/agents') && !url.pathname.startsWith('/agents/api'), handler: 'NetworkFirst', options: { cacheName: 'agents-shell', networkTimeoutSeconds: 3 } },
          // hashed assets are immutable — from the cache if already seen (offline opens even the diagrams already drawn)
          { urlPattern: ({ url }) => url.pathname.startsWith('/agents/assets/'), handler: 'CacheFirst', options: { cacheName: 'agents-assets', expiration: { maxEntries: 80, maxAgeSeconds: 30 * 24 * 3600 } } },
        ],
        cleanupOutdatedCaches: true,
        skipWaiting: true,
        clientsClaim: true,
        // push, notification clicks, background sending of the outbox (§9.9) — a separate file next to the
        // worker
        importScripts: ['sw-extra.js'],
      },
    }),
  ],
  build: { outDir: 'dist', emptyOutDir: true },
  server: { proxy: { '/agents/api': { target: process.env.AGENTS_URL ?? 'http://127.0.0.1:7300', ws: true } } },
});
