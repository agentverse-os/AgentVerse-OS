// State of the "Agents" app: data (workplaces, inbox, models — from the cloudos-agents service), UI state (section,
// Code/Thread view, selection, canvas, panels) and actions on sessions (through the core API via the service proxy). The
// canvas layout is stored in the core (kv agents.layout) — shared across all devices; the rest lives in the browser.
import { api, agents, type Workspaces, type WsProject, type SessionInfo, type PtyInfo, type Inbox, type Models, type Account, type AgentDef, type Skill, type Routine, type Run, type ChatStatus, type SimpleChat, type ChatRoutes, type Engine, type BrowserPrefs, type Limit } from './api';
import { clearChatCache } from './chatCache';
import { familyWindow } from './adapters';
import { toast } from './toast.svelte';
import type { MenuItem } from './menu.svelte';
import { t, fmt } from './i18n';

export type Section = 'work' | 'inbox' | 'chat' | 'routines' | 'models' | 'settings';
export type Ref = { project: string; kind: 'pty' | 'session'; id: string };
/** Canvas card (§4.11): `w` columns (1–3) × `h` rows of 100 px (2–12; 4 ≈ the previous 440 px) */
export type Card = Ref & { w: number; h: number };
export const ROW_UNIT = 100, GRID_GAP = 14, H_DEF = 4, H_MIN = 2, H_MAX = 12, W_MAX = 3;
/** Old layouts stored the height as 1 | 2 (rows of 440 px) — convert to rows of 100 px */
export function normCard(c: Card): Card { const h = c.h <= 2 ? c.h * 4 : c.h; return { ...c, w: Math.min(W_MAX, Math.max(1, Math.round(c.w || 1))), h: Math.min(H_MAX, Math.max(H_MIN, Math.round(h || H_DEF))) }; }
// names are getters: recomputed on a language change (reactive t())
export const SECTIONS: { id: Section; icon: string; readonly name: string }[] = [
  { id: 'work', icon: 'grid', get name() { return t('sec.work'); } }, { id: 'inbox', icon: 'inbox', get name() { return t('sec.inbox'); } }, { id: 'chat', icon: 'chat', get name() { return t('sec.chat'); } },
  { id: 'routines', icon: 'repeat', get name() { return t('sec.routines'); } }, { id: 'models', icon: 'layers', get name() { return t('sec.models'); } }, { id: 'settings', icon: 'settings', get name() { return t('sec.settings'); } },
];
export const key = (r: Ref) => `${r.project}:${r.id}`;
export const TREE_LIMIT = 8;
export const activity = (s: SessionInfo) => s.updated_at || s.created_at;
export const same = (a: Ref, b: Ref) => a.project === b.project && a.id === b.id;
export const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
export const phone = () => typeof matchMedia === 'function' && matchMedia('(max-width: 599px)').matches;
/** Touch device without hover: Enter in the composer inserts a line break, sending is by button */
/** Reset the UI cache: all CacheStorage (including Desktop's cloudos-shell service worker) and a reload bypassing the cache */
export async function hardReload() {
  try { await clearChatCache(); } catch {}   // conversation cache on the device (§4.3.16)
  try { localStorage.removeItem('agents.ws'); } catch {}   // tree snapshot for opening offline (§9.8)
  try { if ('caches' in window) { for (const k of await caches.keys()) await caches.delete(k); } } catch {}
  try { if ('serviceWorker' in navigator) { for (const r of await navigator.serviceWorker.getRegistrations()) await r.update(); } } catch {}
  const u = new URL(location.href); u.searchParams.set('r', String(Date.now())); location.replace(u.toString());
}
export const touch = () => typeof matchMedia === 'function' && matchMedia('(hover: none) and (pointer: coarse)').matches;
const narrow = () => typeof matchMedia === 'function' && matchMedia('(max-width: 1099px)').matches;
const ls = (k: string) => { try { return localStorage.getItem(k); } catch { return null; } };

