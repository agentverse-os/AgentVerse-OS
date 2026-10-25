// API client for the "Agents" app: a copy of the Desktop client, but all core calls go through the cloudos-agents
// service proxy (CORE = /agents/api/core → cloudd /api), and the service's own views — through OWN = /agents/api
// (agents.*). The types mirror the cloudd utoipa schemas (/api/openapi.json).
import { t } from './i18n';
export type Runtime = 'incus' | 'incus-nesting' | 'incus-vm';
export interface WorkspaceSpec { runtime: Runtime; cpu: number; memory: string | number; home: string | number; image?: string | null }
export interface ProjectSpec { schema: number; project: string; workspace: WorkspaceSpec; agents: string[]; capabilities: string[]; description?: string | null }  // description — the owner's note, the core only stores it
export interface Grant { project: string; capability: string; app: string; granted_at: string }
export interface AgentdHealth { version: string; protocol: number; uptime_s: number; ptys: number; ptys_alive: number; user?: string | null; cgroup?: string | null; auth: boolean }
export interface AgentSessionBrief { id: string; title: string; agent: string; status: string; alive: boolean; pending: number; last_text?: string | null; cost_usd: number }
export interface AgentsSummary { total: number; working: number; waiting: number; sessions: AgentSessionBrief[]; updated_at: number }
export interface UsageTotals { cost_usd: number; input: number; output: number; cache_read: number; cache_create: number; turns: number }
export interface UsageBucket extends UsageTotals { key: string }
export interface UsageSummary { since_ts: number; total: UsageTotals; by_project: UsageBucket[]; by_model: UsageBucket[]; by_day: UsageBucket[] }
export interface GateProvider { id: string; title: string; flow: 'device' | 'callback' | 'code'; callback?: string | null; models?: string; since?: string; core?: boolean }
export interface GateLogin { name: string; provider: string; label?: string | null; email?: string | null; status?: string | null; status_message?: string | null; disabled: boolean; unavailable: boolean; plan?: string | null; auth_index?: string | null; limits?: { plan?: string | null; windows: LimitWindow[] } | null }
export interface GatePlugin { id: string; name?: string | null; version?: string | null; enabled: boolean; oauth: boolean; provider?: string | null }
export interface GateStoreItem { id: string; name?: string | null; description?: string | null; version?: string | null; installed: boolean }
export interface GateTarget { app: string; title: string; kind: 'local' | 'external'; managed: boolean; reachable: boolean; error?: string | null; logins: GateLogin[]; plugins: GatePlugin[]; store: GateStoreItem[]; providers: GateProvider[] }
export interface GateAuth { targets: GateTarget[] }
export interface GateStart { app: string; provider: string; url: string; state: string; flow: 'device' | 'callback' | 'code'; user_code?: string | null; expires_in?: number; callback?: string | null; title?: string; core?: boolean }
/** Gate model: `id` — the real one (the proxy masks non-Claude models in the Anthropic list, `listed` — as it returned it), `provider` — the proxy
 *  account channel. */
export interface GateModel { id: string; listed?: string; owned_by?: string | null; type?: string | null; name?: string | null; context?: number | null; max_output?: number | null; provider?: string; provider_title?: string; kind?: 'chat' | 'image' | 'video' }
export interface Account { id: string; provider: string; kind: string; label: string; created_at: string; env_keys: string[]; files: string[]; imported_from?: string | null; pool?: boolean; stamp?: number; models?: string[] }
/** Subscription limit window (docs/agents-ux.md §9.2): the provider id (five_hour, seven_day, primary…), share in %, reset (RFC 3339), window length
 *  in minutes. */
export interface LimitWindow { id: string; used_pct: number; resets_at?: string | null; window_mins?: number | null }
export interface AccountLimits { account: string; provider: string; plan?: string | null; windows: LimitWindow[]; credits?: Record<string, unknown> | null; fetched_at: string; error?: string | null; refreshed_token: boolean }
export interface SessionUsage { input: number; output: number; cache_read: number; cache_create: number; cost_usd: number; turns: number }
export interface SessionInfo { id: string; agent: string; cwd: string; title: string; label?: string | null; import_v?: number | null; created_at: number; provider_session_id?: string | null; model?: string | null; permission_mode?: string | null; account?: string | null; status: 'starting' | 'idle' | 'working' | 'waiting' | 'exited' | 'error'; alive: boolean; last_seq: number; pending: number; usage: SessionUsage; last_text?: string | null; updated_at?: number; imported?: boolean; effort?: string | null; max_thinking_tokens?: number | null; service_tier?: string | null; cache_ttl?: string | null; tui?: string | null; background?: number; tasks?: TaskInfo[]; queued?: QueuedItem[] }
export interface QueuedItem { id: string; content: any; ts: number }
export interface Scheduled { id: string; project: string; session: string; content: string; at: number; status: string; created_at: number; sent_at?: number | null; note?: string | null; error?: string | null }
/** Claude Code background task after a turn (background Bash, Agent, Workflow…): from `task_started`, lives until
 *  `task_notification`. */
export interface TaskInfo { task_id: string; tool_use_id?: string | null; task_type?: string | null; description?: string | null; workflow_name?: string | null; started_at?: number; status?: string }
export type SessionEvent = { seq: number; ts: number; kind: string } & Record<string, any>;
/** Session capabilities for the chat adapters (agentd `caps`): models and slash commands of the installed CLI, efforts, modes, Codex
 *  context. */
