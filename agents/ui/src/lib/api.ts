// Клиент API для приложения «Агенты»: копия клиента Desktop, но все вызовы ядра идут через прокси сервиса cloudos-agents
// (CORE = /agents/api/core → cloudd /api), а собственные представления сервиса — через OWN = /agents/api (agents.*).
// Типы — зеркало utoipa-схем cloudd (/api/openapi.json).
import { t } from './i18n';
export type Runtime = 'incus' | 'incus-nesting' | 'incus-vm';
export interface WorkspaceSpec { runtime: Runtime; cpu: number; memory: string | number; home: string | number; image?: string | null }
export interface ProjectSpec { schema: number; project: string; workspace: WorkspaceSpec; agents: string[]; capabilities: string[]; description?: string | null }  // description — заметка владельца, ядро её только хранит
export interface Grant { project: string; capability: string; app: string; granted_at: string }
export interface AgentdHealth { version: string; protocol: number; uptime_s: number; ptys: number; ptys_alive: number; user?: string | null; cgroup?: string | null; auth: boolean }
export interface AgentSessionBrief { id: string; title: string; agent: string; status: string; alive: boolean; pending: number; last_text?: string | null; cost_usd: number }
export interface AgentsSummary { total: number; working: number; waiting: number; sessions: AgentSessionBrief[]; updated_at: number }
export interface UsageTotals { cost_usd: number; input: number; output: number; cache_read: number; cache_create: number; turns: number }
export interface UsageBucket extends UsageTotals { key: string }
export interface UsageSummary { since_ts: number; total: UsageTotals; by_project: UsageBucket[]; by_model: UsageBucket[]; by_day: UsageBucket[] }
export interface GateProvider { id: string; title: string; flow: 'device' | 'callback' | 'code'; callback?: string | null; models?: string; since?: string; core?: boolean }
export interface GateLogin { name: string; provider: string; label?: string | null; email?: string | null; status?: string | null; status_message?: string | null; disabled: boolean; unavailable: boolean; plan?: string | null }
export interface GatePlugin { id: string; name?: string | null; version?: string | null; enabled: boolean; oauth: boolean; provider?: string | null }
export interface GateStoreItem { id: string; name?: string | null; description?: string | null; version?: string | null; installed: boolean }
export interface GateTarget { app: string; title: string; kind: 'local' | 'external'; managed: boolean; reachable: boolean; error?: string | null; logins: GateLogin[]; plugins: GatePlugin[]; store: GateStoreItem[]; providers: GateProvider[] }
export interface GateAuth { targets: GateTarget[] }
export interface GateStart { app: string; provider: string; url: string; state: string; flow: 'device' | 'callback' | 'code'; user_code?: string | null; expires_in?: number; callback?: string | null; title?: string; core?: boolean }
/** Модель gate: `id` — настоящий (прокси маскирует не-Claude модели в Anthropic-списке, `listed` — как отдал он), `provider` — канал учётки прокси. */
export interface GateModel { id: string; listed?: string; owned_by?: string | null; type?: string | null; name?: string | null; context?: number | null; max_output?: number | null; provider?: string; provider_title?: string; kind?: 'chat' | 'image' | 'video' }
export interface Account { id: string; provider: string; kind: string; label: string; created_at: string; env_keys: string[]; files: string[]; imported_from?: string | null }
export interface SessionUsage { input: number; output: number; cache_read: number; cache_create: number; cost_usd: number; turns: number }
export interface SessionInfo { id: string; agent: string; cwd: string; title: string; created_at: number; provider_session_id?: string | null; model?: string | null; permission_mode?: string | null; account?: string | null; status: 'starting' | 'idle' | 'working' | 'waiting' | 'exited' | 'error'; alive: boolean; last_seq: number; pending: number; usage: SessionUsage; last_text?: string | null; updated_at?: number; imported?: boolean; effort?: string | null; max_thinking_tokens?: number | null; service_tier?: string | null; cache_ttl?: string | null; tui?: string | null; background?: number; tasks?: TaskInfo[]; queued?: QueuedItem[] }
export interface QueuedItem { id: string; content: any; ts: number }
export interface Scheduled { id: string; project: string; session: string; content: string; at: number; status: string; created_at: number; sent_at?: number | null; note?: string | null; error?: string | null }
/** Фоновая задача Claude Code после хода (Bash в фоне, Agent, Workflow…): из `task_started`, живёт до `task_notification`. */
export interface TaskInfo { task_id: string; tool_use_id?: string | null; task_type?: string | null; description?: string | null; workflow_name?: string | null; started_at?: number; status?: string }
export type SessionEvent = { seq: number; ts: number; kind: string } & Record<string, any>;
/** Возможности сессии для адаптеров чата (agentd `caps`): модели и слэш-команды установленного CLI, усилия, режимы, контекст Codex. */
export interface Caps { provider: string; mcp?: number; models: { value?: string; label?: string; description?: string; resolved?: string; efforts?: string[]; effort_hints?: Record<string, string>; default_effort?: string | null; tiers?: { id: string; name: string; description?: string }[]; default?: boolean; hidden?: boolean; fast?: boolean }[]; effort_hints?: Record<string, string>; default_effort?: string | null; tiers?: { id: string; name: string; description?: string }[]; commands: { name: string; description?: string; argumentHint?: string }[]; efforts: string[]; effort?: string | null; max_thinking_tokens?: number | null; service_tier?: string | null; modes: string[]; token_usage?: any; context_window?: number | null; bypass_allowed?: boolean; account?: any; output_style?: string | null; fast_mode?: string | null; protocol?: 'acp' | string; mode_options?: { id: string; name: string; description?: string }[]; current_mode?: string | null; acp_usage?: { used?: number; size?: number; cost?: any } | null; auth_methods?: any[]; config_options?: any; features: { commands: boolean; files: boolean; effort: boolean; fast: boolean; fast_reason?: string | null; thinking?: boolean; compact: boolean } }
/** Движок workspace'а (agentd engines.rs): что установлено, как ставить. */
export interface Engine { id: string; title: string; protocol: 'claude' | 'codex' | 'acp'; icon: string; bin: string; npm: string[]; installed: boolean; path?: string | null; version?: string | null; missing: string[]; login_hint: string; model_flag?: string | null }
export interface PtyInfo { id: string; title: string; cwd: string; cmd: string[]; cols: number; rows: number; pid?: number | null; alive: boolean; exit_code?: number | null; created_at: number; clients: number; session?: string | null; tui?: boolean }
export interface FsEntry { name: string; kind: 'dir' | 'file'; link: boolean; size: number; mtime: number; mode: number }
export interface FsList { path: string; home: string; entries: FsEntry[] }
export interface GitEntry { path: string; x: string; y: string; staged: boolean; unstaged: boolean; kind: 'changed' | 'renamed' | 'unmerged' | 'untracked' | 'ignored' }
export interface GitBranch { name: string; upstream?: string | null; current: boolean; short: string; ts: number; subject: string }
export interface GitWorktree { path: string; name: string; head?: string | null; branch?: string | null; bare: boolean; detached: boolean; locked: boolean; main: boolean; current: boolean; changes: number }
export interface GitStatus { repo: boolean; cwd: string; reason?: string; head?: string; upstream?: string | null; ahead?: number; behind?: number; entries?: GitEntry[] }
export interface GitCommit { hash: string; short: string; author: string; ts: number; subject: string }
export interface ComponentHealth { name: string; status: string; detail?: string | null }
export interface WorkspaceApp { slug: string; display_name: string; url: string; health: string }
export interface WorkspaceView { id: string; name: string; status: string; agent_status?: string | null; instance?: string | null; ip?: string | null; apps: WorkspaceApp[]; url: string }
export interface ProjectView { name: string; spec: ProjectSpec; network: string; subnet: string; gate: ComponentHealth; workspace?: WorkspaceView | null; grants: Grant[]; missing_capabilities: string[]; grant_env: Record<string, string[]>; available_capabilities: string[]; providers?: Record<string, string[]>; agents?: AgentsSummary | null }
export interface RouteSpec { mode: 'path' | 'port' | 'host'; prefix?: string | null; port?: number | null; open: 'iframe' | 'newtab' }
/** Переводы текстов манифеста/рекомендации по языку интерфейса (store-utils: descOf, reasonOf). */
export interface I18nText { description?: string | null; reason?: string | null }
export interface AppManifest { name: string; title?: string | null; description?: string | null; i18n?: Record<string, I18nText>; icon?: string | null; type: 'app' | 'system' | 'external'; external?: { url_env: string; key_env?: string | null; check_path?: string | null } | null; provides: string[]; requires: string[]; route: RouteSpec; endpoint: { service: string; port: number }; origin: string; upstream?: { source?: string; author?: string; version?: string; categories?: string[]; tags?: string[]; website?: string; release_notes?: string; template?: string } | null; settings: AppSetting[]; env: Record<string, string | { generate: string }>; login: LoginSpec; recommended?: { rank: number; reason: string; i18n?: Record<string, I18nText> } | null; host_docker_socket: boolean; notes: string[]; gallery: string[]; readme?: string | null; website?: string | null; tags: string[] }
export interface LoginSpec { mode: 'generated' | 'default' | 'app' | 'none' | 'external'; user?: string | null; password?: string | null; path?: string | null; note?: string | null }
export interface AppSetting { env: string; type: string; label?: string | null; hint?: string | null; required?: boolean; options?: { label: string; value: string }[] | null }
export interface AppView { name: string; manifest: AppManifest; installed: boolean; port?: number | null; url?: string | null; state?: string | null; granted_to: string[]; settings_values: Record<string, string>; links_to: string[]; links_from: string[]; shared_net: boolean; internal_url?: string | null; link_env: Record<string, string>; has_note: boolean }
export interface SystemStatus { edge_host: string; edge_alt_hosts?: string[]; coder_url: string; coder_user?: string; coder_password_set?: boolean; components: ComponentHealth[]; version: string; build?: string; setup_done?: boolean; update_available?: boolean; app_updates?: number }
// мастер первого запуска (setup.rs)
export interface TailscaleInfo { installed: boolean; state: string; auth_url?: string | null; dns_name?: string | null; ips: string[]; tailnet?: string | null; magic_dns: boolean; https_certs: boolean; cert_domains: string[]; health: string[]; version?: string | null; error?: string | null }
export interface EdgeInfo { host: string; alt_hosts: string[]; tls_internal: boolean; matches_tailscale: boolean; reachable: boolean; dns_ok?: boolean | null; cert_ok?: boolean | null; error?: string | null; checked_at: string }
export interface SetupState { done: boolean; tailscale: TailscaleInfo; edge: EdgeInfo; projects: number; apps_installed: number; backups_enabled: boolean; coder_ok: boolean; suggested_host?: string | null; tailscale_admin_url: string; version: string }
export interface SwitchResult { host: string; url: string; alt_hosts: string[]; restart_in_s: number }
// обновления (updates.rs)
export interface ReleaseInfo { version: string; published?: string | null; notes?: string | null; url?: string | null; sha256?: string | null; size?: number | null }
export interface PackageInfo { file: string; version: string; build?: string | null; arch?: string | null; size: number; sha256: string; store_apps: string[]; notes?: string | null; added_at: string; relation: 'newer' | 'same' | 'older'; arch_ok: boolean }
export interface SystemUpdateInfo { version: string; build: string; arch: string; binary: string; channel_url?: string | null; latest?: ReleaseInfo | null; available: boolean; check_error?: string | null; checked_at?: string | null; packages: PackageInfo[]; can_rollback: boolean; prev_version?: string | null; last_applied?: Record<string, unknown> | null }
export interface AppUpdateInfo { name: string; title: string; origin: string; installed_version?: string | null; catalog_version?: string | null; in_catalog: boolean; version_changed: boolean; compose_changed: boolean; floating_tags: string[]; has_update: boolean; state?: string | null; installed_at: string; updated_at?: string | null }
export interface ComponentContainer { name: string; image: string; image_id: string; state: string; created?: string | null }
export interface ComponentInfo { project: string; title: string; containers: ComponentContainer[]; config_files: string[]; working_dir?: string | null; env_file?: string | null; updatable: boolean; note?: string | null }
export interface UpdatesView { system: SystemUpdateInfo; apps: AppUpdateInfo[]; components: ComponentInfo[]; catalog: { importing: boolean; manifests: number; last_import?: Record<string, unknown> | null } }
export interface Event { at: string; kind: string; subject: string; message: string }

