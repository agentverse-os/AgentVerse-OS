<script module lang="ts">
  type McpDraft = { name: string; transport: 'stdio' | 'http' | 'sse'; command: string; url: string; env: string; headers: string };
  type RulesDraft = { allow: string; deny: string; ask: string };
  const mcpDrafts = new Map<string, McpDraft>();
  const rulesDrafts = new Map<string, RulesDraft>();
</script>

<script lang="ts">
  // Settings: agent accounts (the core), routes and defaults (the service), notifications (ntfy through the core), default
  // permissions, appearance (the browser), voice (dictation, §4.7), about.
  import { ui, data, actions, MODELS_FOR, MODES_FOR, modeLabel, hardReload } from '../state.svelte';
  import { byProvider, api, agents, type AgentsSettings, type GateAuth, type GateTarget, type GateLogin, type GateStart , type ChatSettings, type McpServer, type McpList, type AccountLimits, type PushInfo, type PushPrefs, type LimitWindow, type TtsVoice, type McpHealthRow } from '../api';
  import { menu } from '../menu.svelte';
  import { toast } from '../toast.svelte';
  import { t, fmt, locale } from '../i18n';
  import { untrack, onDestroy } from 'svelte';
  import { voice } from '../voice.svelte';
  import { cacheCount } from '../chatCache';
  import Mic from '../Mic.svelte';
  import Appearance from '../Appearance.svelte';
  // voice (docs/agents-ux.md §4.7): the dictation form — provider, model, language, keys (new values are entered anew, only the tail comes from the
  // server)
  type VProv = 'elevenlabs' | 'openai';
  const VOICE_LANGS: [string, string][] = [['ru', 'Русский'], ['en', 'English'], ['uk', 'Українська'], ['es', 'Español'], ['de', 'Deutsch'], ['fr', 'Français'], ['it', 'Italiano'], ['pt', 'Português'], ['pl', 'Polski'], ['tr', 'Türkçe'], ['kk', 'Қазақша'], ['ja', '日本語'], ['zh', '中文']];
  const MODEL_HINT: Record<string, string> = { scribe_v2: 'voice.modelScribeV2', scribe_v1: 'voice.modelScribeV1', 'gpt-4o-mini-transcribe': 'voice.modelGpt4oMini', 'gpt-4o-transcribe': 'voice.modelGpt4o', 'gpt-4o-transcribe-diarize': 'voice.modelGpt4oDiarize', 'whisper-1': 'voice.modelWhisper' };
  let vProvider = $state<VProv>('elevenlabs');
  let vModelSel = $state<Record<VProv, string>>({ elevenlabs: '', openai: '' });
  let vModelCustom = $state<Record<VProv, string>>({ elevenlabs: '', openai: '' });
  let vBase = $state<Record<VProv, string>>({ elevenlabs: '', openai: '' });
  let vKeys = $state<Record<VProv, string>>({ elevenlabs: '', openai: '' });
  let vClear = $state<Record<VProv, boolean>>({ elevenlabs: false, openai: false });
  let vLangSel = $state(''); let vLangCustom = $state('');
  let vSaving = $state(false); let vChecking = $state(false); let vCheck = $state<{ text: string; ok: boolean } | null>(null);
  let vSample = $state(''); let vSampleMs = $state(0);
  // read-aloud (slice 64): model and voice per provider, the ElevenLabs voice list on request, a
  // test
  let vTtsModel = $state<Record<VProv, string>>({ elevenlabs: '', openai: '' });
  let vTtsVoice = $state<Record<VProv, string>>({ elevenlabs: '', openai: '' });
  let vVoices = $state<TtsVoice[]>([]); let vVoicesBusy = $state(false); let vVoicesErr = $state('');
  let vSpeakBusy = $state(false);
  const vTtsModels = $derived(voice.settings?.tts?.models[vProvider] ?? []);
  // text as you speak (slice 72): the toggle and the stream model per
  // provider
  let vStream = $state(false);
  let vStreamModel = $state<Record<VProv, string>>({ elevenlabs: '', openai: '' });
  const vStreamModels = $derived(voice.settings?.stream_models?.[vProvider] ?? []);
  async function loadVoices() {
    if (vVoicesBusy) return; vVoicesBusy = true; vVoicesErr = '';
    try { if (!(await saveVoice())) return; const r = await agents.voiceVoices(); vVoices = r.voices; } catch (e) { vVoicesErr = msg(e); } finally { vVoicesBusy = false; }
  }
  async function trySpeak() {
    if (vSpeakBusy) return; vSpeakBusy = true;
    try { if (!(await saveVoice())) return; await voice.speak('settings-sample', t('voice.ttsSample')); } catch (e) { toast(msg(e), 'bad'); } finally { vSpeakBusy = false; }
  }
  let vSeen: unknown = null;
  $effect(() => { if (ui.settingsTab === 'voice') untrack(() => voice.load(true)); });
  // conversation cache on the device (§4.3.16): the number of
  // entries in "About"
  let chatCacheN = $state(0);
  // the address of "Agents" as a separate app (§9.8): on :443 — an install link, on its own port — a note
  let agentsUrl = $state('');
  // storage (§9.9): used/quota and whether it is
  // persistent
  let storageUse = $state<{ used: number; quota: number } | null>(null);
  async function loadStorage() { try { const e = await navigator.storage?.estimate?.(); if (e) storageUse = { used: e.usage ?? 0, quota: e.quota ?? 0 }; if (navigator.storage?.persisted) data.storagePersisted = await navigator.storage.persisted(); } catch {} }
  const fmtMb = (b: number) => b >= 1024 * 1024 * 1024 ? `${(b / 1024 / 1024 / 1024).toFixed(1)} ГБ` : `${Math.max(1, Math.round(b / 1024 / 1024))} МБ`;
  const onSeparate = $derived.by(() => { try { return !!agentsUrl && new URL(agentsUrl).port === location.port; } catch { return false; } });
  $effect(() => { if (ui.settingsTab === 'about') untrack(() => { cacheCount().then((n) => (chatCacheN = n)).catch(() => {}); api.status().then((st) => { agentsUrl = st.agents_url ?? ''; }).catch(() => {}); loadStorage(); }); });
  async function copyInstallUrl() { try { await navigator.clipboard.writeText(agentsUrl); toast(t('set.installCopied'), 'info'); } catch (e) { toast(msg(e), 'bad'); } }
  // push notifications to this device (§9.9): browser subscription ↔ server, event kinds, the device list, a test
  let pushInfo = $state<PushInfo | null>(null); let pushBusy = $state(false); let pushErr = $state(''); let pushMine = $state<string | null>(null);
  const pushSupported = typeof window !== 'undefined' && 'PushManager' in window && 'serviceWorker' in navigator && 'Notification' in window;
  let pushPerm = $state<string>(typeof Notification !== 'undefined' ? Notification.permission : 'unsupported');
  async function loadPush() {
    pushErr = '';
    try { pushInfo = await agents.pushInfo(); } catch (e) { pushErr = msg(e); }
    try { const reg = await navigator.serviceWorker?.ready; const sub = await reg?.pushManager.getSubscription(); pushMine = sub?.endpoint ?? null; } catch { pushMine = null; }
    pushPerm = typeof Notification !== 'undefined' ? Notification.permission : 'unsupported';
  }
  $effect(() => { if (ui.settingsTab === 'notify') untrack(() => { loadPush(); }); });
  function b64ToBytes(b64: string): Uint8Array { const pad = '='.repeat((4 - (b64.length % 4)) % 4); const raw = atob((b64 + pad).replace(/-/g, '+').replace(/_/g, '/')); return Uint8Array.from(raw, c => c.charCodeAt(0)); }
  async function enablePush() {
    if (pushBusy) return; pushBusy = true; pushErr = '';
    try {
      const perm = await Notification.requestPermission(); pushPerm = perm;
      if (perm !== 'granted') { pushErr = t('push.denied'); return; }
      const info = pushInfo ?? (await agents.pushInfo()); pushInfo = info;
      const reg = await navigator.serviceWorker.ready;
      const sub = (await reg.pushManager.getSubscription()) ?? (await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: b64ToBytes(info.public_key) as BufferSource }));
      const j = sub.toJSON(); await agents.pushSubscribe({ endpoint: sub.endpoint, keys: { p256dh: j.keys?.p256dh ?? '', auth: j.keys?.auth ?? '' }, ua: navigator.userAgent.replace(/\(.*?\)/g, '').replace(/\s+/g, ' ').trim().slice(0, 100) });
      toast(t('push.enabled'), 'info'); await loadPush();
    } catch (e) { pushErr = msg(e); } finally { pushBusy = false; }
  }
  async function disablePush() {
    if (pushBusy) return; pushBusy = true; pushErr = '';
    try { const reg = await navigator.serviceWorker.ready; const sub = await reg.pushManager.getSubscription(); if (sub) { await agents.pushUnsubscribe(sub.endpoint).catch(() => {}); await sub.unsubscribe(); } toast(t('push.disabled'), 'info'); await loadPush(); }
    catch (e) { pushErr = msg(e); } finally { pushBusy = false; }
  }
  async function testPush() {
    if (pushBusy) return; pushBusy = true; pushErr = '';
    try { const r = await agents.pushTest(pushMine ?? undefined); toast(r.sent ? t('push.testSent', { n: r.sent }) : (r.errors[0] ?? t('push.testNone')), r.sent ? 'info' : 'bad'); await loadPush(); }
    catch (e) { pushErr = msg(e); } finally { pushBusy = false; }
  }
  async function setPushPref(k: keyof PushPrefs, v: boolean) { try { const p = await agents.pushPrefs({ [k]: v }); if (pushInfo) pushInfo = { ...pushInfo, prefs: p }; } catch (e) { toast(msg(e), 'bad'); } }
  async function dropPushSub(endpoint: string) { try { await agents.pushUnsubscribe(endpoint); if (endpoint === pushMine) { try { const reg = await navigator.serviceWorker.ready; const sub = await reg.pushManager.getSubscription(); if (sub && sub.endpoint === endpoint) await sub.unsubscribe(); } catch {} } await loadPush(); } catch (e) { toast(msg(e), 'bad'); } }
  // the form is filled from the server settings when they appear/refresh (after saving — fresh key tails)
  $effect(() => {
    const v = voice.settings; if (!v || v === vSeen) return;
    untrack(() => {
      vSeen = v; vProvider = v.provider;
      for (const pr of ['elevenlabs', 'openai'] as VProv[]) {
        const m = v[pr].model; const known = v.models[pr].includes(m) || m === '';
        vModelSel[pr] = known ? (m || v.models[pr][0] || '') : '__custom'; vModelCustom[pr] = known ? '' : m; vBase[pr] = v[pr].base; vKeys[pr] = ''; vClear[pr] = false;
        vTtsModel[pr] = v[pr].tts_model ?? ''; vTtsVoice[pr] = v[pr].tts_voice ?? ''; vStreamModel[pr] = v[pr].stream_model ?? '';
      }
      vStream = !!v.stream;
      const known = v.language === '' || VOICE_LANGS.some(([c]) => c === v.language);
      vLangSel = known ? v.language : '__other'; vLangCustom = known ? '' : v.language;
    });
  });
  const vModels = $derived(voice.settings?.models[vProvider] ?? []);
  const vHasKey = $derived(!!voice.settings?.[vProvider].has_key && !vClear[vProvider] || !!vKeys[vProvider].trim());
  function voicePatch() {
    const prov = (pr: VProv) => ({ model: vModelSel[pr] === '__custom' ? vModelCustom[pr].trim() : vModelSel[pr], base: vBase[pr].trim(), key: vKeys[pr].trim() || undefined, clear: vClear[pr], tts_model: vTtsModel[pr].trim(), tts_voice: vTtsVoice[pr].trim(), stream_model: vStreamModel[pr].trim() });
    return { provider: vProvider, language: vLangSel === '__other' ? vLangCustom.trim().toLowerCase() : vLangSel, elevenlabs: prov('elevenlabs'), openai: prov('openai'), stream: vStream };
  }
  async function saveVoice(): Promise<boolean> {
    if (vSaving) return false; vSaving = true;
    try { await voice.save(voicePatch()); toast(t('toast.saved')); return true; } catch (e) { toast(msg(e), 'bad'); return false; } finally { vSaving = false; }
  }
  /** "Check key" first saves the form (the entered key goes to the server), then asks the provider */
  async function checkVoice() {
    if (vChecking) return; vChecking = true; vCheck = null;
    try {
      if (!(await saveVoice())) return;
      const r = await agents.voiceCheck(vProvider);
      const text = r.provider === 'openai' ? t('voice.checkOkOa', { n: r.models ?? 0 }) + (r.stt_models?.length ? ' · ' + t('voice.sttModels', { list: r.stt_models.join(', ') }) : '') : t('voice.checkOk11', { tier: r.tier ?? '—', used: (r.used ?? 0).toLocaleString(locale()), limit: (r.limit ?? 0).toLocaleString(locale()) });
      vCheck = { text, ok: true };
    } catch (e) { vCheck = { text: msg(e), ok: false }; } finally { vChecking = false; }
  }
  // subscription limits (docs/agents-ux.md §9.2): the provider windows under the account row; the core caches for
  // 60 s, ⟳ — anew
  let limits = $state<Record<string, AccountLimits>>({});
  let limitsBusy = $state(false);
  async function loadLimits(refresh = false) {
    limitsBusy = true;
    try { const r = await api.accountLimits(refresh); limits = Object.fromEntries(r.limits.map((l) => [l.account, l])); } catch (e) { toast(e instanceof Error ? e.message : String(e), 'bad'); } finally { limitsBusy = false; }
  }
  $effect(() => { if (ui.settingsTab === 'accounts' && data.accounts.some((a) => a.kind === 'subscription')) loadLimits(); });
  const winLabel = (w: LimitWindow) => w.id === 'five_hour' ? t('lim.fiveHour') : w.id === 'seven_day' ? t('lim.sevenDay') : w.id === 'seven_day_opus' ? t('lim.sevenDayOpus') : w.id === 'seven_day_sonnet' ? t('lim.sevenDaySonnet') : w.window_mins === 300 ? t('lim.fiveHour') : w.window_mins === 10080 ? t('lim.sevenDay') : w.window_mins ? t('lim.hours', { n: Math.round(w.window_mins / 60) }) : w.id;
  /** "reset in 1 h 20 min" / "reset Thu 09:00" from the core's RFC 3339 */
  function until(iso?: string | null): string {
    if (!iso) return ''; const ms = new Date(iso).getTime() - Date.now(); if (!isFinite(ms)) return '';
    if (ms <= 0) return t('lim.resetNow'); const m = Math.round(ms / 60000);
    if (m < 60) return t('lim.resetIn', { t: t('time.min', { n: m }) });
    if (m < 24 * 60) return t('lim.resetIn', { t: m % 60 ? `${t('time.h', { n: Math.floor(m / 60) })} ${t('time.min', { n: m % 60 })}` : t('time.h', { n: Math.floor(m / 60) }) });
    const d = new Date(iso); return t('lim.resetAt', { when: `${fmt.date(d, { weekday: 'short' })} ${fmt.time(d)}` });
  }
  const lvl = (p: number) => (p >= 95 ? 'bad' : p >= 80 ? 'warn' : '');
  // account cards (§9.3): an avatar per provider, kind, limit refresh for one account, re-sign-in on a revoked token
  const avClass = (p: string) => (p === 'codex' || p === 'openai' ? 'codex' : p === 'claude' || p === 'anthropic' ? 'claude' : p === 'antigravity' ? 'antigravity' : 'shell');
  const avIcon = (p: string) => (p === 'codex' || p === 'openai' ? 'codex' : p === 'claude' || p === 'anthropic' ? 'claude' : p === 'antigravity' ? 'antigravity' : 'key');
  const kindLabel = (k: string) => (k === 'subscription' ? t('set.kindSubscription') : k === 'proxy' ? t('set.kindProxy') : t('set.kindKey'));
  async function loadLimit(id: string) {
    limitsBusy = true;
    try { const l = await api.accountLimit(id, true); limits = { ...limits, [id]: l }; } catch (e) { toast(e instanceof Error ? e.message : String(e), 'bad'); } finally { limitsBusy = false; }
  }
  function relogin(provider: string) {
    login = { ...login, provider: provider === 'codex' || provider === 'openai' ? 'codex' : 'claude', way: 'link', url: undefined, pasted: '', error: undefined, imported: false, done: false };
    form = 'login'; document.querySelector('.content')?.scrollTo({ top: 0, behavior: 'smooth' });
  }
  const updatedLabel = (iso: string) => { const ago = fmt.ago(Math.floor(new Date(iso).getTime() / 1000)); return ago === t('time.now') ? t('lim.updatedNow') : t('lim.updatedAgo', { ago }); };
  import Icon from '../Icon.svelte';
  import Term from '../Term.svelte';
  const TABS = $derived<[string, string, string][]>([['accounts', 'key', t('set.accounts')], ['routes', 'globe', t('set.routes')], ['chat', 'zap', t('set.chat')], ['notify', 'bell', t('set.notify')], ['perm', 'shield', t('set.perm')], ['mcp', 'cpu', t('set.mcp')], ['look', 'sun', t('set.look')], ['voice', 'mic', t('set.voice')], ['browser', 'globe', t('browser.title')], ['about', 'info', t('set.about')]] as const);
  let settings = $state<AgentsSettings | null>(null);
  let settingsLoading = $state(false); let settingsError = $state('');
  let notify = $state<{ app: string | null; server: string | null; topic: string; user: string; password: string | null } | null>(null);
  let form = $state<'none' | 'import' | 'add' | 'login'>('none');
  // OAuth sign-in: a terminal in the workspace with a separate configuration directory; the link is opened on your device, the code is
  // pasted into the terminal
  let login = $state<{ provider: 'claude' | 'codex' | 'antigravity'; project: string; way: 'link' | 'code' | 'term'; callback?: string; pty?: string; dir?: string; done?: boolean; code?: number | null; imported?: boolean; busy?: boolean; error?: string; url?: string; state?: string; pasted: string; paste: string; agyUrl?: string; agyStatus?: string }>({ provider: 'claude', project: '', way: 'link', pasted: '', paste: '' });
  // Antigravity (§9.11): the sign-in is Google OAuth in agy itself; the link is cut out of the terminal text (the TUI wraps it
  // across lines), the core detects the sign-in (agy models under the sign-in directory) and imports the account itself — we poll
  // the status
  let agyPoll: ReturnType<typeof setInterval> | undefined;
  function stopAgyPoll() { if (agyPoll) clearInterval(agyPoll); agyPoll = undefined; }
  $effect(() => { if (login.provider === 'antigravity' && login.way === 'code') login.way = 'link'; });
  function startAgyPoll() {
    stopAgyPoll(); let tick = 0;
    agyPoll = setInterval(async () => {
      const pty = login.pty; if (!pty) return;
      tick++;
      if (!login.agyUrl) { try { const r = await api.ptyText(login.project, pty); const u = agyLinkFrom(r.text); if (u) login = { ...login, agyUrl: u }; } catch {} }
      if (tick % 2 === 0 || login.agyUrl) { try { const st = await api.accountLoginStatus(pty); login = { ...login, agyStatus: st.status };
        if (st.status === 'done') { stopAgyPoll(); login = { ...login, imported: true, done: true, error: '' }; await data.loadAccounts(); toast(t('set.loginImported')); }
        else if (st.status === 'error') { stopAgyPoll(); login = { ...login, error: st.error ?? 'error' }; }
        else if (st.status === 'timeout' || st.status === 'cancelled') { stopAgyPoll(); login = { ...login, done: true, error: st.status === 'timeout' ? t('set.loginAgyTimeout') : '' }; }
      } catch {} }
    }, 3000);
  }
  /** Google link from the terminal text: the TUI draws it wrapped — join up to the first empty line or a hint */
  function agyLinkFrom(text: string): string | undefined {
    const i = text.lastIndexOf('https://accounts.google.com/');
    if (i < 0) return undefined;
    const tail = text.slice(i); const m = tail.match(/^[\s\S]*?(?=\n\s*\n|Copy and paste|If you aren|→|$)/);
    const raw = (m ? m[0] : tail).replace(/\s+/g, '');
    return raw.includes('redirect_uri') ? raw : undefined;
  }
  // sign-in by link: Claude — a link with PKCE and the code from the page (code#state); ChatGPT (Codex) — a one-time
  // device code, the core polls the confirmation itself (device-code, like `codex login --device-auth`), no terminal or
  // workspace needed
  let codexPoll: ReturnType<typeof setInterval> | undefined;
  let userCode = $state(''); let codeUntil = $state(0); let accountLabel = $state('');
  function stopCodexPoll() { if (codexPoll) clearInterval(codexPoll); codexPoll = undefined; }
  async function getLink() {
    login = { ...login, busy: true, error: '' }; userCode = ''; accountLabel = '';
    try {
      // "by link" — an OAuth link as in the proxy panel (sign in in the browser, then the localhost:<port>/… address from
      // the address bar); "by code" — the Codex device-code / the Claude page code
      if (login.provider === 'antigravity' && login.way === 'term') { await startLogin(); return; }
      // Antigravity by link (§9.11): agy's own link from the core's hidden terminal; the code from the Google page goes
      // into the field below
      const project = login.project || data.running[0]?.name;
      const prov = login.way === 'link' ? (login.provider === 'codex' ? 'codex-oauth' : login.provider === 'antigravity' ? 'antigravity' : 'claude-oauth') : login.provider;
      const r = await api.oauthStart(prov, login.provider === 'antigravity' ? project : undefined); login = { ...login, url: r.url, state: r.state, callback: r.callback ?? '', busy: false };
      if (login.provider === 'codex' && login.way === 'code') {
        userCode = r.user_code ?? ''; codeUntil = Date.now() + (r.expires_in ?? 900) * 1000; stopCodexPoll();
        const st = r.state; codexPoll = setInterval(async () => {
          try {
            const p = await api.oauthStatus(st);
            if (p.status === 'ok') { stopCodexPoll(); accountLabel = p.account?.label ?? ''; login = { ...login, imported: true }; await data.loadAccounts(); toast(t('set.loginImported')); }
            else if (p.status === 'error') { stopCodexPoll(); login = { ...login, error: p.error ?? t('set.loginFailed') }; }
          } catch {}
        }, Math.max(3, r.interval ?? 5) * 1000);
      }
    } catch (e) { login = { ...login, busy: false, error: e instanceof Error ? e.message : String(e) }; }
  }
  async function copyCode() { try { await navigator.clipboard.writeText(userCode); toast(t('set.loginCopied'), 'info'); } catch {} }
  const minutesLeft = () => Math.max(0, Math.round((codeUntil - Date.now()) / 60000));
  async function finishLink() {
    const st = login.state, code = login.pasted.trim(); if (!st || !code) return;
    login = { ...login, busy: true, error: '' };
    try { await api.oauthFinish({ state: st, code }); login = { ...login, imported: true, busy: false }; await data.loadAccounts(); toast(t('set.loginImported')); }
    catch (e) { login = { ...login, busy: false, error: e instanceof Error ? e.message : String(e) }; }
  }
  async function copyLink() { try { await navigator.clipboard.writeText(login.url ?? ''); toast(t('set.loginCopied'), 'info'); } catch {} }
  // pasting the code into the sign-in terminal (pasting into xterm on the phone is unreliable):
  // the line + Enter into the PTY
  async function pasteToTerm() { if (!login.pty || !login.paste.trim()) return; try { await api.ptyInput(login.project, login.pty, login.paste.trim() + '\r'); login = { ...login, paste: '' }; } catch (e) { login = { ...login, error: e instanceof Error ? e.message : String(e) }; } }
  async function startLogin() {
    const project = login.project || data.running[0]?.name; if (!project) { toast(t('set.loginNoWs'), 'bad'); return; }
    login = { ...login, project, busy: true, error: '', done: false, imported: false };
    try { const r = await api.accountLogin({ provider: login.provider, project }); login = { ...login, pty: r.pty, dir: r.dir, busy: false, agyUrl: undefined, agyStatus: login.provider === 'antigravity' ? 'waiting' : undefined }; if (login.provider === 'antigravity') startAgyPoll(); } catch (e) { login = { ...login, busy: false, error: e instanceof Error ? e.message : String(e) }; }
  }
  async function finishLogin(code: number) {
    login = { ...login, done: true, code };
    // Antigravity: the core imports on the fact of sign-in (it also kills the terminal) — no manual
    // import
    if (code === 0 && login.provider !== 'antigravity') await importLogin();
  }
  async function importLogin() {
    if (!login.dir) return;
    try { await api.accountImport({ provider: login.provider, project: login.project, dir: login.dir }); login = { ...login, imported: true, error: '' }; await data.loadAccounts(); toast(t('set.loginImported')); }
    catch (e) { login = { ...login, error: e instanceof Error ? e.message : String(e) }; }
  }
  async function closeLogin() { stopCodexPoll(); stopAgyPoll(); if (login.pty && !login.done) { try { await api.ptyKill(login.project, login.pty); } catch {} } login = { provider: login.provider, project: login.project, way: login.way, pasted: '', paste: '' }; userCode = ''; form = 'none'; }
  // providers through the gate (docs/agents-ux.md §9.1): sign-ins into the llm provider proxy via its Management API — device-code (link
  // + code) or callback (a link, then the localhost:<port>/… address from the address bar is pasted back)
  let gate = $state<GateAuth | null>(null); let gateErr = $state('');
  let gl = $state<{ app: string; provider: string; title: string; url: string; state: string; flow: 'device' | 'callback' | 'code'; userCode: string; callback: string; paste: string; busy: boolean; error: string; done: boolean; sent: boolean } | null>(null);
  let gatePoll: ReturnType<typeof setInterval> | undefined;
  function stopGatePoll() { if (gatePoll) clearInterval(gatePoll); gatePoll = undefined; }
  async function loadGate() { try { gate = await api.gateAuth(); gateErr = ''; } catch (e) { gateErr = msg(e); } }
  $effect(() => { if (ui.settingsTab === 'accounts' && !gate) loadGate(); });
  function gateLoginMenu(el: HTMLElement, tg: GateTarget) {
    const plug = tg.plugins.filter(p => p.oauth && p.enabled && p.provider);
    menu.show(el, [
      { header: true, label: t('set.gateLoginHdr') },
      ...tg.providers.filter(p => !p.since).map(p => ({ icon: p.core ? 'key' : 'ext', label: p.title, hint: p.core ? t('set.gateCoreHint') : p.models, on: () => gateStart(tg, p.id, p.title) })),
      ...(plug.length ? [{ sep: true }, ...plug.map(p => ({ icon: 'key', label: `${p.name ?? p.id} · ${t('set.gatePlugin')}`, on: () => gateStart(tg, p.provider!, p.name ?? p.id) }))] : []),
      { sep: true },
      ...tg.providers.filter(p => p.since).map(p => ({ icon: 'key', label: `${p.title} · ${p.since}+`, hint: p.models, on: () => gateStart(tg, p.id, p.title) })),
    ]);
  }
  function gatePluginsMenu(el: HTMLElement, tg: GateTarget) {
    const items = tg.store.length ? tg.store.map(p => ({ icon: p.installed ? 'check' : 'download', label: `${p.name ?? p.id}${p.version ? ' · ' + p.version : ''}`, hint: p.installed ? t('set.gateInstalled') : p.description ?? undefined, disabled: p.installed, on: () => gateInstall(tg, p.id) })) : [{ label: t('set.gateStoreEmpty'), disabled: true }];
    menu.show(el, [{ header: true, label: t('set.gatePluginsHdr') }, ...items, { sep: true }, { label: t('set.gateQwenNote'), disabled: true }]);
  }
  async function gateStart(tg: GateTarget, provider: string, title: string) {
    stopGatePoll(); gl = { app: tg.app, provider, title, url: '', state: '', flow: 'callback', userCode: '', callback: '', paste: '', busy: true, error: '', done: false, sent: false };
    try {
      const r: GateStart = await api.gateAuthStart(tg.app, provider);
      gl = { ...gl!, url: r.url, state: r.state, flow: r.flow, userCode: r.user_code ?? '', callback: r.callback ?? '', busy: false };
      const app = tg.app, st = r.state;
      gatePoll = setInterval(async () => {
        try {
          const p = await api.gateAuthStatus(app, st);
          if (p.status === 'ok') { stopGatePoll(); gl = gl ? { ...gl, done: true } : gl; toast(t('set.gateDone')); await loadGate(); }
          else if (p.status === 'error') { stopGatePoll(); gl = gl ? { ...gl, error: p.error ?? t('set.loginFailed') } : gl; }
        } catch {}
      }, 4000);
    } catch (e) { gl = { ...gl!, busy: false, error: msg(e) }; }
  }
  async function gateCallback() {
    if (!gl || !gl.paste.trim()) return; gl = { ...gl, busy: true, error: '' };
    try { const r = await api.gateAuthCallback(gl.app, gl.state, gl.paste.trim()); if (r.done) { stopGatePoll(); gl = { ...gl, busy: false, sent: true, paste: '', done: true }; toast(t('set.gateDone')); await loadGate(); } else gl = { ...gl, busy: false, sent: true, paste: '' }; } catch (e) { gl = { ...gl, busy: false, error: msg(e) }; }
  }
  function gateCancel() { stopGatePoll(); gl = null; }
  async function gateDelete(tg: GateTarget, l: GateLogin) { if (!confirm(t('set.gateDeleteConfirm', { name: l.name }))) return; try { await api.gateLoginDelete(tg.app, l.name); toast(t('set.gateDeleted')); await loadGate(); } catch (e) { toast(msg(e), 'bad'); } }
  async function gateToggle(tg: GateTarget, l: GateLogin) { try { await api.gateLoginDisabled(tg.app, l.name, !l.disabled); await loadGate(); } catch (e) { toast(msg(e), 'bad'); } }
  async function gateInstall(tg: GateTarget, id: string) { if (!confirm(t('set.gateInstallConfirm', { id }))) return; try { toast(t('set.gateInstalling', { id }), 'info'); await api.gatePluginInstall(tg.app, id); toast(t('set.gateInstalledToast', { id })); await loadGate(); } catch (e) { toast(msg(e), 'bad'); } }
  let imp = $state({ provider: 'claude', project: '', label: '' });
  let add = $state({ provider: 'openai', kind: 'api_key', label: '', env: '' });
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  async function loadSettings() { if (settingsLoading) return; settingsLoading = true; settingsError = ''; try { settings = await agents.settings(); } catch (e) { settingsError = msg(e); } finally { settingsLoading = false; } }
  $effect(() => { untrack(loadSettings); });
  let chatCfg = $state<ChatSettings>({ route: '', model: '', search: { kind: 'none', url: '', key: '' } });
  let chatSaving = $state(false);
  $effect(() => { const c = settings?.chat; if (c) chatCfg = { route: c.route ?? '', model: c.model ?? '', search: { kind: c.search?.kind ?? 'none', url: c.search?.url ?? '', key: c.search?.key ?? '' } }; });
  $effect(() => { if (ui.settingsTab === 'chat' && !data.chatRoutes && !data.chatRoutesLoading && !data.chatRoutesError) untrack(() => data.loadChatRoutes()); });
  const chatRouteGroups = $derived(byProvider(data.chatRoutes?.routes.find(r => r.id === chatCfg.route)?.models ?? []));
  async function save(patch: Partial<AgentsSettings> | Record<string, unknown>) { try { await agents.saveSettings(patch as any); settings = await agents.settings(); toast(t('toast.saved')); } catch (e) { toast(msg(e), 'bad'); } }
  // browser in the session (§4.9): per-workspace toggles, the same setting as the "⋯" menu item
  let browserBusy = $state('');
  $effect(() => { if (ui.settingsTab === 'browser') untrack(() => { data.loadBrowser(true); if (!data.ws) data.loadWs(); }); });
  async function toggleBrowserPreview(project: string) {
    if (browserBusy) return; browserBusy = project;
    const on = !data.browserOf(project).preview;
    try { await data.setBrowser(project, { preview: on }); if (!on) { const prev = await api.previews(project).catch(() => ({ previews: [] })); for (const r of prev.previews) await api.previewRemove(project, r.port).catch(() => {}); } toast(t(on ? 'browser.enabledToast' : 'browser.disabledToast', { project })); }
    catch (e) { toast(msg(e), 'bad'); } finally { browserBusy = ''; }
  }
  async function toggleAgentBrowser(project: string) {
    if (browserBusy) return; browserBusy = project;
    try { await actions.toggleAgentBrowser(project); } finally { browserBusy = ''; }
  }
  async function setBrowserPort(project: string, port: number) {
    if (!port || port < 1 || port > 65535) return;
    try { await data.setBrowser(project, { port }); } catch (e) { toast(msg(e), 'bad'); }
  }
  // prices (§9.6): synchronization with the public source; afterwards — fresh settings and a
  // toast with the report
  let pricesSyncing = $state(false);
  const sourceName = (u: string) => { try { return new URL(u).hostname.replace(/^www\./, '').replace(/\.ai$|\.com$/, ''); } catch { return u; } };
  async function syncPrices() {
    if (pricesSyncing) return; pricesSyncing = true;
    try { const r = await agents.pricesSync(); settings = await agents.settings(); toast(t('prices.result', { updated: r.updated.length, added: r.added.length, missing: r.missing.length ? t('prices.missing', { list: r.missing.slice(0, 6).join(', ') + (r.missing.length > 6 ? '…' : '') }) : '' })); }
    catch (e) { toast(msg(e), 'bad'); } finally { pricesSyncing = false; }
  }
  async function saveChat() { if (chatSaving || !settings || settingsLoading) return; const snapshot = JSON.parse(JSON.stringify(chatCfg)); chatSaving = true; try { await save({ chat: snapshot }); await data.loadChatRoutes(); } finally { chatSaving = false; } }
  async function doImport() { try { await api.accountImport({ provider: imp.provider, project: imp.project || data.projects[0]?.name, label: imp.label || undefined } as any); form = 'none'; await data.loadAccounts(); toast(t('toast.saved')); } catch (e) { toast(msg(e), 'bad'); } }
  async function doAdd() {
    const env: Record<string, string> = {}; for (const line of add.env.split('\n')) { const i = line.indexOf('='); if (i > 0) env[line.slice(0, i).trim()] = line.slice(i + 1).trim(); }
    try { await api.accountCreate({ provider: add.provider, kind: add.kind, label: add.label || add.provider, env } as any); form = 'none'; await data.loadAccounts(); toast(t('toast.saved')); } catch (e) { toast(msg(e), 'bad'); }
  }
  async function del(id: string, label: string) { if (!window.confirm(t('set.deleteConfirm', { label }))) return; try { await api.accountDelete(id); await data.loadAccounts(); } catch (e) { toast(msg(e), 'bad'); } }
  /** Auto-switch pool (§9.7): a flag on the account card */
  async function setPool(a: { id: string; label: string }, pool: boolean) { try { await api.accountUpdate(a.id, { pool }); await data.loadAccounts(); toast(t(pool ? 'pool.on' : 'pool.off', { label: a.label })); } catch (e) { toast(msg(e), 'bad'); } }
  function blockedUntil(id: string): string { const iso = data.accountsBlocked[id]; if (!iso) return ''; const d = new Date(iso); if (!isFinite(d.getTime()) || d.getTime() <= Date.now()) return ''; return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }); }
  const poolCount = $derived<Record<string, number>>(data.accounts.reduce((m, a) => { if (a.pool) m[a.provider] = (m[a.provider] ?? 0) + 1; return m; }, {} as Record<string, number>));
  async function setProjectAccount(project: string, id: string) { try { await api.projectAccount(project, id || null); await data.loadAccounts(); } catch (e) { toast(msg(e), 'bad'); } }
  async function showNotify() { try { notify = await api.agentsNotify(); } catch (e) { toast(msg(e), 'bad'); } }
  const perm = $derived((settings?.defaults as any) ?? {});
  // workspace policy: the Claude Code permission bypass mode (managed-settings) — the owner's
  // decision
  let policy = $state<{ bypass: boolean } | null>(null);
  let policyLoading = $state(false); let policyBusy = $state(false); let policyError = $state('');
  async function loadPolicy() { if (policyLoading) return; policyLoading = true; policyError = ''; try { policy = await api.agentsPolicyGet(); } catch (e) { policyError = msg(e); } finally { policyLoading = false; } }
  async function setPolicy(bypass: boolean) {
    if (policyBusy || !policy) return; policyBusy = true; policyError = '';
    try { const r = await api.agentsPolicyPut(bypass); policy = { bypass: r.bypass }; toast(t('set.policySaved', { n: r.applied.length })); } catch (e) { policyError = msg(e); } finally { policyBusy = false; }
  }
  $effect(() => { if (ui.settingsTab === 'perm') untrack(() => { if (!policy) loadPolicy(); }); });
  // workspace MCP servers (slice 36): the agentd list, the add form, the rollout to the
  // engines
  let mcpWs = $state(''); let mcp = $state<McpList | null>(null); let mcpBusy = $state(false); let mcpLoading = $state(false); let mcpErr = $state(''); let mcpApplied = $state<Record<string, string> | null>(null); let mcpLoaded = $state(''); let mcpRequested = ''; let mcpSeq = 0;
  const blankMcp = (): McpDraft => ({ name: '', transport: 'stdio', command: '', url: '', env: '', headers: '' });
  let mcpForm = $state<McpDraft>(blankMcp());
  function rememberMcp() { if (mcpWs) mcpDrafts.set(mcpWs, { ...mcpForm }); }
  function changeMcp(ws: string) { if (mcpBusy) return; rememberMcp(); mcpWs = ws; mcpForm = { ...(mcpDrafts.get(ws) ?? blankMcp()) }; loadMcp(); }
  const mcpReady = $derived(!!mcp && mcpLoaded === mcpWs && !mcpLoading && !mcpBusy);
  async function loadMcp() {
    const ws = mcpWs; if (!ws || mcpBusy) return;
    const seq = ++mcpSeq; mcpRequested = ws; mcpLoading = true; mcpErr = ''; mcpApplied = null;
    if (mcpLoaded !== ws) { mcp = null; mcpLoaded = ''; }
    try { const r = await api.mcp(ws); if (seq !== mcpSeq || mcpWs !== ws) return; mcp = r; mcpLoaded = ws; }
    catch (e) { if (seq === mcpSeq && mcpWs === ws) mcpErr = msg(e); }
    finally { if (seq === mcpSeq) mcpLoading = false; }
  }
  $effect(() => {
    if (ui.settingsTab !== 'mcp') return;
    const requested = ui.mcpWs; const first = data.running[0]?.name; const saving = mcpBusy;
    untrack(() => { if (saving) return; const want = requested || mcpWs || first || ''; if (requested) ui.mcpWs = null; if (want && want !== mcpRequested) changeMcp(want); });
  });
  const parseArgs = (line: string) => (line.match(/"[^"]*"|'[^']*'|\S+/g) || []).map((a) => a.replace(/^["']|["']$/g, ''));
  function kv(text: string, sep: string): Record<string, string> { const o: Record<string, string> = {}; for (const l of text.split('\n')) { const i = l.indexOf(sep); if (i > 0) o[l.slice(0, i).trim()] = l.slice(i + sep.length).trim(); } return o; }
  async function saveMcp(servers: McpServer[]): Promise<boolean> {
    if (!mcpReady) return false; const ws = mcpWs; const seq = ++mcpSeq;
    mcpBusy = true; mcpErr = '';
    try { const r = await api.mcpSave(ws, servers); if (seq === mcpSeq && mcpWs === ws) { mcp = { servers: r.servers, engines: r.engines }; mcpLoaded = ws; mcpApplied = r.applied; } toast(t('sx.savedTarget', { target: ws })); return true; }
    catch (e) { if (seq === mcpSeq && mcpWs === ws) mcpErr = msg(e); return false; } finally { mcpBusy = false; }
  }
  async function addMcp() {
    const f = mcpForm; const parts = parseArgs(f.command);
    const srv: McpServer = { name: f.name.trim(), transport: f.transport, command: parts[0] ?? '', args: parts.slice(1), url: f.url.trim(), env: kv(f.env, '='), headers: kv(f.headers, ':') };
    const snapshot = JSON.stringify(f); const ws = mcpWs;
    const saved = await saveMcp([...(mcp?.servers ?? []).filter((x) => x.name !== srv.name), srv]);
    if (saved && ws === mcpWs && snapshot === JSON.stringify(mcpForm)) { mcpForm = { ...blankMcp(), transport: f.transport }; mcpDrafts.delete(ws); }
  }
  const removeMcp = (name: string) => saveMcp((mcp?.servers ?? []).filter((x) => x.name !== name));
  // server health (slice 70): by button — the CLI connects to each one, that takes seconds; after an OAuth
  // sign-in — anew
  let mcpHealth = $state<Record<string, McpHealthRow>>({}); let mcpHealthBusy = $state(false); let mcpHealthErr = $state(''); let mcpHealthWs = $state('');
  async function checkMcpHealth(refresh = true) {
    if (mcpHealthBusy) return; mcpHealthBusy = true; mcpHealthErr = ''; const ws = mcpWs;
    try { const r = await api.mcpHealth(ws, refresh); if (ws === mcpWs) { mcpHealth = Object.fromEntries(r.servers.map((x) => [x.name, x])); mcpHealthWs = ws; } }
    catch (e) { if (ws === mcpWs) mcpHealthErr = msg(e); } finally { mcpHealthBusy = false; }
  }
  const healthLabel = (h: McpHealthRow) => h.status === 'ok' ? t('mcph.ok') : h.status === 'auth' ? t('mcph.auth') : h.status === 'fail' ? t('mcph.fail') : h.detail;
  // OAuth sign-in to an MCP server (slice 66): a workspace terminal with `claude mcp login <name> --no-browser` (Codex — `codex
  // mcp login`), the link in the output, the address after sign-in is pasted into the same terminal
  let mcpAuth = $state<{ name: string; engine: 'claude' | 'codex'; pty: string | null; paste: string; done: boolean; code: number | null; error: string } | null>(null);
  async function mcpLogin(name: string, engine: 'claude' | 'codex' = 'claude', logout = false) {
    const ws = mcpWs; if (mcpAuth?.pty && !mcpAuth.done) { try { await api.ptyKill(ws, mcpAuth.pty); } catch {} }
    mcpAuth = { name, engine, pty: null, paste: '', done: false, code: null, error: '' };
    // through a login shell: the CLIs live in ~/.npm-global/bin, which is not in the daemon's PATH; the server name is
    // validated (letters, digits, -, _)
    const safe = name.replace(/[^A-Za-z0-9_.-]/g, '');
    const cmd = ['bash', '-lic', engine === 'codex' ? `codex mcp ${logout ? 'logout' : 'login'} ${safe}` : `claude mcp ${logout ? 'logout' : 'login'} ${safe}${logout ? '' : ' --no-browser'}`];
    try { const p = await api.ptyCreate(ws, { cmd, title: `MCP · ${logout ? 'logout' : 'login'} ${name}` }); if (mcpAuth) mcpAuth = { ...mcpAuth, pty: p.id }; }
    catch (e) { if (mcpAuth) mcpAuth = { ...mcpAuth, error: msg(e) }; }
  }
  async function mcpAuthPaste() { const a = mcpAuth; if (!a?.pty || !a.paste.trim()) return; try { await api.ptyInput(mcpWs, a.pty, a.paste.trim() + '\r'); mcpAuth = { ...a, paste: '' }; } catch (e) { mcpAuth = { ...a, error: msg(e) }; } }
  async function mcpAuthClose() { const a = mcpAuth; mcpAuth = null; if (a?.pty && !a.done) { try { await api.ptyKill(mcpWs, a.pty); } catch {} } if (a?.done) void checkMcpHealth(true); }
  // Claude Code tool rules in the workspace settings (agentd perms.rs): the "user" scope (~/.claude/settings.json) or "project" (.claude/settings.local.json in home)
  let rules = $state<{ ws: string; scope: 'user' | 'project'; allow: string; deny: string; ask: string; path?: string; loaded?: string }>({ ws: '', scope: 'user', allow: '', deny: '', ask: '' });
  let rulesLoading = $state(false); let rulesBusy = $state(false); let rulesErr = $state(''); let rulesSeq = 0;
  let rulesBaseline = '';
  const rulesFields = (): RulesDraft => ({ allow: rules.allow, deny: rules.deny, ask: rules.ask });
  function rememberRules() { if (!rules.ws || rules.loaded !== `${rules.ws}:${rules.scope}`) return; const target = rules.loaded; const values = rulesFields(); if (JSON.stringify(values) === rulesBaseline) rulesDrafts.delete(target); else rulesDrafts.set(target, values); }
  const rulesReady = $derived(!!rules.ws && rules.loaded === `${rules.ws}:${rules.scope}` && !rulesLoading && !rulesBusy);
  const rulesTarget = $derived(`${rules.ws} · ${rules.scope === 'user' ? t('perm.scopeUserShort') : t('perm.scopeProjectShort')}`);
  const lines = (s: string) => s.split('\n').map(x => x.trim()).filter(Boolean);
  const RULE_CHIPS = ['Bash(npm test:*)', 'Bash(git:*)', 'Read', 'Edit', 'WebFetch', 'WebSearch'];
  async function loadRules() {
    const { ws, scope } = rules; if (!ws || rulesBusy) return;
    rememberRules();
    const seq = ++rulesSeq; const target = `${ws}:${scope}`; rulesLoading = true; rulesErr = '';
    if (rules.loaded !== target) rules = { ws, scope, allow: '', deny: '', ask: '' };
    try { const r = await api.permissionsGet(ws, scope, '/home/coder'); if (seq !== rulesSeq || rules.ws !== ws || rules.scope !== scope) return; const fields = { allow: r.allow.join('\n'), deny: r.deny.join('\n'), ask: r.ask.join('\n') }; rulesBaseline = JSON.stringify(fields); rules = { ws, scope, ...fields, ...(rulesDrafts.get(target) ?? {}), path: r.path, loaded: target }; }
    catch (e) { if (seq === rulesSeq && rules.ws === ws && rules.scope === scope) rulesErr = msg(e); }
    finally { if (seq === rulesSeq) rulesLoading = false; }
  }
  function changeRules(ws: string, scope = rules.scope) { if (rulesBusy || (ws === rules.ws && scope === rules.scope && rules.loaded)) return; rememberRules(); rules = { ws, scope, allow: '', deny: '', ask: '' }; loadRules(); }
  async function saveRules() {
    if (!rulesReady) return; const snapshot = { ...rules }; const { ws, scope } = snapshot; const seq = ++rulesSeq; const target = rulesTarget;
    rulesBusy = true; rulesErr = '';
    try { const r = await api.permissionsPut(ws, scope, '/home/coder', { allow: lines(snapshot.allow), deny: lines(snapshot.deny), ask: lines(snapshot.ask) }); rulesDrafts.delete(`${ws}:${scope}`); if (seq === rulesSeq && rules.ws === ws && rules.scope === scope) { rules = { ...snapshot, path: r.path, allow: r.allow.join('\n'), deny: r.deny.join('\n'), ask: r.ask.join('\n') }; rulesBaseline = JSON.stringify(rulesFields()); } toast(t('sx.savedTarget', { target })); }
    catch (e) { if (seq === rulesSeq && rules.ws === ws && rules.scope === scope) rulesErr = msg(e); }
    finally { rulesBusy = false; }
  }
  $effect(() => { if (ui.settingsTab !== 'perm') return; const first = data.running[0]?.name; untrack(() => { if (!rules.ws && first) changeRules(first); }); });
  onDestroy(() => { rememberMcp(); rememberRules(); });