export interface Caps { provider: string; mcp?: number; models: { value?: string; label?: string; description?: string; resolved?: string; efforts?: string[]; effort_hints?: Record<string, string>; default_effort?: string | null; tiers?: { id: string; name: string; description?: string }[]; default?: boolean; hidden?: boolean; fast?: boolean }[]; effort_hints?: Record<string, string>; default_effort?: string | null; tiers?: { id: string; name: string; description?: string }[]; commands: { name: string; description?: string; argumentHint?: string }[]; efforts: string[]; effort?: string | null; max_thinking_tokens?: number | null; service_tier?: string | null; modes: string[]; token_usage?: any; context_window?: number | null; bypass_allowed?: boolean; account?: any; output_style?: string | null; fast_mode?: string | null; protocol?: 'acp' | string; mode_options?: { id: string; name: string; description?: string }[]; current_mode?: string | null; acp_usage?: { used?: number; size?: number; cost?: any } | null; auth_methods?: any[]; config_options?: any; features: { commands: boolean; files: boolean; effort: boolean; fast: boolean; fast_reason?: string | null; thinking?: boolean; compact: boolean } }
/** Workspace engine (agentd engines.rs): what is installed, how to install. */
export interface Engine { id: string; title: string; protocol: 'claude' | 'codex' | 'acp' | 'agy'; icon: string; bin: string; npm: string[]; installed: boolean; path?: string | null; version?: string | null; missing: string[]; login_hint: string; model_flag?: string | null }
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
/** Manifest/recommendation text translations by UI language (store-utils: descOf, reasonOf). */
export interface I18nText { description?: string | null; reason?: string | null }
export interface AppManifest { name: string; title?: string | null; description?: string | null; i18n?: Record<string, I18nText>; icon?: string | null; type: 'app' | 'system' | 'external'; external?: { url_env: string; key_env?: string | null; check_path?: string | null } | null; provides: string[]; requires: string[]; route: RouteSpec; endpoint: { service: string; port: number }; origin: string; upstream?: { source?: string; author?: string; version?: string; categories?: string[]; tags?: string[]; website?: string; release_notes?: string; template?: string } | null; settings: AppSetting[]; env: Record<string, string | { generate: string }>; login: LoginSpec; recommended?: { rank: number; reason: string; i18n?: Record<string, I18nText> } | null; host_docker_socket: boolean; notes: string[]; gallery: string[]; readme?: string | null; website?: string | null; tags: string[] }
export interface LoginSpec { mode: 'generated' | 'default' | 'app' | 'none' | 'external'; user?: string | null; password?: string | null; path?: string | null; note?: string | null }
export interface AppSetting { env: string; type: string; label?: string | null; hint?: string | null; required?: boolean; options?: { label: string; value: string }[] | null }
export interface AppView { name: string; manifest: AppManifest; installed: boolean; port?: number | null; url?: string | null; state?: string | null; granted_to: string[]; settings_values: Record<string, string>; links_to: string[]; links_from: string[]; shared_net: boolean; internal_url?: string | null; link_env: Record<string, string>; has_note: boolean }
export interface SystemStatus { edge_host: string; edge_alt_hosts?: string[]; coder_url: string; agents_url?: string; coder_user?: string; coder_password_set?: boolean; components: ComponentHealth[]; version: string; build?: string; setup_done?: boolean; update_available?: boolean; app_updates?: number }
// first-run wizard (setup.rs)
export interface TailscaleInfo { installed: boolean; state: string; auth_url?: string | null; dns_name?: string | null; ips: string[]; tailnet?: string | null; magic_dns: boolean; https_certs: boolean; cert_domains: string[]; health: string[]; version?: string | null; error?: string | null }
export interface EdgeInfo { host: string; alt_hosts: string[]; tls_internal: boolean; matches_tailscale: boolean; reachable: boolean; dns_ok?: boolean | null; cert_ok?: boolean | null; error?: string | null; checked_at: string }
export interface SetupState { done: boolean; tailscale: TailscaleInfo; edge: EdgeInfo; projects: number; apps_installed: number; backups_enabled: boolean; coder_ok: boolean; suggested_host?: string | null; tailscale_admin_url: string; version: string }
export interface SwitchResult { host: string; url: string; alt_hosts: string[]; restart_in_s: number }
// updates (updates.rs)
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
/** The request hung longer than the per-attempt limit (§9.10): non-idempotent requests must not be retried — the server
 *  may have accepted it */
export class TimeoutError extends Error { constructor() { super(t('api.timeout')); } }
/** Default per-attempt limit for GET: on a bad channel the spinner must not hang for minutes */
const GET_TIMEOUT = 20000;