export const CORE = '/agents/api/core';
export const OWN = '/agents/api';
export class OfflineError extends Error { constructor() { super(t('api.offline')); } }

/** timeoutMs — предел на одну попытку (зависший запрос считается ошибкой сети), retries — сколько раз повторить GET (по умолчанию 3). */
async function call<T>(method: string, path: string, body?: unknown, opts: { timeoutMs?: number; retries?: number; headers?: Record<string, string> } = {}): Promise<T> {
  let r: Response | undefined;
  const retries = method === 'GET' ? (opts.retries ?? 3) : 0;
  // GET безопасно повторить: браузер на том же хосте рвёт запросы при смене docker-сетей (ERR_NETWORK_CHANGED), мобильная сеть моргает
  for (let attempt = 0; ; attempt++) {
    const ctl = opts.timeoutMs ? new AbortController() : undefined;
    const timer = ctl ? setTimeout(() => ctl.abort(), opts.timeoutMs) : undefined;
    try {
      r = await fetch(`${CORE}${path}`, { method, headers: { ...(body ? { 'Content-Type': 'application/json' } : {}), ...(opts.headers ?? {}) }, body: body ? JSON.stringify(body) : undefined, cache: 'no-store', signal: ctl?.signal });
      break;
    } catch {
      if (attempt >= retries) throw new OfflineError();
      await new Promise(res => setTimeout(res, 1200 * (attempt + 1)));
    } finally { if (timer) clearTimeout(timer); }
  }
  if (!r) throw new OfflineError();
  if (r.status === 204) return undefined as T;
  const text = await r.text();
  if (!r.ok) {
    let msg = text;
    try { msg = JSON.parse(text).error ?? text; } catch {}
    throw new Error(msg || `HTTP ${r.status}`);
  }
  try { return JSON.parse(text) as T; } catch { return text as T; }
}