// ---------- data ----------
class Data {
  ws = $state<Workspaces | null>(null);
  inbox = $state<Inbox | null>(null);
  models = $state<Models | null>(null);
  accounts = $state<Account[]>([]);
  projectAccounts = $state<Record<string, string>>({});
  /** Accounts blocked by a limit hit (§9.7): id → RFC3339 */
  accountsBlocked = $state<Record<string, string>>({});
  error = $state('');
  health = $state<{ ok: boolean; cloudd: boolean; version: string; cloudd_version?: string | null; hub_subscriptions?: number; hub_derived?: { inbox: boolean; chat: boolean } } | null>(null);
  loading = $state(false);
  wsLoading = $state(false);
  wsError = $state('');
  /** The tree is shown from the last snapshot on the device (no connection, §9.8): the snapshot timestamp; 0
   *  — live */
  wsOffline = $state(0);
  private wsSaveT: ReturnType<typeof setTimeout> | null = null;
  /** The last tree snapshot — in localStorage (≈75 KB) so the installed app opens offline: conversations are then drawn from the feed cache */
  private persistWs() {
    if (this.wsSaveT) clearTimeout(this.wsSaveT);
    this.wsSaveT = setTimeout(() => { this.wsSaveT = null; try { if (this.ws) localStorage.setItem('agents.ws', JSON.stringify({ at: Date.now(), ws: $state.snapshot(this.ws) })); } catch {} }, 2000);
  }
  private restoreWs(): boolean {
    try {
      const raw = localStorage.getItem('agents.ws'); if (!raw) return false;
      const c = JSON.parse(raw) as { at: number; ws: Workspaces }; if (!c?.ws?.projects) return false;
      this.ws = c.ws; this.wsOffline = c.at || Date.now(); return true;
    } catch { return false; }
  }
  inboxLoading = $state(false);
  inboxError = $state('');
  modelsLoading = $state(false);
  modelsError = $state('');
  modelsToday = $state<Models | null>(null);
  modelsTodayError = $state('');
  accountsLoading = $state(false);
  accountsError = $state('');
  chatLoading = $state(false);
  chatError = $state('');
  chatsError = $state('');
  chatsLoaded = $state(false);
  routinesLoading = $state(false);
  routinesError = $state('');
  private inboxSeq = 0;
  private modelsSeq = 0;
  private accountsSeq = 0;
  private chatSeq = 0;
  private routinesSeq = 0;
  /** Number of the last tree request: the answer to an earlier request (a periodic poll started before the session was created)
   *  does not overwrite
   * a fresher one — otherwise a new session flickered in the tree until the
   * next poll */
  /** Event hub (ADR-007): the tree and statuses arrive by push; while the hub is alive, polling
   *  /workspaces is not needed */
  hubLive = $state(false);
  /** Derived values have already arrived over the hub (otherwise the section
   *  loads them itself once) */
  hubInbox = $state(false);
  hubChat = $state(false);
  private hubSock: WebSocket | null = null;
  private hubWait = 2;
  private hubTimer: ReturnType<typeof setTimeout> | null = null;
  private hubLast = 0;
  private hubWatch: ReturnType<typeof setInterval> | null = null;
  private netWatched = false;
  /** The network came back or the app left the background (§9.10): do not wait for the reconnect pause, bring
   *  the hub up right away;
   * if the socket counts as alive — check it with a ping: no frame within 5 s (half-open after a network change) — close and bring
   * up anew */
  kickHub() {
    if (typeof WebSocket === 'undefined') return;
    const sock = this.hubSock;
    if (sock) {
      if (sock.readyState !== WebSocket.OPEN) return;
      // open, but there was never a snapshot (frames got lost) — do not check, bring up
      // anew
      if (!this.hubLive) { this.hubWait = 2; try { sock.close(); } catch {} return; }
      const before = this.hubLast; try { sock.send('{"kind":"ping"}'); } catch {}
      setTimeout(() => { if (this.hubSock === sock && this.hubLast === before) { this.hubWait = 2; try { sock.close(); } catch {} } }, 5000);
      return;
    }
    if (this.hubTimer) { clearTimeout(this.hubTimer); this.hubTimer = null; }
    this.hubWait = 2; this.connectHub();
  }
  private watchNetwork() {
    if (this.netWatched || typeof window === 'undefined') return; this.netWatched = true;
    window.addEventListener('online', () => this.kickHub());
    document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') this.kickHub(); });
  }
  connectHub() {
    if (this.hubSock || typeof WebSocket === 'undefined') return;
    this.watchNetwork();
    const url = `${location.origin.replace(/^http/, 'ws')}/agents/api/hub`;
    let sock: WebSocket;
    try { sock = new WebSocket(url); } catch { return; }
    this.hubSock = sock; this.hubLast = Date.now();
    // silence watchdog: the service sends a frame or a ping at least every 25 s; a minute without frames means a half-open socket (a network change on the
    // phone), close and reconnect
    if (this.hubWatch) clearInterval(this.hubWatch);
    this.hubWatch = setInterval(() => { if (this.hubSock === sock && Date.now() - this.hubLast > 60000) { try { sock.close(); } catch {} } }, 10000);
    sock.onopen = () => { this.hubWait = 2; this.hubLast = Date.now(); };
    sock.onmessage = (ev) => {
      this.hubLast = Date.now();
      let m: any; try { m = JSON.parse(String(ev.data)); } catch { return; }
      if (m.kind === 'workspaces') { this.ws = { projects: m.projects ?? [], today: m.today ?? this.ws?.today ?? { cost_usd: 0, turns: 0 } } as Workspaces; this.wsError = ''; this.error = ''; this.wsOffline = 0; if (!this.hubLive) this.hubLive = true; ui.prune(); this.persistWs(); for (const p of m.projects ?? []) this.noteEnginesV(p.name, p.engines_v); }
      else if (m.kind === 'project' && this.ws) {
        const p = this.ws.projects.find(x => x.name === m.name);
        if (p) { p.sessions = m.sessions ?? []; p.ptys = m.ptys ?? []; p.reachable = m.reachable ?? p.reachable; this.ws = { ...this.ws, projects: [...this.ws.projects] }; ui.prune(); this.noteEnginesV(m.name, m.engines_v); }
        else this.loadWs();
      }
      // slice 60: the service derives "Inbox" and the "Chat" status from the same snapshot and sends them on change;
      // polling — only without the hub
      else if (m.kind === 'inbox') { this.inbox = { waiting: m.waiting ?? [], failed: m.failed ?? [], done: m.done ?? [], today: m.today ?? this.inbox?.today ?? { cost_usd: 0, turns: 0 } } as Inbox; this.inboxError = ''; this.inboxLoading = false; this.hubInbox = true; this.syncBadge(); }
      else if (m.kind === 'chat') { const { kind: _k, ...st } = m; this.chat = st as ChatStatus; this.chatError = ''; this.hubChat = true; }
      else if (m.kind === 'chats') { if (ui.section === 'chat') this.loadChats(); }
      else if (m.kind === 'limits') { this.limits = (m.limits ?? []) as Limit[]; }
    };
    const down = () => {
      if (this.hubSock !== sock) return;
      this.hubSock = null; this.hubLive = false; this.hubInbox = false; this.hubChat = false;
      if (this.hubWatch) { clearInterval(this.hubWatch); this.hubWatch = null; }
      const w = this.hubWait; this.hubWait = Math.min(30, w * 2);
      if (this.hubTimer) clearTimeout(this.hubTimer);
      this.hubTimer = setTimeout(() => { this.hubTimer = null; this.connectHub(); }, w * 1000);
    };
    sock.onclose = down; sock.onerror = () => { try { sock.close(); } catch {} };
  }
  private wsSeq = 0;
  async loadWs() {
    const n = ++this.wsSeq; this.wsLoading = true;
    try { const ws = await agents.workspaces(); if (n !== this.wsSeq) return; this.ws = ws; this.wsError = ''; this.error = ''; this.wsOffline = 0; this.persistWs(); }
    catch (e) { if (n === this.wsSeq) { this.wsError = msg(e); this.error = this.wsError; if (!this.ws) this.restoreWs(); } }
    finally { if (n === this.wsSeq) this.wsLoading = false; }
  }
  async loadInbox() {
    const seq = ++this.inboxSeq; this.inboxLoading = true;
    try { const value = await agents.inbox(); if (seq !== this.inboxSeq) return false; this.inbox = value; this.inboxError = ''; this.syncBadge(); return true; }
    catch (e) { if (seq === this.inboxSeq) this.inboxError = msg(e); return false; }
    finally { if (seq === this.inboxSeq) this.inboxLoading = false; }
  }
  /** Model context window from the price catalog (Settings → Routes, "window"), otherwise by family; null — unknown */
  contextOf(model: string | null | undefined): number | null {
    if (!model) return null;
    const base = model.replace(/\[1m\]$/, '').replace(/:batch$/, '');
    const row = this.models?.models.find(m => m.id === base);
    const c = row?.price?.context; if (typeof c === 'number' && c > 0) return c;
    return familyWindow(model);
  }
  async loadModels(days = 1) {
    const seq = ++this.modelsSeq; this.modelsLoading = true;
    const period = agents.models(days);
    const [report, today] = await Promise.allSettled([period, days === 1 ? period : agents.models(1)]);
    if (seq !== this.modelsSeq) return false;
    if (report.status === 'fulfilled') { this.models = report.value; this.modelsError = ''; } else this.modelsError = msg(report.reason);
    if (today.status === 'fulfilled') { this.modelsToday = today.value; this.modelsTodayError = ''; } else this.modelsTodayError = msg(today.reason);
    this.modelsLoading = false; return report.status === 'fulfilled' && today.status === 'fulfilled';
  }
  async loadAccounts() {
    const seq = ++this.accountsSeq; this.accountsLoading = true;
    try { const a = await api.accounts(); if (seq !== this.accountsSeq) return false; this.accounts = a.accounts; this.projectAccounts = a.defaults; this.accountsBlocked = a.blocked ?? {}; this.accountsError = ''; return true; }
    catch (e) { if (seq === this.accountsSeq) this.accountsError = msg(e); return false; }
    finally { if (seq === this.accountsSeq) this.accountsLoading = false; }
  }
  /** Browser in the session (docs/agents-ux.md §4.9): per-workspace settings from the service; off until the owner
   *  enables it */
  browser = $state<Record<string, BrowserPrefs>>({});
  browserLoaded = false;
  async loadBrowser(force = false) { if (this.browserLoaded && !force) return; try { const s = await agents.settings(); this.browser = s.browser ?? {}; this.browserLoaded = true; } catch {} }
  browserOf(project: string): BrowserPrefs { return this.browser[project] ?? {}; }
  /** Spending limits (§8.3, slice 67): the numbers from the service settings, the usage — from the hub snapshot (cost_today per
   *  workspace, today in total) */
  limits = $state<Limit[]>([]);
  async loadLimits() { try { const s = await agents.settings(); this.limits = (s.limits ?? []) as Limit[]; } catch {} }
  /** The tightest of the project's limits (its own and the total) from 80 %; `blocked` — the action is "stop" and the
   *  usage ≥ the limit */
  budgetOf(project: string): { scope: string; per_day: number; action: string; spent: number; pct: number; blocked: boolean } | null {
    const lims = this.limits.filter(l => l.per_day > 0 && (l.scope === 'total' || l.scope === project));
    if (!lims.length) return null;
    const p = this.project(project);
    let worst: { scope: string; per_day: number; action: string; spent: number; pct: number; blocked: boolean } | null = null;
    for (const l of lims) {
      const spent = l.scope === 'total' ? (this.ws?.today?.cost_usd ?? 0) : (p?.cost_today ?? 0);
      const pct = spent / l.per_day * 100; const cand = { scope: l.scope, per_day: l.per_day, action: l.action, spent, pct, blocked: l.action === 'stop' && spent >= l.per_day };
      if (!worst || cand.blocked && !worst.blocked || cand.blocked === worst.blocked && pct > worst.pct) worst = cand;
    }
    return worst && worst.pct >= 80 ? worst : null;
  }
  async setBrowser(project: string, patch: BrowserPrefs) {
    const next = { ...this.browser, [project]: { ...(this.browser[project] ?? {}), ...patch } };
    await agents.saveSettings({ browser: next } as any); this.browser = next;
  }
  /** What was shared from the phone (share_target, §9.9): waits for the composer of
   *  the open conversation */
  pendingShare = $state<{ text: string; files: { name: string; type: string; data: string; size: number }[] } | null>(null);
  /** Badge on the app icon (§9.9): the number of pending requests from "Inbox"; 0 — clear */
  private badgeN = -1;
  syncBadge() {
    const n = (this.inbox?.waiting ?? []).reduce((a, w) => a + Math.max(1, (w.pending ?? []).length), 0);
    if (n === this.badgeN) return; this.badgeN = n;
    try { const nav = navigator as Navigator & { setAppBadge?: (n: number) => Promise<void>; clearAppBadge?: () => Promise<void> }; if (n > 0) void nav.setAppBadge?.(n); else void nav.clearAppBadge?.(); } catch {}
  }
  /** Persistent storage (§9.9): so the browser does not purge the conversation cache and the tree snapshot; the answer
   *  is remembered for "About" */
  storagePersisted = $state<boolean | null>(null);
  async persistStorage() {
    try { const st = navigator.storage; if (!st?.persist) return; this.storagePersisted = (await st.persisted()) || (await st.persist()); } catch {}
  }
  /** Deferred beforeinstallprompt event (Chrome): the "Install app" button in Settings → About */
  installPrompt = $state<(Event & { prompt: () => Promise<void> }) | null>(null);
  installed = $state(typeof matchMedia === 'function' && matchMedia('(display-mode: standalone)').matches);
  /** Hash of the main script of the current UI build (assets/index-XXXX.js) */
  uiBuild(): string { const m = Array.from(document.scripts).map(s => s.src.match(/assets\/index-([\w-]+)\.js/)?.[1]).find(Boolean); return m ?? ''; }
  /** The UI on the server is newer than the open one: index.html is read bypassing the cache and the main
   *  script hash is compared */
  uiStale = $state(false);
  uiServerBuild = $state('');
  async checkUiFresh() {
    try {
      const base = (document.querySelector('base')?.getAttribute('href') ?? '/agents/').replace(/[^/]*$/, '');
      const r = await fetch(`${base}index.html?fresh=${Date.now()}`, { cache: 'no-store' }); if (!r.ok) return;
      const srv = (await r.text()).match(/assets\/index-([\w-]+)\.js/)?.[1] ?? '';
      const mine = this.uiBuild(); this.uiServerBuild = srv;
      if (srv && mine && srv !== mine) this.uiStale = true;
    } catch {}
  }
  async loadHealth() { try { this.health = await agents.health(); } catch { this.health = { ok: false, cloudd: false, version: '' }; } }
  // scratch chat and agents/routines/runs (the
  // service)
  chat = $state<ChatStatus | null>(null);
  chats = $state<SimpleChat[]>([]);
  /** Engines per workspace (agentd engines.rs): what is installed; loaded when the "+" menu opens and cached for 5 minutes */
  engines = $state<Record<string, Engine[]>>({});
  private enginesAt: Record<string, number> = {};
  /** Engine catalog version from the hub (the install stamp in the workspace): when it changes the catalog is
   *  re-read, the "+" menu and
   * the engine list refresh without a reload, in any tab and on the phone (the install may have run from another window) */
  private enginesV: Record<string, number> = {};
  noteEnginesV(project: string, v: unknown) {
    if (typeof v !== 'number') return;
    const prev = this.enginesV[project]; this.enginesV[project] = v;
    if (prev !== undefined && prev !== v) { this.enginesAt[project] = 0; if (this.engines[project]) void this.loadEngines(project, true); }
  }
  async loadEngines(project: string, force = false, refresh = false) { if (!force && this.engines[project] && Date.now() - (this.enginesAt[project] ?? 0) < 300000) return this.engines[project]; try { const r = await api.engines(project, refresh); this.engines[project] = r.engines; this.enginesAt[project] = Date.now(); return r.engines; } catch { return this.engines[project] ?? []; } }
  chatRoutes = $state<ChatRoutes | null>(null);
  chatRoutesLoading = $state(false);
  chatRoutesError = $state('');
  async loadChatRoutes() {
    if (this.chatRoutesLoading) return; this.chatRoutesLoading = true;
    try { this.chatRoutes = await agents.chatRoutes(); this.chatRoutesError = ''; }
    catch (e) { this.chatRoutesError = msg(e); }
    finally { this.chatRoutesLoading = false; }
  }
  agentsList = $state<AgentDef[]>([]);
  routines = $state<Routine[]>([]);
  runs = $state<Run[]>([]);
  skills = $state<Skill[]>([]);
  mailPaused = $state(false);
  /** Only the simple chat list (the hub's `chats` signal) */
  private chatsSeq = 0;
  async loadChats() {
    const seq = ++this.chatsSeq;
    try { const r = await agents.chats(); if (seq !== this.chatsSeq) return false; this.chats = r.chats; this.chatsLoaded = true; this.chatsError = ''; return true; }
    catch (e) { if (seq === this.chatsSeq) this.chatsError = msg(e); return false; }
  }
  async loadChat() {
    const seq = ++this.chatSeq; this.chatLoading = true;
    const [scratch, simple] = await Promise.allSettled([agents.chat(), agents.chats()]);
    if (seq !== this.chatSeq) return false;
    if (scratch.status === 'fulfilled') { this.chat = scratch.value; this.chatError = ''; } else this.chatError = msg(scratch.reason);
    if (simple.status === 'fulfilled') { this.chats = simple.value.chats; this.chatsLoaded = true; this.chatsError = ''; } else this.chatsError = msg(simple.reason);
    this.chatLoading = false; return scratch.status === 'fulfilled' && simple.status === 'fulfilled';
  }
  async loadRoutines() {
    const seq = ++this.routinesSeq; this.routinesLoading = true;
    const [a, r, x, sk, st] = await Promise.allSettled([agents.agentsList(), agents.routines(), agents.runs({ limit: 100 }), agents.skills(), agents.settings()]);
    if (seq !== this.routinesSeq) return false;
    if (a.status === 'fulfilled') this.agentsList = a.value.agents;
    if (r.status === 'fulfilled') this.routines = r.value.routines;
    if (x.status === 'fulfilled') this.runs = x.value.runs;
    if (sk.status === 'fulfilled') this.skills = sk.value.skills;
    if (st.status === 'fulfilled') this.mailPaused = !!st.value.mail_paused;
    this.routinesError = [a, r, x, sk, st].filter(v => v.status === 'rejected').map(v => msg((v as PromiseRejectedResult).reason)).join(' · ');
    this.routinesLoading = false; return !this.routinesError;
  }
  /** All core projects, including service ones (scratch): for finding a session by link and for
   *  "Inbox". */
  get allProjects(): WsProject[] { return this.ws?.projects ?? []; }
  /** The owner's projects — without service ones (`hidden`): the "Workplaces" tree, the create menu,
   *  settings. */
  get projects(): WsProject[] { return this.allProjects.filter(p => !p.hidden); }
  get running(): WsProject[] { return this.projects.filter(p => p.status === 'running'); }
  /** Running ones together with the service scratch (the "Chat" section): for account
   *  defaults. */
  get runningAll(): WsProject[] { return this.allProjects.filter(p => p.status === 'running'); }
  project(name: string) { return this.allProjects.find(p => p.name === name); }
  sessions(): (SessionInfo & { ws: string })[] { return this.allProjects.flatMap(p => p.sessions.map(s => ({ ...s, ws: p.name }))); }
  infoOf(r: Ref | null): SessionInfo | PtyInfo | null {
    if (!r) return null; const p = this.project(r.project); if (!p) return null;
    return r.kind === 'pty' ? p.ptys.find(x => x.id === r.id) ?? null : p.sessions.find(x => x.id === r.id) ?? null;
  }
  refOf(s: SessionInfo & { ws: string }): Ref { return { project: s.ws, kind: 'session', id: s.id }; }
  get waiting() { return this.sessions().filter(s => s.status === 'waiting'); }
}
export const data = new Data();

