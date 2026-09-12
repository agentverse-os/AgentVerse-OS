// Оконный менеджер Desktop (3.12): всё — окна. Системные приложения (Проекты, Store, Система, Настройки, карточка приложения)
// рендерятся как Svelte-компоненты, внешние приложения и VS Code Web — как iframes. Собственный WM на CSS transform.
// Каноническое состояние — список окон и раскладка per device-class; хранится в ядре (kv) и в localStorage.
import { api } from './api';
import { t } from './i18n.svelte';

export type WinKind = 'iframe' | 'component';
export type SysApp = 'projects' | 'store' | 'system' | 'settings' | 'appdetail' | 'project' | 'files' | 'appready' | 'vault' | 'reader' | 'setup' | 'updates';
export interface Win {
  id: string;
  kind: WinKind;
  title: string;
  /** Ключ словаря для заголовка системного окна: заголовок следует за языком интерфейса (title — снимок на момент открытия). */
  titleKey?: string;
  icon?: string | null;      // URL картинки или emoji
  url?: string;              // iframe
  component?: SysApp;        // component
  props?: Record<string, unknown>;
  x: number; y: number; w: number; h: number;
  z: number;
  minimized: boolean;
  maximized: boolean;
}
export type DeviceClass = 'phone' | 'tablet' | 'desktop';
export function deviceClass(): DeviceClass { const w = innerWidth; return w < 700 ? 'phone' : w < 1100 ? 'tablet' : 'desktop'; }

export const TASKBAR_H = 48;
const KEY = () => `cloudos.windows.${deviceClass()}`;
let zTop = 10;

/** title — ключ словаря (i18n/messages/common.ts, sys.*): показывать через sysTitle(app) или t(meta.title). */
export const SYS_APPS: Record<SysApp, { title: string; icon: string; w: number; h: number }> = {
  projects: { title: 'sys.projects', icon: '🗂️', w: 1040, h: 720 },
  store: { title: 'sys.store', icon: '🛍️', w: 1180, h: 780 },
  system: { title: 'sys.system', icon: '🩺', w: 980, h: 720 },
  settings: { title: 'sys.settings', icon: '⚙️', w: 1120, h: 760 },
  appdetail: { title: 'sys.appdetail', icon: '📦', w: 760, h: 600 },
  project: { title: 'sys.project', icon: '🧩', w: 820, h: 620 },
  files: { title: 'sys.files', icon: '📁', w: 1100, h: 700 },
  appready: { title: 'sys.appready', icon: '🔑', w: 640, h: 540 },
  vault: { title: 'sys.vault', icon: '🔐', w: 980, h: 720 },
  reader: { title: 'sys.reader', icon: '📰', w: 780, h: 680 },
  setup: { title: 'sys.setup', icon: '🧭', w: 860, h: 700 },
  updates: { title: 'sys.updates', icon: '⬆️', w: 1000, h: 740 },
};
/** Название системного приложения на текущем языке. */
export function sysTitle(app: SysApp) { return t(SYS_APPS[app].title); }
/** Заголовок окна на текущем языке: системные окна без своего имени — по ключу словаря, остальные — как открыты. */
export function winTitle(w: Win) { return w.titleKey ? t(w.titleKey) : w.title; }
// Сессии, сохранённые до локализации, хранят русские заголовки системных окон — такие окна получают ключ словаря при загрузке.
const LEGACY_TITLES: Record<string, SysApp> = { 'Проекты': 'projects', 'Store': 'store', 'Система': 'system', 'Настройки': 'settings', 'Файлы': 'files', 'Пароли и доступы': 'vault', 'Первый запуск': 'setup', 'Обновления': 'updates', 'Готово': 'appready' };
function migrate(ws: Win[]) { for (const w of ws) if (w.kind === 'component' && !w.titleKey && w.component && LEGACY_TITLES[w.title] === w.component) w.titleKey = SYS_APPS[w.component].title; return ws; }

class WindowManager {
  wins = $state<Win[]>([]);
  active = $state<string | null>(null);
  startOpen = $state(false);
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor() {
    try { const s = JSON.parse(localStorage.getItem(KEY()) ?? '[]') as Win[]; this.wins = migrate(s.filter(w => w.kind)); zTop = Math.max(10, ...this.wins.map(w => w.z)); } catch {}
    api.desktopSettings().then(s => {
      const remote = (s?.session?.[deviceClass()] as Win[] | undefined)?.filter(w => w.kind);
      if (remote && !this.wins.length) { this.wins = migrate(remote); zTop = Math.max(10, ...remote.map(w => w.z)); }
    }).catch(() => {});
  }