export const api = {
  // заголовок присутствия: ядро шлёт push на телефон, только если видимой вкладки Desktop не было полторы минуты
  status: () => call<SystemStatus>('GET', '/status', undefined, { timeoutMs: 6000, retries: 1, headers: { 'X-Desktop-Presence': typeof document !== 'undefined' && document.visibilityState === 'visible' ? 'visible' : 'hidden' } }),  // опрос связи: коротко, чтобы следующий опрос не ждал зависший
  coderCredentials: () => call<{ url: string; user: string; password: string }>('GET', '/system/credentials/coder/reveal'),
  repair: (name: string) => call<string[]>('POST', `/apps/${name}/repair`),
  // мастер первого запуска
  setup: () => call<SetupState>('GET', '/setup'),
  setupLogin: () => call<TailscaleInfo>('POST', '/setup/tailscale/login'),
  setupSwitchHost: (host: string) => call<SwitchResult>('POST', '/setup/edge-host', { host }),
  setupDone: (done: boolean) => call<SetupState>('PUT', '/setup/done', { done }),
  // обновления
  updates: () => call<UpdatesView>('GET', '/updates'),
  updatesCheck: () => call<SystemUpdateInfo>('POST', '/updates/check'),
  updatesChannel: (url: string) => call<SystemUpdateInfo>('PUT', '/updates/channel', { url }),
  updatesDownload: () => call<PackageInfo>('POST', '/updates/download'),
  updatesApply: (file: string) => call<{ from: string; to: string; restart_in_s: number; store_apps: number }>('POST', '/updates/apply', { file }),
  updatesRollback: () => call<{ ok: boolean }>('POST', '/updates/rollback'),
  updatesDeletePackage: (file: string) => call<{ ok: boolean }>('DELETE', `/updates/packages/${encodeURIComponent(file)}`),
  updatesCatalog: (source = 'all') => call<{ started: boolean }>('POST', '/updates/catalog', { source }),
  updateApp: (name: string) => call<AppView>('POST', `/apps/${name}/update`),
  updateComponent: (project: string) => call<{ project: string; changed: string[]; log: string }>('POST', `/updates/components/${encodeURIComponent(project)}`),
  /** Пакет обновления файлом (tar.gz): тело запроса — сам файл. */
  updatesUpload: async (f: File): Promise<PackageInfo> => {
    const r = await fetch(`${CORE}/updates/upload?name=${encodeURIComponent(f.name)}`, { method: 'POST', body: f });
    const text = await r.text();
    if (!r.ok) { let msg = text; try { msg = JSON.parse(text).error ?? text; } catch {} throw new Error(msg || `HTTP ${r.status}`); }
    return JSON.parse(text) as PackageInfo;
  },
  backups: () => call<BackupStatus>('GET', '/backups'),
  setBackups: (c: BackupConfig) => call<BackupConfig>('PUT', '/backups', c),
  backupSnapshot: () => call<Record<string, unknown>>('POST', '/backups/snapshot'),
  backupRun: () => call<Record<string, unknown>>('POST', '/backups/run'),
  appSnapshots: (name: string) => call<RestorePoint[]>('GET', `/apps/${name}/snapshots`),
  appRestore: (name: string, snap: string) => call<AppView>('POST', `/apps/${name}/restore/${encodeURIComponent(snap)}`),
  events: () => call<Event[]>('GET', '/events'),
  projects: () => call<ProjectView[]>('GET', '/projects'),
  // Agent Runtime (ADR-006): терминалы workspace'а через cloudos-agentd; поток — WebSocket ptyWsUrl
  agentd: (name: string) => call<AgentdHealth>('GET', `/projects/${name}/agentd`),
  ptys: (name: string) => call<PtyInfo[]>('GET', `/projects/${name}/ptys`),
  engines: (name: string) => call<{ engines: Engine[] }>('GET', `/projects/${name}/engines`),
  engineInstall: (name: string, id: string) => call<PtyInfo>('POST', `/projects/${name}/engines/${id}/install`, {}),
  ptyCreate: (name: string, body: { cmd?: string[]; cwd?: string; cols?: number; rows?: number; title?: string; session?: string; engine?: string }) => call<PtyInfo>('POST', `/projects/${name}/ptys`, body),
  // файлы и git workspace'а (рендеринги Files и Git карточки сессии) — через agentd
  fsList: (name: string, path: string) => call<FsList>('GET', `/projects/${name}/fs?path=${encodeURIComponent(path)}`),
  fsFileUrl: (name: string, path: string, download = false) => `${CORE}/projects/${name}/fs/file?path=${encodeURIComponent(path)}${download ? '&download=1' : ''}`,
  fsReadText: async (name: string, path: string) => { const r = await fetch(api.fsFileUrl(name, path)); if (!r.ok) throw new Error((await r.json().catch(() => ({})))?.error ?? `HTTP ${r.status}`); return r.text(); },
  fsWrite: async (name: string, path: string, body: Blob | string) => { const r = await fetch(`${CORE}/projects/${name}/fs/file?path=${encodeURIComponent(path)}`, { method: 'PUT', body }); if (!r.ok) throw new Error((await r.json().catch(() => ({})))?.error ?? `HTTP ${r.status}`); return r.json() as Promise<{ path: string; size: number }>; },
  fsMkdir: (name: string, path: string) => call<{ path: string }>('POST', `/projects/${name}/fs/mkdir`, { path }),
  fsRename: (name: string, from: string, to: string) => call<{ path: string }>('POST', `/projects/${name}/fs/rename`, { from, to }),
  fsDelete: (name: string, path: string, recursive = false) => call<void>('DELETE', `/projects/${name}/fs?path=${encodeURIComponent(path)}&recursive=${recursive}`),
  gitStatus: (name: string, cwd: string) => call<GitStatus>('GET', `/projects/${name}/git/status?cwd=${encodeURIComponent(cwd)}`),
  gitDiff: (name: string, cwd: string, path?: string, staged = false, untracked = false) => call<{ diff: string; truncated: boolean }>('GET', `/projects/${name}/git/diff?cwd=${encodeURIComponent(cwd)}${path ? `&path=${encodeURIComponent(path)}` : ''}&staged=${staged}&untracked=${untracked}`),
  gitBranches: (name: string, cwd: string) => call<{ head: string; branches: GitBranch[]; remotes: string[] }>('GET', `/projects/${name}/git/branches?cwd=${encodeURIComponent(cwd)}`),
  gitWorktrees: (name: string, cwd: string) => call<{ cwd: string; worktrees: GitWorktree[] }>('GET', `/projects/${name}/git/worktrees?cwd=${encodeURIComponent(cwd)}`),
  /** Действия git-панели: stage | unstage | discard | commit | checkout | fetch | pull | push | worktrees/add | worktrees/remove | worktrees/merge | suggest | init | remote */
  gitOp: (name: string, op: string, body: Record<string, unknown>) => call<any>('POST', `/projects/${name}/git/${op}`, body),
  gitLog: (name: string, cwd: string, n = 30) => call<{ entries: GitCommit[] }>('GET', `/projects/${name}/git/log?cwd=${encodeURIComponent(cwd)}&n=${n}`),
  ptyKill: (name: string, id: string) => call<void>('DELETE', `/projects/${name}/ptys/${id}`),
  ptyWsUrl: (name: string, id: string) => `${location.origin.replace(/^http/, 'ws')}${CORE}/projects/${name}/ptys/${id}/ws`,
  // сессии агентов (Chat): журнал по WebSocket sessionWsUrl, ввод и ответы — HTTP
  sessions: (name: string) => call<SessionInfo[]>('GET', `/projects/${name}/sessions`),
  sessionCreate: (name: string, body: { agent?: string; cwd?: string; title?: string; resume?: string; model?: string; permission_mode?: string; cache_ttl?: string }) => call<SessionInfo>('POST', `/projects/${name}/sessions`, body),
  sessionGet: (name: string, id: string) => call<{ info: SessionInfo; pending: unknown[]; caps?: Caps }>('GET', `/projects/${name}/sessions/${id}`),
  // композер: усилие модели, запрос к провайдеру (контекст, подсказки файлов, usage) и поиск файлов для «@»
  sessionEffort: (name: string, id: string, effort: string | null) => call<unknown>('POST', `/projects/${name}/sessions/${id}/effort`, { effort }),
  sessionThinking: (name: string, id: string, tokens: number | null) => call<unknown>('POST', `/projects/${name}/sessions/${id}/thinking`, { tokens }),
  sessionTier: (name: string, id: string, tier: string | null) => call<unknown>('POST', `/projects/${name}/sessions/${id}/tier`, { tier }),
  sessionCache: (name: string, id: string, ttl: string | null) => call<unknown>('POST', `/projects/${name}/sessions/${id}/cache`, { ttl }),
  ptyInput: (name: string, id: string, data: string) => call<void>('POST', `/projects/${name}/ptys/${id}/input`, { data }),
  // вход Claude по ссылке и коду (PKCE в ядре): start → ссылка, finish → код со страницы → аккаунт
  oauthStart: (provider: 'claude' | 'codex' | 'claude-oauth' | 'codex-oauth') => call<{ provider: string; state: string; url: string; hint: string; flow?: string; callback?: string; user_code?: string; interval?: number; expires_in?: number }>('POST', '/accounts/oauth/start', { provider }),
  oauthStatus: (state: string) => call<{ status: 'wait' | 'ok' | 'error'; error?: string | null; account?: Account }>('GET', `/accounts/oauth/status?state=${encodeURIComponent(state)}`),
  // провайдеры через gate: OAuth-входы в прокси провайдера llm (Antigravity, Kimi, xAI, Codex, Claude, плагины) через его Management API
  gateAuth: () => call<GateAuth>('GET', '/gate/auth'),
  gateAuthStart: (app: string, provider: string) => call<GateStart>('POST', '/gate/auth/start', { app, provider }),
  gateAuthStatus: (app: string, state: string) => call<{ status: 'wait' | 'ok' | 'error'; error?: string | null }>('GET', `/gate/auth/status?app=${encodeURIComponent(app)}&state=${encodeURIComponent(state)}`),
  gateAuthCallback: (app: string, state: string, redirect_url: string) => call<{ status: string; done?: boolean; file?: string }>('POST', '/gate/auth/callback', { app, state, redirect_url }),
  gateLoginDelete: (app: string, name: string) => call<void>('DELETE', `/gate/auth/login?app=${encodeURIComponent(app)}&name=${encodeURIComponent(name)}`),
  gateLoginDisabled: (app: string, name: string, disabled: boolean) => call<{ name: string; disabled: boolean }>('PATCH', '/gate/auth/login', { app, name, disabled }),
  gateModels: (project?: string) => call<{ targets: { app: string; title: string; models: GateModel[] }[] }>('GET', `/gate/models${project ? `?project=${encodeURIComponent(project)}` : ''}`),
  gatePluginInstall: (app: string, id: string) => call<any>('POST', '/gate/plugins/install', { app, id }),
  gatePluginEnabled: (app: string, id: string, enabled: boolean) => call<{ id: string; enabled: boolean }>('POST', '/gate/plugins/enabled', { app, id, enabled }),
  oauthFinish: (body: { state: string; code: string; label?: string }) => call<Account>('POST', '/accounts/oauth/finish', body),
  sessionControl: (name: string, id: string, req: Record<string, unknown>) => call<any>('POST', `/projects/${name}/sessions/${id}/control`, req),
  fsFind: (name: string, cwd: string, q: string, limit = 20) => call<{ suggestions: { path: string }[]; cwd: string }>('GET', `/projects/${name}/fs/find?cwd=${encodeURIComponent(cwd)}&q=${encodeURIComponent(q)}&limit=${limit}`),
  sessionEvents: (name: string, id: string, since = 0, upto?: number) => call<SessionEvent[]>('GET', `/projects/${name}/sessions/${id}/events?since=${since}&limit=${upto != null ? Math.max(1, Math.min(5000, upto - since)) : 5000}`),
  sessionPrompt: (name: string, id: string, content: unknown) => call<SessionEvent>('POST', `/projects/${name}/sessions/${id}/prompt`, { content }),
  sessionPermission: (name: string, id: string, body: { request_id: string; behavior: 'allow' | 'deny'; updated_input?: unknown; message?: string; answers?: unknown; remember?: boolean; scope?: 'session' | 'project' | 'user'; permissions?: unknown }) => call<SessionEvent>('POST', `/projects/${name}/sessions/${id}/permission`, body),
  sessionFork: (name: string, id: string, body: { at_seq?: number; title?: string } = {}) => call<SessionInfo>('POST', `/projects/${name}/sessions/${id}/fork`, body),
  sessionEdit: (name: string, id: string, body: { seq: number; content: unknown }) => call<SessionEvent>('POST', `/projects/${name}/sessions/${id}/edit`, body),
  sessionQueue: (name: string, id: string, content: unknown) => call<QueuedItem>('POST', `/projects/${name}/sessions/${id}/queue`, { content }),
  sessionUnqueue: (name: string, id: string, qid: string) => call<void>('DELETE', `/projects/${name}/sessions/${id}/queue/${qid}`),
  sessionTaskStop: (name: string, id: string, task: string) => call<unknown>('POST', `/projects/${name}/sessions/${id}/tasks/${task}/stop`, {}),
  permissionsGet: (name: string, scope: 'user' | 'project', cwd: string) => call<{ scope: string; path: string; exists: boolean; allow: string[]; deny: string[]; ask: string[]; default_mode?: string | null }>('GET', `/projects/${name}/permissions?scope=${scope}&cwd=${encodeURIComponent(cwd)}`),
  permissionsPut: (name: string, scope: 'user' | 'project', cwd: string, body: { allow: string[]; deny: string[]; ask: string[] }) => call<{ scope: string; path: string; allow: string[]; deny: string[]; ask: string[] }>('PUT', `/projects/${name}/permissions?scope=${scope}&cwd=${encodeURIComponent(cwd)}`, body),
  sessionInterrupt: (name: string, id: string) => call<unknown>('POST', `/projects/${name}/sessions/${id}/interrupt`, {}),
  sessionMode: (name: string, id: string, mode: string) => call<unknown>('POST', `/projects/${name}/sessions/${id}/mode`, { mode }),
  sessionModel: (name: string, id: string, model: string | null, context_window?: number | null) => call<unknown>('POST', `/projects/${name}/sessions/${id}/model`, { model, context_window: context_window ?? null }),
  sessionDelete: (name: string, id: string) => call<void>('DELETE', `/projects/${name}/sessions/${id}`),
  // терминальный режим (как в CloudCLI): родной TUI агента на той же беседе в PTY сессии; поток — ptyWsUrl
  sessionTui: (name: string, id: string, body: { bypass?: boolean; cols?: number; rows?: number }) => call<PtyInfo>('POST', `/projects/${name}/sessions/${id}/tui`, body),
  sessionTuiStop: (name: string, id: string) => call<void>('DELETE', `/projects/${name}/sessions/${id}/tui`),
  sessionWsUrl: (name: string, id: string, since = 0) => `${location.origin.replace(/^http/, 'ws')}${CORE}/projects/${name}/sessions/${id}/ws?since=${since}`,
  usage: (name: string, since = 0) => call<Record<string, unknown>[]>('GET', `/projects/${name}/usage?since=${since}`),
  usageSummary: (days = 30) => call<UsageSummary>('GET', `/usage?days=${days}`),
  // аккаунты агентов (ADR-006): реестр, импорт входа из workspace, аккаунт проекта по умолчанию, переназначение сессии
  accounts: () => call<{ accounts: Account[]; defaults: Record<string, string> }>('GET', '/accounts'),
  accountCreate: (body: { provider: string; kind: string; label: string; env?: Record<string, string>; files?: Record<string, string> }) => call<Account>('POST', '/accounts', body),
  accountImport: (body: { provider: string; project: string; label?: string; dir?: string }) => call<Account>('POST', '/accounts/import', body),
  // вход в подписку через терминал workspace'а (claude auth login / codex login --device-auth); после выхода — импорт из каталога
  accountLogin: (body: { provider: 'claude' | 'codex'; project: string }) => call<{ pty: string; dir: string; project: string; provider: string }>('POST', '/accounts/login', body),
  accountDelete: (id: string) => call<void>('DELETE', `/accounts/${id}`),
  projectAccount: (name: string, account: string | null) => call<void>('PUT', `/projects/${name}/account`, { account }),
  sessionAccount: (name: string, id: string, account: string | null) => call<SessionInfo>('POST', `/projects/${name}/sessions/${id}/account`, { account }),
  agentsAll: () => call<{ away: boolean; presence_seen_ms: number; projects: Record<string, AgentsSummary> }>('GET', '/agents'),
  // подписка на уведомления ядра с телефона: сервер ntfy, тема, пользователь и пароль (секрет — только по запросу)
  // раскладка холста «Агентов» — в ядре, общая для всех устройств владельца
  // политика workspace'ов: разрешён ли режим обхода разрешений Claude Code (managed-settings, решение владельца)
  agentsPolicyGet: () => call<{ bypass: boolean }>('GET', '/agents/policy'),
  agentsPolicyPut: (bypass: boolean) => call<{ bypass: boolean; applied: string[] }>('PUT', '/agents/policy', { bypass }),
  agentsLayoutGet: () => call<{ cards?: { project: string; kind: 'pty' | 'session'; id: string; w: 1 | 2; h: 1 | 2 }[]; cols?: 0 | 1 | 2 | 3 } | null>('GET', '/agents/layout'),
  agentsLayoutPut: (layout: unknown) => call<void>('PUT', '/agents/layout', layout),
  agentsNotify: () => call<{ app: string | null; server: string | null; topic: string; user: string; password: string | null; token: string | null }>('GET', '/agents/notify'),
  applyProject: (spec: ProjectSpec) => call<ProjectView>('POST', '/projects', spec),
  deleteProject: (name: string) => call<void>('DELETE', `/projects/${name}`),
  wsStart: (name: string) => call<ProjectView>('POST', `/projects/${name}/workspace/start`),
  wsStop: (name: string) => call<ProjectView>('POST', `/projects/${name}/workspace/stop`),
  grant: (name: string, capability: string, app?: string) => call<ProjectView>('POST', `/projects/${name}/grants`, app ? { capability, app } : { capability }),  // app — какой провайдер, если их несколько
  revoke: (name: string, capability: string) => call<ProjectView>('DELETE', `/projects/${name}/grants/${capability}`),
  apps: () => call<AppView[]>('GET', '/apps'),
  install: (name: string) => call<AppView>('POST', `/apps/${name}/install`),
  app: (name: string) => call<AppView>('GET', `/apps/${name}`),
  /** Установка устойчива к обрыву связи: ядро при развёртывании создаёт docker-сеть, и браузер на том же хосте может оборвать
   *  запрос (ERR_NETWORK_CHANGED). Тогда повторяем запрос (install идемпотентен) и ждём статуса опросом карточки. */
  /** Этапы операции над приложением — из ленты событий ядра (deploy запущен, стек running, health OK, маршрут…). */
  stages: async (name: string, since: string): Promise<string[]> => {
    try { const ev = await call<{ at: string; subject: string; message: string }[]>('GET', '/events'); return ev.filter(e => e.subject === name && e.at > since).map(e => e.message).reverse(); } catch { return []; }
  },
  installWait: async (name: string, onProgress?: (s: string) => void): Promise<AppView> => {
    const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));
    const since = new Date().toISOString();
    // stopped: запрос этапов мог быть в полёте в момент завершения — иначе поздний onProgress снова пометит карточку занятой
    let stopped = false;
    const timer = setInterval(async () => { if (stopped) return; const st = await api.stages(name, since); if (!stopped && st.length) onProgress?.(st[st.length - 1]); }, 2500);
    try { return await api.installWaitInner(name, s => { if (!stopped) onProgress?.(s); }); } finally { stopped = true; clearInterval(timer); }
  },
  installWaitInner: async (name: string, onProgress?: (s: string) => void): Promise<AppView> => {
    const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));
    for (let attempt = 0; attempt < 2; attempt++) {
      try { return await call<AppView>('POST', `/apps/${name}/install`); }
      catch (e) {
        if (!(e instanceof OfflineError)) throw e;
        onProgress?.(t('api.waitingCore')); await sleep(2500);
        const a = await call<AppView>('GET', `/apps/${name}`).catch(() => null);
        if (a?.installed) { attempt = 2; }
      }
    }
    // запрос ушёл, но ответ потерян: ждём, пока стек поднимется
    for (let i = 0; i < 100; i++) {
      await sleep(3000);
      const a = await call<AppView>('GET', `/apps/${name}`).catch(() => null);
      if (a?.installed && a.url && (a.state === 'running' || i > 20)) return a;
      onProgress?.(a?.installed ? t('api.startingState', { state: a.state ?? '…' }) : t('api.installing'));
    }
    throw new Error(t('api.installTimeout'));
  },
  remove: (name: string, purge = false) => call<void>('DELETE', `/apps/${name}${purge ? '?purge=true' : ''}`),
  /** Удаление с той же устойчивостью к обрыву связи, что и installWait: повтор запроса и ожидание, пока приложение исчезнет. */
  removeWait: async (name: string, purge = false, onProgress?: (s: string) => void): Promise<void> => {
    const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));
    const since = new Date().toISOString();
    let stopped = false;
    const timer = setInterval(async () => { if (stopped) return; const st = await api.stages(name, since); if (!stopped && st.length) onProgress?.(st[st.length - 1]); }, 2500);
    try { await api.removeWaitInner(name, purge); } finally { stopped = true; clearInterval(timer); }
  },
  removeWaitInner: async (name: string, purge = false): Promise<void> => {
    const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));
    for (let attempt = 0; attempt < 2; attempt++) {
      try { await call<void>('DELETE', `/apps/${name}${purge ? '?purge=true' : ''}`); return; }
      catch (e) {
        if (!(e instanceof OfflineError)) throw e;
        await sleep(2500);
        const a = await call<AppView>('GET', `/apps/${name}`).catch(() => null);
        if (a && !a.installed) return;
      }
    }
    for (let i = 0; i < 60; i++) {
      await sleep(3000);
      const a = await call<AppView>('GET', `/apps/${name}`).catch(() => null);
      if (a && !a.installed) return;
    }
    throw new Error(t('api.removeTimeout'));
  },
  logs: (name: string, tail = 100) => call<string>('GET', `/apps/${name}/logs?tail=${tail}`),
  settings: (name: string, values: Record<string, string>) => call<AppView>('PUT', `/apps/${name}/settings`, values),
  mcp: (name: string) => call<McpList>('GET', `/projects/${name}/mcp`),
  mcpSave: (name: string, servers: McpServer[]) => call<McpSaveResult>('POST', `/projects/${name}/mcp`, { servers }, { timeoutMs: 130000 }),
  overrides: (name: string) => call<{ env: string; compose: string; effective_compose: string }>('GET', `/apps/${name}/overrides`),
  putOverrides: (name: string, o: { env: string; compose: string }) => call<AppView>('PUT', `/apps/${name}/overrides`, o),
  fileRoots: () => call<{ id: string; title: string; kind: string }[]>('GET', '/files/roots'),
  fileList: (root: string, path: string) => call<{ root: string; path: string; entries: { name: string; dir: boolean; size: number; modified?: string | null; text: boolean }[] }>('GET', `/files/${root}/list?path=${encodeURIComponent(path)}`),
  fileReadUrl: (root: string, path: string) => `${CORE}/files/${root}/read?path=${encodeURIComponent(path)}`,
  fileRead: async (root: string, path: string) => { const r = await fetch(`${CORE}/files/${root}/read?path=${encodeURIComponent(path)}`); if (!r.ok) throw new Error(await r.text()); return r.text(); },
  fileWrite: (root: string, path: string, content: string) => call<void>('PUT', `/files/${root}/write?path=${encodeURIComponent(path)}`, { content }),
  fileUpload: async (root: string, path: string, file: File) => { const r = await fetch(`${CORE}/files/${root}/upload?path=${encodeURIComponent(path)}`, { method: 'POST', body: file }); if (!r.ok) throw new Error(await r.text()); },
  fileMkdir: (root: string, path: string) => call<void>('POST', `/files/${root}/mkdir?path=${encodeURIComponent(path)}`),
  fileRename: (root: string, path: string, to: string) => call<void>('POST', `/files/${root}/rename?path=${encodeURIComponent(path)}`, { to }),
  fileDelete: (root: string, path: string) => call<void>('DELETE', `/files/${root}/delete?path=${encodeURIComponent(path)}`),
  setOpen: (name: string, open: 'iframe' | 'newtab') => call<AppView>('PUT', `/apps/${name}/open`, { open }),
  link: (name: string, provider: string) => call<AppView>('POST', `/apps/${name}/links`, { provider }),
  unlink: (name: string, provider: string) => call<AppView>('DELETE', `/apps/${name}/links/${provider}`),
  setShared: (name: string, shared: boolean) => call<AppView>('PUT', `/apps/${name}/shared`, { shared }),
  rotate: (name: string, key: string) => call<AppView>('POST', `/apps/${name}/settings/${key}/rotate`),
  note: (name: string) => call<{ text: string }>('GET', `/apps/${name}/note`),
  setNote: (name: string, text: string) => call<void>('PUT', `/apps/${name}/note`, { text }),
  reveal: (name: string, key: string) => call<{ key: string; value: string }>('GET', `/apps/${name}/settings/${key}/reveal`),
  readme: async (name: string) => { const r = await fetch(`${CORE}/apps/${name}/readme`); return r.ok ? r.text() : ''; },
  desktopSettings: () => call<any>('GET', '/settings/desktop'),
  putDesktopSettings: (v: unknown) => call<any>('PUT', '/settings/desktop', v),
};