// ---------- UI ----------
class Ui {
  section = $state<Section>('work');
  view = $state<'code' | 'thread'>(phone() ? 'thread' : 'code');
  cols = $state<0 | 1 | 2 | 3>(0);
  sel = $state<Ref | null>(null);
  cards = $state<Card[]>([]);
  groups = $state<Record<string, string[]>>({});
  closed = $state<Record<string, boolean>>({});
  /** Expanded session lists of a workspace (by default the last LIMIT by activity are shown). */
  more = $state<Record<string, boolean>>({});
  cardTab = $state<Record<string, string>>({});
  inspector = $state(!narrow());
  popen = $state(false);
  filter = $state<'all' | 'live' | 'wait'>('all');
  q = $state('');
  flash = $state('');
  settingsTab = $state('accounts');
  /** Workspace to open in the MCP tab of "Settings" (the composer chip) */
  mcpWs = $state<string | null>(null);
  /** The "Agents" section: the tab and the selected card; the "Chat" section: the
   *  selected chat. */
  routinesTab = $state<'agents' | 'routines' | 'runs' | 'skills'>('agents');
  routinesSel = $state<string | null>(null);
  chatSel = $state<string | null>(null);
  chatSelection = $state<Record<'simple' | 'advanced', string | null>>({ simple: null, advanced: null });
  /** Engine of the "Chat" section: simple (API) or advanced (scratch) — the switch in the
   *  panel */
  chatEngine = $state<'simple' | 'advanced'>('simple');
  /** Jump to a message from the search results (slice 34): the chat loads the history and highlights
   *  `.m.hit` */
  focusSeq = $state<{ id: string; seq: number } | null>(null);
  focusMid = $state<{ chat: string; mid: string } | null>(null);
  period = $state(1);
  private layoutLoaded = false;
  private lastSaved = '';
  private saveT: ReturnType<typeof setTimeout> | undefined;
  constructor() {
    if (typeof matchMedia === 'function') { const m = matchMedia('(max-width: 599px)'); try { m.addEventListener('change', () => { this.isPhone = m.matches; }); } catch {} }
    if (typeof matchMedia === 'function') { const m = matchMedia('(max-width: 1099px)'); try { m.addEventListener('change', () => { this.isNarrow = m.matches; }); } catch {} }
    try {
      const u = JSON.parse(ls('agents.ui') || '{}');
      if (u.view && !phone()) this.view = u.view; if (u.sel) this.sel = u.sel; if (u.cols != null) this.cols = u.cols; if (u.closed) this.closed = u.closed; if (u.cardTab) this.cardTab = u.cardTab;
      if (Array.isArray(u.cards)) this.cards = (u.cards as Card[]).map(normCard); if (u.groups) this.groups = u.groups; if (u.settingsTab) this.settingsTab = u.settingsTab; if (u.routinesTab) this.routinesTab = u.routinesTab; if (u.chatSel) this.chatSel = u.chatSel; if (u.chatEngine === 'simple' || u.chatEngine === 'advanced') this.chatEngine = u.chatEngine;
      if (u.sideTab === 'preview' || u.sideTab === 'inspector' || u.sideTab === 'browser') this.sideTab = u.sideTab;
      if (u.chatSelection) this.chatSelection = { simple: u.chatSelection.simple ?? null, advanced: u.chatSelection.advanced ?? null };
      else this.chatSelection[this.chatEngine] = this.chatSel;
    } catch {}
    this.applyUrl();
  }
  /** URL parameters from shortcuts, notifications and share_target (§9.9): parsed once and removed from the
   *  address bar.
   * open=<project>/<id> — a conversation in Thread; section=inbox|chat|models|settings[&tab=…]; new=1 — the "+" menu of the
   * first workspace; share=<id> — pick up what was shared into the draft */
  wantNew = $state(false);
  shareId = $state<string | null>(null);
  applyUrl(href?: string) {
    if (typeof location === 'undefined') return;
    try {
      const u = new URL(href ?? location.href); const q = u.searchParams; let touched = false;
      const open = q.get('open'); if (open) { const i = open.indexOf('/'); if (i > 0) { this.sel = { project: open.slice(0, i), kind: 'session', id: open.slice(i + 1) }; this.section = 'work'; this.view = 'thread'; touched = true; } }
      const sec = q.get('section'); if (sec && ['work', 'inbox', 'chat', 'routines', 'models', 'settings'].includes(sec)) { this.section = sec as Section; touched = true; }
      const tab = q.get('tab'); if (tab && this.section === 'settings') { this.settingsTab = tab; touched = true; }
      if (q.get('new') === '1') { this.section = 'work'; this.wantNew = true; touched = true; }
      const share = q.get('share'); if (share) { this.shareId = share; touched = true; }
      for (const k of ['open', 'section', 'tab', 'new', 'share', 'src']) if (q.has(k)) { q.delete(k); touched = true; }
      if (touched && !href) { history.replaceState(null, '', u.pathname + (q.toString() ? `?${q}` : '') + u.hash); }
    } catch {}
  }
  selectChat(id: string | null) { this.chatSel = id; this.chatSelection[this.chatEngine] = id; }
  setChatEngine(engine: 'simple' | 'advanced') { if (engine === this.chatEngine) return; this.chatSelection[this.chatEngine] = this.chatSel; this.chatEngine = engine; this.chatSel = this.chatSelection[engine]; }
  persist() {
    try { localStorage.setItem('agents.ui', JSON.stringify({ view: this.view, sel: this.sel, cols: this.cols, closed: this.closed, cardTab: this.cardTab, cards: this.cards, groups: this.groups, settingsTab: this.settingsTab, routinesTab: this.routinesTab, chatSel: this.chatSel, chatEngine: this.chatEngine, chatSelection: this.chatSelection, sideTab: this.sideTab })); } catch {}
  }
  /** Canvas layout from the core; the server beats the local
   *  cache. */
  async loadLayout() {
    try {
      const l = await api.agentsLayoutGet();
      if (l && Array.isArray(l.cards)) { this.cards = (l.cards as Card[]).map(normCard); this.cols = (l.cols ?? 0) as 0 | 1 | 2 | 3; this.groups = ((l as any).groups ?? {}) as Record<string, string[]>; }
      this.lastSaved = this.layoutSnap();
    } catch {} finally { this.layoutLoaded = true; }
  }
  layoutSnap() { return JSON.stringify({ cards: this.cards, cols: this.cols, groups: this.groups }); }
  saveLayout() {
    const snap = this.layoutSnap(); if (!this.layoutLoaded || snap === this.lastSaved) return;
    clearTimeout(this.saveT); this.saveT = setTimeout(() => { this.lastSaved = snap; api.agentsLayoutPut(JSON.parse(snap)).catch(() => {}); }, 800);
  }
  onCanvas(r: Ref) { return this.cards.some(c => same(c, r)); }
  addCard(r: Ref) { if (!this.onCanvas(r)) this.cards = [...this.cards, { ...r, w: 1, h: H_DEF }]; this.focus(r); this.setFolded(r, false); }
  /** Collapsible cards (docs/agents-ux.md §4.5): on the phone cards are collapsed by default and work as an
   *  accordion
   * (one expanded), the state is on the device (localStorage `agents.fold`); on desktop and tablet (2026-10-22) — expanded by
   * default and collapsed independently, a collapsed one takes one grid cell with its header; the state is separate
   * (`agents.foldd`) so the phone accordion does not collapse everything on the desktop. `isPhone` is reactive: on a window
   * width change its own set is taken. */
  /** The "Session name" dialog (§4.8): which session is being renamed */
  rename = $state<Ref | null>(null);
  isPhone = $state(phone());
  /** Narrow screen (≤ 1099px): the sidebar slides over the conversation, the preview stays a tab of the session
   *  panel (§4.10) */
  isNarrow = $state(narrow());
  /** Sidebar on the right (§4.10): tab and width; width 0 — the CSS default, changed only with the
   *  "wider/narrower" buttons */
  sideTab = $state<'inspector' | 'preview' | 'browser'>('inspector');
  sideW = $state<number>((() => { try { return Number(localStorage.getItem('agents.side') || 0) || 0; } catch { return 0; } })());
  /** Step of the sidebar width buttons; the limit is
   *  60% of the window */
  sideStep = 180;
  sideDefault(): number { return typeof matchMedia === 'function' && matchMedia('(min-width: 1400px)').matches ? 340 : 320; }
  sideWider() { const cur = this.sideW || this.sideDefault(); const max = Math.max(this.sideDefault(), Math.floor(window.innerWidth * 0.6)); this.setSideW(Math.min(max, cur + this.sideStep)); }
  sideNarrower() { const cur = this.sideW || this.sideDefault(); const next = cur - this.sideStep; this.setSideW(next <= this.sideDefault() ? 0 : next); }
  setSideW(w: number) { this.sideW = w; try { if (w) localStorage.setItem('agents.side', String(w)); else localStorage.removeItem('agents.side'); } catch {} }
  /** Open the workspace preview: on desktop — the sidebar on the "Preview" tab, on a narrow screen — a tab in the
   *  session panel */
  openPreview() { this.sideTab = 'preview'; this.inspector = true; }
  openAgentBrowser() { this.sideTab = 'browser'; this.inspector = true; }
  fold = $state<Record<string, boolean>>(JSON.parse((() => { try { return localStorage.getItem('agents.fold') ?? '{}'; } catch { return '{}'; } })()));
  foldD = $state<Record<string, boolean>>(JSON.parse((() => { try { return localStorage.getItem('agents.foldd') ?? '{}'; } catch { return '{}'; } })()));
  folded(r: Ref): boolean { return this.isPhone ? (this.fold[key(r)] ?? true) : (this.foldD[key(r)] ?? false); }
  setFolded(r: Ref, folded: boolean) {
    if (this.isPhone) {
      const next = { ...this.fold };
      if (!folded) { for (const c of this.cards) next[key(c)] = true; }
      next[key(r)] = folded; this.fold = next;
      try { localStorage.setItem('agents.fold', JSON.stringify(next)); } catch {}
    } else {
      const next = { ...this.foldD, [key(r)]: folded }; this.foldD = next;
      try { localStorage.setItem('agents.foldd', JSON.stringify(next)); } catch {}
    }
  }
  toggleFold(r: Ref) { this.setFolded(r, !this.folded(r)); }
  focus(r: Ref) {
    this.flash = key(r);
    setTimeout(() => document.querySelector(`[data-card="${CSS.escape(key(r))}"]`)?.scrollIntoView({ block: 'nearest', behavior: 'smooth' }), 60);
    setTimeout(() => { if (this.flash === key(r)) this.flash = ''; }, 1600);
  }
  removeCard(r: Ref) { this.cards = this.cards.filter(c => !same(c, r)); }
  open(r: Ref) { this.sel = r; this.popen = false; if (this.section !== 'work') this.section = 'work'; if (phone()) this.view = 'thread'; if (this.view === 'code') this.addCard(r); }
  goto(r: Ref) { this.section = 'work'; this.view = 'thread'; this.sel = r; this.popen = false; }
  expand(r: Ref) { this.sel = r; this.view = 'thread'; }
  openMcp(project: string) { this.mcpWs = project; this.settingsTab = 'mcp'; this.section = 'settings'; this.popen = false; }
  toCanvas(r: Ref) { this.view = 'code'; this.addCard(r); }
  /** Card size in columns and rows (§4.11); values are clamped to the allowed range */
  resize(r: Ref, w?: number, h?: number) { this.cards = this.cards.map(c => (same(c, r) ? normCard({ ...c, w: w ?? c.w, h: h ?? c.h }) : c)); }
  reorder(srcKey: string, beforeKey: string | null) {
    const src = this.cards.find(c => key(c) === srcKey); if (!src) return;
    const rest = this.cards.filter(c => key(c) !== srcKey); const at = beforeKey ? rest.findIndex(c => key(c) === beforeKey) : -1;
    if (at < 0) rest.push(src); else rest.splice(at, 0, src); this.cards = rest;
  }
  /** Cards of vanished sessions are removed by themselves (an unreachable agentd is
   *  not a reason). */
  prune() {
    const next = this.cards.filter(c => { const p = data.project(c.project); return !p || !p.reachable || data.infoOf(c); });
    if (next.length !== this.cards.length) this.cards = next;
  }
}
export const ui = new Ui();

