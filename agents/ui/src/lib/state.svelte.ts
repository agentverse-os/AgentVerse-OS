// Состояние приложения «Агенты»: данные (рабочие места, входящие, модели — из сервиса cloudos-agents), состояние интерфейса
// (раздел, вид Code/Thread, выбор, холст, панели) и действия над сессиями (через API ядра по прокси сервиса).
// Раскладка холста хранится в ядре (kv agents.layout) — общая для всех устройств; остальное — в браузере.
import { api, agents, type Workspaces, type WsProject, type SessionInfo, type PtyInfo, type Inbox, type Models, type Account, type AgentDef, type Skill, type Routine, type Run, type ChatStatus, type SimpleChat, type ChatRoutes, type Engine } from './api';
import { toast } from './toast.svelte';
import type { MenuItem } from './menu.svelte';
import { t, fmt } from './i18n';

export type Section = 'work' | 'inbox' | 'chat' | 'routines' | 'models' | 'settings';
export type Ref = { project: string; kind: 'pty' | 'session'; id: string };
export type Card = Ref & { w: 1 | 2; h: 1 | 2 };
// названия — геттеры: пересчитываются при смене языка (реактивный t())
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
const narrow = () => typeof matchMedia === 'function' && matchMedia('(max-width: 1099px)').matches;
const ls = (k: string) => { try { return localStorage.getItem(k); } catch { return null; } };

