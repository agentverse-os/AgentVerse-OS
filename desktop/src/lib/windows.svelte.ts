// Desktop window manager (3.12): everything is a window. System apps (Projects, Store, System, Settings, the app card) render
// as Svelte components, external apps and VS Code Web — as iframes. Its own WM on CSS transform. The canonical state is the
// window list and the layout per device class; stored in the core (kv) and in localStorage.
//
// Geometry: a window keeps its floating rectangle (x, y, w, h) as the user left it. What is drawn is `rect(w)`: the floating
// rectangle clamped into the current work area, a snap zone (half, third, two thirds) or the whole work area when maximized.
// So a layout saved on a wide monitor stays reachable on a laptop without being rewritten, and snapped windows follow the
// viewport like tiles.
import { api } from './api';
import { t } from './i18n.svelte';

export type WinKind = 'iframe' | 'component';
export type SysApp = 'projects' | 'store' | 'system' | 'settings' | 'appdetail' | 'project' | 'files' | 'appready' | 'vault' | 'reader' | 'setup' | 'updates' | 'agents';
/** Snap zones: halves, and on wide screens thirds and two thirds. */
export type SnapZone = 'left' | 'right' | 'l3' | 'c3' | 'r3' | 'l23' | 'r23';
export interface Win {
  id: string;
  kind: WinKind;
  title: string;
  /** Dictionary key for a system window title: the title follows the UI language (title is a snapshot at open time). */
  titleKey?: string;
  icon?: string | null;      // an image URL or an emoji
  url?: string;              // iframe
  component?: SysApp;        // component
  props?: Record<string, unknown>;
  x: number; y: number; w: number; h: number;
  z: number;
  minimized: boolean;
  maximized: boolean;
  snap?: SnapZone | null;
}
export type DeviceClass = 'phone' | 'tablet' | 'desktop';
/** phone — the mobile shell (one app full screen): narrow screens, low landscape screens (a phone on its side) and touch
 *  screens up to a small tablet (an unfolded foldable, a tablet in portrait); tablet and desktop — windows. */
export function deviceClass(): DeviceClass {
  const w = innerWidth, h = innerHeight;
  const coarse = typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches;
  if (w < 700 || (h < 520 && w < 1000) || (coarse && w < 900)) return 'phone';
  return w < 1100 ? 'tablet' : 'desktop';
}

/** Height of the attached taskbar; kept for the code that only needs a number. The real strip is shellBottom(). */
export const TASKBAR_H = 48;
/** The strip the shell keeps at the bottom of the screen — mirrors --shell-b in app.css: the dock of the umbrel look, the
 *  floating shelf of the glass surface, the attached taskbar otherwise. */
export function shellBottom(): number {
  const d = typeof document !== 'undefined' ? document.documentElement.dataset : ({} as DOMStringMap);
  return d.look === 'umbrel' ? 96 : d.surface === 'glass' ? 70 : 48;
}
/** Gap between snapped windows and the screen edges: floating looks keep air, the attached taskbar tiles edge to edge. */
function tileGap(): number {
  const d = typeof document !== 'undefined' ? document.documentElement.dataset : ({} as DOMStringMap);
  return d.look === 'umbrel' ? 12 : d.surface === 'glass' ? 8 : 0;
}
export interface Rect { x: number; y: number; w: number; h: number }
const MIN_W = 340, MIN_H = 220;
const KEY = () => `cloudos.windows.${deviceClass()}`;
let zTop = 10;

/** title — a dictionary key (i18n/messages/common.ts, sys.*): show via sysTitle(app) or t(meta.title). */
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
  agents: { title: 'sys.agents', icon: '🤖', w: 1180, h: 760 },  // ADR-006: agent sessions and workspace terminals
};
/** System app name in the current language. */
export function sysTitle(app: SysApp) { return t(SYS_APPS[app].title); }
/** Window title in the current language: system windows without their own name — by the dictionary key, the rest —
 *  as opened. */
export function winTitle(w: Win) { return w.titleKey ? t(w.titleKey) : w.title; }
// Sessions saved before localization hold Russian titles of system windows — such windows get a dictionary key on load.
const LEGACY_TITLES: Record<string, SysApp> = { 'Проекты': 'projects', 'Store': 'store', 'Система': 'system', 'Настройки': 'settings', 'Файлы': 'files', 'Пароли и доступы': 'vault', 'Первый запуск': 'setup', 'Обновления': 'updates', 'Готово': 'appready' };
function migrate(ws: Win[]) { for (const w of ws) if (w.kind === 'component' && !w.titleKey && w.component && LEGACY_TITLES[w.title] === w.component) w.titleKey = SYS_APPS[w.component].title; return ws; }

