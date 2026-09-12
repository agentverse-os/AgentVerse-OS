// Типы — зеркало utoipa-схем cloudd (/api/openapi.json). Пока вручную; генерация — когда схема стабилизируется.
import { t } from './i18n.svelte';
export type Runtime = 'incus' | 'incus-nesting' | 'incus-vm';
export interface WorkspaceSpec { runtime: Runtime; cpu: number; memory: string | number; home: string | number; image?: string | null }
export interface ProjectSpec { schema: number; project: string; workspace: WorkspaceSpec; agents: string[]; capabilities: string[] }
export interface Grant { project: string; capability: string; app: string; granted_at: string }
export interface ComponentHealth { name: string; status: string; detail?: string | null }
export interface WorkspaceApp { slug: string; display_name: string; url: string; health: string }
export interface WorkspaceView { id: string; name: string; status: string; agent_status?: string | null; instance?: string | null; ip?: string | null; apps: WorkspaceApp[]; url: string }
export interface ProjectView { name: string; spec: ProjectSpec; network: string; subnet: string; gate: ComponentHealth; workspace?: WorkspaceView | null; grants: Grant[]; missing_capabilities: string[]; grant_env: Record<string, string[]>; available_capabilities: string[] }
export interface RouteSpec { mode: 'path' | 'port' | 'host'; prefix?: string | null; port?: number | null; open: 'iframe' | 'newtab' }
/** Переводы текстов манифеста/рекомендации по языку интерфейса (store-utils: descOf, reasonOf). */
export interface I18nText { description?: string | null; reason?: string | null }
export interface AppManifest { name: string; title?: string | null; description?: string | null; i18n?: Record<string, I18nText>; icon?: string | null; type: 'app' | 'system'; provides: string[]; requires: string[]; route: RouteSpec; endpoint: { service: string; port: number }; origin: string; upstream?: { source?: string; author?: string; version?: string; categories?: string[]; tags?: string[]; website?: string; release_notes?: string; template?: string } | null; settings: AppSetting[]; env: Record<string, string | { generate: string }>; login: LoginSpec; recommended?: { rank: number; reason: string; i18n?: Record<string, I18nText> } | null; host_docker_socket: boolean; notes: string[]; gallery: string[]; readme?: string | null; website?: string | null; tags: string[] }
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

export class OfflineError extends Error { constructor() { super(t('api.offline')); } }

/** timeoutMs — предел на одну попытку (зависший запрос считается ошибкой сети), retries — сколько раз повторить GET (по умолчанию 3). */
async function call<T>(method: string, path: string, body?: unknown, opts: { timeoutMs?: number; retries?: number } = {}): Promise<T> {
  let r: Response | undefined;
  const retries = method === 'GET' ? (opts.retries ?? 3) : 0;
  // GET безопасно повторить: браузер на том же хосте рвёт запросы при смене docker-сетей (ERR_NETWORK_CHANGED), мобильная сеть моргает
  for (let attempt = 0; ; attempt++) {
    const ctl = opts.timeoutMs ? new AbortController() : undefined;
    const timer = ctl ? setTimeout(() => ctl.abort(), opts.timeoutMs) : undefined;
    try {
      r = await fetch(`/api${path}`, { method, headers: body ? { 'Content-Type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined, cache: 'no-store', signal: ctl?.signal });
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
  status: () => call<SystemStatus>('GET', '/status', undefined, { timeoutMs: 6000, retries: 1 }),  // опрос связи: коротко, чтобы следующий опрос не ждал зависший
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
    const r = await fetch(`/api/updates/upload?name=${encodeURIComponent(f.name)}`, { method: 'POST', body: f });
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
  applyProject: (spec: ProjectSpec) => call<ProjectView>('POST', '/projects', spec),
  deleteProject: (name: string) => call<void>('DELETE', `/projects/${name}`),
  wsStart: (name: string) => call<ProjectView>('POST', `/projects/${name}/workspace/start`),
  wsStop: (name: string) => call<ProjectView>('POST', `/projects/${name}/workspace/stop`),
  grant: (name: string, capability: string) => call<ProjectView>('POST', `/projects/${name}/grants`, { capability }),
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
  overrides: (name: string) => call<{ env: string; compose: string; effective_compose: string }>('GET', `/apps/${name}/overrides`),
  putOverrides: (name: string, o: { env: string; compose: string }) => call<AppView>('PUT', `/apps/${name}/overrides`, o),
  fileRoots: () => call<{ id: string; title: string; kind: string }[]>('GET', '/files/roots'),
  fileList: (root: string, path: string) => call<{ root: string; path: string; entries: { name: string; dir: boolean; size: number; modified?: string | null; text: boolean }[] }>('GET', `/files/${root}/list?path=${encodeURIComponent(path)}`),
  fileReadUrl: (root: string, path: string) => `/api/files/${root}/read?path=${encodeURIComponent(path)}`,
  fileRead: async (root: string, path: string) => { const r = await fetch(`/api/files/${root}/read?path=${encodeURIComponent(path)}`); if (!r.ok) throw new Error(await r.text()); return r.text(); },
  fileWrite: (root: string, path: string, content: string) => call<void>('PUT', `/files/${root}/write?path=${encodeURIComponent(path)}`, { content }),
  fileUpload: async (root: string, path: string, file: File) => { const r = await fetch(`/api/files/${root}/upload?path=${encodeURIComponent(path)}`, { method: 'POST', body: file }); if (!r.ok) throw new Error(await r.text()); },
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
  readme: async (name: string) => { const r = await fetch(`/api/apps/${name}/readme`); return r.ok ? r.text() : ''; },
  desktopSettings: () => call<any>('GET', '/settings/desktop'),
  putDesktopSettings: (v: unknown) => call<any>('PUT', '/settings/desktop', v),
};

export interface BackupConfig { enabled: boolean; hourly: number; daily: number; weekly: number; restic: boolean; restic_keep_daily: number; restic_keep_weekly: number; restic_keep_monthly: number; remote: string }
export interface DatasetInfo { name: string; mountpoint: string; used: number; snapshots: number; snapshots_used: number; last: number | null }
export interface BackupStatus { config: BackupConfig; datasets: DatasetInfo[]; restic_installed: boolean; restic_repo: string; restic_snapshots: number; restic_size: number; last: Record<string, any>; running: boolean }
export interface RestorePoint { snapshot: string; kind: string; created: number; path: string }