// ---------- данные ----------
class Data {
  ws = $state<Workspaces | null>(null);
  inbox = $state<Inbox | null>(null);
  models = $state<Models | null>(null);
  accounts = $state<Account[]>([]);
  projectAccounts = $state<Record<string, string>>({});
  error = $state('');
  health = $state<{ ok: boolean; cloudd: boolean; version: string; cloudd_version?: string | null } | null>(null);
  loading = $state(false);
  async loadWs() { try { this.ws = await agents.workspaces(); this.error = ''; } catch (e) { this.error = msg(e); } }
  async loadInbox() { try { this.inbox = await agents.inbox(); this.error = ''; } catch (e) { this.error = msg(e); } }
  async loadModels(days = 1) { try { this.models = await agents.models(days); this.error = ''; } catch (e) { this.error = msg(e); } }
  async loadAccounts() { try { const a = await api.accounts(); this.accounts = a.accounts; this.projectAccounts = a.defaults; } catch {} }
  async loadHealth() { try { this.health = await agents.health(); } catch { this.health = { ok: false, cloudd: false, version: '' }; } }
  // чат scratch и агенты/рутины/запуски (сервис)
  chat = $state<ChatStatus | null>(null);
  chats = $state<SimpleChat[]>([]);
  /** Движки по workspace'ам (agentd engines.rs): что установлено; грузится при открытии меню «+» и кэшируется на 5 минут */
  engines = $state<Record<string, Engine[]>>({});
  private enginesAt: Record<string, number> = {};
  async loadEngines(project: string, force = false) { if (!force && this.engines[project] && Date.now() - (this.enginesAt[project] ?? 0) < 300000) return this.engines[project]; try { const r = await api.engines(project); this.engines[project] = r.engines; this.enginesAt[project] = Date.now(); return r.engines; } catch { return this.engines[project] ?? []; } }
  chatRoutes = $state<ChatRoutes | null>(null);
  async loadChatRoutes() { try { this.chatRoutes = await agents.chatRoutes(); } catch (e) { this.chatRoutes = { routes: [], defaults: {}, search: { ready: false, kind: 'none' } }; } }
  agentsList = $state<AgentDef[]>([]);
  routines = $state<Routine[]>([]);
  runs = $state<Run[]>([]);
  skills = $state<Skill[]>([]);
  mailPaused = $state(false);
  async loadChat() { try { const [c, l] = await Promise.all([agents.chat(), agents.chats().catch(() => ({ chats: [] }))]); this.chat = c; this.chats = l.chats; this.error = ''; } catch (e) { this.error = msg(e); } }
  async loadRoutines() { try { const [a, r, x, sk, st] = await Promise.all([agents.agentsList(), agents.routines(), agents.runs({ limit: 100 }), agents.skills(), agents.settings()]); this.agentsList = a.agents; this.routines = r.routines; this.runs = x.runs; this.skills = sk.skills; this.mailPaused = !!st.mail_paused; this.error = ''; } catch (e) { this.error = msg(e); } }
  /** Все проекты ядра, включая служебные (scratch): для поиска сессии по ссылке и «Входящих». */
  get allProjects(): WsProject[] { return this.ws?.projects ?? []; }
  /** Проекты владельца — без служебных (`hidden`): дерево «Рабочих мест», меню создания, настройки. */
  get projects(): WsProject[] { return this.allProjects.filter(p => !p.hidden); }
  get running(): WsProject[] { return this.projects.filter(p => p.status === 'running'); }
  /** Запущенные вместе со служебным scratch (раздел «Чат»): для умолчаний аккаунта. */
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

// ---------- интерфейс ----------
class Ui {
  section = $state<Section>('work');
  view = $state<'code' | 'thread'>(phone() ? 'thread' : 'code');
  cols = $state<0 | 1 | 2 | 3>(0);
  sel = $state<Ref | null>(null);
  cards = $state<Card[]>([]);
  groups = $state<Record<string, string[]>>({});
  closed = $state<Record<string, boolean>>({});
  /** Развёрнутые списки сессий workspace'а (по умолчанию показываются последние LIMIT по активности). */
  more = $state<Record<string, boolean>>({});
  cardTab = $state<Record<string, string>>({});
  inspector = $state(!narrow());
  popen = $state(false);
  filter = $state<'all' | 'live' | 'wait'>('all');
  q = $state('');
  flash = $state('');
  settingsTab = $state('accounts');
  /** Workspace, который открыть во вкладке MCP «Настроек» (чип композера) */
  mcpWs = $state<string | null>(null);
  /** Раздел «Агенты»: вкладка и выбранная карточка; раздел «Чат»: выбранный чат. */
  routinesTab = $state<'agents' | 'routines' | 'runs' | 'skills'>('agents');
  routinesSel = $state<string | null>(null);
  chatSel = $state<string | null>(null);
  /** Движок раздела «Чат»: простой (API) или продвинутый (scratch) — переключатель в панели */
  chatEngine = $state<'simple' | 'advanced'>('simple');
  /** Переход к сообщению из результатов поиска (срез 34): чат догружает историю и подсвечивает `.m.hit` */
  focusSeq = $state<{ id: string; seq: number } | null>(null);
  focusMid = $state<{ chat: string; mid: string } | null>(null);
  period = $state(1);
  private layoutLoaded = false;
  private lastSaved = '';
  private saveT: ReturnType<typeof setTimeout> | undefined;
  constructor() {
    try {
      const u = JSON.parse(ls('agents.ui') || '{}');
      if (u.view && !phone()) this.view = u.view; if (u.sel) this.sel = u.sel; if (u.cols != null) this.cols = u.cols; if (u.closed) this.closed = u.closed; if (u.cardTab) this.cardTab = u.cardTab;
      if (Array.isArray(u.cards)) this.cards = u.cards; if (u.groups) this.groups = u.groups; if (u.settingsTab) this.settingsTab = u.settingsTab; if (u.routinesTab) this.routinesTab = u.routinesTab; if (u.chatSel) this.chatSel = u.chatSel; if (u.chatEngine === 'simple' || u.chatEngine === 'advanced') this.chatEngine = u.chatEngine;
    } catch {}
  }
  persist() {
    try { localStorage.setItem('agents.ui', JSON.stringify({ view: this.view, sel: this.sel, cols: this.cols, closed: this.closed, cardTab: this.cardTab, cards: this.cards, groups: this.groups, settingsTab: this.settingsTab, routinesTab: this.routinesTab, chatSel: this.chatSel, chatEngine: this.chatEngine })); } catch {}
  }
  /** Раскладка холста из ядра; сервер побеждает локальный кэш. */
  async loadLayout() {
    try {
      const l = await api.agentsLayoutGet();
      if (l && Array.isArray(l.cards)) { this.cards = l.cards as Card[]; this.cols = (l.cols ?? 0) as 0 | 1 | 2 | 3; this.groups = ((l as any).groups ?? {}) as Record<string, string[]>; }
      this.lastSaved = this.layoutSnap();
    } catch {} finally { this.layoutLoaded = true; }
  }
  layoutSnap() { return JSON.stringify({ cards: this.cards, cols: this.cols, groups: this.groups }); }
  saveLayout() {
    const snap = this.layoutSnap(); if (!this.layoutLoaded || snap === this.lastSaved) return;
    clearTimeout(this.saveT); this.saveT = setTimeout(() => { this.lastSaved = snap; api.agentsLayoutPut(JSON.parse(snap)).catch(() => {}); }, 800);
  }
  onCanvas(r: Ref) { return this.cards.some(c => same(c, r)); }
  addCard(r: Ref) { if (!this.onCanvas(r)) this.cards = [...this.cards, { ...r, w: 1, h: 1 }]; this.focus(r); }
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
  resize(r: Ref, w?: 1 | 2, h?: 1 | 2) { this.cards = this.cards.map(c => (same(c, r) ? { ...c, w: w ?? c.w, h: h ?? c.h } : c)); }
  reorder(srcKey: string, beforeKey: string | null) {
    const src = this.cards.find(c => key(c) === srcKey); if (!src) return;
    const rest = this.cards.filter(c => key(c) !== srcKey); const at = beforeKey ? rest.findIndex(c => key(c) === beforeKey) : -1;
    if (at < 0) rest.push(src); else rest.splice(at, 0, src); this.cards = rest;
  }
  /** Карточки исчезнувших сессий снимаются сами (недоступный agentd — не повод). */
  prune() {
    const next = this.cards.filter(c => { const p = data.project(c.project); return !p || !p.reachable || data.infoOf(c); });
    if (next.length !== this.cards.length) this.cards = next;
  }
}
export const ui = new Ui();

// ---------- действия над сессиями ----------
/** Движки: подписи и значки без запроса к workspace'у (каталог agentd тот же). */
export const ENGINES: { id: string; title: string; icon: string }[] = [{ id: 'claude', title: 'Claude Code', icon: 'claude' }, { id: 'codex', title: 'Codex', icon: 'codex' }, { id: 'gemini', title: 'Gemini CLI', icon: 'gemini' }, { id: 'pi', title: 'pi', icon: 'pi' }];
export const engineTitle = (id?: string | null) => ENGINES.find(e => e.id === id)?.title ?? (id === 'claude' ? 'Claude' : id ?? '');
export const engineIcon = (id?: string | null) => ENGINES.find(e => e.id === id)?.icon ?? 'cpu';
const TUI_BIN: Record<string, string> = { claude: 'claude', codex: 'codex', gemini: 'gemini', pi: 'pi' };
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
  /** Установка движка — в терминале (npm install -g), карточка открывается сразу; каталог движков перечитывается после. */
  async installEngine(project: string, id: string) {
    try {
      const p = await api.engineInstall(project, id);
      await data.loadWs(); ui.open({ project, kind: 'pty', id: p.id }); toast(t('toast.engineInstall', { name: engineTitle(id) }), 'info');
      setTimeout(() => data.loadEngines(project, true), 90000);
    } catch (e) { toast(msg(e), 'bad'); }
  },
  async createPty(project: string, kind: string) {
    try {
      const p = await api.ptyCreate(project, { cmd: kind === 'shell' ? undefined : ['bash', '-lic', `exec ${TUI_BIN[kind] ?? kind}`], title: kind === 'shell' ? t('agents.newShell') : engineTitle(kind), engine: kind === 'shell' ? undefined : kind });
      await data.loadWs(); ui.open({ project, kind: 'pty', id: p.id }); toast(t('toast.shellCreated', { ws: project }));
    } catch (e) { toast(msg(e), 'bad'); }
  },
  /** Живую сессию остановить; завершённую — убрать совсем (журнал удаляется). */
  async stopOrRemove(r: Ref) {
    const info = data.infoOf(r); if (!info) return;
    try {
      if (!live(info)) { if (!window.confirm(t('sess.deleteConfirm', { title: titleOf(r) }))) return; }
      if (r.kind === 'pty') await api.ptyKill(r.project, r.id); else await api.sessionDelete(r.project, r.id);
      await data.loadWs();
      if (!data.infoOf(r)) { ui.removeCard(r); if (ui.sel && same(ui.sel, r)) ui.sel = null; toast(t('toast.removed'), 'info'); } else toast((info as SessionInfo).tui && !info.alive ? t('toast.cliOff') : t('toast.stopped'), 'info');
    } catch (e) { toast(msg(e), 'bad'); }
  },
  /** Форк беседы: новая сессия с копией истории (до события at_seq или целиком), открывается рядом. */
  async fork(r: Ref, at_seq?: number) {
    try { const ns = await api.sessionFork(r.project, r.id, at_seq != null ? { at_seq } : {}); await data.loadWs(); toast(t('toast.forked', { title: ns.title })); ui.open({ project: r.project, kind: 'session', id: ns.id }); } catch (e) { toast(msg(e), 'bad'); }
  },
  async resume(r: Ref) { try { await api.sessionCreate(r.project, { resume: r.id }); await data.loadWs(); toast(t('toast.resumed')); } catch (e) { toast(msg(e), 'bad'); } },
  async setModel(r: Ref, model: string | null) { try { await api.sessionModel(r.project, r.id, model); await data.loadWs(); } catch (e) { toast(msg(e), 'bad'); } },
  async setMode(r: Ref, mode: string) { try { await api.sessionMode(r.project, r.id, mode); await data.loadWs(); } catch (e) { toast(msg(e), 'bad'); } },
  /** Переназначить аккаунт: ядро перезапускает процесс сессии с той же историей (для завершённой — просто запускает заново). */
  async setAccount(r: Ref, account: string | null) { try { await api.sessionAccount(r.project, r.id, account); await data.loadWs(); toast(t('toast.account', { label: account ? (data.accounts.find(a => a.id === account)?.label ?? account) : t('set.gate') })); } catch (e) { toast(msg(e), 'bad'); } },
  async permission(project: string, id: string, body: { request_id: string; behavior: 'allow' | 'deny'; answers?: unknown; remember?: boolean; message?: string; scope?: 'session' | 'project' | 'user' }) {
    try { await api.sessionPermission(project, id, body); toast(body.behavior === 'allow' ? (body.answers ? t('toast.answered') : t('toast.allowed')) : t('toast.denied'), body.behavior === 'allow' ? 'ok' : 'bad'); await data.loadInbox(); await data.loadWs(); } catch (e) { toast(msg(e), 'bad'); }
  },
  ide(project: string) { const a = data.project(project)?.apps.find(a => a.slug === 'code-server'); if (a) window.open(a.url, '_blank', 'noopener'); },
};