class WindowManager {
  wins = $state<Win[]>([]);
  active = $state<string | null>(null);
  startOpen = $state(false);
  /** Viewport size as state: rect() and the snap zones depend on it. */
  vw = $state(typeof innerWidth === 'number' ? innerWidth : 1280);
  vh = $state(typeof innerHeight === 'number' ? innerHeight : 800);
  /** Bumped when the look or the surface changes (the bottom strip has another height). */
  shellRev = $state(0);
  /** The window that is moving to a new place by itself (maximize, restore, snap): its frame is animated for a moment. */
  settling = $state<string | null>(null);
  private settleTimer: ReturnType<typeof setTimeout> | undefined;
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor() {
    try { const s = JSON.parse(localStorage.getItem(KEY()) ?? '[]') as Win[]; this.wins = migrate(s.filter(w => w.kind)); zTop = Math.max(10, ...this.wins.map(w => w.z)); } catch {}
    // the top window of the saved session is the active one again; the phone starts from the home screen as before
    const restoreActive = () => { if (deviceClass() !== 'phone') this.active = this.topVisible()?.id ?? null; };
    restoreActive();
    api.desktopSettings().then(s => {
      const remote = (s?.session?.[deviceClass()] as Win[] | undefined)?.filter(w => w.kind);
      if (remote && !this.wins.length) { this.wins = migrate(remote); zTop = Math.max(10, ...remote.map(w => w.z)); restoreActive(); }
    }).catch(() => {});
    if (typeof window !== 'undefined') {
      window.addEventListener('resize', () => { this.vw = innerWidth; this.vh = innerHeight; });
      // the look and the surface are attributes of <html> (theme.ts → apply)
      new MutationObserver(() => { this.shellRev++; }).observe(document.documentElement, { attributes: true, attributeFilter: ['data-look', 'data-surface'] });
    }
  }

  private persist() {
    const snap = $state.snapshot(this.wins);
    try { localStorage.setItem(KEY(), JSON.stringify(snap)); } catch {}
    clearTimeout(this.timer);
    this.timer = setTimeout(async () => {
      try { const s = (await api.desktopSettings()) ?? {}; s.session = { ...(s.session ?? {}), [deviceClass()]: snap }; await api.putDesktopSettings(s); } catch {}
    }, 800);
  }

  /** The area windows live in: the viewport without the bottom strip of the shell. */
  workArea(): Rect { void this.shellRev; return { x: 0, y: 0, w: this.vw, h: Math.max(200, this.vh - shellBottom()) }; }
  /** Wide enough for thirds: 21:9 and wider, or simply a lot of pixels. */
  get wide() { return this.vw >= 2200 || (this.vw >= 1700 && this.vw / this.vh >= 2.1); }
  /** Snap zones on offer for this screen, in the order of the layout picker. */
  get zones(): SnapZone[] { return this.wide ? ['left', 'right', 'l3', 'c3', 'r3', 'l23', 'r23'] : ['left', 'right']; }
  zoneRect(zone: SnapZone): Rect {
    const a = this.workArea(), g = tileGap();
    const span = (from: number, to: number): Rect => {
      const x0 = a.x + a.w * from, x1 = a.x + a.w * to;
      return { x: Math.round(x0 + (from === 0 ? g : g / 2)), y: a.y + g, w: Math.round(x1 - x0 - (from === 0 ? g : g / 2) - (to === 1 ? g : g / 2)), h: a.h - g * 2 };
    };
    switch (zone) {
      case 'left': return span(0, 1 / 2); case 'right': return span(1 / 2, 1);
      case 'l3': return span(0, 1 / 3); case 'c3': return span(1 / 3, 2 / 3); case 'r3': return span(2 / 3, 1);
      case 'l23': return span(0, 2 / 3); case 'r23': return span(1 / 3, 1);
    }
  }
  /** The rectangle a window is drawn in (not for the phone — there a window is laid out by CSS). A maximized window takes the
   *  whole work area; in the umbrel look it floats with margins. */
  rect(w: Win): Rect {
    const a = this.workArea();
    if (w.maximized) { const m = typeof document !== 'undefined' && document.documentElement.dataset.look === 'umbrel' ? 12 : 0; return { x: a.x + m, y: a.y + m, w: a.w - m * 2, h: a.h - m }; }
    if (w.snap) return this.zoneRect(w.snap);
    const W = Math.min(w.w, Math.max(MIN_W, a.w - 16)), H = Math.min(w.h, Math.max(MIN_H, a.h - 16));
    return { x: Math.round(Math.max(-W + 120, Math.min(w.x, a.w - 120))), y: Math.round(Math.max(0, Math.min(w.y, a.h - 44))), w: W, h: H };
  }
  /** Write the drawn rectangle into the window before a move or a resize, so the gesture starts from what is on screen. */
  adopt(id: string) { const w = this.wins.find(w => w.id === id); if (w && !w.maximized && !w.snap) Object.assign(w, this.rect(w)); }