// ---------- actions on sessions ----------
/** Engines: labels and icons without a request to the workspace (the agentd
 *  catalog is the same). */
export const ENGINES: { id: string; title: string; icon: string }[] = [{ id: 'claude', title: 'Claude Code', icon: 'claude' }, { id: 'codex', title: 'Codex', icon: 'codex' }, { id: 'gemini', title: 'Gemini CLI', icon: 'gemini' }, { id: 'pi', title: 'pi', icon: 'pi' }, { id: 'antigravity', title: 'Antigravity', icon: 'antigravity' }, { id: 'omp', title: 'Oh My Pi', icon: 'omp' }];
export const engineTitle = (id?: string | null) => ENGINES.find(e => e.id === id)?.title ?? (id === 'claude' ? 'Claude' : id ?? '');
export const engineIcon = (id?: string | null) => ENGINES.find(e => e.id === id)?.icon ?? 'cpu';
const TUI_BIN: Record<string, string> = { claude: 'claude', codex: 'codex', gemini: 'gemini', pi: 'pi', antigravity: 'agy', omp: 'omp' };
export const actions = {
  async createSession(project: string, agent: string) {
    try {
      const st = await agents.settings().catch(() => null);
      const mode = (st as any)?.defaults?.[`perm_${agent}`] || undefined;
      const model = (st as any)?.defaults?.[agent] || undefined;
      const cache_ttl = agent === 'claude' ? ((st as any)?.defaults?.cache_claude || undefined) : undefined;
      const s = await api.sessionCreate(project, { agent, title: agent === 'codex' ? 'Codex' : agent === 'claude' ? 'Claude' : engineTitle(agent), permission_mode: mode, model, cache_ttl: cache_ttl === 'auto' ? undefined : cache_ttl });
      await data.loadWs(); ui.open({ project, kind: 'session', id: s.id }); toast(t('toast.created', { ws: project }));
    } catch (e) { toast(msg(e), 'bad'); }
  },
  /** Engine install — in a terminal (npm install -g), the card opens right away; then the engine catalog is polled every
   * 15 s until the engine shows up as installed (agentd re-reads it by the stamp the install command sets) — then a
   * toast, and the "+" menu has "Chat with …". One re-read after 90 s was not enough: the install takes longer, and the
   * agentd cache managed to fill with the "not installed" state. */
  async installEngine(project: string, id: string) {
    try {
      const p = await api.engineInstall(project, id);
      await data.loadWs(); ui.open({ project, kind: 'pty', id: p.id }); toast(t('toast.engineInstall', { name: engineTitle(id) }), 'info');
      const started = Date.now();
      const tick = async () => {
        if (Date.now() - started > 20 * 60000) return;
        const list = await data.loadEngines(project, true);
        if (list.find(e => e.id === id)?.installed) { toast(t('toast.engineInstalled', { name: engineTitle(id) }), 'ok'); return; }
        setTimeout(tick, 15000);
      };
      setTimeout(tick, 15000);
    } catch (e) { toast(msg(e), 'bad'); }
  },
  async createPty(project: string, kind: string) {
    try {
      const p = await api.ptyCreate(project, { cmd: kind === 'shell' ? undefined : ['bash', '-lic', `exec ${TUI_BIN[kind] ?? kind}`], title: kind === 'shell' ? t('agents.newShell') : engineTitle(kind), engine: kind === 'shell' ? undefined : kind });
      await data.loadWs(); ui.open({ project, kind: 'pty', id: p.id }); toast(t('toast.shellCreated', { ws: project }));
    } catch (e) { toast(msg(e), 'bad'); }
  },
  /** Stop a live session; remove a finished one entirely (the journal is
   *  deleted). */
  /** Delete a conversation in one action, for a live session too: stop (or detach the CLI), wait and remove — with a
   *  confirmation.
   * The owner could not find deletion for Antigravity: a live session and a session with an open TUI had only "stop"/"detach CLI" in the
   * menu. */
  async deleteSession(r: Ref) {
    const info = data.infoOf(r); if (!info) return;
    if (!window.confirm(t('sess.deleteConfirm', { title: titleOf(r) }))) return;
    try {
      for (let i = 0; i < 4 && data.infoOf(r); i++) {
        if (r.kind === 'pty') await api.ptyKill(r.project, r.id); else await api.sessionDelete(r.project, r.id);
        for (let w = 0; w < 20; w++) { await data.loadWs(); const cur = data.infoOf(r); if (!cur || !live(cur)) break; await new Promise(res => setTimeout(res, 400)); }
      }
      if (!data.infoOf(r)) { ui.removeCard(r); if (ui.sel && same(ui.sel, r)) ui.sel = null; toast(t('toast.removed'), 'info'); }
      else toast(t('toast.stopped'), 'info');
    } catch (e) { toast(msg(e), 'bad'); }
  },
  async stopOrRemove(r: Ref) {
    const info = data.infoOf(r); if (!info) return;
    try {
      if (!live(info)) { if (!window.confirm(t('sess.deleteConfirm', { title: titleOf(r) }))) return; }
      if (r.kind === 'pty') await api.ptyKill(r.project, r.id); else await api.sessionDelete(r.project, r.id);
      await data.loadWs();
      if (!data.infoOf(r)) { ui.removeCard(r); if (ui.sel && same(ui.sel, r)) ui.sel = null; toast(t('toast.removed'), 'info'); } else toast((info as SessionInfo).tui && !info.alive ? t('toast.cliOff') : t('toast.stopped'), 'info');
    } catch (e) { toast(msg(e), 'bad'); }
  },
  /** Conversation fork: a new session with a copy of the history (up to event at_seq or whole), opens
   *  next to it. */
  async fork(r: Ref, at_seq?: number) {
    try { const ns = await api.sessionFork(r.project, r.id, at_seq != null ? { at_seq } : {}); await data.loadWs(); toast(t('toast.forked', { title: ns.title })); ui.open({ project: r.project, kind: 'session', id: ns.id }); } catch (e) { toast(msg(e), 'bad'); }
  },
  async resume(r: Ref) { try { await api.sessionCreate(r.project, { resume: r.id }); await data.loadWs(); toast(t('toast.resumed')); } catch (e) { toast(msg(e), 'bad'); } },
  /** Dev server preview (§4.9): the "⋯" menu item — the same per-workspace setting as in Settings → Browser; turning it off removes the
   *  routes */
  async togglePreview(project: string) {
    const on = !data.browserOf(project).preview;
    try {
      await data.setBrowser(project, { preview: on });
      if (!on) { const prev = await api.previews(project).catch(() => ({ previews: [] as { port: number }[] })); for (const r of prev.previews) await api.previewRemove(project, r.port).catch(() => {}); if (ui.sideTab === 'preview') ui.sideTab = 'inspector'; }
      else if (!ui.isNarrow && ui.sel?.project === project) ui.openPreview();
      toast(t(on ? 'browser.enabledToast' : 'browser.disabledToast', { project }));
    } catch (e) { toast(msg(e), 'bad'); }
  },
  /** Agent browser (§4.9 step 2): enable — Chromium and the MCP server in the workspace, the "Browser" tab in the sidebar; disable —
   *  remove both */
  async toggleAgentBrowser(project: string) {
    const on = !data.browserOf(project).agent;
    try {
      if (on) { const st = await api.browserSet(project, true); if (!st.installed) { toast(t('abrowser.notInstalled'), 'bad'); } }
      else await api.browserSet(project, false).catch(() => {});
      await data.setBrowser(project, { agent: on });
      if (on && ui.sel?.project === project) ui.openAgentBrowser(); else if (!on && ui.sideTab === 'browser') ui.sideTab = 'inspector';
      toast(t(on ? 'abrowser.enabledToast' : 'abrowser.disabledToast', { project }));
    } catch (e) { toast(msg(e), 'bad'); }
  },
  /** Own name (§4.8): the dialog and saving; an empty string brings back the auto-title */
  rename(r: Ref) { ui.rename = r; },
  async setLabel(r: Ref, label: string) { try { await api.sessionLabel(r.project, r.id, label); await data.loadWs(); toast(t('toast.renamed')); } catch (e) { toast(msg(e), 'bad'); throw e; } },
  async setModel(r: Ref, model: string | null) { try { await api.sessionModel(r.project, r.id, model); await data.loadWs(); } catch (e) { toast(msg(e), 'bad'); } },
  async setMode(r: Ref, mode: string) { try { await api.sessionMode(r.project, r.id, mode); await data.loadWs(); } catch (e) { toast(msg(e), 'bad'); } },
  /** Reassign the account: the core restarts the session process with the same history (for a finished one — simply starts it
   *  again). */
  async setAccount(r: Ref, account: string | null) { try { await api.sessionAccount(r.project, r.id, account); await data.loadWs(); toast(t('toast.account', { label: account ? (data.accounts.find(a => a.id === account)?.label ?? account) : t('set.gate') })); } catch (e) { toast(msg(e), 'bad'); } },
  async permission(project: string, id: string, body: { request_id: string; behavior: 'allow' | 'deny'; answers?: unknown; remember?: boolean; message?: string; scope?: 'session' | 'project' | 'user' }) {
    try { await api.sessionPermission(project, id, body); toast(body.behavior === 'allow' ? (body.answers ? t('toast.answered') : t('toast.allowed')) : t('toast.denied'), body.behavior === 'allow' ? 'ok' : 'bad'); await data.loadInbox(); await data.loadWs(); } catch (e) { toast(msg(e), 'bad'); }
  },
  ide(project: string) { const a = data.project(project)?.apps.find(a => a.slug === 'code-server'); if (a) window.open(a.url, '_blank', 'noopener'); },
};