  private persist() {
    const snap = $state.snapshot(this.wins);
    try { localStorage.setItem(KEY(), JSON.stringify(snap)); } catch {}
    clearTimeout(this.timer);
    this.timer = setTimeout(async () => {
      try { const s = (await api.desktopSettings()) ?? {}; s.session = { ...(s.session ?? {}), [deviceClass()]: snap }; await api.putDesktopSettings(s); } catch {}
    }, 800);
  }

  private place(w: number, h: number) {
    const phone = deviceClass() === 'phone';
    const n = this.wins.length;
    const W = Math.min(w, innerWidth - 40), H = Math.min(h, innerHeight - TASKBAR_H - 40);
    return { x: phone ? 0 : Math.max(20, Math.min(innerWidth - W - 20, 60 + (n % 6) * 32)), y: phone ? 0 : Math.max(20, Math.min(innerHeight - TASKBAR_H - H - 10, 40 + (n % 6) * 28)), w: phone ? innerWidth : W, h: phone ? innerHeight - TASKBAR_H : H, maximized: phone };
  }

  /** Внешнее приложение или VS Code Web — iframe. */
  open(id: string, title: string, url: string, icon?: string | null, size?: { w: number; h: number }) {
    const ex = this.wins.find(w => w.id === id);
    if (ex) { ex.minimized = false; this.focus(id); this.persist(); return; }
    const p = this.place(size?.w ?? 1100, size?.h ?? 720);
    this.wins.push({ id, kind: 'iframe', title, url, icon, ...p, z: ++zTop, minimized: false });
    this.active = id; this.startOpen = false; this.persist();
  }
  /** Системное приложение — компонент. */
  openSys(app: SysApp, props?: Record<string, unknown>, title?: string) {
    const id = props?.name ? `${app}:${props.name}` : props?.root ? `${app}:${props.root}` : props?.link ? `${app}:${props.link}` : `sys:${app}`;
    const ex = this.wins.find(w => w.id === id);
    if (ex) { ex.minimized = false; this.focus(id); this.persist(); return; }
    const meta = SYS_APPS[app];
    const p = this.place(meta.w, meta.h);
    this.wins.push({ id, kind: 'component', component: app, props, title: title ?? t(meta.title), titleKey: title ? undefined : meta.title, icon: meta.icon, ...p, z: ++zTop, minimized: false });
    this.active = id; this.startOpen = false; this.persist();
  }
  close(id: string) { this.wins = this.wins.filter(w => w.id !== id); if (this.active === id) this.active = this.topVisible()?.id ?? null; this.persist(); }
  closeAll() { this.wins = []; this.active = null; this.persist(); }
  focus(id: string) { const w = this.wins.find(w => w.id === id); if (w) { w.z = ++zTop; w.minimized = false; this.active = id; } }
  minimize(id: string) { const w = this.wins.find(w => w.id === id); if (w) { w.minimized = true; if (this.active === id) this.active = this.topVisible()?.id ?? null; this.persist(); } }
  toggle(id: string) { const w = this.wins.find(w => w.id === id); if (!w) return; if (w.minimized || this.active !== id) this.focus(id); else this.minimize(id); this.persist(); }
  toggleMax(id: string) { const w = this.wins.find(w => w.id === id); if (w) { w.maximized = !w.maximized; this.focus(id); this.persist(); } }
  move(id: string, dx: number, dy: number) { const w = this.wins.find(w => w.id === id); if (w && !w.maximized) { w.x = Math.max(-w.w + 80, Math.min(innerWidth - 80, w.x + dx)); w.y = Math.max(0, Math.min(innerHeight - TASKBAR_H - 40, w.y + dy)); } }
  resize(id: string, dw: number, dh: number) { const w = this.wins.find(w => w.id === id); if (w && !w.maximized) { w.w = Math.max(360, w.w + dw); w.h = Math.max(240, w.h + dh); } }
  cycle() { const vis = this.wins.filter(w => !w.minimized).sort((a, b) => a.z - b.z); if (vis.length) this.focus(vis[0].id); }
  topVisible() { return this.wins.filter(w => !w.minimized).sort((a, b) => b.z - a.z)[0]; }
  save() { this.persist(); }
}
export const wm = new WindowManager();