// ---------- подписи ----------
/** Аккаунты провайдера сессии: у Claude — claude/anthropic, у Codex — codex/openai. */
export const accountsFor = (agent?: string | null) => data.accounts.filter(a => (agent === 'codex' ? ['codex', 'openai'].includes(a.provider) : agent === 'gemini' ? ['gemini', 'google'].includes(a.provider) : agent === 'pi' ? false : ['claude', 'anthropic'].includes(a.provider)));
/** Секция «Аккаунт» для меню сессии (карточка, дерево, Thread, чат): gate проекта и аккаунты провайдера, текущий отмечен. */
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
export const MODELS_FOR: Record<string, string[]> = { claude: ['claude-fable-5-1', 'claude-opus-5-5', 'claude-sonnet-5', 'claude-haiku-4-5-20251001'], codex: ['gpt-6-astra', 'gpt-5.5', 'gpt-5-codex', 'gpt-oss-120b-medium'], gemini: ['gemini-3.1-pro-low', 'gemini-pro-agent', 'gemini-3-flash', 'gemini-3.5-flash-lite'], pi: ['agentverse-gate/claude-sonnet-4-6', 'agentverse-gate/claude-opus-4-6', 'agentverse-gate/gpt-6-astra'] };
export const MODES_FOR: Record<string, string[]> = { claude: ['default', 'acceptEdits', 'plan', 'bypassPermissions'], codex: ['on-request', 'untrusted', 'never'], gemini: ['default', 'autoEdit', 'yolo', 'plan'], pi: [] };
export function titleOf(r: Ref): string { const i = data.infoOf(r); return !i ? '…' : r.kind === 'pty' ? ((i as PtyInfo).title || (i as PtyInfo).cmd.join(' ')) : (i as SessionInfo).title; }
export function agentOf(r: Ref): string { return r.kind === 'pty' ? 'shell' : ((data.infoOf(r) as SessionInfo | null)?.agent ?? 'claude'); }
/** Живая: работает headless-процесс или подключён родной TUI (терминальный режим). */
export const live = (i: SessionInfo | PtyInfo | null | undefined) => !!i && (i.alive || !!(i as SessionInfo).tui);
/** Подпись кнопки остановки: у TUI — «отключить CLI», у процесса — «остановить», у завершённой — «убрать». */
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
/** Дата и время по локали языка: сегодня — только время, иначе день и месяц. */
export function when(ts?: number | null): string {
  if (!ts) return ''; const d = new Date(ts * 1000); const now = new Date();
  const hm = fmt.time(d);
  return d.toDateString() === now.toDateString() ? hm : `${fmt.date(d, { day: 'numeric', month: 'short' })} ${hm}`;
}
export const runIcon = (st: string) => (st === 'done' ? 'check' : st === 'running' ? 'play' : st === 'skipped' ? 'more' : 'alert');
export const runClass = (st: string) => (st === 'done' ? 'ok' : st === 'running' ? 'working' : st === 'skipped' ? '' : 'failed');
export const fmtK = (n?: number | null) => (n == null ? '—' : n >= 1e6 ? (n / 1e6).toFixed(1) + 'M' : n >= 1000 ? (n / 1000).toFixed(1) + 'k' : String(n));
export const ago = (ts?: number | null) => fmt.ago(ts);