// ---------- labels ----------
/** Accounts of the session provider: for Claude — claude/anthropic, for Codex —
 *  codex/openai. */
export const accountsFor = (agent?: string | null) => data.accounts.filter(a => (agent === 'codex' ? ['codex', 'openai'].includes(a.provider) : agent === 'gemini' ? ['gemini', 'google'].includes(a.provider) : agent === 'antigravity' ? a.provider === 'antigravity' : agent === 'pi' || agent === 'omp' ? false : ['claude', 'anthropic'].includes(a.provider)));
/** The "Account" section for the session menu (card, tree, Thread, chat): the project gate and the provider accounts, the
 *  current one marked. */
export function accountMenu(r: Ref): MenuItem[] {
  const s = data.infoOf(r) as SessionInfo | null; if (!s || r.kind !== 'session') return [];
  return [
    { sep: true }, { header: true, label: t('sess.menu.accountHdr') },
    { icon: 'globe', label: t('set.gate'), checked: !s.account, on: () => actions.setAccount(r, null) },
    ...accountsFor(s.agent).map(a => ({ avatar: engineIcon(s.agent), label: a.label, hint: a.kind === 'subscription' ? t('set.kindSubscription') : a.kind === 'proxy' ? t('set.kindProxy') : t('set.kindKey'), checked: s.account === a.id, on: () => actions.setAccount(r, a.id) })),
  ];
}
export const agentName = (a: string) => (a === 'claude' ? 'Claude' : a === 'codex' ? 'Codex' : ENGINES.some(e => e.id === a) ? engineTitle(a) : t('agents.newShell').toLowerCase());
export const shortModel = (m?: string | null) => (m || '').replace('claude-', '').replace('-medium', '');
export const modeLabel = (m?: string | null) => ({ default: t('agents.modeDefault'), acceptEdits: t('agents.modeAcceptEdits'), plan: t('agents.modePlan'), bypassPermissions: t('agents.modeBypass'), 'on-request': t('agents.modeOnRequest'), untrusted: t('agents.modeUntrusted'), never: t('agents.modeNever'), auto_edit: t('acpmode.auto_edit'), autoEdit: t('acpmode.auto_edit'), yolo: t('acpmode.yolo') } as Record<string, string>)[m || 'default'] || m || '';
export const MODELS_FOR: Record<string, string[]> = { claude: ['claude-fable-5-1', 'claude-opus-5-5', 'claude-sonnet-5', 'claude-haiku-4-5-20251001'], codex: ['gpt-6-astra', 'gpt-5.5', 'gpt-5-codex', 'gpt-oss-120b-medium'], gemini: ['gemini-3.1-pro-low', 'gemini-pro-agent', 'gemini-3-flash', 'gemini-3.5-flash-lite'], pi: ['agentverse-gate/claude-sonnet-4-6', 'agentverse-gate/claude-opus-4-6', 'agentverse-gate/gpt-6-astra'], antigravity: [], omp: [] };
export const MODES_FOR: Record<string, string[]> = { claude: ['default', 'acceptEdits', 'plan', 'bypassPermissions'], codex: ['on-request', 'untrusted', 'never'], gemini: ['default', 'autoEdit', 'yolo', 'plan'], pi: [], antigravity: ['default', 'acceptEdits', 'plan', 'auto'], omp: ['default', 'plan'] };
export function titleOf(r: Ref): string { const i = data.infoOf(r); return !i ? '…' : r.kind === 'pty' ? ((i as PtyInfo).title || (i as PtyInfo).cmd.join(' ')) : ((i as SessionInfo).label || (i as SessionInfo).title); }
/** Session auto-title (the first message / the provider journal) — shown in tooltips when the owner gave their own name
 *  (§4.8) */