export interface BackupConfig { enabled: boolean; hourly: number; daily: number; weekly: number; restic: boolean; restic_keep_daily: number; restic_keep_weekly: number; restic_keep_monthly: number; remote: string }
export interface DatasetInfo { name: string; mountpoint: string; used: number; snapshots: number; snapshots_used: number; last: number | null }
export interface BackupStatus { config: BackupConfig; datasets: DatasetInfo[]; restic_installed: boolean; restic_repo: string; restic_snapshots: number; restic_size: number; last: Record<string, any>; running: boolean }
export interface RestorePoint { snapshot: string; kind: string; created: number; path: string }

// ---- собственные представления сервиса «Агенты» (cloudos-agents) ----
export interface WsProject { name: string; description?: string | null; status: string; apps: { slug: string; url: string; display_name: string }[]; reachable: boolean; cost_today: number; sessions: SessionInfo[]; ptys: PtyInfo[]; hidden?: boolean }
export interface Workspaces { projects: WsProject[]; today: UsageTotals }
export interface PendingReq { request_id: string; tool_name: string; input: any; seq: number; kind?: string; suggestions?: any[] | null }
export interface InboxItem { project: string; session: SessionInfo; pending?: PendingReq[] }
export interface Inbox { waiting: InboxItem[]; failed: InboxItem[]; done: InboxItem[]; today: UsageTotals }
export interface ModelPrice { input: number; output: number; cache_read: number; cache_write: number; note?: string }
export interface ModelRow { id: string; agent: string; turns: number; input: number; output: number; cache_read: number; cache_create: number; cost_usd: number; price?: ModelPrice | null }
export interface Route { id: string; kind: string; name: string; sub?: string; provider?: string; health: string }
export interface Limit { scope: string; per_day: number; action: string }
export interface Models { period_days: number; total: UsageTotals; by_day: UsageBucket[]; by_project: UsageBucket[]; routes: Route[]; models: ModelRow[]; defaults: { claude: string; codex: string }; limits: Limit[]; project_defaults: Record<string, string> }
export interface ChatSearchCfg { kind: 'none' | 'searxng' | 'brave' | 'tavily'; url: string; key: string }
export interface ChatSettings { route: string; model: string; search: ChatSearchCfg }
export interface AgentsSettings { prices: Record<string, ModelPrice>; defaults: { claude: string; codex: string }; limits: Limit[]; mail_paused?: boolean; chat?: ChatSettings }
// агенты (сотрудники), рутины и запуски — routines.rs сервиса; чат scratch — chat.rs
export interface RunBrief { id: string; status: string; started_at: number; ended_at?: number | null; cost_usd: number; summary?: string | null; reason?: string | null; session?: string | null; project: string }
export interface AgentDef { id: string; name: string; brief: string; provider: string; model?: string | null; account?: string | null; workspace: string; mode: 'ask' | 'auto' | 'bypass'; effort?: string | null; rules?: { allow: string[]; deny: string[] }; memory: string; remember: boolean; place: string; can_mail: boolean; can_routine: boolean; memory_budget: number; reflect: boolean; skills: string[]; reflected_at?: number | null; created_at: number; updated_at: number; routines?: number; last_run?: RunBrief | null }
export interface Trigger { kind: 'schedule' | 'webhook' | 'event' | 'manual'; cron: string; event: string; contains: string }
export interface Routine { id: string; name: string; agent: string; prompt: string; trigger: Trigger; budget: { run_usd: number; day_usd: number }; notify: 'ask_error' | 'always' | 'never'; enabled: boolean; mode?: '' | 'ask' | 'auto' | 'bypass'; token: string; created_at: number; updated_at: number; last_run_at?: number | null; last_status?: string | null; agent_name?: string; workspace?: string; schedule_text?: string; next_at?: number | null; last?: RunBrief | null }
export interface Run extends RunBrief { routine?: string | null; routine_name?: string | null; agent: string; agent_name: string; trigger: string; prompt?: string | null; from_agent?: string | null; reply_to?: string | null; hops?: number }
/** Навык из каталога сервиса (skills.rs): повторяемая процедура; source — user | agent:<id>. */
export interface Skill { name: string; description: string; body: string; source: string; created_at: number; updated_at: number; agents?: string[] }
// простой чат (simplechat.rs): беседы напрямую по API провайдера через маршруты ядра, инкогнито в памяти сервиса
export interface ChatTools { search: boolean; fetch: boolean }
export interface SimpleChat { id: string; kind: 'simple'; title: string; route: string; model: string; system: string; tools: ChatTools; incognito: boolean; created_at: number; updated_at: number; cost_usd: number; tokens_in: number; tokens_out: number; messages: number; expires_at?: number }
export interface ChatAttachment { file: string; name: string; mime: string; size: number; text?: string }
export interface ChatPreset { id: string; name: string; text: string }
export interface ChatToolUse { name: string; args: Record<string, unknown>; result: string; ok: boolean }
export interface ChatMessage { id: string; chat: string; seq: number; role: 'user' | 'assistant'; text: string; attachments: ChatAttachment[]; tools: ChatToolUse[]; tokens_in: number; tokens_out: number; cost_usd: number; created_at: number; error?: string | null }
export interface ChatModel { id: string; name: string; context?: number | null; price_in?: number | null; price_out?: number | null; vision?: boolean | null; tools?: boolean | null; provider?: string; provider_title?: string; vendor?: string | null; kind?: 'chat' | 'image' | 'video' }
/** Модели по провайдерам в порядке, который отдал сервер (подписки Claude Code и Codex первыми); внутри — новее выше. */
export function byProvider<T extends { id: string; name?: string | null; provider?: string; provider_title?: string; kind?: string }>(list: T[]): { key: string; title: string; models: T[] }[] {
  const groups = new Map<string, { key: string; title: string; models: T[] }>();
  for (const m of list) { if ((m.kind ?? 'chat') !== 'chat') continue; const k = m.provider ?? 'other'; if (!groups.has(k)) groups.set(k, { key: k, title: m.provider_title ?? k, models: [] }); groups.get(k)!.models.push(m); }
  for (const g of groups.values()) g.models.sort((a, b) => (b.name ?? b.id).localeCompare(a.name ?? a.id, undefined, { numeric: true }));
  return [...groups.values()];
}
export interface ChatRoute { id: string; kind: 'gate' | 'key'; title: string; provider: string; host: string; models: ChatModel[]; error?: string }
export interface ChatRoutes { routes: ChatRoute[]; defaults: { route?: string | null; model?: string | null }; search: { ready: boolean; kind: string } }
export type ChatEvent = { kind: 'user'; message: ChatMessage } | { kind: 'delta'; text: string } | { kind: 'tool_start'; name: string; args: Record<string, unknown> } | { kind: 'tool'; name: string; args: Record<string, unknown>; result: string; ok: boolean } | { kind: 'done'; message: ChatMessage; chat: SimpleChat } | { kind: 'error'; error: string; message?: ChatMessage; chat?: SimpleChat };
export interface ChatStatus { project: string; exists: boolean; status: string; reachable: boolean; sessions: SessionInfo[]; cost_today?: number; busy: boolean; error?: string }
async function own<T>(method: string, path: string, body?: unknown): Promise<T> {
  const r = await fetch(`${OWN}${path}`, { method, headers: body ? { 'Content-Type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined, cache: 'no-store' });
  if (!r.ok) { let m = `HTTP ${r.status}`; try { m = (await r.json()).error ?? m; } catch {} throw new Error(m); }
  const t = await r.text(); return (t ? JSON.parse(t) : undefined) as T;
}
export const agents = {
  health: () => own<{ ok: boolean; version: string; cloudd: boolean; cloudd_version?: string | null }>('GET', '/health'),
  workspaces: () => own<Workspaces>('GET', '/workspaces'),
  inbox: () => own<Inbox>('GET', '/inbox'),
  models: (days = 1) => own<Models>('GET', `/models?days=${days}`),
  settings: () => own<AgentsSettings>('GET', '/settings'),
  saveSettings: (patch: Partial<AgentsSettings>) => own<void>('PUT', '/settings', patch),
  agentsList: () => own<{ agents: AgentDef[] }>('GET', '/agents'),
  agentCreate: (a: Partial<AgentDef>) => own<AgentDef>('POST', '/agents', a),
  agentUpdate: (id: string, patch: Partial<AgentDef>) => own<AgentDef>('PUT', `/agents/${id}`, patch),
  agentDelete: (id: string) => own<void>('DELETE', `/agents/${id}`),
  agentRun: (id: string, prompt: string) => own<{ run: Run }>('POST', `/agents/${id}/run`, { prompt }),
  routines: () => own<{ routines: Routine[]; scheduler: boolean; now: number }>('GET', '/routines'),
  routineCreate: (r: Partial<Routine>) => own<Routine>('POST', '/routines', r),
  routineUpdate: (id: string, patch: Partial<Routine>) => own<Routine>('PUT', `/routines/${id}`, patch),
  routineDelete: (id: string) => own<void>('DELETE', `/routines/${id}`),
  routineRun: (id: string, prompt?: string) => own<{ run: Run }>('POST', `/routines/${id}/run`, prompt ? { prompt } : { prompt: '' }),
  runs: (q: { routine?: string; agent?: string; limit?: number } = {}) => own<{ runs: Run[] }>('GET', `/runs?${new URLSearchParams(Object.fromEntries(Object.entries(q).filter(([, v]) => v != null && v !== '').map(([k, v]) => [k, String(v)]))).toString()}`),
  runDelete: (id: string) => own<void>('DELETE', `/runs/${id}`),
  agentMail: (id: string, text: string, from?: string) => own<{ run: Run }>('POST', `/agents/${id}/mail`, { text, from }),
  agentDraft: (description: string, workspace?: string) => own<{ name: string; brief: string; workspace: string }>('POST', '/agents/draft', { description, workspace }),
  skills: () => own<{ skills: Skill[] }>('GET', '/skills'),
  skillsMaterialize: (project: string, names: string[]) => own<{ ok: boolean; count: number }>('POST', '/skills/materialize', { project, names }),
  skillCreate: (s: Pick<Skill, 'name' | 'description' | 'body'>) => own<Skill>('POST', '/skills', s),
  skillUpdate: (name: string, patch: Partial<Pick<Skill, 'description' | 'body'>>) => own<Skill>('PUT', `/skills/${name}`, patch),
  skillDelete: (name: string) => own<void>('DELETE', `/skills/${name}`),
  scheduled: (project: string, session: string) => own<{ scheduled: Scheduled[]; now: number }>('GET', `/scheduled?project=${encodeURIComponent(project)}&session=${encodeURIComponent(session)}`),
  scheduleCreate: (b: { project: string; session: string; content: string; at: number }) => own<Scheduled>('POST', '/scheduled', b),
  scheduleDelete: (id: string) => own<void>('DELETE', `/scheduled/${id}`),
  chat: () => own<ChatStatus>('GET', '/chat'),
  chatEnsure: () => own<ChatStatus>('POST', '/chat/ensure', {}),
  chatCreate: (b: { agent: string; model?: string; permission_mode?: string; cache_ttl?: string }) => own<SessionInfo>('POST', '/chat', b),
  chatMove: (id: string, to: string) => own<{ project: string; session: SessionInfo }>('POST', `/chat/${id}/move`, { to }),
  chatRoutes: () => own<ChatRoutes>('GET', '/chat/routes'),
  chats: () => own<{ chats: SimpleChat[] }>('GET', '/chats'),
  chatNew: (b: { route?: string; model?: string; system?: string; tools?: ChatTools; incognito?: boolean; title?: string }) => own<SimpleChat>('POST', '/chats', b),
  search: (q: string) => own<SearchResult>('GET', `/search?q=${encodeURIComponent(q)}`),
  chatGet: (id: string) => own<{ chat: SimpleChat; messages: ChatMessage[]; busy: boolean }>('GET', `/chats/${id}`),
  chatPatch: (id: string, patch: Partial<Pick<SimpleChat, 'title' | 'model' | 'route' | 'system' | 'tools'>>) => own<SimpleChat>('PUT', `/chats/${id}`, patch),
  chatRemove: (id: string) => own<void>('DELETE', `/chats/${id}`),
  chatStop: (id: string) => own<{ stopped: boolean }>('POST', `/chats/${id}/stop`, {}),
  chatKeep: (id: string) => own<SimpleChat>('POST', `/chats/${id}/keep`, {}),
  chatHandoff: (id: string, agent: string) => own<{ session: SessionInfo; project: string }>('POST', `/chats/${id}/handoff`, { agent }),
  chatFileUrl: (id: string, file: string) => `${OWN}/chats/${id}/files/${file}`,
  chatPresets: () => own<{ presets: ChatPreset[] }>('GET', '/chat/presets'),
  chatPresetsSave: (presets: ChatPreset[]) => own<{ presets: ChatPreset[] }>('PUT', '/chat/presets', { presets }),
  /** Сообщение в простой чат: поток SSE (delta, tool, done, error); возвращает функцию отмены чтения (ответ на сервере продолжится — остановка через chatStop). */
  async chatSend(id: string, body: { text: string; attachments?: { name: string; mime: string; data: string }[]; model?: string; tools?: ChatTools }, onEvent: (e: ChatEvent) => void, signal?: AbortSignal): Promise<void> {
    const r = await fetch(`${OWN}/chats/${id}/messages`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), cache: 'no-store', signal });
    if (!r.ok || !r.body) { let m = `HTTP ${r.status}`; try { m = (await r.json()).error ?? m; } catch {} throw new Error(m); }
    const reader = r.body.getReader(); const dec = new TextDecoder(); let buf = '';
    for (;;) {
      const { value, done } = await reader.read(); if (done) break;
      buf += dec.decode(value, { stream: true });
      let i;
      while ((i = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, i).replace(/\r$/, ''); buf = buf.slice(i + 1);
        if (!line.startsWith('data:')) continue;
        try { onEvent(JSON.parse(line.slice(5).trim()) as ChatEvent); } catch {}
      }
    }
  },
};

// ---- поиск по беседам и экспорт (срез 34, docs/agents-ux.md §4.6) ----
export interface SearchHit { kind: 'session' | 'chat'; project?: string; id: string; title: string; agent?: string; model?: string; ts: number; seq?: number; mid?: string; role: string; snippet: string; count: number }
export interface SearchResult { q: string; words: string[]; hits: SearchHit[]; stopped: string[]; total: number }
export const downloadUrl = {
  session: (project: string, id: string, fmt: 'md' | 'json') => `/agents/api/core/projects/${project}/sessions/${id}/download?format=${fmt}`,
  chat: (id: string, fmt: 'md' | 'json') => `/agents/api/chats/${id}/export?format=${fmt}`,
  usageCsv: (days: number) => `/agents/api/core/usage/export.csv?days=${days}`,
};
/** Имя файла из названия беседы: буквы и цифры любого алфавита, остальное — дефис; дата в конце. */
export function fileName(title: string, ext: string): string {
  const base = (title || '').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, '-').replace(/^-+|-+$/g, '').slice(0, 60) || 'export';
  return `${base}-${new Date().toISOString().slice(0, 10)}.${ext}`;
}
/** Скачать ответ сервера файлом (blob + <a download>). */
export async function download(url: string, name: string): Promise<void> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${r.status} ${(await r.text()).slice(0, 200)}`);
  const b = await r.blob(); const a = document.createElement('a'); a.href = URL.createObjectURL(b); a.download = name; document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 5000);
}

// ---- MCP-серверы workspace'а (срез 36, docs/agents-ux.md §4.3.10) ----
export interface McpServer { name: string; transport: 'stdio' | 'http' | 'sse'; command: string; args: string[]; url: string; env: Record<string, string>; headers: Record<string, string> }
export interface McpList { servers: McpServer[]; engines: Record<string, number | null> }
export interface McpSaveResult extends McpList { applied: Record<string, string> }