</script>

<main class="main">
  <div class="top"><button class="icon ghost burger" aria-label={t('nav.sections')} onclick={() => (ui.popen = !ui.popen)}><Icon name="menu" /></button><div class="title"><h1>{t('set.title')}</h1></div></div>
  <div class="content">
    <div class="settings">
      <div class="subnav" aria-label={t('set.title')}>{#each TABS as [k, i, l] (k)}<button class:active={ui.settingsTab === k} aria-current={ui.settingsTab === k ? 'page' : undefined} onclick={() => (ui.settingsTab = k)}><Icon name={i} size="s" /> {l}</button>{/each}</div>
      <div style="min-width: 0">
        {#if settingsError}<div class="load-error" role="alert"><span>{settingsError}</span><button class="sm" disabled={settingsLoading} onclick={loadSettings}>{t('sx.retry')}</button></div>{/if}
        {#if ui.settingsTab === 'accounts'}
          {#if data.accountsError}<div class="load-error" role="alert"><span>{data.accountsError}</span><button class="sm" disabled={data.accountsLoading} onclick={() => data.loadAccounts()}>{t('sx.retry')}</button></div>{/if}
          {#if data.accountsLoading && !data.accounts.length}<p class="muted sm" role="status">{t('common.loading')}</p>{/if}
          <div class="page-h" style="margin-bottom: 12px"><div><h2>{t('set.accountsTitle')}</h2><p>{t('set.accountsHint')} {t('pool.hint')}</p></div><span class="sp"></span><button class="sm" onclick={() => (form = form === 'import' ? 'none' : 'import')}><Icon name="download" size="s" /> {t('set.import')}</button><button class="sm" onclick={() => { login = { provider: 'claude', project: data.running[0]?.name ?? '', way: 'link', pasted: '', paste: '' }; form = form === 'login' ? 'none' : 'login'; }}><Icon name="key" size="s" /> {t('set.login')}</button><button class="sm primary" onclick={() => (form = form === 'add' ? 'none' : 'add')}><Icon name="plus" size="s" /> {t('set.add')}</button></div>
          {#if form === 'import'}
            <div class="card-s fs" style="margin-bottom: 14px"><h3>{t('set.importTitle')}</h3><div class="frm"><span>{t('set.importProvider')}</span><select class="sm" aria-label={t('set.importProvider')} bind:value={imp.provider}><option value="claude">Claude (~/.claude)</option><option value="codex">Codex (~/.codex/auth.json)</option><option value="gate">{t('set.gate')}</option></select><span>{t('set.importProject')}</span><select class="sm" aria-label={t('set.importProject')} bind:value={imp.project}>{#each data.projects as p (p.name)}<option value={p.name}>{p.name}</option>{/each}</select><span>{t('set.importLabel')}</span><input class="sm" aria-label={t('set.importLabel')} bind:value={imp.label} placeholder="Claude Max · charlie" /></div><div class="row"><button class="sm primary" onclick={doImport}>{t('set.importDo')}</button><button class="sm ghost" onclick={() => (form = 'none')}>{t('set.cancel')}</button></div></div>
          {:else if form === 'login'}
            <div class="card-s fs" style="margin-bottom: 14px"><h3>{t('set.loginTitle')}</h3><p>{t('set.loginHint')}</p>
              {#if login.imported && login.way !== 'term'}
                <div class="row" style="gap: 8px; margin-top: 6px; flex-wrap: wrap"><span class="ok-note"><Icon name="check" size="s" /> {t('set.loginImported')}</span><span class="sp"></span><button class="sm ghost" onclick={closeLogin}>{t('common.close')}</button><span class="muted sm" style="flex-basis: 100%">{t('set.loginUseHint')}</span></div>
              {:else if !login.pty && !(login.way !== 'term' && login.url)}
                <div class="frm"><span>{t('set.importProvider')}</span><select class="sm" aria-label={t('set.importProvider')} bind:value={login.provider}><option value="claude">{t('set.optClaudeSub')}</option><option value="codex">{t('set.optCodexSub')}</option><option value="antigravity">{t('set.optAntigravitySub')}</option></select>
                  {#if login.provider === 'antigravity'}<span>{t('set.loginWay')}</span><span class="radios"><label><input type="radio" name="lw" value="link" bind:group={login.way} /> {t('set.loginByLink')}</label><label><input type="radio" name="lw" value="term" bind:group={login.way} /> {t('set.loginAgyWay')}</label></span>{:else}<span>{t('set.loginWay')}</span><span class="radios"><label><input type="radio" name="lw" value="link" bind:group={login.way} /> {t('set.loginByLink')}</label><label><input type="radio" name="lw" value="code" bind:group={login.way} /> {t('set.loginByCode')}</label><label><input type="radio" name="lw" value="term" bind:group={login.way} /> {t('set.loginByTerm')}</label></span>{/if}
                  {#if login.way === 'term' || login.provider === 'antigravity'}<span>{t('set.loginWs')}</span><select class="sm" aria-label={t('set.loginWs')} bind:value={login.project}>{#each data.running as p (p.name)}<option value={p.name}>{p.name}</option>{/each}</select>{/if}</div>
                <div class="row" style="gap: 8px; margin-top: 10px">
                  {#if login.way !== 'term'}<button class="sm primary" disabled={login.busy} onclick={getLink}>{login.busy ? t('common.loading') : t('set.loginGetLink')}</button>
                  {:else}<button class="sm primary" disabled={login.busy || !data.running.length} onclick={startLogin}>{login.busy ? t('common.loading') : t('set.loginStart')}</button>{/if}
                  <button class="sm ghost" onclick={closeLogin}>{t('set.cancel')}</button></div>
                {#if login.error}<div class="err" style="margin-top: 8px">{login.error}</div>{/if}
              {:else if login.way === 'link' && login.url && login.provider === 'antigravity'}
                <p class="muted sm agy-link-hint">{t('set.loginAgyLinkHint')}</p>
                <div class="row" style="gap: 8px; flex-wrap: wrap; margin-top: 6px"><a class="btn sm primary agy-login-link" href={login.url} target="_blank" rel="noopener"><Icon name="ext" size="s" /> {t('set.loginOpenGoogle')}</a><button class="sm" onclick={copyLink}><Icon name="copy" size="s" /> {t('set.copy')}</button></div>
                <div class="frm" style="margin-top: 10px"><span>{t('set.loginAgyCode')}</span><input class="sm mono agy-code" placeholder={t('set.loginAgyCodePh')} aria-label={t('set.loginAgyCode')} bind:value={login.pasted} onkeydown={(e) => { if (e.key === 'Enter') finishLink(); }} /></div>
                <div class="row" style="gap: 8px; margin-top: 10px; align-items: center"><button class="sm primary" disabled={login.busy || !login.pasted.trim()} onclick={finishLink}>{login.busy ? t('set.loginAgyWait') : t('set.loginFinish')}</button><button class="sm ghost" onclick={closeLogin}>{t('set.cancel')}</button></div>
                {#if login.error}<div class="err" style="margin-top: 8px">{login.error}</div>{/if}
              {:else if login.way === 'link' && login.url}
                <p class="muted sm">{t('set.gateCallbackHint', { url: login.callback || 'http://localhost:…' })}</p>
                <div class="row" style="gap: 8px; flex-wrap: wrap; margin-top: 6px"><a class="btn sm primary" href={login.url} target="_blank" rel="noopener"><Icon name="ext" size="s" /> {t('set.loginOpen')}</a><button class="sm" onclick={copyLink}><Icon name="copy" size="s" /> {t('set.copy')}</button></div>
                <div class="frm" style="margin-top: 10px"><span>{t('set.loginRedirect')}</span><input class="sm mono cbaddr" placeholder={(login.callback || 'http://localhost:…/…') + '?code=…&state=…'} aria-label={t('set.loginRedirect')} bind:value={login.pasted} onkeydown={(e) => { if (e.key === 'Enter') finishLink(); }} /></div>
                <div class="row" style="gap: 8px; margin-top: 10px"><button class="sm primary" disabled={login.busy || !login.pasted.trim()} onclick={finishLink}>{login.busy ? t('common.loading') : t('set.loginFinish')}</button><button class="sm ghost" onclick={closeLogin}>{t('set.cancel')}</button></div>
                {#if login.error}<div class="err" style="margin-top: 8px">{login.error}</div>{/if}
              {:else if login.way === 'code' && login.url && login.provider === 'codex'}
                <p class="muted sm">{t('set.loginCodexHint')}</p>
                <div class="row" style="gap: 8px; flex-wrap: wrap; margin-top: 6px"><a class="btn sm primary" href={login.url} target="_blank" rel="noopener"><Icon name="ext" size="s" /> {t('set.loginOpen')}</a><button class="sm" onclick={copyLink}><Icon name="copy" size="s" /> {t('set.copy')}</button></div>
                <div class="devcode" style="margin-top: 10px"><span class="muted sm">{t('set.loginDeviceCode')}</span><code class="code">{userCode}</code><button class="sm" onclick={copyCode}><Icon name="copy" size="s" /> {t('set.copy')}</button><span class="muted xs">{t('set.loginExpires', { m: minutesLeft() })}</span></div>
                <div class="row" style="gap: 8px; margin-top: 10px; align-items: center">
                  {#if login.imported}<span class="ok-note"><Icon name="check" size="s" /> {t('set.loginImported')}{accountLabel ? ` · ${accountLabel}` : ''}</span><span class="muted sm" style="flex-basis: 100%">{t('set.loginUseHint')}</span>{:else if !login.error}<span class="muted sm waiting">⏳ {t('set.loginDeviceWait')}</span>{/if}
                  <span class="sp"></span><button class="sm ghost" onclick={closeLogin}>{login.imported ? t('common.close') : t('set.cancel')}</button></div>
                {#if login.error}<div class="err" style="margin-top: 8px">{login.error}</div>{/if}
              {:else if login.way === 'code' && login.url}
                <p class="muted sm">{t('set.loginLinkHint')}</p>
                <div class="row" style="gap: 8px; flex-wrap: wrap; margin-top: 6px"><a class="btn sm primary" href={login.url} target="_blank" rel="noopener"><Icon name="ext" size="s" /> {t('set.loginOpen')}</a><button class="sm" onclick={copyLink}><Icon name="copy" size="s" /> {t('set.copy')}</button></div>
                <div class="frm" style="margin-top: 10px"><span>{t('set.loginCode')}</span><input class="sm mono" placeholder="xxxxxxxx#yyyyyyyy" aria-label={t('set.loginCode')} bind:value={login.pasted} onkeydown={(e) => { if (e.key === 'Enter') finishLink(); }} /></div>
                <div class="row" style="gap: 8px; margin-top: 10px"><button class="sm primary" disabled={login.busy || !login.pasted.trim()} onclick={finishLink}>{login.busy ? t('common.loading') : t('set.loginFinish')}</button><button class="sm ghost" onclick={closeLogin}>{t('set.cancel')}</button></div>
                {#if login.error}<div class="err" style="margin-top: 8px">{login.error}</div>{/if}
              {:else}
                {#if login.provider === 'antigravity'}<p class="muted sm agy-login-hint">{t('set.loginAgyHint')}</p>
                  {#if login.agyUrl && !login.imported}<div class="row" style="gap: 8px; flex-wrap: wrap; margin-bottom: 8px"><a class="btn sm primary agy-login-link" href={login.agyUrl} target="_blank" rel="noopener"><Icon name="ext" size="s" /> {t('set.loginOpenGoogle')}</a><button class="sm" onclick={() => { try { navigator.clipboard.writeText(login.agyUrl ?? ''); toast(t('set.loginCopied'), 'info'); } catch {} }}><Icon name="copy" size="s" /> {t('set.copy')}</button></div>{/if}{/if}
                <div class="login-term">{#key login.pty}<Term project={login.project} id={login.pty ?? ""} onexit={finishLogin} />{/key}</div>
                {#if !login.done}<div class="row" style="gap: 8px; margin-top: 8px"><input class="sm mono" style="flex: 1" placeholder={login.provider === 'antigravity' ? t('set.loginAgyPaste') : t('set.loginPaste')} aria-label={login.provider === 'antigravity' ? t('set.loginAgyPaste') : t('set.loginPaste')} bind:value={login.paste} onkeydown={(e) => { if (e.key === 'Enter') pasteToTerm(); }} /><button class="sm" disabled={!login.paste.trim()} onclick={pasteToTerm}>{t('set.loginPasteDo')}</button></div>{/if}
                <div class="row" style="gap: 8px; margin-top: 10px; flex-wrap: wrap; align-items: center">
                  {#if login.imported}<span class="ok-note"><Icon name="check" size="s" /> {t('set.loginImported')}</span>
                  {:else if login.provider === 'antigravity' && !login.done}<span class="muted sm waiting agy-login-wait">⏳ {t('set.loginAgyWait')}</span>
                  {:else if login.done && login.provider === 'antigravity'}<span class="muted sm">{t('set.loginExited', { code: login.code ?? 0 })}</span>
                  {:else if login.done}<span class="muted sm">{t('set.loginExited', { code: login.code ?? 0 })}</span><button class="sm primary" onclick={importLogin}>{t('set.loginImport')}</button>
                  {:else}<span class="muted sm">{t('set.loginWaiting')}</span><button class="sm" onclick={importLogin}>{t('set.loginImport')}</button>{/if}
                  <span class="sp"></span><button class="sm ghost" onclick={closeLogin}>{login.imported ? t('common.close') : t('set.cancel')}</button>
                </div>
                {#if login.error}<div class="err" style="margin-top: 8px">{login.error}</div>{/if}
              {/if}
            </div>
          {:else if form === 'add'}
            <div class="card-s fs" style="margin-bottom: 14px"><h3>{t('set.addTitle')}</h3><div class="frm"><span>{t('set.importProvider')}</span><select class="sm" aria-label={t('set.importProvider')} bind:value={add.provider}><option value="anthropic">Anthropic</option><option value="openai">OpenAI</option><option value="openrouter">OpenRouter</option><option value="claude">{t('set.optClaudeProxy')}</option><option value="codex">{t('set.optCodexProxy')}</option></select><span>{t('set.addKind')}</span><select class="sm" aria-label={t('set.addKind')} bind:value={add.kind}><option value="api_key">{t('set.optKey')}</option><option value="proxy">{t('set.optProxy')}</option></select><span>{t('set.addLabel')}</span><input class="sm" aria-label={t('set.addLabel')} bind:value={add.label} /><span>{t('set.addEnv')}</span><textarea rows="3" class="mono" aria-label={t('set.addEnv')} bind:value={add.env} placeholder="OPENAI_API_KEY=…&#10;OPENAI_BASE_URL=https://openrouter.ai/api/v1"></textarea></div><div class="row"><button class="sm primary" onclick={doAdd}>{t('set.add')}</button><button class="sm ghost" onclick={() => (form = 'none')}>{t('set.cancel')}</button></div></div>
          {/if}
          <div class="acct-grid">
            {#each data.accounts as a (a.id)}
              {@const L = limits[a.id]}
              {@const defs = Object.entries(data.projectAccounts).filter(([, id]) => id === a.id).map(([pn]) => pn)}
              <div class="acct-card" data-acc={a.id}>
                <div class="ah">
                  <span class="av {avClass(a.provider)}"><Icon name={avIcon(a.provider)} size="s" /></span>
                  <div class="ttl"><b class="ell">{a.label}</b>
                    <div class="badges"><span class="chip">{kindLabel(a.kind)}</span>{#if L?.plan}<span class="chip plan">{L.plan}</span>{/if}{#if a.imported_from}<span class="chip muted">{t('set.fromWs', { ws: a.imported_from })}</span>{/if}</div>
                  </div>
                  <div class="acts">
                    {#if a.kind === 'subscription'}<button class="icon xs ghost" title={t('lim.refresh')} disabled={limitsBusy} onclick={() => loadLimit(a.id)}><Icon name="refresh" size="s" /></button>{/if}
                    <button class="icon xs ghost danger" title={t('set.delete')} onclick={() => del(a.id, a.label)}><Icon name="trash" size="s" /></button>
                  </div>
                </div>
                {#if a.kind === 'subscription'}
                  {#if L?.error}
                    <div class="lerr"><Icon name="alert" size="s" /> <span title={L.error}>{t('lim.revoked')}</span><button class="xs" onclick={() => relogin(a.provider)}>{t('lim.relogin')}</button></div>
                  {:else if L}
                    <div class="lims">
                      {#each L.windows as w (w.id)}<div class="lrow {lvl(w.used_pct)}"><span class="lbl">{winLabel(w)}</span><span class="bar"><i style="width: {Math.min(100, Math.max(0, w.used_pct))}%"></i></span><span class="pct">{Math.round(w.used_pct)}%</span><span class="rst">{until(w.resets_at)}</span></div>{/each}
                      {#if !L.windows.length}<span class="muted xs">{t('lim.none')}</span>{/if}
                    </div>
                  {:else}<div class="muted xs">{limitsBusy ? t('common.loading') : ''}</div>{/if}
                {/if}
                {#if blockedUntil(a.id)}<div class="lerr pool-blocked"><Icon name="alert" size="s" /> <span>{t('pool.blocked', { when: blockedUntil(a.id) })}</span></div>{/if}
                <div class="af"><span>{t('set.defaultFor')}: {defs.length ? defs.join(', ') : '—'}</span>{#if a.env_keys.length || a.files.length}<code class="muted ell">{[...a.env_keys, ...a.files].join(', ')}</code>{/if}{#if L?.fetched_at}<span class="muted xs">{updatedLabel(L.fetched_at)}</span>{/if}
                  <label class="pool-row" title={a.pool && (poolCount[a.provider] ?? 0) < 2 ? t('pool.single') : t('pool.hint')}><button class="switch xs pool-switch" role="switch" aria-checked={!!a.pool} aria-label={`${t('pool.switch')} · ${a.label}`} class:on={!!a.pool} onclick={() => setPool(a, !a.pool)}></button><span class="muted xs">{t('pool.switch')}</span></label>
                </div>
              </div>
            {/each}
            {#if !data.accounts.length}<div class="card-s muted sm" style="padding: 14px 16px; grid-column: 1 / -1">{t('set.noAccounts')}</div>{/if}
          </div>
          <div class="card-s fs gate" style="margin-top: 14px"><h3>{t('set.gateTitle')}</h3><p>{t('set.gateProvHint')}</p>
            {#if gateErr}<div class="err">{gateErr}</div>{/if}
            {#if gate && !gate.targets.length}<p class="muted sm">{t('set.gateNone')}</p>{/if}
            {#each gate?.targets ?? [] as tg (tg.app)}
              <div class="gtarget" data-app={tg.app}>
                <div class="row" style="gap: 8px; flex-wrap: wrap; align-items: center"><b>{tg.title}</b>{#if !tg.managed}<span class="muted sm ghint">{t('set.gateNoKey')}</span>{:else if tg.reachable}<span class="dot ok"></span>{:else}<span class="err sm ghint">{tg.error ?? t('set.gateUnreachable')}</span>{/if}<span class="sp"></span>
                  {#if tg.managed && tg.reachable}<button class="sm primary glogin" onclick={(e) => { e.stopPropagation(); gateLoginMenu(e.currentTarget as HTMLElement, tg); }}><Icon name="key" size="s" /> {t('set.gateLogin')} ▾</button><button class="sm gplugins" onclick={(e) => { e.stopPropagation(); gatePluginsMenu(e.currentTarget as HTMLElement, tg); }}><Icon name="download" size="s" /> {t('set.gatePlugins')} ▾</button>{/if}
                  <button class="sm ghost" title={t('agents.refresh')} onclick={loadGate}>⟳</button></div>
                {#each tg.logins as l (l.name)}
                  <div class="glogin-row" class:off={l.disabled || l.unavailable}><span class="av {avClass(l.provider)}"><Icon name={avIcon(l.provider)} size="s" /></span>
                    <span class="gp">{l.provider}</span><span class="ell">{l.label ?? l.email ?? l.name}{#if l.plan || l.limits?.plan} <span class="chip plan">{l.plan ?? l.limits?.plan}</span>{/if}</span>
                    <span class="pill" class:off={l.disabled} class:warn={!l.disabled && l.unavailable} title={l.status_message ?? ''}>{l.disabled ? t('set.gateOff') : l.unavailable ? (l.status_message ?? l.status ?? '') : (l.status ?? 'ok')}</span>
                    <span class="gacts"><button class="xs ghost" onclick={() => gateToggle(tg, l)}>{l.disabled ? t('set.gateEnable') : t('set.gateDisable')}</button><button class="icon xs ghost danger" title={t('set.gateDelete')} onclick={() => gateDelete(tg, l)}><Icon name="trash" size="s" /></button></span></div>
                  {#if l.limits?.windows.length}
                    <div class="lims glims">{#each l.limits.windows as w (w.id)}<div class="lrow {lvl(w.used_pct)}"><span class="lbl ell" title={w.id}>{winLabel(w)}</span><span class="bar"><i style="width: {Math.min(100, Math.max(0, w.used_pct))}%"></i></span><span class="pct">{Math.round(w.used_pct)}%</span><span class="rst">{until(w.resets_at)}</span></div>{/each}</div>
                  {/if}
                {/each}
                {#if tg.managed && tg.reachable && !tg.logins.length}<div class="muted sm">{t('set.gateNoLogins')}</div>{/if}
                {#if gl && gl.app === tg.app}
                  <div class="gflow">
                    <div class="row" style="gap: 8px; align-items: center"><b>{gl.title}</b><span class="sp"></span><button class="xs ghost" onclick={gateCancel}>{gl.done ? t('common.close') : t('set.cancel')}</button></div>
                    {#if gl.busy && !gl.url}<div class="muted sm">{t('common.loading')}</div>
                    {:else if gl.url}
                      <div class="row" style="gap: 8px; flex-wrap: wrap; margin-top: 6px"><a class="btn sm primary" href={gl.url} target="_blank" rel="noopener"><Icon name="ext" size="s" /> {t('set.loginOpen')}</a><button class="sm" onclick={async () => { try { await navigator.clipboard.writeText(gl?.url ?? ''); toast(t('set.loginCopied'), 'info'); } catch {} }}><Icon name="copy" size="s" /> {t('set.copy')}</button></div>
                      {#if gl.flow === 'device'}
                        {#if gl.userCode}<div class="devcode" style="margin-top: 8px"><span class="muted sm">{t('set.loginDeviceCode')}</span><code class="code">{gl.userCode}</code><button class="sm" onclick={async () => { try { await navigator.clipboard.writeText(gl?.userCode ?? ''); toast(t('set.loginCopied'), 'info'); } catch {} }}><Icon name="copy" size="s" /> {t('set.copy')}</button></div>{/if}
                      {:else if gl.flow === 'code'}
                        <p class="muted sm" style="margin: 8px 0 4px">{t('set.loginLinkHint')}</p>
                        <div class="row" style="gap: 8px"><input class="sm mono" style="flex: 1" placeholder="xxxxxxxx#yyyyyyyy" aria-label={t('set.loginCode')} bind:value={gl.paste} onkeydown={(e) => { if (e.key === 'Enter') gateCallback(); }} /><button class="sm primary" disabled={gl.busy || !gl.paste.trim()} onclick={gateCallback}>{t('set.loginFinish')}</button></div>
                      {:else}
                        <p class="muted sm" style="margin: 8px 0 4px">{t('set.gateCallbackHint', { url: gl.callback || 'http://localhost:…' })}</p>
                        <div class="row" style="gap: 8px"><input class="sm mono" style="flex: 1" placeholder={gl.callback ? gl.callback + '?code=…&state=…' : 'http://localhost:…/…?code=…&state=…'} aria-label={t('set.loginCode')} bind:value={gl.paste} onkeydown={(e) => { if (e.key === 'Enter') gateCallback(); }} /><button class="sm primary" disabled={gl.busy || !gl.paste.trim()} onclick={gateCallback}>{t('set.gateFinish')}</button></div>
                      {/if}
                      <div class="row" style="gap: 8px; margin-top: 8px; align-items: center">{#if gl.done}<span class="ok-note"><Icon name="check" size="s" /> {t('set.gateDone')}</span>{:else if gl.error}<span class="err sm">{gl.error}</span>{:else}<span class="muted sm waiting">⏳ {gl.sent ? t('set.gateSentWait') : gl.flow === 'device' ? t('set.loginDeviceWait') : gl.flow === 'code' ? t('set.gateCodeWait') : t('set.gateCallbackWait')}</span>{/if}</div>
                    {:else if gl.error}<div class="err" style="margin-top: 6px">{gl.error}</div>{/if}
                  </div>
                {/if}
              </div>
            {/each}
          </div>
          {#if data.accounts.length && data.runningAll.length}
            <div class="card-s fs" style="margin-top: 14px"><h3>{t('set.projectAccount')}</h3><p>{t('set.projectAccountHint')}</p><div class="frm">{#each data.runningAll as p (p.name)}<span>{p.hidden ? `${t('chat.title')} (${p.name})` : p.name}</span><select class="sm" aria-label={`${t('set.projectAccount')} · ${p.name}`} value={data.projectAccounts[p.name] ?? ''} onchange={(e) => setProjectAccount(p.name, (e.target as HTMLSelectElement).value)}><option value="">{t('set.gate')}</option>{#each data.accounts as a (a.id)}<option value={a.id}>{a.label}</option>{/each}</select>{/each}</div></div>
          {/if}
        {:else if ui.settingsTab === 'chat'}
          <div class="card-s fs" style="max-width: 760px">
            <h3>{t('set.chatDefaults')}</h3>
            {#if data.chatRoutesError}<div class="load-error" role="alert"><span>{data.chatRoutesError}</span><button class="sm" disabled={data.chatRoutesLoading} onclick={() => data.loadChatRoutes()}>{t('sx.retry')}</button></div>{/if}
            {#if data.chatRoutesLoading && !data.chatRoutes}<p class="muted sm" role="status">{t('common.loading')}</p>{/if}
            <fieldset class="frm" disabled={settingsLoading || !settings || chatSaving}>
              <span>{t('chat.route')}</span><select class="sm" aria-label={t('chat.route')} disabled={!data.chatRoutes || data.chatRoutesLoading} bind:value={chatCfg.route} onchange={() => (chatCfg.model = '')}><option value="">{t('set.chatAuto')}</option>{#each data.chatRoutes?.routes ?? [] as r (r.id)}<option value={r.id}>{r.title}{r.error ? ' · ' + r.error : ''}</option>{/each}</select>
              <span>{t('chat.model')}</span><select class="sm" aria-label={t('chat.model')} disabled={!data.chatRoutes || data.chatRoutesLoading} bind:value={chatCfg.model}><option value="">{t('set.chatAuto')}</option>{#each chatRouteGroups as g (g.key)}<optgroup label={g.title}>{#each g.models as m, i (m.id + i)}<option value={m.id}>{m.name && m.name !== m.id ? `${m.name} · ${m.id}` : m.id}</option>{/each}</optgroup>{/each}</select>
              <span>{t('set.chatSearch')}<div class="hint">{t('set.chatSearchHint')}</div></span>
              <span class="stack" style="gap: 6px">
                <select class="sm search-kind" aria-label={t('set.chatSearch')} bind:value={chatCfg.search.kind}><option value="none">{t('set.chatSearchNone')}</option><option value="searxng">SearXNG</option><option value="brave">Brave Search</option><option value="tavily">Tavily</option></select>
                {#if chatCfg.search.kind === 'searxng'}<input class="sm mono search-url" aria-label={t('mcp.url')} bind:value={chatCfg.search.url} placeholder="http://127.0.0.1:8450" />{/if}
                {#if chatCfg.search.kind === 'brave' || chatCfg.search.kind === 'tavily'}<input class="sm mono" type="password" aria-label={t('set.chatSearchKey')} bind:value={chatCfg.search.key} placeholder={t('set.chatSearchKey')} autocomplete="off" />{/if}
              </span>
            </fieldset>
            <div class="acts"><button class="sm primary" disabled={!settings || settingsLoading || chatSaving} onclick={saveChat}>{chatSaving ? t('sx.saving') : t('common.save')}</button></div>
            {#if data.chatRoutes && !data.chatRoutes.routes.length}<p class="muted sm">{t('chat.noRoutes')}</p>{/if}
          </div>
        {:else if ui.settingsTab === 'routes'}
          <div class="card-s fs"><h3>{t('set.routesTitle')}</h3><p>{t('set.routesHint')}</p>
            {#if settings}<div class="frm"><span>Claude</span><select class="sm" aria-label="Claude" value={settings.defaults.claude ?? ''} onchange={(e) => save({ defaults: { ...settings!.defaults, claude: (e.target as HTMLSelectElement).value } })}><option value="">{t('agents.modelDefault')}</option>{#each MODELS_FOR.claude as x (x)}<option value={x}>{x}</option>{/each}</select><span>Codex</span><select class="sm" aria-label="Codex" value={settings.defaults.codex ?? ''} onchange={(e) => save({ defaults: { ...settings!.defaults, codex: (e.target as HTMLSelectElement).value } })}><option value="">{t('agents.modelDefault')}</option>{#each MODELS_FOR.codex as x (x)}<option value={x}>{x}</option>{/each}</select><span>gate</span><span class="muted sm">{t('set.gateHint')}</span></div>{/if}
          </div>
          {#if settings}<div class="card-s fs prices-card" style="margin-top: 14px"><div class="prices-head"><div><h3>{t('set.prices')}</h3><p>{t('set.pricesHint')}</p></div><button class="sm prices-sync" disabled={pricesSyncing} onclick={syncPrices}><Icon name="refresh" size="s" /> {pricesSyncing ? t('prices.syncing') : t('prices.sync')}</button></div>
            <p class="muted sm prices-synced">{settings.prices_sync ? t('prices.syncedAt', { source: sourceName(settings.prices_sync.source), when: fmt.date(settings.prices_sync.at * 1000, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }), n: settings.prices_sync.models }) : t('prices.never')} · {t('prices.hint')}</p><div class="tbl prices"><div class="tw"><table><thead><tr><th>{t('models.model')}</th><th class="num">{t('set.colIn')}</th><th class="num">{t('set.colOut')}</th><th class="num">{t('set.colCacheR')}</th><th class="num">{t('set.colCacheW')}</th><th class="num">{t('set.colWindow')}</th></tr></thead><tbody>{#each Object.entries(settings.prices) as [id, p] (id)}<tr><td class="mono">{id}</td>{#if p.note}<td class="note" colspan="4">{p.note}</td>{:else}<td class="num" data-l={t('set.colIn')}>${p.input}</td><td class="num" data-l={t('set.colOut')}>${p.output}</td><td class="num" data-l={t('set.colCacheR')}>${p.cache_read}</td><td class="num" data-l={t('set.colCacheW')}>${p.cache_write}</td>{/if}<td class="num win" data-l={t('set.colWindow')}>{p.context ? (p.context >= 1e6 ? `${Math.round(p.context / 1e5) / 10}M` : `${Math.round(p.context / 1000)}k`) : '—'}</td></tr>{/each}</tbody></table></div></div></div>{/if}
        {:else if ui.settingsTab === 'notify'}
          <div class="card-s fs"><h3>{t('set.notifyTitle')}</h3><p>{t('set.notifyHint')}</p>
            {#if !notify}<button class="sm" style="justify-self: start" onclick={showNotify}><Icon name="bell" size="s" /> {t('set.notifyShow')}</button>
            {:else if notify.app}<div class="frm"><span>{t('set.notifyServer')}</span><code>{notify.server ?? '—'}</code><span>{t('set.notifyTopic')}</span><code>{notify.topic}</code><span>{t('set.notifyUser')}</span><code>{notify.user}</code><span>{t('set.notifyPassword')}</span><span class="row"><code>{notify.password ?? '—'}</code>{#if notify.password}<button class="icon xs ghost" aria-label={t('set.copy')} data-tip={t('set.copy')} onclick={() => navigator.clipboard?.writeText(notify!.password!)}><Icon name="copy" size="s" /></button>{/if}</span></div><p class="muted sm">{t('agents.notifyHow')}</p>
            {:else}<p>{t('set.notifyNo')}</p>{/if}
          </div>
          <div class="card-s fs push" style="margin-top: 14px"><h3>{t('push.title')}</h3><p>{t('push.hint')}</p>
            {#if !pushSupported}<p class="muted sm push-unsupported">{t('push.unsupported')}</p>
            {:else}
              <div class="row push-state" style="gap: 8px; flex-wrap: wrap; align-items: center">
                <span class="chip push-status" class:ok={!!pushMine} data-perm={pushPerm}>{pushMine ? t('push.on') : pushPerm === 'denied' ? t('push.deniedShort') : t('push.off')}</span>
                {#if pushMine}<button class="sm push-off" disabled={pushBusy} onclick={disablePush}>{t('push.disable')}</button><button class="sm push-test" disabled={pushBusy} onclick={testPush}><Icon name="bell" size="s" /> {t('push.test')}</button>
                {:else}<button class="sm primary push-on" disabled={pushBusy || pushPerm === 'denied'} onclick={enablePush}><Icon name="bell" size="s" /> {t('push.enable')}</button>{/if}
              </div>
              {#if pushErr}<p class="sm bad push-err" role="alert">{pushErr}</p>{/if}
              {#if pushInfo}
                <fieldset class="frm push-prefs" style="margin-top: 12px"><legend>{t('push.kinds')}</legend>
                  {#each [['permission', t('push.kindPermission')], ['done', t('push.kindDone')], ['error', t('push.kindError')], ['limit', t('push.kindLimit')]] as [k, l] (k)}
                    <span></span><label class="row" style="gap: 8px"><input type="checkbox" class="push-pref" data-kind={k} checked={pushInfo.prefs[k as keyof PushPrefs]} onchange={(e) => setPushPref(k as keyof PushPrefs, e.currentTarget.checked)} /> {l}</label>
                  {/each}
                </fieldset>
                <div class="push-devices" style="margin-top: 12px">
                  <p class="muted sm">{t('push.devices', { n: pushInfo.count })}</p>
                  {#each pushInfo.subscriptions as sub (sub.endpoint)}
                    <div class="row push-device" class:mine={sub.endpoint === pushMine} style="gap: 8px; align-items: center; flex-wrap: wrap">
                      <span class="mono sm">{sub.host}…{sub.tail}</span><span class="muted sm">{sub.ua || '—'}{#if sub.endpoint === pushMine} · {t('push.thisDevice')}{/if}{#if sub.fails} · {t('push.fails', { n: sub.fails })}{/if}</span>
                      <button class="xs ghost push-device-drop" onclick={() => dropPushSub(sub.endpoint)}>{t('push.drop')}</button>
                    </div>
                  {/each}
                </div>
              {/if}
              <p class="muted sm" style="margin-top: 10px">{t('push.ios')}</p>
            {/if}
          </div>
        {:else if ui.settingsTab === 'mcp'}
          <div class="card-s fs mcp"><h3>{t('mcp.title')}</h3><p>{t('mcp.hint')}</p>
            <div class="frm"><label for="mcp-workspace">{t('rt.workspace')}</label><select id="mcp-workspace" class="sm" value={mcpWs} disabled={mcpBusy || !data.running.length} onchange={(e) => changeMcp(e.currentTarget.value)}>{#each data.running as p (p.name)}<option value={p.name}>{p.name}</option>{/each}</select></div>
            {#if mcpWs}<p class="save-target">{t('sx.target', { target: mcpWs })}</p>{:else}<p class="muted sm">{t('sx.noWorkspace')}</p>{/if}
            {#if mcpLoading}<p class="muted sm" role="status">{t('common.loading')}</p>{/if}
            {#if mcpErr}<div class="load-error" role="alert"><span>{mcpErr}</span><button class="sm" disabled={mcpLoading || mcpBusy} onclick={loadMcp}>{t('sx.retry')}</button></div>{/if}
            {#if mcp && mcpLoaded === mcpWs}
              {#if mcp.servers.length}<div class="row mcp-health-bar" style="gap: 8px; margin: 4px 0 6px; flex-wrap: wrap; align-items: center"><button class="xs mcp-health-check" disabled={mcpHealthBusy || !mcpReady} onclick={() => checkMcpHealth(true)}><Icon name="refresh" size="s" /> {mcpHealthBusy ? t('mcph.checking') : t('mcph.check')}</button><span class="muted xs">{t('mcph.hint')}</span>{#if mcpHealthErr}<span class="sm bad" role="alert">{mcpHealthErr}</span>{/if}</div>{/if}
              <div class="mcp-list">
                {#each mcp.servers as s_ (s_.name)}<div class="mcp-row" data-mcp={s_.name}><span class="mono"><b>{s_.name}</b> <span class="muted">· {s_.transport}</span></span><span class="muted ell mono">{s_.transport === 'stdio' ? [s_.command, ...s_.args].join(' ') : s_.url}</span>{#if mcpHealthWs === mcpWs && mcpHealth[s_.name]}<span class="chip mcp-health {mcpHealth[s_.name].status}" title={mcpHealth[s_.name].detail}>{healthLabel(mcpHealth[s_.name])}</span>{/if}{#if s_.transport !== 'stdio'}<button class="xs mcp-login" aria-label={`${t('mcpa.login')} · ${s_.name}`} disabled={!mcpReady} onclick={() => mcpLogin(s_.name, 'claude')}><Icon name="key" size="s" /> {t('mcpa.login')}</button>{/if}<button class="xs ghost" aria-label={t('sx.removeMcp', { name: s_.name })} disabled={!mcpReady} onclick={() => removeMcp(s_.name)}>{t('mcp.remove')}</button></div>{:else}<div class="muted sm">{t('mcp.empty')}</div>{/each}
              </div>
              {#if mcpAuth}
                <div class="card-s mcp-auth" style="margin-top: 10px">
                  <div class="row" style="gap: 8px; align-items: center; flex-wrap: wrap"><b>{t('mcpa.title', { name: mcpAuth.name })}</b><span class="seg"><button class:active={mcpAuth.engine === 'claude'} onclick={() => mcpLogin(mcpAuth!.name, 'claude')}>Claude Code</button><button class:active={mcpAuth.engine === 'codex'} onclick={() => mcpLogin(mcpAuth!.name, 'codex')}>Codex</button></span><span class="sp"></span><button class="xs ghost" onclick={() => mcpLogin(mcpAuth!.name, mcpAuth!.engine, true)}>{t('mcpa.logout')}</button><button class="xs ghost" onclick={mcpAuthClose}>{t('mcpa.close')}</button></div>
                  <p class="muted sm" style="margin: 6px 0">{t('mcpa.hint')}</p>
                  {#if mcpAuth.error}<div class="err">{t('mcpa.failed', { e: mcpAuth.error })}</div>{/if}
                  {#if mcpAuth.pty}<div class="login-term">{#key mcpAuth.pty}<Term project={mcpWs} id={mcpAuth.pty} onexit={(code) => { if (mcpAuth) mcpAuth = { ...mcpAuth, done: true, code }; }} />{/key}</div>{:else if !mcpAuth.error}<p class="muted sm">{t('mcpa.starting')}</p>{/if}
                  {#if mcpAuth.pty && !mcpAuth.done}<div class="row" style="gap: 8px; margin-top: 8px"><input class="sm mono mcp-auth-paste" style="flex: 1" placeholder={t('mcpa.paste')} aria-label={t('mcpa.paste')} bind:value={mcpAuth.paste} onkeydown={(e) => { if (e.key === 'Enter') mcpAuthPaste(); }} /><button class="sm" disabled={!mcpAuth.paste.trim()} onclick={mcpAuthPaste}>{t('mcpa.pasteDo')}</button></div>{/if}
                  {#if mcpAuth.done}<p class="muted sm" style="margin-top: 8px">{t('mcpa.done', { code: mcpAuth.code ?? 0 })}</p>{/if}
                </div>
              {/if}
              <div class="frm mcp-form">
                <label for="mcp-name">{t('mcp.name')}</label><input id="mcp-name" class="sm mono name" disabled={!mcpReady} bind:value={mcpForm.name} placeholder="context7" />
                <label for="mcp-transport">{t('mcp.transport')}</label><select id="mcp-transport" class="sm transport" disabled={!mcpReady} bind:value={mcpForm.transport}><option value="stdio">stdio</option><option value="http">http</option><option value="sse">sse</option></select>
                {#if mcpForm.transport === 'stdio'}
                  <label for="mcp-command">{t('mcp.command')}</label><input id="mcp-command" class="sm mono command" disabled={!mcpReady} bind:value={mcpForm.command} placeholder="npx -y @modelcontextprotocol/server-everything" />
                  <label for="mcp-env">{t('mcp.env')}</label><textarea id="mcp-env" class="mono env" rows="2" disabled={!mcpReady} bind:value={mcpForm.env} placeholder="API_KEY=…"></textarea>
                {:else}
                  <label for="mcp-url">{t('mcp.url')}</label><input id="mcp-url" class="sm mono url" disabled={!mcpReady} bind:value={mcpForm.url} placeholder="https://mcp.example.com/mcp" />
                  <label for="mcp-headers">{t('mcp.headers')}</label><textarea id="mcp-headers" class="mono headers" rows="2" disabled={!mcpReady} bind:value={mcpForm.headers} placeholder="Authorization: Bearer …"></textarea>
                {/if}
                <span></span><span class="inl"><button class="sm primary add" disabled={!mcpReady || !mcpForm.name.trim() || (mcpForm.transport === 'stdio' ? !mcpForm.command.trim() : !mcpForm.url.trim())} onclick={addMcp}>{mcpBusy ? t('sx.saving') : t('mcp.add')}</button></span>
              </div>
              <p class="muted sm engines">{t('mcp.engines', { c: mcp.engines.claude ?? '—', x: mcp.engines.codex ?? '—', g: mcp.engines.gemini ?? '—' })}{#if mcpApplied} · {Object.entries(mcpApplied).map(([k, v]) => `${k}: ${v}`).join(' · ')}{/if}</p>
              <p class="muted xs">{t('mcp.newSessions')}</p>
            {/if}
          </div>
        {:else if ui.settingsTab === 'perm'}
          <div class="card-s fs"><h3>{t('set.policyTitle')}</h3><p>{t('set.policyHint')}</p>
            <div class="frm"><span>{t('set.policyBypass')}</span><span class="row" style="gap: 10px"><button class="switch" aria-label={t('set.policyBypass')} class:on={policy?.bypass} aria-pressed={!!policy?.bypass} disabled={!policy || policyBusy || policyLoading} onclick={() => setPolicy(!policy?.bypass)}></button><span class="muted sm">{policy ? (policy.bypass ? t('set.policyOn') : t('set.policyOff')) : policyError ? t('sx.settingsFailed') : t('common.loading')}</span></span></div>
            {#if policyError}<div class="load-error" role="alert"><span>{policyError}</span><button class="sm" disabled={policyLoading || policyBusy} onclick={loadPolicy}>{t('sx.retry')}</button></div>{/if}
          </div>
          <div class="card-s fs rules"><h3>{t('perm.rulesTitle')}</h3><p>{t('perm.rulesHint')}</p>
            <div class="frm">
              <label for="rules-workspace">{t('rt.workspace')}</label><select id="rules-workspace" class="sm" value={rules.ws} disabled={rulesBusy || !data.running.length} onchange={(e) => changeRules(e.currentTarget.value)}>{#each data.running as p (p.name)}<option value={p.name}>{p.name}</option>{/each}</select>
              <span>{t('perm.scope')}</span><span class="seg"><button class:active={rules.scope === 'user'} disabled={rulesBusy} aria-pressed={rules.scope === 'user'} onclick={() => changeRules(rules.ws, 'user')}>{t('perm.scopeUserShort')}</button><button class:active={rules.scope === 'project'} disabled={rulesBusy} aria-pressed={rules.scope === 'project'} onclick={() => changeRules(rules.ws, 'project')}>{t('perm.scopeProjectShort')}</button></span>
              <label for="rules-allow">{t('perm.allow')}</label><textarea id="rules-allow" class="mono" rows="4" disabled={!rulesReady} bind:value={rules.allow} placeholder="Bash(npm test:*)&#10;Read"></textarea>
              <label for="rules-deny">{t('perm.deny')}</label><textarea id="rules-deny" class="mono" rows="3" disabled={!rulesReady} bind:value={rules.deny} placeholder="Bash(rm -rf:*)"></textarea>
              <label for="rules-ask">{t('perm.ask')}</label><textarea id="rules-ask" class="mono" rows="2" disabled={!rulesReady} bind:value={rules.ask}></textarea>
              <span></span><span class="opts">{#each RULE_CHIPS as c (c)}<button class="xs" disabled={!rulesReady} onclick={() => (rules.allow = (rules.allow.trim() ? rules.allow.trimEnd() + '\n' : '') + c)}>+ {c}</button>{/each}</span>
            </div>
            {#if rules.ws}<p class="save-target">{t('sx.target', { target: rulesTarget })}</p>{:else}<p class="muted sm">{t('sx.noWorkspace')}</p>{/if}
            {#if rulesLoading}<p class="muted sm" role="status">{t('common.loading')}</p>{/if}
            {#if rulesErr}<div class="load-error" role="alert"><span>{rulesErr}</span><button class="sm" disabled={rulesLoading || rulesBusy} onclick={loadRules}>{t('sx.retry')}</button></div>{/if}
            <div class="row" style="gap: 10px; flex-wrap: wrap"><button class="sm primary" disabled={!rulesReady} onclick={saveRules}>{rulesBusy ? t('sx.saving') : t('common.save')}</button><span class="muted xs mono ell">{rules.path ?? ''}</span></div>
            <p class="muted sm" style="margin-top: 0">{t('perm.codexNote')}</p>
          </div>
          <div class="card-s fs"><h3>{t('set.permTitle')}</h3><p>{t('set.permHint')}</p>
            {#if settings}<div class="frm"><span>{t('set.cacheClaude')}</span><select class="sm" aria-label={t('set.cacheClaude')} value={perm.cache_claude ?? 'auto'} onchange={(e) => save({ defaults: { ...settings!.defaults, cache_claude: (e.target as HTMLSelectElement).value } })}><option value="auto">auto</option><option value="5m">5m</option><option value="1h">1h</option></select><span class="muted xs" style="grid-column: 1 / -1">{t('set.cacheClaudeHint')}</span></div>
            <div class="frm"><span>{t('set.permClaude')}</span><select class="sm" aria-label={t('set.permClaude')} value={perm.perm_claude ?? 'default'} onchange={(e) => save({ defaults: { ...settings!.defaults, perm_claude: (e.target as HTMLSelectElement).value } })}>{#each MODES_FOR.claude as x (x)}<option value={x}>{modeLabel(x)}</option>{/each}</select><span>{t('set.permCodex')}</span><select class="sm" aria-label={t('set.permCodex')} value={perm.perm_codex ?? 'on-request'} onchange={(e) => save({ defaults: { ...settings!.defaults, perm_codex: (e.target as HTMLSelectElement).value } })}>{#each MODES_FOR.codex as x (x)}<option value={x}>{modeLabel(x)}</option>{/each}</select><span>{t('set.permGemini')}</span><select class="sm" aria-label={t('set.permGemini')} value={perm.perm_gemini ?? 'default'} onchange={(e) => save({ defaults: { ...settings!.defaults, perm_gemini: (e.target as HTMLSelectElement).value } })}>{#each MODES_FOR.gemini as x (x)}<option value={x}>{modeLabel(x)}</option>{/each}</select></div>{/if}
          </div>
        {:else if ui.settingsTab === 'look'}
          <Appearance />
        {:else if ui.settingsTab === 'browser'}
          <div class="card-s fs browser-card" style="max-width: 760px">
            <h3>{t('browser.title')}</h3>
            <p class="muted sm">{t('browser.intro')}</p>
            {#if !data.projects.length}<p class="muted sm">{t('browser.none')}</p>
            {:else}
              <div class="tbl browser-tbl"><div class="tw"><table><thead><tr><th>{t('browser.colWorkspace')}</th><th>{t('browser.colPreview')}</th><th class="num">{t('browser.colPort')}</th><th>{t('browser.colAgent')}</th></tr></thead><tbody>
                {#each data.projects as p (p.name)}
                  <tr data-project={p.name}>
                    <td class="mono">{p.name}</td>
                    <td><button class="switch browser-preview" class:on={!!data.browserOf(p.name).preview} aria-pressed={!!data.browserOf(p.name).preview} aria-label={t('browser.colPreview')} disabled={browserBusy === p.name} onclick={() => toggleBrowserPreview(p.name)}></button></td>
                    <td class="num"><input class="sm mono browser-port" type="number" min="1" max="65535" aria-label={t('browser.colPort')} value={data.browserOf(p.name).port ?? 3000} disabled={browserBusy === p.name} onchange={(e) => setBrowserPort(p.name, Number((e.currentTarget as HTMLInputElement).value))} /></td>
                    <td><button class="switch browser-agent" class:on={!!data.browserOf(p.name).agent} aria-pressed={!!data.browserOf(p.name).agent} aria-label={t('browser.colAgent')} disabled={browserBusy === p.name} onclick={() => toggleAgentBrowser(p.name)}></button></td>
                  </tr>
                {/each}
              </tbody></table></div></div>
            {/if}
          </div>
        {:else if ui.settingsTab === 'voice'}
          <div class="card-s fs voice" style="max-width: 760px">
            <h3>{t('voice.title')}</h3>
            <p class="muted sm">{t('voice.intro')}</p>
            {#if voice.error}<div class="load-error" role="alert"><span>{voice.error}</span><button class="sm" disabled={voice.loading} onclick={() => voice.load(true)}>{t('sx.retry')}</button></div>{/if}
            {#if voice.loading && !voice.settings}<p class="muted sm" role="status">{t('common.loading')}</p>{/if}
            {#if voice.settings}
              <p class="sm voice-ready" class:ok={voice.settings.ready} role="status">{voice.settings.ready ? t('voice.readyYes', { provider: voice.settings.provider === 'openai' ? 'OpenAI' : 'ElevenLabs', model: voice.settings.model }) : t('voice.readyNo')}</p>
              <fieldset class="frm" disabled={vSaving}>
                <span>{t('voice.provider')}</span>
                <span class="seg voice-provider" role="group" aria-label={t('voice.provider')}>
                  <button type="button" class:active={vProvider === 'elevenlabs'} aria-pressed={vProvider === 'elevenlabs'} onclick={() => { vProvider = 'elevenlabs'; vCheck = null; }}>{t('voice.elevenlabs')}</button>
                  <button type="button" class:active={vProvider === 'openai'} aria-pressed={vProvider === 'openai'} onclick={() => { vProvider = 'openai'; vCheck = null; }}>{t('voice.openai')}</button>
                </span>
                <span>{t('voice.model')}</span>
                <span class="stack" style="gap: 6px">
                  <select class="sm voice-model" aria-label={t('voice.model')} bind:value={vModelSel[vProvider]}>{#each vModels as m (m)}<option value={m}>{m}{MODEL_HINT[m] ? ` — ${t(MODEL_HINT[m])}` : ''}</option>{/each}<option value="__custom">{t('voice.customModel')}</option></select>
                  {#if vModelSel[vProvider] === '__custom'}<input class="sm mono voice-model-custom" aria-label={t('voice.customModelLabel')} bind:value={vModelCustom[vProvider]} placeholder={vProvider === 'openai' ? 'whisper-large-v3-turbo' : 'scribe_v2'} autocomplete="off" />{/if}
                </span>
                <span>{t('voice.language')}<div class="hint">{t('voice.langHint')}</div></span>
                <span class="stack" style="gap: 6px">
                  <select class="sm voice-lang" aria-label={t('voice.language')} bind:value={vLangSel}><option value="">{t('voice.langAuto')}</option>{#each VOICE_LANGS as [c, n] (c)}<option value={c}>{n} · {c}</option>{/each}<option value="__other">{t('voice.langOther')}</option></select>
                  {#if vLangSel === '__other'}<input class="sm mono voice-lang-custom" aria-label={t('voice.langCode')} maxlength="3" bind:value={vLangCustom} placeholder="xx" autocomplete="off" />{/if}
                </span>
                {#each ['elevenlabs', 'openai'] as VProv[] as pr (pr)}
                  <span>{t('voice.keyFor', { provider: pr === 'openai' ? 'OpenAI' : 'ElevenLabs' })}</span>
                  <span class="stack keyrow" style="gap: 4px">
                    <input class="sm mono voice-key-{pr}" type="password" autocomplete="off" aria-label={t('voice.keyFor', { provider: pr === 'openai' ? 'OpenAI' : 'ElevenLabs' })} bind:value={vKeys[pr]} placeholder={t('voice.keyPlaceholder')} oninput={() => { if (vKeys[pr]) vClear[pr] = false; }} />
                    <span class="muted sm voice-key-state" data-provider={pr}>{#if voice.settings[pr].has_key && !vClear[pr]}{t('voice.keySaved', { tail: voice.settings[pr].key_tail })} · <button type="button" class="link voice-key-clear" onclick={() => { vClear[pr] = true; vKeys[pr] = ''; }}>{t('voice.clearKey')}</button>{:else if vClear[pr]}{t('voice.keyNone')} ({t('voice.clearKey')})  · <button type="button" class="link" onclick={() => (vClear[pr] = false)}>↶</button>{:else}{t('voice.keyNone')}{/if}</span>
                  </span>
                {/each}
                <span>{t('voice.base')}<div class="hint">{t('voice.baseHint')}</div></span>
                <input class="sm mono voice-base" aria-label={t('voice.base')} bind:value={vBase[vProvider]} placeholder={voice.settings[vProvider].default_base} autocomplete="off" />
              </fieldset>
              <div class="acts voice-acts">
                <button class="sm primary voice-save" disabled={vSaving || vChecking} onclick={saveVoice}>{vSaving ? t('sx.saving') : t('common.save')}</button>
                <button class="sm voice-check" disabled={vSaving || vChecking || !vHasKey} onclick={checkVoice}>{vChecking ? t('voice.checking') : t('voice.check')}</button>
                {#if vCheck}<span class="sm voice-check-result" class:ok={vCheck.ok} class:bad={!vCheck.ok} role="status">{vCheck.text}</span>{/if}
              </div>
              <div class="voice-sample">
                <span class="lbl">{t('voice.sample')}</span>
                <Mic onText={(txt, ms) => { vSample = txt; vSampleMs = ms; }} onPartial={(txt) => { vSample = txt; vSampleMs = 0; }} label={t('voice.hold')} cls="sm" />
                {#if vSample}<blockquote class="voice-sample-text">{vSample}</blockquote><span class="muted sm">{t('voice.sampleResult', { ms: vSampleMs, model: voice.settings.model })}</span>{:else}<span class="muted sm">{t('voice.sampleHint')}</span>{/if}
              </div>
              <fieldset class="frm voice-stream" style="margin-top: 14px">
                <legend>{t('voice.streamTitle')}</legend>
                <p class="muted sm" style="grid-column: 1 / -1; margin: 0 0 6px">{t('voice.streamIntro')}</p>
                <span></span>
                <label class="row voice-stream-on" style="gap: 8px"><input type="checkbox" bind:checked={vStream} /> {t('voice.streamOn')}{#if vStream && !vHasKey}<span class="muted sm"> · {t('voice.streamNeedsKey')}</span>{/if}</label>
                <span>{t('voice.streamModel')}</span>
                <span class="row" style="gap: 8px; flex-wrap: wrap">
                  {#if vProvider === 'openai'}
                    <input class="sm mono voice-stream-model" aria-label={t('voice.streamModel')} list="voice-stream-models" placeholder={vStreamModels[0] ?? 'gpt-live-transcribe'} bind:value={vStreamModel.openai} />
                    <datalist id="voice-stream-models">{#each vStreamModels as m (m)}<option value={m}></option>{/each}</datalist>
                  {:else}
                    <span class="mono voice-stream-model-fixed">{vStreamModels[0] ?? 'scribe_v2_realtime'}</span>
                  {/if}
                  <span class="muted sm">{t('voice.streamModelHint')}</span>
                </span>
              </fieldset>
              <fieldset class="frm voice-tts" style="margin-top: 14px">
                <legend>{t('voice.ttsTitle')}</legend>
                <p class="muted sm" style="grid-column: 1 / -1; margin: 0 0 6px">{t('voice.ttsIntro', { n: voice.settings.tts?.max_chars ?? 2000 })}</p>
                <span>{t('voice.ttsModel')}</span>
                <select class="sm voice-tts-model" aria-label={t('voice.ttsModel')} bind:value={vTtsModel[vProvider]}><option value="">{t('voice.ttsDefault', { m: vTtsModels[0] ?? '' })}</option>{#each vTtsModels as m (m)}<option value={m}>{m}</option>{/each}</select>
                <span>{t('voice.ttsVoice')}</span>
                <span class="row voice-tts-voice-row" style="gap: 6px; flex-wrap: wrap">
                  {#if vProvider === 'openai'}
                    <select class="sm voice-tts-voice" aria-label={t('voice.ttsVoice')} bind:value={vTtsVoice.openai}><option value="">{t('voice.ttsDefault', { m: voice.settings.tts?.default_voice?.openai ?? 'nova' })}</option>{#each voice.settings.tts?.openai_voices ?? [] as v (v)}<option value={v}>{v}</option>{/each}</select>
                  {:else}
                    <select class="sm voice-tts-voice" aria-label={t('voice.ttsVoice')} bind:value={vTtsVoice.elevenlabs} disabled={!vVoices.length}>
                      <option value="">{t('voice.ttsDefault', { m: voice.settings.tts?.default_voice?.elevenlabs ?? 'Sarah' })}</option>
                      {#if vTtsVoice.elevenlabs && !vVoices.some(v => v.id === vTtsVoice.elevenlabs)}<option value={vTtsVoice.elevenlabs}>{vTtsVoice.elevenlabs}</option>{/if}
                      {#each vVoices as v (v.id)}<option value={v.id}>{v.name}{v.desc ? ` — ${v.desc}` : ''}</option>{/each}
                    </select>
                    <button type="button" class="sm voice-tts-voices" disabled={vVoicesBusy || !vHasKey} onclick={loadVoices}>{vVoicesBusy ? t('common.loading') : t('voice.ttsLoadVoices')}</button>
                    {#if vVoicesErr}<span class="sm bad" role="alert">{vVoicesErr}</span>{/if}
                  {/if}
                </span>
                <span></span>
                <span class="row" style="gap: 8px">
                  <button type="button" class="sm voice-tts-try" disabled={vSpeakBusy || vSaving || !vHasKey} aria-pressed={voice.speaking === 'settings-sample'} onclick={trySpeak}><Icon name={voice.speaking === 'settings-sample' ? 'square' : 'volume'} size="s" /> {voice.speaking === 'settings-sample' ? t('voice.stop') : vSpeakBusy ? t('common.loading') : t('voice.ttsTry')}</button>
                  <span class="muted sm">{t('voice.ttsTryHint')}</span>
                </span>
              </fieldset>
              <p class="muted sm">{t('voice.privacy')}</p>
            {/if}
          </div>
        {:else}
          <div class="card-s fs"><h3>{t('set.aboutTitle')}</h3><div class="frm"><span>{t('set.service')}</span><span>cloudos-agents {data.health?.version ?? '—'}</span><span>{t('set.core')}</span><span>cloudd {data.health?.cloudd_version ?? (data.health?.cloudd ? 'ok' : t('api.offline'))}</span><span>agentd</span><span>{data.running.filter(p => p.reachable).length} {t('set.of')} {data.running.length} {t('set.aboutWs')}</span><span>{t('hub.events')}</span><span class="hub-state">{data.hubLive ? t('hub.live') : t('hub.poll')}{#if data.health?.hub_subscriptions != null} · {t('hub.subs', { n: data.health.hub_subscriptions })}{/if}</span><span>{t('set.docs')}</span><span><a href="https://github.com/" target="_blank" rel="noopener">docs/agents-ux.md</a> · ADR-006</span></div></div>
          <div class="card-s fs" style="margin-top: 14px"><h3>{t('set.uiTitle')}</h3><p>{t('set.uiHint')}</p>
            <p class="muted sm chat-cache-line">{t('set.chatCache', { n: chatCacheN })} · {t('set.chatCacheHint')}</p>
            {#if storageUse}<p class="muted sm storage-line">{t('set.storage', { used: fmtMb(storageUse.used), quota: fmtMb(storageUse.quota) })} · {data.storagePersisted ? t('set.storagePersistent') : t('set.storageBestEffort')}{#if data.storagePersisted === false || data.storagePersisted === null} <button class="xs ghost storage-persist" onclick={() => data.persistStorage().then(loadStorage)}>{t('set.storageAsk')}</button>{/if}</p>{/if}
            <div class="frm"><span>{t('set.uiBuild')}</span><span class="mono">{data.uiBuild() || '—'}{#if data.uiServerBuild && data.uiServerBuild !== data.uiBuild()} · <span style="color: var(--warn)">{t('set.uiServer', { b: data.uiServerBuild })}</span>{/if}</span></div>
            <div class="row" style="gap: 8px; margin-top: 8px; flex-wrap: wrap"><button class="sm primary reset-cache" onclick={hardReload}><Icon name="refresh" size="s" /> {t('set.resetCache')}</button><button class="sm ghost" onclick={() => data.checkUiFresh().then(() => toast(data.uiStale ? t('upd.banner') : t('set.uiFresh')))}>{t('set.uiCheck')}</button></div>
            <!-- install to the home screen: a button when the browser gave beforeinstallprompt; otherwise a per-platform
                 hint -->
            <div class="install" style="margin-top: 14px">
              {#if data.installed}<p class="muted sm"><Icon name="check" size="s" /> {t('set.installDone')}</p>
              {:else if data.installPrompt}<button class="sm primary install-btn" onclick={async () => { const p = data.installPrompt!; try { await p.prompt(); } catch {} }}><Icon name="download" size="s" /> {t('set.install')}</button><p class="muted sm" style="margin-top: 6px">{t('set.installHint')}</p>
              {:else}<p class="muted sm install-hint">{t('set.installManual')}</p>{/if}
              {#if agentsUrl && !onSeparate}<p class="muted sm install-separate" style="margin-top: 6px"><Icon name="download" size="s" /> {t('set.installSeparate')} <a class="install-url mono" href={agentsUrl}>{agentsUrl}</a> <button type="button" class="xs ghost install-copy" onclick={copyInstallUrl}>{t('set.installCopy')}</button><br><span>{t('set.installSeparateHint')}</span></p>
              {:else if onSeparate}<p class="muted sm install-separate-here"><Icon name="check" size="s" /> {t('set.installSeparateHere')}</p>{/if}
            </div>
          </div>
        {/if}
      </div>
    </div>
  </div>
</main>

<style>
  fieldset.frm { border: 0; padding: 0; min-width: 0; margin: 0 0 14px; }
  .prices-head { display: flex; align-items: flex-start; gap: 10px; flex-wrap: wrap; } .prices-head > div { flex: 1 1 240px; min-width: 0; } .prices-head .prices-sync { flex: none; }
  .prices-synced { margin: 2px 0 8px; overflow-wrap: anywhere; }
  .voice-ready { color: var(--fg-3); } .voice-ready.ok { color: var(--ok); }
  .browser-tbl .browser-port { width: 92px; }
  .voice-acts { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .voice-check-result { flex: 1 1 100%; overflow-wrap: anywhere; } .voice-check-result.ok { color: var(--ok); } .voice-check-result.bad { color: var(--bad); }
  .voice .keyrow .link, .voice-key-state .link { background: none; border: 0; padding: 0; height: auto; min-height: 0 !important; color: var(--accent); text-decoration: underline; cursor: pointer; font: inherit; }
  .voice-sample { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin: 14px 0 10px; padding-top: 12px; border-top: 1px solid var(--line-2); }
  .voice-sample .lbl { color: var(--fg-2); font-size: 12px; }
  .voice-sample-text { flex: 1 1 100%; margin: 0; padding: 8px 10px; border-left: 3px solid var(--accent); background: var(--bg-3); border-radius: 0 var(--r-1) var(--r-1) 0; overflow-wrap: anywhere; }
  .save-target { color: var(--fg-2); font-size: 12px; overflow-wrap: anywhere; }
  .load-error { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; color: var(--bad); font-size: 13px; padding: 10px 0; }
  .load-error > span { flex: 1 1 180px; overflow-wrap: anywhere; }
  .frm > label { color: var(--fg-2); }
  .opts { display: flex; flex-wrap: wrap; gap: 6px; }
  @media (pointer: coarse) { .settings button:not(.switch), .settings select, .settings input:not([type="checkbox"]):not([type="radio"]) { min-height: 44px; } .settings .radios label { min-height: 44px; } }
  @media (max-width: 599px) { .settings :global(.page-h) { flex-wrap: wrap; gap: 8px; } .settings :global(.page-h > div) { flex: 1 1 100%; } .frm > label { margin-top: 4px; } }
  .devcode { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .devcode .code { font-size: 22px; letter-spacing: .12em; padding: 6px 14px; border: 1px dashed var(--line-1); border-radius: var(--r-2); background: var(--bg-3); }
  .gate .gtarget { border-top: 1px solid var(--line-2); padding: 10px 0 4px; }
  .gate .gtarget:first-of-type { border-top: 0; }
  .glogin-row { display: grid; grid-template-columns: 24px 90px minmax(0, 1fr) auto auto; align-items: center; gap: 8px; padding: 5px 0; font-size: 13px; }
  .glogin-row.off { opacity: .6; }
  .glogin-row .gp { font-family: var(--mono); font-size: 12px; color: var(--fg-2); }
  .glogin-row .pill { display: inline-flex; align-items: center; height: 20px; padding: 0 8px; border-radius: 999px; font-size: 11px; background: rgb(var(--ok-rgb, 80 200 120) / .15); color: var(--ok); white-space: nowrap; max-width: 220px; overflow: hidden; text-overflow: ellipsis; }
  .glogin-row .pill.off { background: var(--bg-4); color: var(--fg-3); } .glogin-row .pill.warn { background: rgb(242 184 75 / .15); color: var(--warn); }
  .glogin-row .gacts { display: inline-flex; gap: 4px; align-items: center; }
  .glims { padding: 0 0 6px 32px; }
  @media (max-width: 599px) { .glogin-row { grid-template-columns: 24px minmax(0, 1fr) auto; } .glogin-row .gp { display: none; } .glogin-row .av { grid-column: 1; grid-row: 1; } .glogin-row .ell { grid-column: 2; grid-row: 1; } .glogin-row .pill { grid-column: 2; grid-row: 2; justify-self: start; max-width: 100%; } .glogin-row .gacts { grid-column: 3; grid-row: 1 / 3; } .glims { padding-left: 0; } .gate .gtarget .ghint { flex: 1 1 160px; min-width: 0; } .gate .gtarget .row > b { overflow-wrap: anywhere; } }
  .gflow { margin-top: 8px; padding: 10px 12px; border: 1px solid var(--line-1); border-radius: var(--r-2); background: var(--bg-2); }
  .login-term { height: 360px; margin-top: 10px; }
  .ok-note { color: var(--ok); display: inline-flex; align-items: center; gap: 6px; font-size: 12.5px; }
  .err { color: var(--bad); font-size: 12.5px; }
</style>
