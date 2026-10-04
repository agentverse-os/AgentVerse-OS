import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Приложение «Агенты» живёт под /agents/ за edge; статику отдаёт сервис cloudos-agents (rust-embed из ui/dist).
// В dev API проксируется на сервис (AGENTS_URL), который сам ходит в ядро.
export default defineConfig({
  base: '/agents/',
  plugins: [svelte()],
  build: { outDir: 'dist', emptyOutDir: true },
  server: { proxy: { '/agents/api': { target: process.env.AGENTS_URL ?? 'http://127.0.0.1:7300', ws: true } } },
});