  private place(w: number, h: number) {
    const phone = deviceClass() === 'phone';
    if (phone) return { x: 0, y: 0, w: innerWidth, h: innerHeight - TASKBAR_H, maximized: false };
    const a = this.workArea(), m = 16;
    const W = Math.min(w, a.w - m * 2), H = Math.min(h, a.h - m * 2);
    // a cascade around the centre: a new window lands in the middle of the screen, the next ones step aside
    const n = this.wins.filter(x => !x.minimized).length, step = [0, 1, 2, -1, -2, 3][n % 6] * 28;
    const x = Math.max(m, Math.min(a.w - W - m, Math.round((a.w - W) / 2 + step)));
    const y = Math.max(m, Math.min(a.h - H - m, Math.round((a.h - H) / 2 + step * 0.75)));
    return { x, y, w: W, h: H, maximized: false };
  }

  /** An external app or VS Code Web — an iframe. */
  open(id: string, title: string, url: string, icon?: string | null, size?: { w: number; h: number }) {
    const ex = this.wins.find(w => w.id === id);
    if (ex) { ex.minimized = false; this.focus(id); this.persist(); return; }
    const p = this.place(size?.w ?? 1100, size?.h ?? 720);
    this.wins.push({ id, kind: 'iframe', title, url, icon, ...p, z: ++zTop, minimized: false });
    this.active = id; this.startOpen = false; this.persist();
  }
  /** A system app — a component. */
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
  private settle(id: string) { this.settling = id; clearTimeout(this.settleTimer); this.settleTimer = setTimeout(() => { this.settling = null; }, 260); }
  toggleMax(id: string) { const w = this.wins.find(w => w.id === id); if (w) { this.settle(id); if (w.snap) { w.snap = null; w.maximized = true; } else w.maximized = !w.maximized; this.focus(id); this.persist(); } }
  /** Snap into a zone; null returns the window to its floating rectangle. */
  snap(id: string, zone: SnapZone | null) { const w = this.wins.find(w => w.id === id); if (w) { this.settle(id); w.snap = zone; w.maximized = false; this.focus(id); this.persist(); } }
  move(id: string, dx: number, dy: number) {
    const w = this.wins.find(w => w.id === id);
    if (w && !w.maximized && !w.snap) { const a = this.workArea(); w.x = Math.max(-w.w + 120, Math.min(a.w - 120, w.x + dx)); w.y = Math.max(0, Math.min(a.h - 44, w.y + dy)); }
  }
  resize(id: string, dw: number, dh: number) { this.resizeDir(id, 'se', dw, dh); }
  /** Resize by an edge or a corner: dir is a combination of n, s, e, w. The opposite edge stays in place. */
  resizeDir(id: string, dir: string, dx: number, dy: number) {
    const w = this.wins.find(w => w.id === id);
    if (!w || w.maximized || w.snap) return;
    const minW = Math.min(MIN_W, this.vw - 16), minH = MIN_H;
    if (dir.includes('e')) w.w = Math.max(minW, w.w + dx);
    if (dir.includes('s')) w.h = Math.max(minH, w.h + dy);
    if (dir.includes('w')) { const nw = Math.max(minW, w.w - dx); w.x += w.w - nw; w.w = nw; }
    if (dir.includes('n')) { const nh = Math.max(minH, w.h - dy), ny = w.y + (w.h - nh); if (ny >= 0) { w.y = ny; w.h = nh; } }
  }
  cycle() { const vis = this.wins.filter(w => !w.minimized).sort((a, b) => a.z - b.z); if (vis.length) this.focus(vis[0].id); }
  topVisible() { return this.wins.filter(w => !w.minimized).sort((a, b) => b.z - a.z)[0]; }
  save() { this.persist(); }
}
export const wm = new WindowManager();
