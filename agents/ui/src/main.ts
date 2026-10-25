import { mount } from 'svelte';
import { registerSW } from 'virtual:pwa-register';
import App from './App.svelte';
import './app.css';
import './lib/appearance.css';
mount(App, { target: document.getElementById('app')! });
// the "Agents" service worker (docs/agents-ux.md §9.8): the shell offline; updates are picked up automatically (skipWaiting +
// clientsClaim), a new build is announced by the "UI updated" banner (slice 46), not by the worker
if (typeof navigator !== 'undefined' && 'serviceWorker' in navigator && !import.meta.env.DEV) {
  try { registerSW({ immediate: true, onRegisterError: () => {} }); } catch {}
}