/** timeoutMs — the per-attempt limit (a hung request counts as a network error), retries — how many times to retry a GET (3 by default). */
async function call<T>(method: string, path: string, body?: unknown, opts: { timeoutMs?: number; retries?: number; headers?: Record<string, string> } = {}): Promise<T> {
  let r: Response | undefined;
  const retries = method === 'GET' ? (opts.retries ?? 3) : 0;
  const timeoutMs = opts.timeoutMs ?? (method === 'GET' ? GET_TIMEOUT : undefined);
  // a GET is safe to retry: the browser on the same host drops requests when docker networks change (ERR_NETWORK_CHANGED), the mobile
  // network flickers
  for (let attempt = 0; ; attempt++) {
    const ctl = timeoutMs ? new AbortController() : undefined;
    const timer = ctl ? setTimeout(() => ctl.abort(), timeoutMs) : undefined;
    try {
      r = await fetch(`${CORE}${path}`, { method, headers: { ...(body ? { 'Content-Type': 'application/json' } : {}), ...(opts.headers ?? {}) }, body: body ? JSON.stringify(body) : undefined, cache: 'no-store', signal: ctl?.signal });
      break;
    } catch (e) {
      // a hung non-idempotent request is a separate error (§9.10): it must not be retried, the server may
      // have accepted it
      if (method !== 'GET' && (e as { name?: string })?.name === 'AbortError') throw new TimeoutError();
      // the browser knows there is no network (the installed app offline, §9.8) — do not wait for three retries, "no
      // connection" right away
      if (attempt >= retries || (typeof navigator !== 'undefined' && navigator.onLine === false)) throw new OfflineError();
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
  // presence header: the core pushes to the phone only if there was no visible Desktop tab for a minute and a
  // half
  status: () => call<SystemStatus>('GET', '/status', undefined, { timeoutMs: 6000, retries: 1, headers: { 'X-Desktop-Presence': typeof document !== 'undefined' && document.visibilityState === 'visible' ? 'visible' : 'hidden' } }),  // connectivity poll: short, so the next poll does not wait for a hung one
  coderCredentials: () => call<{ url: string; user: string; password: string }>('GET', '/system/credentials/coder/reveal'),
  repair: (name: string) => call<string[]>('POST', `/apps/${name}/repair`),
  // first-run wizard
  setup: () => call<SetupState>('GET', '/setup'),
  setupLogin: () => call<TailscaleInfo>('POST', '/setup/tailscale/login'),
  setupSwitchHost: (host: string) => call<SwitchResult>('POST', '/setup/edge-host', { host }),
  setupDone: (done: boolean) => call<SetupState>('PUT', '/setup/done', { done }),
  // updates
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
  /** Update package as a file (tar.gz): the request body is the
   *  file itself. */
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
  // Agent Runtime (ADR-006): workspace terminals via cloudos-agentd; the stream is the ptyWsUrl
  // WebSocket
  agentd: (name: string) => call<AgentdHealth>('GET', `/projects/${name}/agentd`),
  ptys: (name: string) => call<PtyInfo[]>('GET', `/projects/${name}/ptys`),
  engines: (name: string, refresh = false) => call<{ engines: Engine[] }>('GET', `/projects/${name}/engines${refresh ? '?refresh=1' : ''}`),
  engineInstall: (name: string, id: string) => call<PtyInfo>('POST', `/projects/${name}/engines/${id}/install`, {}),
  ptyCreate: (name: string, body: { cmd?: string[]; cwd?: string; cols?: number; rows?: number; title?: string; session?: string; engine?: string }) => call<PtyInfo>('POST', `/projects/${name}/ptys`, body),
  // workspace files and git (the Files and Git renderings of the session card) —
  // via agentd
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
  /** Git panel actions: stage | unstage | discard | commit | checkout | fetch | pull | push | worktrees/add | worktrees/remove | worktrees/merge | suggest | init | remote */
  gitOp: (name: string, op: string, body: Record<string, unknown>) => call<any>('POST', `/projects/${name}/git/${op}`, body),
  gitLog: (name: string, cwd: string, n = 30) => call<{ entries: GitCommit[] }>('GET', `/projects/${name}/git/log?cwd=${encodeURIComponent(cwd)}&n=${n}`),
  ptyKill: (name: string, id: string) => call<void>('DELETE', `/projects/${name}/ptys/${id}`),
  ptyWsUrl: (name: string, id: string) => `${location.origin.replace(/^http/, 'ws')}${CORE}/projects/${name}/ptys/${id}/ws`,
  // agent sessions (Chat): the journal over the sessionWsUrl WebSocket, input and
  // answers — HTTP
  sessions: (name: string) => call<SessionInfo[]>('GET', `/projects/${name}/sessions`),
  /** Agent browser (§4.9 step 2): headless Chromium in the workspace, shared by the agent (Playwright MCP) and the human
   *  (screencast) */
  browserStatus: (name: string) => call<BrowserStatus>('GET', `/projects/${name}/browser`),
  browserSet: (name: string, enabled: boolean) => call<BrowserStatus>('POST', `/projects/${name}/browser`, { enabled }, { timeoutMs: 70000 }),
  browserInstall: (name: string) => call<PtyInfo>('POST', `/projects/${name}/browser/install`, {}),
  browserWsUrl: (name: string, target = '') => `${location.origin.replace(/^http/, 'ws')}${CORE}/projects/${name}/browser/ws${target ? `?target=${encodeURIComponent(target)}` : ''}`,
  /** Workspace dev server preview (docs/agents-ux.md §4.9): a port inside the workspace → an edge port route */
  previews: (name: string) => call<{ previews: PreviewRoute[] }>('GET', `/projects/${name}/previews`),
  previewAdd: (name: string, port: number) => call<PreviewRoute>('POST', `/projects/${name}/previews`, { port }),
  previewRemove: (name: string, port: number) => call<void>('DELETE', `/projects/${name}/previews/${port}`),
  sessionCreate: (name: string, body: { agent?: string; cwd?: string; title?: string; resume?: string; model?: string; permission_mode?: string; cache_ttl?: string }) => call<SessionInfo>('POST', `/projects/${name}/sessions`, body),
  sessionGet: (name: string, id: string) => call<{ info: SessionInfo; pending: unknown[]; caps?: Caps }>('GET', `/projects/${name}/sessions/${id}`),
  // composer: model effort, a provider request (context, file suggestions, usage) and the file search for
  // "@"
  sessionEffort: (name: string, id: string, effort: string | null) => call<unknown>('POST', `/projects/${name}/sessions/${id}/effort`, { effort }),
  sessionThinking: (name: string, id: string, tokens: number | null) => call<unknown>('POST', `/projects/${name}/sessions/${id}/thinking`, { tokens }),
  sessionTier: (name: string, id: string, tier: string | null) => call<unknown>('POST', `/projects/${name}/sessions/${id}/tier`, { tier }),
  sessionCache: (name: string, id: string, ttl: string | null) => call<unknown>('POST', `/projects/${name}/sessions/${id}/cache`, { ttl }),
  ptyInput: (name: string, id: string, data: string) => call<void>('POST', `/projects/${name}/ptys/${id}/input`, { data }),
  // Claude sign-in by link and code (PKCE in the core): start → link, finish → the code from the
  // page → account
  oauthStart: (provider: 'claude' | 'codex' | 'claude-oauth' | 'codex-oauth' | 'antigravity', project?: string) => call<{ provider: string; state: string; url: string; hint: string; flow?: string; callback?: string; user_code?: string; interval?: number; expires_in?: number }>('POST', '/accounts/oauth/start', { provider, project }),
  oauthStatus: (state: string) => call<{ status: 'wait' | 'ok' | 'error'; error?: string | null; account?: Account }>('GET', `/accounts/oauth/status?state=${encodeURIComponent(state)}`),
  // providers through the gate: OAuth sign-ins into the llm provider proxy (Antigravity, Kimi, xAI, Codex, Claude, plugins) via the proxy's
  // Management API
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
  /** Events after `since` (up to `upto` if set); `lite` — lightweight tool results and inputs (§4.3.16) */
  sessionEvents: (name: string, id: string, since = 0, upto?: number, lite = false) => call<SessionEvent[]>('GET', `/projects/${name}/sessions/${id}/events?since=${since}&limit=${upto != null ? Math.max(1, Math.min(5000, upto - since)) : 5000}${lite ? '&lite=1' : ''}`),
  /** One whole event by number ("show whole") */
  sessionEvent: (name: string, id: string, seq: number) => call<SessionEvent>('GET', `/projects/${name}/sessions/${id}/events/${seq}`),
  sessionPrompt: (name: string, id: string, content: unknown) => call<SessionEvent>('POST', `/projects/${name}/sessions/${id}/prompt`, { content }, { timeoutMs: 30000 }),
  sessionPermission: (name: string, id: string, body: { request_id: string; behavior: 'allow' | 'deny'; updated_input?: unknown; message?: string; answers?: unknown; remember?: boolean; scope?: 'session' | 'project' | 'user'; permissions?: unknown }) => call<SessionEvent>('POST', `/projects/${name}/sessions/${id}/permission`, body, { timeoutMs: 20000 }),
  sessionFork: (name: string, id: string, body: { at_seq?: number; title?: string } = {}) => call<SessionInfo>('POST', `/projects/${name}/sessions/${id}/fork`, body),
  sessionEdit: (name: string, id: string, body: { seq: number; content: unknown }) => call<SessionEvent>('POST', `/projects/${name}/sessions/${id}/edit`, body),
  sessionQueue: (name: string, id: string, content: unknown) => call<QueuedItem>('POST', `/projects/${name}/sessions/${id}/queue`, { content }, { timeoutMs: 30000 }),
  sessionUnqueue: (name: string, id: string, qid: string) => call<void>('DELETE', `/projects/${name}/sessions/${id}/queue/${qid}`),
  sessionTaskStop: (name: string, id: string, task: string) => call<unknown>('POST', `/projects/${name}/sessions/${id}/tasks/${task}/stop`, {}),
  permissionsGet: (name: string, scope: 'user' | 'project', cwd: string) => call<{ scope: string; path: string; exists: boolean; allow: string[]; deny: string[]; ask: string[]; default_mode?: string | null }>('GET', `/projects/${name}/permissions?scope=${scope}&cwd=${encodeURIComponent(cwd)}`),
  permissionsPut: (name: string, scope: 'user' | 'project', cwd: string, body: { allow: string[]; deny: string[]; ask: string[] }) => call<{ scope: string; path: string; allow: string[]; deny: string[]; ask: string[] }>('PUT', `/projects/${name}/permissions?scope=${scope}&cwd=${encodeURIComponent(cwd)}`, body),
  sessionInterrupt: (name: string, id: string) => call<unknown>('POST', `/projects/${name}/sessions/${id}/interrupt`, {}),
  sessionMode: (name: string, id: string, mode: string) => call<unknown>('POST', `/projects/${name}/sessions/${id}/mode`, { mode }),
  /** Own session name (§4.8); an empty string brings back the auto-title */
  sessionLabel: (name: string, id: string, label: string) => call<{ ok: boolean; label: string | null; title: string }>('POST', `/projects/${name}/sessions/${id}/label`, { label }),
  sessionModel: (name: string, id: string, model: string | null, context_window?: number | null) => call<unknown>('POST', `/projects/${name}/sessions/${id}/model`, { model, context_window: context_window ?? null }),
  sessionDelete: (name: string, id: string) => call<void>('DELETE', `/projects/${name}/sessions/${id}`),
  // terminal mode (as in CloudCLI): the agent's native TUI on the same conversation in the session PTY;
  // the stream — ptyWsUrl
  sessionTui: (name: string, id: string, body: { bypass?: boolean; cols?: number; rows?: number }) => call<PtyInfo>('POST', `/projects/${name}/sessions/${id}/tui`, body),
  sessionTuiStop: (name: string, id: string) => call<void>('DELETE', `/projects/${name}/sessions/${id}/tui`),
  sessionWsUrl: (name: string, id: string, since = 0, lite = false) => `${location.origin.replace(/^http/, 'ws')}${CORE}/projects/${name}/sessions/${id}/ws?since=${since}${lite ? '&lite=1' : ''}`,
  usage: (name: string, since = 0) => call<Record<string, unknown>[]>('GET', `/projects/${name}/usage?since=${since}`),
  usageSummary: (days = 30) => call<UsageSummary>('GET', `/usage?days=${days}`),
  // agent accounts (ADR-006): registry, sign-in import from the workspace, the project default account, session
  // reassignment
  accounts: () => call<{ accounts: Account[]; defaults: Record<string, string>; blocked?: Record<string, string> }>('GET', '/accounts'),
  /** Account edit: name, the auto-switch pool flag (§9.7) */
  accountUpdate: (id: string, body: { label?: string; pool?: boolean }) => call<Account>('PATCH', `/accounts/${id}`, body),
  // subscription limits (§9.2): all accounts in parallel (60 s cache in the core),
  // refresh — anew
  accountLimits: (refresh = false) => call<{ limits: AccountLimits[] }>('GET', `/accounts/limits${refresh ? '?refresh=1' : ''}`, undefined, { timeoutMs: 45000, retries: 0 }),
  accountLimit: (id: string, refresh = false) => call<AccountLimits>('GET', `/accounts/${id}/limits${refresh ? '?refresh=1' : ''}`, undefined, { timeoutMs: 45000, retries: 0 }),
  accountCreate: (body: { provider: string; kind: string; label: string; env?: Record<string, string>; files?: Record<string, string> }) => call<Account>('POST', '/accounts', body),
  accountImport: (body: { provider: string; project: string; label?: string; dir?: string }) => call<Account>('POST', '/accounts/import', body),
  // subscription sign-in through a workspace terminal (claude auth login / codex login --device-auth); after exit — import from
  // the directory
  accountLogin: (body: { provider: 'claude' | 'codex' | 'antigravity'; project: string }) => call<{ pty: string; dir: string; project: string; provider: string }>('POST', '/accounts/login', body),
  /** Antigravity sign-in (§9.11): the core watches the sign-in in the terminal itself — waiting | done (account) | error |
   *  cancelled | timeout */
  accountLoginStatus: (pty: string) => call<{ status: string; account?: string; label?: string; error?: string }>('GET', `/accounts/login/${pty}`),
  ptyText: (name: string, id: string) => call<{ id: string; alive: boolean; text: string }>('GET', `/projects/${name}/ptys/${id}/text`),
  accountDelete: (id: string) => call<void>('DELETE', `/accounts/${id}`),
  projectAccount: (name: string, account: string | null) => call<void>('PUT', `/projects/${name}/account`, { account }),
  sessionAccount: (name: string, id: string, account: string | null) => call<SessionInfo>('POST', `/projects/${name}/sessions/${id}/account`, { account }),
  agentsAll: () => call<{ away: boolean; presence_seen_ms: number; projects: Record<string, AgentsSummary> }>('GET', '/agents'),
  // subscription to core notifications from the phone: ntfy server, topic, user and password (the secret — only on
  // request). The "Agents" canvas layout — in the core, shared across all of the owner's devices. Workspace
  // policy: whether the Claude Code permission bypass mode is allowed (managed-settings, the owner's decision)
  agentsPolicyGet: () => call<{ bypass: boolean }>('GET', '/agents/policy'),
  agentsPolicyPut: (bypass: boolean) => call<{ bypass: boolean; applied: string[] }>('PUT', '/agents/policy', { bypass }),
  agentsLayoutGet: () => call<{ cards?: { project: string; kind: 'pty' | 'session'; id: string; w: number; h: number }[]; cols?: 0 | 1 | 2 | 3 } | null>('GET', '/agents/layout'),
  agentsLayoutPut: (layout: unknown) => call<void>('PUT', '/agents/layout', layout),
  agentsNotify: () => call<{ app: string | null; server: string | null; topic: string; user: string; password: string | null; token: string | null }>('GET', '/agents/notify'),
  applyProject: (spec: ProjectSpec) => call<ProjectView>('POST', '/projects', spec),
  deleteProject: (name: string) => call<void>('DELETE', `/projects/${name}`),
  wsStart: (name: string) => call<ProjectView>('POST', `/projects/${name}/workspace/start`),
  wsStop: (name: string) => call<ProjectView>('POST', `/projects/${name}/workspace/stop`),
  grant: (name: string, capability: string, app?: string) => call<ProjectView>('POST', `/projects/${name}/grants`, app ? { capability, app } : { capability }),  // app — which provider, if there are several
  revoke: (name: string, capability: string) => call<ProjectView>('DELETE', `/projects/${name}/grants/${capability}`),
  apps: () => call<AppView[]>('GET', '/apps'),
  install: (name: string) => call<AppView>('POST', `/apps/${name}/install`),
  app: (name: string) => call<AppView>('GET', `/apps/${name}`),
  /** The install is resilient to a connection drop: during deployment the core creates a docker network, and the browser on
   *  the same host may abort
   * the request (ERR_NETWORK_CHANGED). Then the request is retried (install is idempotent) and the status is awaited
   * by polling the card. */
  /** Stages of an app operation — from the core event feed (deploy started, stack running, health OK, route…). */
  stages: async (name: string, since: string): Promise<string[]> => {
    try { const ev = await call<{ at: string; subject: string; message: string }[]>('GET', '/events'); return ev.filter(e => e.subject === name && e.at > since).map(e => e.message).reverse(); } catch { return []; }
  },
  installWait: async (name: string, onProgress?: (s: string) => void): Promise<AppView> => {
    const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));
    const since = new Date().toISOString();
    // stopped: a stages request may have been in flight at the moment of completion — otherwise a late onProgress marks the
    // card busy again
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
    // the request went out but the answer was lost: wait until
    // the stack comes up
    for (let i = 0; i < 100; i++) {
      await sleep(3000);
      const a = await call<AppView>('GET', `/apps/${name}`).catch(() => null);
      if (a?.installed && a.url && (a.state === 'running' || i > 20)) return a;
      onProgress?.(a?.installed ? t('api.startingState', { state: a.state ?? '…' }) : t('api.installing'));
    }
    throw new Error(t('api.installTimeout'));
  },
  remove: (name: string, purge = false) => call<void>('DELETE', `/apps/${name}${purge ? '?purge=true' : ''}`),
  /** Removal with the same resilience to a connection drop as installWait: retry the request and wait until the app disappears. */
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
  /** MCP server health via `claude mcp list` (slice 70): ok | auth | fail | unknown with the CLI text */
  mcpHealth: (name: string, refresh = false) => call<{ servers: McpHealthRow[] }>('GET', `/projects/${name}/mcp/health${refresh ? '?refresh=1' : ''}`, undefined, { timeoutMs: 160000, retries: 0 }),
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

// ---- the "Agents" service's own views (cloudos-agents) ----
export interface WsProject { name: string; description?: string | null; status: string; apps: { slug: string; url: string; display_name: string }[]; reachable: boolean; engines_v?: number; cost_today: number; sessions: SessionInfo[]; ptys: PtyInfo[]; hidden?: boolean }
export interface Workspaces { projects: WsProject[]; today: UsageTotals }
export interface PendingReq { request_id: string; tool_name: string; input: any; seq: number; kind?: string; suggestions?: any[] | null }
export interface InboxItem { project: string; session: SessionInfo; pending?: PendingReq[] }
export interface Inbox { waiting: InboxItem[]; failed: InboxItem[]; done: InboxItem[]; today: UsageTotals }
export interface ModelPrice { input: number; output: number; cache_read: number; cache_write: number; context?: number; note?: string; source?: string; source_id?: string; synced_at?: number }
export interface PricesSync { at: number; source: string; models: number; updated: number; added: number; missing: string[] }
export interface PricesSyncResult { ok: boolean; at: number; source: string; models: number; updated: string[]; added: string[]; missing: string[] }
export interface ModelRow { id: string; agent: string; turns: number; input: number; output: number; cache_read: number; cache_create: number; cost_usd: number; price?: ModelPrice | null }
export interface Route { id: string; kind: string; name: string; sub?: string; provider?: string; health: string }
export interface Limit { scope: string; per_day: number; action: string }
export interface Models { period_days: number; total: UsageTotals; by_day: UsageBucket[]; by_project: UsageBucket[]; routes: Route[]; models: ModelRow[]; prices_sync?: PricesSync | null; defaults: { claude: string; codex: string }; limits: Limit[]; project_defaults: Record<string, string> }
export interface ChatSearchCfg { kind: 'none' | 'searxng' | 'brave' | 'tavily'; url: string; key: string }
export interface ChatSettings { route: string; model: string; search: ChatSearchCfg }
export interface BrowserTab { id: string; url: string; title: string }
export interface BrowserStatus { installed: boolean; bin?: string | null; running: boolean; own: boolean; port: number; mcp: boolean; pages: BrowserTab[] }
export interface PreviewRoute { project: string; port: number; edge_port: number; url: string; created_at: string }
/** Browser in the session (§4.9): per workspace — the dev server preview (and later the agent browser); off by
 *  default */
export interface BrowserPrefs { preview?: boolean; port?: number; agent?: boolean }
export interface AgentsSettings { prices: Record<string, ModelPrice>; browser?: Record<string, BrowserPrefs>; prices_sync?: PricesSync | null; prices_source?: string; defaults: { claude: string; codex: string }; limits: Limit[]; mail_paused?: boolean; chat?: ChatSettings }
// agents (employees), routines and runs — the service's routines.rs; the scratch
// chat — chat.rs
export interface RunBrief { id: string; status: string; started_at: number; ended_at?: number | null; cost_usd: number; summary?: string | null; reason?: string | null; session?: string | null; project: string }
export interface AgentDef { id: string; name: string; brief: string; provider: string; model?: string | null; account?: string | null; workspace: string; mode: 'ask' | 'auto' | 'bypass'; effort?: string | null; rules?: { allow: string[]; deny: string[] }; memory: string; remember: boolean; place: string; can_mail: boolean; can_routine: boolean; memory_budget: number; reflect: boolean; skills: string[]; reflected_at?: number | null; created_at: number; updated_at: number; routines?: number; last_run?: RunBrief | null }
export interface Trigger { kind: 'schedule' | 'webhook' | 'event' | 'manual'; cron: string; event: string; contains: string }
export interface Routine { id: string; name: string; agent: string; prompt: string; trigger: Trigger; budget: { run_usd: number; day_usd: number }; notify: 'ask_error' | 'always' | 'never'; enabled: boolean; mode?: '' | 'ask' | 'auto' | 'bypass'; token: string; created_at: number; updated_at: number; last_run_at?: number | null; last_status?: string | null; agent_name?: string; workspace?: string; schedule_text?: string; next_at?: number | null; last?: RunBrief | null }
export interface Run extends RunBrief { routine?: string | null; routine_name?: string | null; agent: string; agent_name: string; trigger: string; prompt?: string | null; from_agent?: string | null; reply_to?: string | null; hops?: number }
/** Skill from the service catalog (skills.rs): a repeatable procedure; source — user |
 *  agent:<id>. */
export interface Skill { name: string; description: string; body: string; source: string; created_at: number; updated_at: number; agents?: string[] }
// simple chat (simplechat.rs): conversations directly over the provider API through the core's routes, incognito
// in the service memory
export interface ChatTools { search: boolean; fetch: boolean }
export interface SimpleChat { id: string; kind: 'simple'; title: string; route: string; model: string; system: string; tools: ChatTools; incognito: boolean; created_at: number; updated_at: number; cost_usd: number; tokens_in: number; tokens_out: number; messages: number; expires_at?: number }
export interface ChatAttachment { file: string; name: string; mime: string; size: number; text?: string }
export interface ChatPreset { id: string; name: string; text: string }
export interface ChatToolUse { name: string; args: Record<string, unknown>; result: string; ok: boolean }
export interface ChatMessage { id: string; chat: string; seq: number; role: 'user' | 'assistant'; text: string; attachments: ChatAttachment[]; tools: ChatToolUse[]; tokens_in: number; tokens_out: number; cost_usd: number; created_at: number; error?: string | null }
export interface ChatModel { id: string; name: string; context?: number | null; price_in?: number | null; price_out?: number | null; vision?: boolean | null; tools?: boolean | null; provider?: string; provider_title?: string; vendor?: string | null; kind?: 'chat' | 'image' | 'video' }
/** Models by provider in the order the server returned (the Claude Code and Codex subscriptions first); within — newer
 *  on top. */
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
export interface PushPrefs { permission: boolean; done: boolean; error: boolean; limit: boolean }
export interface PushSub { endpoint: string; host: string; tail: string; ua: string; created_at: number; last_ok: number; fails: number }
export interface PushInfo { public_key: string; subscriptions: PushSub[]; count: number; prefs: PushPrefs }
export interface SharePayload { title: string; text: string; url: string; files: { name: string; type: string; size: number; data: string }[]; at: number }
export interface VoiceProvider { model: string; base: string; default_base: string; has_key: boolean; key_tail: string; tts_model?: string; tts_voice?: string; stream_model?: string }
export interface VoiceSettings { provider: 'elevenlabs' | 'openai'; language: string; ready: boolean; model: string; stream?: boolean; stream_model?: string; stream_models?: { elevenlabs: string[]; openai: string[] }; elevenlabs: VoiceProvider; openai: VoiceProvider; models: { elevenlabs: string[]; openai: string[] }; tts?: { model: string; voice: string; max_chars: number; models: { elevenlabs: string[]; openai: string[] }; openai_voices: string[]; default_voice?: { elevenlabs: string; openai: string } } }
/** Read-aloud provider voice (slice 64): ElevenLabs — from /v1/voices, OpenAI — the built-in
 *  list */
export interface TtsVoice { id: string; name: string; desc?: string; category?: string | null }
export interface VoiceProviderPatch { model?: string; base?: string; key?: string; clear?: boolean; tts_model?: string; tts_voice?: string; stream_model?: string }
export interface VoicePatch { provider?: 'elevenlabs' | 'openai'; language?: string; elevenlabs?: VoiceProviderPatch; openai?: VoiceProviderPatch; stream?: boolean }
export interface VoiceCheck { ok: boolean; provider: string; tier?: string | null; used?: number | null; limit?: number | null; reset_unix?: number | null; models?: number; stt_models?: string[] }
export interface VoiceResult { text: string; language?: string | null; ms: number; provider: string; model: string }
/** Service API: GET — the same three retries and per-attempt limit as for the core (§9.10); the rest — no retries and
 *  no limit
 * (audio upload, read-aloud and the simple chat stream take long by nature) */
async function own<T>(method: string, path: string, body?: unknown): Promise<T> {
  let r: Response | undefined;
  const retries = method === 'GET' ? 3 : 0;
  for (let attempt = 0; ; attempt++) {
    const ctl = method === 'GET' ? new AbortController() : undefined;
    const timer = ctl ? setTimeout(() => ctl.abort(), GET_TIMEOUT) : undefined;
    try {
      r = await fetch(`${OWN}${path}`, { method, headers: body ? { 'Content-Type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined, cache: 'no-store', signal: ctl?.signal });
      break;
    } catch {
      if (attempt >= retries || (typeof navigator !== 'undefined' && navigator.onLine === false)) throw new OfflineError();
      await new Promise(res => setTimeout(res, 1200 * (attempt + 1)));
    } finally { if (timer) clearTimeout(timer); }
  }
  if (!r.ok) { let m = `HTTP ${r.status}`; try { m = (await r.json()).error ?? m; } catch {} throw new Error(m); }
  const t = await r.text(); return (t ? JSON.parse(t) : undefined) as T;
}
export const agents = {
  health: () => own<{ ok: boolean; version: string; cloudd: boolean; cloudd_version?: string | null; hub_subscriptions?: number }>('GET', '/health'),
  workspaces: () => own<Workspaces>('GET', '/workspaces'),
  inbox: () => own<Inbox>('GET', '/inbox'),
  models: (days = 1) => own<Models>('GET', `/models?days=${days}`),
  settings: () => own<AgentsSettings>('GET', '/settings'),
  /** Model price synchronization with the public source (docs/agents-ux.md §9.6) */
  pricesSync: () => own<PricesSyncResult>('POST', '/prices/sync'),
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
  // voice (docs/agents-ux.md §4.7): settings without keys, key check, audio → text
  voice: () => own<VoiceSettings>('GET', '/voice'),
  voicePut: (patch: VoicePatch) => own<VoiceSettings>('PUT', '/voice', patch),
  voiceCheck: (provider?: 'elevenlabs' | 'openai') => own<VoiceCheck>('POST', '/voice/check', { provider }),
  /** Push notifications and share_target
   *  (§9.9) */
  pushInfo: () => own<PushInfo>('GET', '/push'),
  pushSubscribe: (sub: { endpoint: string; keys: { p256dh: string; auth: string }; ua?: string }) => own<{ ok: boolean; count: number }>('POST', '/push/subscribe', sub),
  pushUnsubscribe: (endpoint: string) => own<{ ok: boolean; count: number }>('DELETE', '/push/subscribe', { endpoint }),
  pushPrefs: (p: Partial<PushPrefs>) => own<PushPrefs>('PUT', '/push/prefs', p),
  pushTest: (endpoint?: string) => own<{ sent: number; failed: number; errors: string[] }>('POST', '/push/test', endpoint ? { endpoint } : {}),
  shareTake: (id: string) => own<SharePayload>('GET', `/share/${id}`),
  /** Streaming recognition (§4.7, slice 72): a WebSocket to the service, the provider key stays on the
   *  server */
  voiceStreamUrl: (lang = '') => `${location.origin.replace(/^http/, 'ws')}${OWN}/voice/stream?lang=${encodeURIComponent(lang)}`,
  voiceTranscribe: async (audio: Blob, lang = ''): Promise<VoiceResult> => {
    const r = await fetch(`${OWN}/voice/transcribe${lang ? `?lang=${encodeURIComponent(lang)}` : ''}`, { method: 'POST', headers: { 'Content-Type': audio.type || 'audio/webm' }, body: audio, cache: 'no-store' });
    if (!r.ok) { let m = `HTTP ${r.status}`; try { m = (await r.json()).error ?? m; } catch {} throw new Error(m); }
    return (await r.json()) as VoiceResult;
  },
  voiceVoices: () => own<{ provider: string; voices: TtsVoice[] }>('GET', '/voice/voices'),
  /** Text read-aloud (slice 64): mp3 from the provider; markdown is stripped and the text is cut
   *  on the service */
  voiceSpeak: async (text: string): Promise<Blob> => {
    const r = await fetch(`${OWN}/voice/speak`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ text }), cache: 'no-store' });
    if (!r.ok) { let m = `HTTP ${r.status}`; try { m = (await r.json()).error ?? m; } catch {} throw new Error(m); }
    return await r.blob();
  },
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
  /** Message to the simple chat: an SSE stream (delta, tool, done, error); returns a function that cancels reading (the answer on the server continues — stop via
   *  chatStop). */
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

// ---- conversation search and export (slice 34, docs/agents-ux.md §4.6)
// ----
export interface SearchHit { kind: 'session' | 'chat'; project?: string; id: string; title: string; agent?: string; model?: string; ts: number; seq?: number; mid?: string; role: string; snippet: string; count: number }
export interface SearchResult { q: string; words: string[]; hits: SearchHit[]; stopped: string[]; total: number }
export const downloadUrl = {
  session: (project: string, id: string, fmt: 'md' | 'json') => `/agents/api/core/projects/${project}/sessions/${id}/download?format=${fmt}`,
  chat: (id: string, fmt: 'md' | 'json') => `/agents/api/chats/${id}/export?format=${fmt}`,
  usageCsv: (days: number) => `/agents/api/core/usage/export.csv?days=${days}`,
};
/** File name from the conversation title: letters and digits of any alphabet, the rest — a hyphen;
 *  the date at the end. */
export function fileName(title: string, ext: string): string {
  const base = (title || '').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, '-').replace(/^-+|-+$/g, '').slice(0, 60) || 'export';
  return `${base}-${new Date().toISOString().slice(0, 10)}.${ext}`;
}
/** Download the server answer as a file (blob + <a
 *  download>). */
export async function download(url: string, name: string): Promise<void> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${r.status} ${(await r.text()).slice(0, 200)}`);
  const b = await r.blob(); const a = document.createElement('a'); a.href = URL.createObjectURL(b); a.download = name; document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 5000);
}

// ---- workspace MCP servers (slice 36, docs/agents-ux.md §4.3.10) ----
export interface McpServer { name: string; transport: 'stdio' | 'http' | 'sse'; command: string; args: string[]; url: string; env: Record<string, string>; headers: Record<string, string> }
export interface McpList { servers: McpServer[]; engines: Record<string, number | null> }
export interface McpHealthRow { name: string; transport: string; status: 'ok' | 'auth' | 'fail' | 'unknown'; detail: string }
export interface McpSaveResult extends McpList { applied: Record<string, string> }