export function autoTitleOf(r: Ref): string { const i = data.infoOf(r); return !i ? '' : r.kind === 'pty' ? ((i as PtyInfo).title || (i as PtyInfo).cmd.join(' ')) : (i as SessionInfo).title; }
export function agentOf(r: Ref): string { return r.kind === 'pty' ? 'shell' : ((data.infoOf(r) as SessionInfo | null)?.agent ?? 'claude'); }
/** Live: the headless process is running or the native TUI is attached (terminal
 *  mode). */
export const live = (i: SessionInfo | PtyInfo | null | undefined) => !!i && (i.alive || !!(i as SessionInfo).tui);
/** Stop button label: for a TUI — "detach CLI", for a process — "stop", for a finished one — "remove". */
export const stopLabel = (i: SessionInfo | PtyInfo | null | undefined, menu = false) => !live(i) ? (menu ? t('sess.menu.delete') : t('insp.remove')) : (i as SessionInfo).tui && !i!.alive ? (menu ? t('sess.menu.detach') : t('thread.detach')) : menu ? t('sess.menu.stop') : t('thread.stop');
export function dotOf(r: Ref): string {
  const i = data.infoOf(r); if (!i) return '';
  if (r.kind === 'pty') return i.alive ? 'ok' : '';
  const s = i as SessionInfo; return s.status === 'waiting' ? 'waiting' : s.status === 'error' ? 'failed' : s.tui ? 'ok' : !s.alive ? '' : s.status === 'working' ? 'working' : s.background ? 'background' : 'ok';
}
export function statusText(s: SessionInfo): string {
  if (s.tui && !s.alive) return t('st.tui');
  if (s.alive && s.status === 'idle' && s.background) return t('st.background', { n: s.background });
  return s.status === 'working' ? t('st.working') : s.status === 'waiting' ? t('st.waiting') : s.status === 'error' ? t('st.failed') : !s.alive || s.status === 'exited' ? t('st.exited') : s.status === 'starting' ? t('st.starting') : t('st.idle');
}
export const money = (n?: number | null) => '$' + (n ?? 0).toFixed(2);
/** Date and time in the language locale: today — time only, otherwise day and
 *  month. */
export function when(ts?: number | null): string {
  if (!ts) return ''; const d = new Date(ts * 1000); const now = new Date();
  const hm = fmt.time(d);
  return d.toDateString() === now.toDateString() ? hm : `${fmt.date(d, { day: 'numeric', month: 'short' })} ${hm}`;
}
export const runIcon = (st: string) => (st === 'done' ? 'check' : st === 'running' ? 'play' : st === 'skipped' ? 'more' : 'alert');
export const runClass = (st: string) => (st === 'done' ? 'ok' : st === 'running' ? 'working' : st === 'skipped' ? '' : 'failed');
export const fmtK = (n?: number | null) => (n == null ? '—' : n >= 1e6 ? (n / 1e6).toFixed(1) + 'M' : n >= 1000 ? (n / 1000).toFixed(1) + 'k' : String(n));
export const ago = (ts?: number | null) => fmt.ago(ts);
