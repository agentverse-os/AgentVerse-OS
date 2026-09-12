import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { VitePWA } from 'vite-plugin-pwa';

// В dev API проксируется на cloudd (CLOUDD_URL), в prod cloudd сам отдаёт эту статику под /.
export default defineConfig({
  plugins: [
    svelte(),
    VitePWA({
      registerType: 'autoUpdate',
      manifest: {
        name: 'AgentVerse OS', short_name: 'AgentVerse OS', start_url: '/', scope: '/', display: 'standalone',
        background_color: '#121416', theme_color: '#121416',
        icons: [{ src: '/icon-192.png', sizes: '192x192', type: 'image/png' }, { src: '/icon-512.png', sizes: '512x512', type: 'image/png', purpose: 'any' }, { src: '/icon-512-maskable.png', sizes: '512x512', type: 'image/png', purpose: 'maskable' }],
      },
      workbox: {
        // Оболочка — сеть в приоритете (после деплоя новая версия приходит сразу), кэш — только как офлайн-запас.
        // В precache — только хэшированные ассеты; index.html и /api/ туда не попадают.
        globPatterns: ['assets/*.{js,css}', '*.png'],
        globIgnores: ['**/node_modules/**/*', 'wallpaper-*.webp'],  // обои (альбомные и портретные, по 150–350 КБ) — не в precache, а в кэш по обращению; StaleWhileRevalidate — после смены картинки устройство подтянет новую при следующем открытии
        navigateFallback: null,
        runtimeCaching: [
          { urlPattern: ({ request }) => request.mode === 'navigate', handler: 'NetworkFirst', options: { cacheName: 'cloudos-shell', networkTimeoutSeconds: 4 } },
          { urlPattern: ({ url }) => /\/wallpaper-[\w-]+\.webp$/.test(url.pathname), handler: 'StaleWhileRevalidate', options: { cacheName: 'cloudos-wallpapers', expiration: { maxEntries: 24, maxAgeSeconds: 30 * 24 * 3600 } } },
        ],
        cleanupOutdatedCaches: true,
      },
    }),
  ],
  server: { proxy: { '/api': process.env.CLOUDD_URL ?? 'http://127.0.0.1:7100' } },
});
