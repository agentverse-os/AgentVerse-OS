import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { VitePWA } from 'vite-plugin-pwa';

// In dev the API is proxied to cloudd (CLOUDD_URL), in prod cloudd itself serves these
// statics under /.
export default defineConfig({
  plugins: [
    svelte(),
    VitePWA({
      registerType: 'autoUpdate',
      manifest: {
        name: 'AgentVerse OS', short_name: 'AgentVerse OS', start_url: '/', scope: '/', display: 'standalone',
        background_color: '#121416', theme_color: '#121416',
        // "Agents" lives inside the Desktop scope ("/"), and Chrome Android does not install it separately ("already installed");
        // the app shortcut — a long press on the Desktop icon → "Agents" → drag to the home screen: its own icon, the same
        // standalone
        shortcuts: [{ name: 'Агенты', short_name: 'Agents', description: 'Беседы с агентами, рутины, модели и расходы', url: '/agents/?src=shortcut',
          icons: [{ src: '/agents/icon-192.png', sizes: '192x192', type: 'image/png' }] }],
        icons: [{ src: '/icon-192.png', sizes: '192x192', type: 'image/png' }, { src: '/icon-512.png', sizes: '512x512', type: 'image/png', purpose: 'any' }, { src: '/icon-512-maskable.png', sizes: '512x512', type: 'image/png', purpose: 'maskable' }],
      },
      workbox: {
        // The shell is network-first (after a deploy the new version arrives at once), the cache only as an
        // offline fallback. Only hashed assets go into the precache; index.html and /api/ do not.
        globPatterns: ['assets/*.{js,css}', '*.png'],
        globIgnores: ['**/node_modules/**/*', 'wallpaper-*.webp'],  // wallpapers (landscape and portrait, 150–350 KB each) — not in the precache but cached on access; StaleWhileRevalidate — after an image change the device pulls the new one at the next open
        navigateFallback: null,
        runtimeCaching: [
          // only the Desktop shell ("/"): the "Agents" app on /agents/ lives at its own service, and its index.html through
          // this service worker (scope "/") was served on a slow network from the cloudos-shell cache — the phone showed the
          // old UI
          { urlPattern: ({ request, url }) => request.mode === 'navigate' && !url.pathname.startsWith('/agents'), handler: 'NetworkFirst', options: { cacheName: 'cloudos-shell', networkTimeoutSeconds: 4 } },
          { urlPattern: ({ url }) => /\/wallpaper-[\w-]+\.webp$/.test(url.pathname), handler: 'StaleWhileRevalidate', options: { cacheName: 'cloudos-wallpapers', expiration: { maxEntries: 24, maxAgeSeconds: 30 * 24 * 3600 } } },
        ],
        cleanupOutdatedCaches: true,
      },
    }),
  ],
  server: { proxy: { '/api': process.env.CLOUDD_URL ?? 'http://127.0.0.1:7100' } },
});
