// Виджеты рабочего стола: слой между обоями и окнами. Каталог типов, экземпляры с раскладкой per device-class,
// хранение в ядре (kv desktop.settings.widgets) и localStorage. На телефоне — столбец без перетаскивания.
import { api } from './api';
import { deviceClass } from './windows.svelte';

export type WidgetType = 'monitor' | 'news' | 'clock' | 'projects' | 'health' | 'events' | 'notes' | 'apps' | 'weather';
export interface WidgetInst { id: string; type: WidgetType; x: number; y: number; w: number; h: number; config: Record<string, any> }

/** title и description — ключи словаря (i18n/messages/widgets.ts): показывать через t(...). */
export const WIDGETS: Record<WidgetType, { title: string; icon: string; w: number; h: number; description: string; defaults: Record<string, any> }> = {
  monitor: { title: 'widgets.monitor.title', icon: '📈', w: 420, h: 300, description: 'widgets.monitor.desc', defaults: { show: 'all' } },
  news: { title: 'widgets.news.title', icon: '📰', w: 420, h: 380, description: 'widgets.news.desc', defaults: { interval: 15, images: true, compact: false, preset: 'ai', count: 10, feeds: [] as string[], lang: 'both' } },
  clock: { title: 'widgets.clock.title', icon: '🕒', w: 260, h: 140, description: 'widgets.clock.desc', defaults: { tz: '', seconds: false } },
  projects: { title: 'widgets.projects.title', icon: '🧩', w: 340, h: 220, description: 'widgets.projects.desc', defaults: {} },
  health: { title: 'widgets.health.title', icon: '🩺', w: 300, h: 160, description: 'widgets.health.desc', defaults: {} },
  events: { title: 'widgets.events.title', icon: '🗒', w: 420, h: 240, description: 'widgets.events.desc', defaults: { count: 12 } },
  notes: { title: 'widgets.notes.title', icon: '📝', w: 300, h: 220, description: 'widgets.notes.desc', defaults: { text: '' } },
  apps: { title: 'widgets.apps.title', icon: '🚀', w: 320, h: 160, description: 'widgets.apps.desc', defaults: {} },
  weather: { title: 'widgets.weather.title', icon: '🌤', w: 300, h: 220, description: 'widgets.weather.desc', defaults: { city: '', lat: 0, lon: 0 } },
};

/** label — ключ словаря. */
export const NEWS_PRESETS: Record<string, { label: string; feeds: { url: string; lang: 'en' | 'ru' }[] }> = {
  ai: { label: 'widgets.news.preset.ai', feeds: [
    { url: 'https://hnrss.org/newest?q=AI+OR+LLM+OR+GPT+OR+Claude&points=50', lang: 'en' },
    { url: 'https://www.theverge.com/rss/ai-artificial-intelligence/index.xml', lang: 'en' },
    { url: 'https://feeds.arstechnica.com/arstechnica/technology-lab', lang: 'en' },
    { url: 'https://techcrunch.com/category/artificial-intelligence/feed/', lang: 'en' },
    { url: 'https://www.technologyreview.com/topic/artificial-intelligence/feed', lang: 'en' },
    { url: 'https://habr.com/ru/rss/hubs/artificial_intelligence/articles/?fl=ru', lang: 'ru' },
    { url: 'https://habr.com/ru/rss/hubs/machine_learning/articles/?fl=ru', lang: 'ru' },
  ] },
  hitech: { label: 'widgets.news.preset.hitech', feeds: [
    { url: 'https://hnrss.org/frontpage', lang: 'en' },
    { url: 'https://www.theverge.com/rss/index.xml', lang: 'en' },
    { url: 'https://feeds.arstechnica.com/arstechnica/index', lang: 'en' },
    { url: 'https://www.wired.com/feed/rss', lang: 'en' },
    { url: 'https://habr.com/ru/rss/news/?fl=ru', lang: 'ru' },
    { url: 'https://3dnews.ru/news/rss/', lang: 'ru' },
  ] },
  custom: { label: 'widgets.news.preset.custom', feeds: [] },
};

const KEY = () => `cloudos.widgets.${deviceClass()}`;

class WidgetStore {
  items = $state<WidgetInst[]>([]);
  picker = $state(false);
  private timer: ReturnType<typeof setTimeout> | undefined;
  private loaded = false;

  constructor() {
    // пустой список считается «настройкой» только если пользователь явно убрал все виджеты (маркер cleared)
    const cleared = () => { try { return localStorage.getItem(`${KEY()}.cleared`) === '1'; } catch { return false; } };
    try { const local = JSON.parse(localStorage.getItem(KEY()) ?? 'null') as WidgetInst[] | null; this.items = local && (local.length || cleared()) ? local : this.defaults(); } catch { this.items = this.defaults(); }
    api.desktopSettings().then(s => { const r = s?.widgets?.[deviceClass()] as WidgetInst[] | undefined; if (r && (r.length || cleared())) this.items = r; this.loaded = true; }).catch(() => { this.loaded = true; });
  }
  defaults(): WidgetInst[] {
    const phone = deviceClass() === 'phone';
    return [
      { id: 'w-monitor', type: 'monitor', x: innerWidth - 440, y: 20, w: 420, h: 300, config: { ...WIDGETS.monitor.defaults } },
      { id: 'w-clock', type: 'clock', x: innerWidth - 440, y: 340, w: 260, h: 140, config: { ...WIDGETS.clock.defaults } },
      ...(phone ? [] : [{ id: 'w-news', type: 'news' as WidgetType, x: innerWidth - 440, y: 500, w: 420, h: 340, config: { ...WIDGETS.news.defaults } }]),
    ];
  }
  private persist() {
    const snap = $state.snapshot(this.items);
    try { localStorage.setItem(KEY(), JSON.stringify(snap)); } catch {}
    clearTimeout(this.timer);
    this.timer = setTimeout(async () => {
      try { const s = (await api.desktopSettings()) ?? {}; s.widgets = { ...(s.widgets ?? {}), [deviceClass()]: snap }; await api.putDesktopSettings(s); } catch {}
    }, 800);
  }
  add(type: WidgetType, config?: Record<string, any>) {
    try { localStorage.setItem(`${KEY()}.cleared`, '0'); } catch {}
    const meta = WIDGETS[type]; const n = this.items.length;
    this.items.push({ id: `w-${type}-${Date.now().toString(36)}`, type, x: Math.max(12, innerWidth - meta.w - 40 - (n % 3) * 30), y: 20 + (n % 5) * 40, w: meta.w, h: meta.h, config: { ...meta.defaults, ...(config ?? {}) } });
    this.picker = false; this.persist();
  }
  remove(id: string) { this.items = this.items.filter(w => w.id !== id); try { localStorage.setItem(`${KEY()}.cleared`, this.items.length ? '0' : '1'); } catch {} this.persist(); }
  update(id: string, patch: Partial<WidgetInst>) { const w = this.items.find(w => w.id === id); if (w) Object.assign(w, patch); this.persist(); }
  setConfig(id: string, patch: Record<string, any>) { const w = this.items.find(w => w.id === id); if (w) w.config = { ...w.config, ...patch }; this.persist(); }
  move(id: string, dx: number, dy: number) { const w = this.items.find(w => w.id === id); if (w) { w.x = Math.max(0, Math.min(innerWidth - 60, w.x + dx)); w.y = Math.max(0, Math.min(innerHeight - 120, w.y + dy)); } }
  resize(id: string, dw: number, dh: number) { const w = this.items.find(w => w.id === id); if (w) { w.w = Math.max(200, w.w + dw); w.h = Math.max(100, w.h + dh); } }
  save() { this.persist(); }
}
export const widgets = new WidgetStore();
