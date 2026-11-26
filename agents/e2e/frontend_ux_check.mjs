#!/usr/bin/env node
/**
 * Isolated reliability regressions and responsive UX checks for Agents.
 * Start Vite separately on 5179, then run this file. All API/WS traffic is mocked.
 * Optional: UI_URL, PLAYWRIGHT_MODULE, CHROME_PATH, FRONTEND_UX_OUT, TEST_FILTER.
 * FRONTEND_UX_STRICT=1 also checks touch target sizes and button names.
 * Compact phone composer, session-menu and session-tab buttons use 32px height; other touch targets use 44px.
 */
import assert from 'node:assert/strict';
import { mkdir, writeFile, access } from 'node:fs/promises';
import { homedir } from 'node:os';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const uiUrl = process.env.UI_URL ?? 'http://127.0.0.1:5179/agents/';
const origin = new URL(uiUrl).origin;
const out = resolve(process.env.FRONTEND_UX_OUT ?? '/tmp/frontend-ux-check');
let chromium;
for (const candidate of [process.env.PLAYWRIGHT_MODULE, 'playwright'].filter(Boolean)) {
  try { ({ chromium } = await import(candidate.startsWith('/') ? pathToFileURL(candidate).href : candidate)); break; } catch {}
}
assert(chromium, 'Install Playwright or set PLAYWRIGHT_MODULE.');
let executablePath = process.env.CHROME_PATH;
if (!executablePath) for (const candidate of [chromium.executablePath(), join(homedir(), '.cache/ms-playwright/chromium-1243/chrome-linux64/chrome'), join(homedir(), '.cache/puppeteer/chrome/linux-148.0.7778.97/chrome-linux64/chrome')]) {
  try { await access(candidate); executablePath = candidate; break; } catch {}
}
await mkdir(out, { recursive: true });
const browser = await chromium.launch({ headless: true, executablePath });
const results = []; const audit = [];
const ts = Math.floor(Date.now() / 1000);
const usage = { input: 2500, output: 1900, cache_read: 42000, cache_create: 2000, cost_usd: 4, turns: 8 };
const baseSession = { id: 'code-a', agent: 'claude', cwd: '/home/coder/app', title: 'Review the interface', created_at: ts - 600, updated_at: ts, model: 'claude-fable-5-1[1m]', permission_mode: 'default', effort: 'max', status: 'idle', alive: true, last_seq: 2, pending: 0, queued: [], usage, last_text: 'Ready to improve the frontend.' };
const caps = { provider: 'claude', mcp: 1, models: [{ value: baseSession.model, label: 'Fable' }], commands: [], efforts: ['low', 'medium', 'high', 'max'], modes: ['default', 'acceptEdits', 'plan', 'auto', 'bypassPermissions'], bypass_allowed: true, features: { commands: true, files: true, effort: true, fast: false, compact: true } };
const history = [{ seq: 1, ts, kind: 'user', content: [{ type: 'text', text: 'Improve the frontend reliability.' }] }, { seq: 2, ts: ts + 1, kind: 'assistant', content: [{ type: 'text', text: 'The previous answer should remain visible if a refresh fails.' }] }];
const simple = { id: 'simple-a', kind: 'simple', title: 'Discuss the interface', route: 'demo', model: 'demo-model', system: '', tools: { search: false, fetch: false }, incognito: false, created_at: ts, updated_at: ts, cost_usd: 0.02, tokens_in: 500, tokens_out: 200, messages: 2 };
const routes = { routes: [{ id: 'demo', kind: 'gate', title: 'Project gate', provider: 'anthropic', host: 'fixture.invalid', models: [{ id: 'demo-model', name: 'Demo model', context: 200000, provider: 'anthropic', provider_title: 'Claude', kind: 'chat' }] }], defaults: { route: 'demo', model: 'demo-model' }, search: { ready: true, kind: 'searxng' } };
const messages = [{ id: 'm1', chat: 'simple-a', seq: 1, role: 'user', text: 'Why preserve drafts?', attachments: [], tools: [], tokens_in: 0, tokens_out: 0, cost_usd: 0, created_at: ts }, { id: 'm2', chat: 'simple-a', seq: 2, role: 'assistant', text: 'A failed refresh should not discard the current conversation.', attachments: [], tools: [], tokens_in: 500, tokens_out: 200, cost_usd: .02, created_at: ts }];
const question = { request_id: 'question-a', seq: 3, tool_name: 'AskUserQuestion', input: { questions: [{ header: 'Layout', question: 'Which layout should we use?', multiSelect: false, options: [{ label: 'Compact', description: 'Keep related actions together.' }, { label: 'Spacious', description: 'Allow more space for reading.' }] }] } };
const permission = { request_id: 'permission-a', seq: 4, tool_name: 'Bash', input: { command: 'npm run check' }, suggestions: [] };
const baseAgent = { id: 'agent-a', name: 'Frontend reviewer', brief: 'Review interface changes.', provider: 'claude', model: '', account: '', workspace: 'east', mode: 'ask', effort: '', rules: { allow: [], deny: [] }, memory: '', remember: true, place: '', can_mail: false, can_routine: true, memory_budget: 6000, reflect: false, skills: [], created_at: ts, updated_at: ts, routines: 1 };
const baseRoutine = { id: 'routine-a', name: 'Review each morning', agent: 'agent-a', prompt: 'Review the changes.', trigger: { kind: 'schedule', cron: '0 9 * * 1-5', event: '', contains: '' }, budget: { run_usd: 1, day_usd: 5 }, notify: 'ask_error', enabled: true, mode: '', token: 'fixture', created_at: ts, updated_at: ts, agent_name: 'Frontend reviewer', workspace: 'east', next_at: ts + 3600, schedule_text: 'Weekdays at 09:00' };
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==', 'base64');
const nap = ms => new Promise(r => setTimeout(r, ms));

async function fixture({ section = 'work', view = 'thread', width = 1280, height = 800, touch = false, theme = 'dark', lang = 'en', settingsTab = 'accounts', routinesTab = 'agents', initialFailures = {}, cols = 3, zone = 'Asia/Tbilisi', zoneOffset = 240, runs = [], routine = {}, owner = false, channels = [] } = {}) {
  const sessions = [structuredClone(baseSession), { ...structuredClone(baseSession), id: 'code-b', title: 'Refine workspace settings' }, { ...structuredClone(baseSession), id: 'code-c', title: 'Review mobile navigation' }];
  const state = {
    calls: [], unexpected: [], pageErrors: [], failures: new Map(Object.entries(initialFailures)), delays: new Map(), sessions, searches: {}, scratchSessions: [{ ...structuredClone(baseSession), id: 'scratch-a', title: 'Advanced discussion' }],
    workspaces: { projects: [{ name: 'east', description: 'Main project', status: 'running', reachable: true, apps: [], cost_today: 3, sessions, ptys: [] }, { name: 'west', description: 'Other project', status: 'running', reachable: true, apps: [], cost_today: 1, sessions: [], ptys: [] }], today: usage },
    inbox: { waiting: [{ project: 'east', session: { ...baseSession, id: 'question-session', title: 'Layout question', status: 'waiting', pending: 1 }, pending: [question] }, { project: 'west', session: { ...baseSession, id: 'permission-session', title: 'Validate changes', status: 'waiting', pending: 1 }, pending: [permission] }], failed: [], done: [{ project: 'west', session: { ...baseSession, id: 'finished', title: 'Previous review', status: 'exited' } }], today: usage },
    chats: [structuredClone(simple)], agents: [structuredClone(baseAgent), { ...structuredClone(baseAgent), id: 'agent-b', name: 'Second reviewer' }], routines: [{ ...structuredClone(baseRoutine), ...structuredClone(routine) }], skills: [{ name: 'review-ux', description: 'Review interaction problems', body: 'Inspect the controls.', source: 'user', created_at: ts, updated_at: ts }],
    zone, zoneOffset, runs: structuredClone(runs),
    // notification channels (§9.12): the lists the service keeps, whether the Telegram code has arrived, a channel's journal
    channels: structuredClone(channels), tgFound: false, notifyLog: [], testError: null,
  };
  const context = await browser.newContext({ viewport: { width, height }, hasTouch: touch, isMobile: width < 600, serviceWorkers: 'block', timezoneId: 'Asia/Tbilisi' });
  // Playwright marks the browser as automated, and an automated browser never names the owner's time zone
  if (owner) await context.addInitScript(() => { Object.defineProperty(Navigator.prototype, 'webdriver', { get: () => false }); });
  await context.addInitScript(({ sessions, section, view, settingsTab, routinesTab, theme, lang, cols }) => {
    localStorage.setItem('agents.lang', lang); localStorage.setItem('agents.theme', theme);
    localStorage.setItem('agents.ui', JSON.stringify({ section, view, sel: { project: 'east', kind: 'session', id: 'code-a' }, cards: sessions.map(s => ({ project: 'east', kind: 'session', id: s.id, w: 1, h: 1 })), cols, inspector: false, chatSel: 'simple-a', chatEngine: 'simple', settingsTab, routinesTab, routinesSel: 'agent-a' }));
    const streams = [];
    class MockWebSocket {
      static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3; readyState = 0;
      constructor(url) { this.url = url; streams.push(this); setTimeout(() => { if (this.readyState === 3) return; this.readyState = 1; this.onopen?.({}); const id = new URL(url).pathname.split('/').at(-2); const info = sessions.find(s => s.id === id); if (info) this.onmessage?.({ data: JSON.stringify({ kind: 'hello', info, replay: 0 }) }); }, 25); }
      send() {} close() { this.readyState = 3; this.onclose?.({ code: 1006 }); }
    }
    window.WebSocket = MockWebSocket;
    window.__frontendFixture = { streams, emit: event => streams.filter(s => s.readyState === 1).forEach(s => s.onmessage?.({ data: JSON.stringify(event) })) };
  }, { sessions, section, view, settingsTab, routinesTab, theme, lang, cols });
  await context.route('**/*', async route => {
    const request = route.request(); const u = new URL(request.url()); const method = request.method();
    if (u.hostname === 'fonts.googleapis.com') return route.fulfill({ contentType: 'text/css', body: '' });
    if (u.origin !== origin) { state.unexpected.push(`${method} ${u.href}`); return route.abort(); }
    if (!u.pathname.includes('/api/')) return route.continue();
    const path = u.pathname.replace(/^\/agents\/api(?:\/core)?/, ''); const key = `${method} ${path}`;
    const body = request.postData() && request.headers()['content-type']?.includes('json') ? request.postDataJSON() : undefined;
    state.calls.push({ method, path, query: u.search, body });
    const exact = key + u.search; const delay = state.delays.get(exact) ?? state.delays.get(key) ?? 0;
    const failureKey = state.failures.has(exact) ? exact : key; const failures = state.failures.get(failureKey) ?? 0;
    if (failures > 0) { state.failures.set(failureKey, failures - 1); if (delay) await nap(delay); return route.fulfill({ status: 503, json: { error: state.failureText ?? `Fixture unavailable: ${path}` } }); }
    let result; let text;
    if (path === '/health') result = { ok: true, cloudd: true, version: 'fixture' };
    else if (path === '/workspaces') result = structuredClone(state.workspaces);
    else if (path === '/agents/layout') result = { cards: sessions.map(s => ({ project: 'east', kind: 'session', id: s.id, w: 1, h: 1 })), cols, groups: {} };
    else if (path === '/accounts') result = { accounts: [], defaults: {} };
    else if (path === '/settings/desktop') result = { appearance: { language: lang } };
    else if (path === '/settings' && method === 'PUT') { if (body?.timezone) { state.zone = body.timezone; state.zoneOffset = 240; } result = {}; }
    else if (path === '/settings') result = { prices: {}, defaults: { claude: '', codex: '' }, limits: [], mail_paused: false };
    else if (path === '/inbox') result = structuredClone(state.inbox);
    else if (path === '/models') { const days = Number(u.searchParams.get('days') ?? 1); const cost = days === 1 ? 4 : days === 7 ? 84 : 320; result = { period_days: days, total: { ...usage, cost_usd: cost }, by_day: Array.from({ length: days }, (_, i) => ({ ...usage, key: String(i) })), by_project: [{ ...usage, key: 'east', cost_usd: days === 1 ? 3 : 80 }], models: [{ id: baseSession.model, agent: 'Claude', ...usage }], routes: [], defaults: { claude: '', codex: '' }, limits: [{ scope: 'total', per_day: 10, action: 'warn' }, { scope: 'east', per_day: 5, action: 'warn' }], project_defaults: {} }; }
    else if (path === '/agents') result = { agents: structuredClone(state.agents) };
    else if (path === '/routines') result = { routines: structuredClone(state.routines), scheduler: true, now: ts, timezone: state.zone, utc_offset_min: state.zoneOffset };
    else if (path === '/runs') result = { runs: structuredClone(state.runs) };
    else if (path === '/skills') result = { skills: structuredClone(state.skills) };
    else if (path === '/scheduled') result = { scheduled: [], now: ts };
    else if (path === '/voice') result = { provider: 'openai', language: '', ready: false, model: 'fixture', elevenlabs: { model: 'fixture', base: '', default_base: '', has_key: false, key_tail: '' }, openai: { model: 'fixture', base: '', default_base: '', has_key: false, key_tail: '' }, models: { elevenlabs: ['fixture'], openai: ['fixture'] } };
    else if (path === '/agents/policy') result = { bypass: true };
    else if (path === '/gate/auth') result = { targets: [] };
    else if (path === '/gate/models') result = { targets: [{ app: 'demo', title: 'Demo', models: routes.routes[0].models }] };
    else if (path === '/agents/notify') result = { app: null, server: null, topic: '', user: '', password: null, token: null };
    else if (path === '/push') result = { public_key: '', count: 0, subscriptions: [], prefs: { permission: true, done: true, error: true, limit: true, memory: true } };
    else if (path === '/chat') result = { project: 'scratch', exists: true, status: 'running', reachable: true, sessions: structuredClone(state.scratchSessions), busy: false };
    else if (path === '/chat/routes') result = routes;
    else if (path === '/chat/presets') result = { presets: [] };
    else if (path === '/chats') result = { chats: structuredClone(state.chats) };
    else if (/^\/chats\/[^/]+$/.test(path)) { const chat = state.chats.find(c => c.id === path.split('/').at(-1)); result = method === 'GET' ? { chat, messages, busy: false } : chat; }
    else if (/^\/projects\/[^/]+\/mcp$/.test(path)) { const workspace = path.split('/')[2]; result = { servers: [{ name: `${workspace}-server`, transport: 'stdio', command: 'npx', args: [], url: '', env: {}, headers: {} }], engines: { claude: 1, codex: 1, gemini: 1 }, applied: {} }; }
    else if (/^\/projects\/[^/]+\/permissions$/.test(path)) { const workspace = path.split('/')[2]; const scope = u.searchParams.get('scope'); result = { scope, path: `/home/coder/${workspace}/${scope}.json`, exists: true, allow: [`Read(${workspace}:${scope})`], deny: [], ask: [] }; }
    else if (/^\/projects\/[^/]+\/fs$/.test(path)) { const p = u.searchParams.get('path'); result = { path: p, home: '/home/coder', entries: [{ name: 'alpha.txt', kind: 'file', size: 30, mtime: ts, link: false, mode: 420 }, { name: 'beta.txt', kind: 'file', size: 30, mtime: ts, link: false, mode: 420 }, { name: 'folder-a', kind: 'dir', size: 0, mtime: ts, link: false, mode: 493 }] }; }
    else if (path.endsWith('/fs/file')) text = `Content for ${u.searchParams.get('path')}`;
    else if (path.endsWith('/git/status')) result = { repo: true, cwd: u.searchParams.get('cwd'), head: 'main', upstream: null, ahead: 0, behind: 0, entries: [{ path: 'alpha.txt', x: '.', y: 'M', staged: false, unstaged: true, kind: 'changed' }, { path: 'beta.txt', x: '.', y: 'M', staged: false, unstaged: true, kind: 'changed' }] };
    else if (path.endsWith('/git/diff')) result = { diff: `--- a/${u.searchParams.get('path')}\n+++ b/${u.searchParams.get('path')}\n@@ -1 +1 @@\n-previous value\n+current value ${u.searchParams.get('path')}\n`, truncated: false };
    else if (path.endsWith('/git/log')) result = { entries: [] };
    else if (path.endsWith('/git/branches')) result = { head: 'main', branches: [{ name: 'main', current: true, short: 'abc123', ts, subject: 'Fixture' }], remotes: [] };
    else if (path.endsWith('/git/worktrees')) result = { cwd: '/home/coder/app', worktrees: [{ path: '/home/coder/app', name: 'app', main: true, current: true, bare: false, detached: false, locked: false, branch: 'main', changes: 2 }] };
    else if (path.endsWith('/ptys')) result = [];
    else if (path.endsWith('/engines')) result = { engines: [] };
    else if (path.endsWith('/events')) result = history;
    else if (/^\/projects\/[^/]+\/sessions\/[^/]+$/.test(path)) result = { info: [...sessions, ...state.scratchSessions].find(s => s.id === path.split('/').at(-1)) ?? baseSession, caps, pending: [] };
    else if (path.endsWith('/control')) result = { maxTokens: 1000000, totalTokens: 740000, percentage: 74 };
    else if (path.endsWith('/permission')) result = {};
    else if (path.endsWith('/prompt') && method === 'POST') result = { seq: 3, ts: ts + 2, kind: 'user', content: body.content };
    else if (/^\/(agents|routines|skills)\/[^/]+$/.test(path) && method === 'PUT') { const type = path.split('/')[1]; const id = path.split('/').at(-1); const item = state[type].find(item => (type === 'skills' ? item.name : item.id) === id); Object.assign(item, body); result = structuredClone(item); }
    else if (/^\/routines\/[^/]+\/token$/.test(path) && method === 'POST') { const item = state.routines.find(r => r.id === path.split('/')[2]); item.token = 'rotated'; result = structuredClone(item); }
    else if (/^\/skills\/[^/]+\/accept$/.test(path) && method === 'POST') { const item = state.skills.find(k => k.name === path.split('/')[2]); item.source = 'user'; result = structuredClone(item); }
    else if (/^\/(agents|routines)\/[^/]+\/run$/.test(path)) result = { run: { id: 'run-fixture', status: 'running', project: 'east', started_at: ts, cost_usd: 0 } };
    else if (path === '/search') result = structuredClone(state.searches[u.searchParams.get('q')] ?? { q: u.searchParams.get('q'), words: [], hits: [], stopped: [], total: 0 });
    else if (path === '/notify/channels' && method === 'GET') result = { channels: structuredClone(state.channels), rate_per_min: 20 };
    else if (path === '/notify/channels' && method === 'POST') {
      const c = { id: `ch_${state.channels.length + 1}`, kind: body.kind, name: body.name || (body.kind === 'telegram' ? 'Alex' : 'fixture.invalid'), enabled: true, bot: body.kind === 'telegram' ? '@stand_bot' : '', chat_title: body.kind === 'telegram' ? 'Alex' : '', chat_linked: body.kind === 'telegram', protect: body.protect ?? true, buttons: !!body.buttons,
        slack_channel: body.slack_channel ?? '', host: body.kind === 'telegram' ? '' : body.kind === 'slack_bot' ? 'slack.com' : 'fixture.invalid', tail: body.kind === 'telegram' ? '' : String(body.url ?? body.token ?? '').slice(-4), internal: !!body.internal, signed: body.kind === 'webhook' && body.signed !== false, created_at: ts, files: body.kind !== 'slack', bindings: [], sent_today: 0, last: null };
      state.channels.push(c); result = { ...structuredClone(c), ...(c.signed ? { signing_key: 'f'.repeat(64) } : {}) };
    }
    else if (/^\/notify\/channels\/[^/]+$/.test(path)) {
      const id = path.split('/').at(-1); const c = state.channels.find(x => x.id === id);
      if (method === 'DELETE') { state.channels = state.channels.filter(x => x.id !== id); result = null; }
      else { for (const k of ['name', 'enabled', 'protect', 'buttons', 'internal', 'slack_channel']) if (body[k] !== undefined) c[k] = body[k]; if (body.url || body.token) c.tail = String(body.url ?? body.token).slice(-4); result = structuredClone(c); }
    }
    else if (/^\/notify\/channels\/[^/]+\/test$/.test(path)) { const c = state.channels.find(x => x.id === path.split('/')[3]); if (!state.testError) c.sent_today += 1; c.last = { at: ts, status: state.testError ? 'failed' : 'sent', error: state.testError, event: 'test' }; result = { ok: !state.testError, error: state.testError }; }
    else if (/^\/notify\/channels\/[^/]+\/log$/.test(path)) result = { log: structuredClone(state.notifyLog) };
    else if (path === '/notify/telegram/begin') result = { pending: 'tg_fixture', bot: '@stand_bot', code: '483920' };
    else if (path === '/notify/telegram/find') result = state.tgFound ? { found: true, chat_title: 'Alex', chat_type: 'private' } : { found: false };
    else if (path === '/notify/bindings' && method === 'POST') {
      const c = state.channels.find(x => x.id === body.channel); const key = b => [b.scope, b.project ?? '', b.session ?? '', b.routine ?? ''].join('|');
      let b = c.bindings.find(x => key(x) === key(body));
      if (!b) { b = { id: `nb_${Date.now()}_${c.bindings.length}`, channel: c.id, scope: body.scope, project: body.scope === 'routine' ? '' : body.project ?? '', session: body.session ?? '', routine: body.routine ?? '', when: 'always', content: 'fact', reports: false, away_only: body.scope === 'workspace' || body.scope === 'all', created_at: ts }; c.bindings.push(b); }
      for (const k of ['when', 'content', 'reports', 'away_only']) if (body[k] !== undefined) b[k] = body[k];
      result = structuredClone(b);
    }
    else if (/^\/notify\/bindings\/[^/]+$/.test(path) && method === 'DELETE') { const id = path.split('/').at(-1); for (const c of state.channels) c.bindings = c.bindings.filter(b => b.id !== id); result = null; }
    else if (path === '/notify/share' && method === 'POST') result = { ok: true, channel: state.channels.find(c => c.id === body.channel)?.name ?? '' };
    else { state.unexpected.push(`${method} ${path}`); return route.fulfill({ status: 501, json: { error: `Missing fixture: ${method} ${path}` } }); }
    if (delay) await nap(delay);
    try { if (text !== undefined) await route.fulfill({ contentType: 'text/plain', body: text }); else await route.fulfill({ json: result }); } catch (error) { if (!/closed|disposed/i.test(error.message)) throw error; }
  });
  const page = await context.newPage(); page.setDefaultTimeout(6000); page.on('pageerror', error => state.pageErrors.push(error.message));
  await page.goto(uiUrl, { waitUntil: 'networkidle' });
  // Section is intentionally not restored from localStorage by the app.
  if (section !== 'work') {
    const names = lang === 'ru' ? { chat: 'Чат', inbox: 'Входящие', models: 'Модели', settings: 'Настройки', routines: 'Агенты' } : { chat: 'Chat', inbox: 'Inbox', models: 'Models', settings: 'Settings', routines: 'Agents' };
    if (width < 600 && ['settings', 'routines'].includes(section)) { await page.locator('.tabbar').getByRole('button', { name: lang === 'ru' ? 'Ещё' : 'More', exact: true }).click(); await page.getByRole('dialog').getByRole('button', { name: names[section], exact: true }).click(); }
    else await nav(page, names[section]);
    await page.waitForLoadState('networkidle');
  }
  return { page, context, state };
}
const calls = (state, method, path) => state.calls.filter(c => c.method === method && c.path === path);
function safe(state) { assert.deepEqual(state.unexpected, [], 'Every API request must use an explicit fixture.'); assert.deepEqual(state.pageErrors, [], 'No browser JavaScript errors.'); }
async function nav(page, name) { const links = page.locator('nav').getByRole('button', { name, exact: true }); for (let i = 0; i < await links.count(); i++) { if (await links.nth(i).isVisible()) return links.nth(i).click(); } throw Error(`Navigation missing: ${name}`); }
async function shot(page, name) { await page.screenshot({ path: join(out, `${name}.png`) }); }
async function scenario(name, run) { if (process.env.TEST_FILTER && !new RegExp(process.env.TEST_FILTER, 'i').test(name)) return; try { const detail = await run(); results.push({ name, pass: true, detail }); console.log(`PASS ${name}`); } catch (error) { results.push({ name, pass: false, error: error.stack }); console.error(`FAIL ${name}: ${error.message}`); } }
async function waitCalls(state, method, path, n) { for (let i = 0; i < 250; i++) { if (calls(state, method, path).length >= n) return; await nap(50); } assert.fail(`Missing request ${method} ${path}, expected ${n}.`); }

await scenario('Inbox failure survives successful workspace refresh and retry recovers', async () => {
  const { page, state, context } = await fixture({ section: 'inbox', initialFailures: { 'GET /inbox': Infinity } });
  try {
    await page.getByText(/Fixture unavailable: \/inbox/).waitFor();
    await waitCalls(state, 'GET', '/workspaces', 2);
    assert(await page.getByText(/Fixture unavailable: \/inbox/).isVisible(), 'Successful workspace polling must not erase Inbox errors.');
    state.failures.set('GET /inbox', 0); await page.getByRole('button', { name: 'retry', exact: true }).click();
    await page.locator('.inbox .item').filter({ hasText: 'Which layout should we use?' }).waitFor(); safe(state);
  } finally { await context.close(); }
});

await scenario('Transient simple-chat list error retains open conversation and draft', async () => {
  const { page, state, context } = await fixture({ section: 'chat' });
  try {
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.fill('Keep this draft during a refresh');
    state.failures.set('GET /chats', Infinity); const previous = calls(state, 'GET', '/chats').length;
    await waitCalls(state, 'GET', '/chats', previous + 1);
    await page.getByText(/Fixture unavailable: \/chats/).waitFor();
    assert.equal(await input.inputValue(), 'Keep this draft during a refresh');
    assert(await page.getByText(messages[1].text, { exact: true }).isVisible(), 'A failed list refresh must leave the conversation mounted.');
    safe(state);
  } finally { await context.close(); }
});

await scenario('Unavailable scratch does not hide a successful simple chat', async () => {
  const { page, state, context } = await fixture({ section: 'chat', initialFailures: { 'GET /chat': Infinity } });
  try { await page.getByText(messages[1].text, { exact: true }).waitFor(); assert(await page.getByRole('textbox', { name: 'Message', exact: true }).isVisible()); safe(state); }
  finally { await context.close(); }
});

await scenario('Chat-route failure offers retry without a request storm or losing the conversation', async () => {
  const { page, state, context } = await fixture({ section: 'chat', initialFailures: { 'GET /chat/routes': Infinity } });
  try {
    const error = page.locator('.load-warning').filter({ hasText: 'Fixture unavailable: /chat/routes' }); await error.waitFor();
    await page.getByText(messages[1].text, { exact: true }).waitFor(); const n = calls(state, 'GET', '/chat/routes').length; await nap(800);
    assert.equal(calls(state, 'GET', '/chat/routes').length, n, 'A failed route loader must not loop automatically.');
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.fill('Keep this while reloading the model route');
    state.failures.set('GET /chat/routes', 0); await error.getByRole('button', { name: 'retry', exact: true }).click(); await error.waitFor({ state: 'hidden' });
    assert.equal(await input.inputValue(), 'Keep this while reloading the model route'); assert(await page.getByRole('button', { name: 'Demo model' }).isVisible()); safe(state);
  } finally { await context.close(); }
});

await scenario('Initial conversation failure shows retry and opens the same chat after recovery', async () => {
  const { page, state, context } = await fixture({ section: 'chat', initialFailures: { 'GET /chats/simple-a': Infinity } });
  try {
    const error = page.locator('.load-warning').filter({ hasText: 'Fixture unavailable: /chats/simple-a' }); await error.waitFor();
    assert.equal(await page.locator('.msgs .m').count(), 0); state.failures.set('GET /chats/simple-a', 0); await error.getByRole('button', { name: 'retry', exact: true }).click();
    await page.getByText(messages[1].text, { exact: true }).waitFor(); await error.waitFor({ state: 'hidden' }); assert.equal(calls(state, 'GET', '/chats/simple-a').length, 2); safe(state);
  } finally { await context.close(); }
});

await scenario('Simple and advanced modes each remember their selected conversation', async () => {
  const { page, state, context } = await fixture({ section: 'chat' });
  try {
    await page.locator('[data-engine="advanced"]').click(); await page.locator('.panel [data-id="scratch-a"]').click(); await page.locator('.t-head h2').getByText('Advanced discussion', { exact: true }).waitFor();
    await page.locator('[data-engine="simple"]').click(); await page.locator('.t-head h2').getByText(simple.title, { exact: true }).waitFor();
    await page.locator('[data-engine="advanced"]').click(); await page.locator('.t-head h2').getByText('Advanced discussion', { exact: true }).waitFor(); safe(state);
  } finally { await context.close(); }
});

await scenario('Search failure marks previous results and retry replaces them', async () => {
  const { page, state, context } = await fixture();
  try {
    state.searches.draft = { q: 'draft', words: ['draft'], hits: [{ kind: 'session', project: 'east', id: 'code-a', title: 'Saved draft result', agent: 'claude', model: baseSession.model, ts: ts * 1000, role: 'assistant', snippet: 'Keep the reference image draft.', count: 1 }], stopped: [], total: 1 };
    const search = page.locator('.panel .search input'); await search.fill('draft'); await page.locator('.hits .hit').filter({ hasText: 'Saved draft result' }).waitFor();
    state.failures.set('GET /search?q=missing', 1); await search.fill('missing'); await page.locator('.search-error').waitFor();
    assert(await page.locator('.hits .hit').filter({ hasText: 'Saved draft result' }).isVisible()); assert((await page.locator('.search-stale').innerText()).includes('draft'));
    await page.locator('.search-error').getByRole('button', { name: 'retry', exact: true }).click(); await page.locator('.search-error').waitFor({ state: 'hidden' });
    await page.waitForFunction(() => !document.querySelector('.hits .hit') && !document.querySelector('.search-error') && !document.querySelector('.hits .spin'));
    assert.equal(await page.locator('.hits .hit').count(), 0); safe(state);
  } finally { await context.close(); }
});

await scenario('Code draft and attachment survive section navigation and reload', async () => {
  const { page, state, context } = await fixture();
  try {
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.fill('A draft with a reference image');
    await page.locator('.composer input[type="file"]').first().setInputFiles({ name: 'reference.png', mimeType: 'image/png', buffer: png });
    await page.getByRole('button', { name: 'Remove attachment reference.png', exact: true }).waitFor();
    await nav(page, 'Chat'); await page.locator('.msgs').waitFor(); await nav(page, 'Workspaces');
    assert.equal(await input.inputValue(), 'A draft with a reference image');
    assert(await page.getByRole('button', { name: 'Remove attachment reference.png', exact: true }).isVisible(), 'Code attachments must survive navigation.');
    await page.reload({ waitUntil: 'networkidle' });
    assert.equal(await input.inputValue(), 'A draft with a reference image');
    // Binary attachments are retained in the open tab; the existing reload contract covers text.
    safe(state);
  } finally { await context.close(); }
});

await scenario('Simple chat draft and attachment survive section navigation', async () => {
  const { page, state, context } = await fixture({ section: 'chat' });
  try {
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.fill('A simple chat draft');
    await page.locator('.composer input[type="file"]').first().setInputFiles({ name: 'reference.png', mimeType: 'image/png', buffer: png });
    await page.getByRole('button', { name: 'Remove attachment reference.png', exact: true }).waitFor();
    await nav(page, 'Workspaces'); await nav(page, 'Chat'); assert.equal(await input.inputValue(), 'A simple chat draft');
    assert(await page.getByRole('button', { name: 'Remove attachment reference.png', exact: true }).isVisible());
    safe(state);
  } finally { await context.close(); }
});

await scenario('Pending Code send stays guarded across remount and clears its accepted draft', async () => {
  const { page, state, context } = await fixture();
  try {
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.fill('Send this once while changing sections');
    state.delays.set('POST /projects/east/sessions/code-a/prompt', 1500); await page.getByRole('button', { name: 'Send', exact: true }).click();
    await waitCalls(state, 'POST', '/projects/east/sessions/code-a/prompt', 1); await nav(page, 'Chat'); await page.locator('.msgs').waitFor(); await nav(page, 'Workspaces');
    assert(await page.locator('.composer .composer-send').isDisabled(), 'An in-flight send must remain guarded after remount.');
    await page.locator('.composer .composer-send').evaluate(el => el.click()); await page.waitForFunction(() => document.querySelector('.composer textarea')?.value === '');
    assert.equal(calls(state, 'POST', '/projects/east/sessions/code-a/prompt').length, 1, 'Remount must not duplicate the accepted send.'); safe(state);
  } finally { await context.close(); }
});

await scenario('Daily budget remains based on today while report period changes', async () => {
  const { page, state, context } = await fixture({ section: 'models' });
  try {
    const budget = page.locator('.two.section').filter({ has: page.getByRole('button', { name: 'add', exact: true }) });
    await page.getByRole('button', { name: '7 days', exact: true }).click();
    await page.locator('.kpis .kpi').first().getByText('$84.00', { exact: true }).waitFor();
    assert((await budget.innerText()).includes('$4.00 / $10.00'), 'Daily total must use $4 today, not $84 report spend.');
    assert((await budget.innerText()).includes('$3.00 / $5.00'), 'Daily project spend must be independent of the report period.');
    await page.getByRole('button', { name: '30 days', exact: true }).click(); await page.locator('.kpis .kpi').first().getByText('$320.00', { exact: true }).waitFor();
    assert((await budget.innerText()).includes('$4.00 / $10.00')); safe(state);
  } finally { await context.close(); }
});

await scenario('Inbox rejects empty answer and duplicate permission actions', async () => {
  const { page, state, context } = await fixture({ section: 'inbox' });
  try {
    const card = page.locator('.inbox .item').filter({ hasText: 'Which layout should we use?' });
    const answer = card.locator('.acts .primary'); await answer.waitFor(); assert(await answer.isDisabled(), 'An unanswered question cannot be sent.');
    await card.getByRole('button', { name: 'Compact', exact: true }).click(); assert(await answer.isEnabled());
    state.delays.set('POST /projects/east/sessions/question-session/permission', 450);
    await answer.evaluate(el => { el.click(); el.click(); }); await waitCalls(state, 'POST', '/projects/east/sessions/question-session/permission', 1);
    await nap(80); assert.equal(calls(state, 'POST', '/projects/east/sessions/question-session/permission').length, 1, 'Only one answer may be submitted.');
    assert.equal(calls(state, 'POST', '/projects/east/sessions/question-session/permission')[0].body.answers['Which layout should we use?'], 'Compact');
    const allow = page.getByRole('button', { name: 'Allow', exact: true }); state.delays.set('POST /projects/west/sessions/permission-session/permission', 450);
    await allow.evaluate(el => { el.click(); el.click(); }); await waitCalls(state, 'POST', '/projects/west/sessions/permission-session/permission', 1);
    assert.equal(calls(state, 'POST', '/projects/west/sessions/permission-session/permission').length, 1); safe(state);
  } finally { await context.close(); }
});

await scenario('Inbox failed answer retains selection and offers retry in the same card', async () => {
  const { page, state, context } = await fixture({ section: 'inbox' });
  try {
    const card = page.locator('.inbox .item').filter({ hasText: 'Which layout should we use?' });
    await card.getByRole('button', { name: 'Compact', exact: true }).click();
    state.failures.set('POST /projects/east/sessions/question-session/permission', 1);
    await card.locator('.acts .primary').click(); await card.getByText('Fixture unavailable: /projects/east/sessions/question-session/permission', { exact: true }).waitFor();
    assert((await card.getByRole('button', { name: 'Compact', exact: true }).getAttribute('class')).includes('active'));
    assert(await card.locator('.acts .primary').isEnabled(), 'A failed answer can be retried.'); safe(state);
  } finally { await context.close(); }
});

await scenario('Inbox removes a stale workspace filter after background refresh', async () => {
  const { page, state, context } = await fixture({ section: 'inbox' });
  try {
    await page.locator('.top .seg').getByRole('button', { name: 'east', exact: true }).click();
    state.inbox.waiting = state.inbox.waiting.filter(i => i.project === 'west'); const n = calls(state, 'GET', '/inbox').length;
    await waitCalls(state, 'GET', '/inbox', n + 1); await page.getByText('Validate changes', { exact: true }).waitFor(); safe(state);
  } finally { await context.close(); }
});

await scenario('Inbox answer drafts remain isolated when sessions share a request ID', async () => {
  const { page, state, context } = await fixture();
  try {
    state.inbox.waiting = [{ project: 'east', session: { ...baseSession, id: 'question-east', title: 'East question', status: 'waiting', pending: 1 }, pending: [structuredClone(question)] }, { project: 'west', session: { ...baseSession, id: 'question-west', title: 'West question', status: 'waiting', pending: 1 }, pending: [structuredClone(question)] }];
    state.inbox.done = []; await nav(page, 'Inbox');
    const east = page.locator('.inbox .item').filter({ hasText: 'East question' }); const west = page.locator('.inbox .item').filter({ hasText: 'West question' });
    await east.getByRole('button', { name: 'Compact', exact: true }).click();
    assert.equal(await west.getByRole('button', { name: 'Compact', exact: true }).getAttribute('aria-pressed'), 'false', 'An identical request ID in another session must not share choices.');
    assert(await west.locator('.acts .primary').isDisabled()); await west.locator('.opts input').fill('A separate custom answer');
    assert.equal(await east.locator('.opts input').inputValue(), '', 'Custom text must stay scoped to its own session.');
    state.failures.set('POST /projects/west/sessions/question-west/permission', 1); await west.locator('.acts .primary').click();
    await west.locator('[role="alert"]').waitFor(); assert.equal(await west.locator('.opts input').inputValue(), 'A separate custom answer');
    assert.equal(await east.getByRole('button', { name: 'Compact', exact: true }).getAttribute('aria-pressed'), 'true');
    assert.equal(calls(state, 'POST', '/projects/west/sessions/question-west/permission')[0].body.answers['Which layout should we use?'], 'A separate custom answer'); safe(state);
  } finally { await context.close(); }
});

await scenario('Agent editor survives list growth, record switching and section navigation', async () => {
  const { page, state, context } = await fixture({ section: 'routines' });
  try {
    await page.locator('[data-agent="agent-a"]').click(); const name = page.locator('.rt-form input').first(); await name.fill('My unsaved reviewer');
    state.agents.push({ ...structuredClone(baseAgent), id: 'agent-added', name: 'Background teammate' }); const n = calls(state, 'GET', '/agents').length;
    await waitCalls(state, 'GET', '/agents', n + 1); await page.locator('[data-agent="agent-added"]').waitFor();
    assert.equal(await name.inputValue(), 'My unsaved reviewer', 'A changed list length must not overwrite an edited form.');
    await page.locator('[data-agent="agent-b"]').click(); await page.locator('[data-agent="agent-a"]').click(); assert.equal(await name.inputValue(), 'My unsaved reviewer');
    await nav(page, 'Workspaces'); await nav(page, 'Agents'); await page.locator('[data-agent="agent-a"]').click(); assert.equal(await name.inputValue(), 'My unsaved reviewer'); safe(state);
  } finally { await context.close(); }
});

await scenario('Routine editor survives list growth and record switching', async () => {
  const { page, state, context } = await fixture({ section: 'routines', routinesTab: 'routines' });
  try {
    await page.locator('[data-routine="routine-a"]').click(); const name = page.locator('.rt-form input').first(); await name.fill('My unsaved schedule');
    state.routines.push({ ...structuredClone(baseRoutine), id: 'routine-added', name: 'Background schedule' }); const n = calls(state, 'GET', '/routines').length;
    await waitCalls(state, 'GET', '/routines', n + 1); await page.locator('[data-routine="routine-added"]').waitFor(); assert.equal(await name.inputValue(), 'My unsaved schedule');
    await page.locator('[data-routine="routine-added"]').click(); await page.locator('[data-routine="routine-a"]').click(); assert.equal(await name.inputValue(), 'My unsaved schedule'); safe(state);
  } finally { await context.close(); }
});

await scenario('Dirty routine saves the edited task before running and ignores duplicate clicks', async () => {
  const { page, state, context } = await fixture({ section: 'routines', routinesTab: 'routines' });
  try {
    await page.locator('[data-routine="routine-a"]').click(); await page.locator('.rt-form textarea').first().fill('Validate the currently edited task.');
    state.delays.set('PUT /routines/routine-a', 450); state.delays.set('POST /routines/routine-a/run', 450);
    const run = page.getByRole('button', { name: 'Save and run', exact: true }); await run.waitFor(); await run.evaluate(el => { el.click(); el.click(); });
    await waitCalls(state, 'PUT', '/routines/routine-a', 1); await nap(100); assert.equal(calls(state, 'POST', '/routines/routine-a/run').length, 0, 'Run must wait for the edited routine to be saved.');
    await waitCalls(state, 'POST', '/routines/routine-a/run', 1); await nap(500);
    assert.equal(calls(state, 'PUT', '/routines/routine-a').length, 1); assert.equal(calls(state, 'POST', '/routines/routine-a/run').length, 1);
    assert.equal(calls(state, 'PUT', '/routines/routine-a')[0].body.prompt, 'Validate the currently edited task.'); safe(state);
  } finally { await context.close(); }
});

await scenario('Failed routine save retains edited task and prevents a run', async () => {
  const { page, state, context } = await fixture({ section: 'routines', routinesTab: 'routines' });
  try {
    await page.locator('[data-routine="routine-a"]').click(); const task = page.locator('.rt-form textarea').first(); await task.fill('Do not lose this task if saving fails.');
    state.failures.set('PUT /routines/routine-a', 1); await page.getByRole('button', { name: 'Save and run', exact: true }).click();
    await page.getByText('Fixture unavailable: /routines/routine-a', { exact: true }).waitFor(); assert.equal(await task.inputValue(), 'Do not lose this task if saving fails.');
    assert.equal(calls(state, 'POST', '/routines/routine-a/run').length, 0, 'A failed save must not run the previous saved task.'); safe(state);
  } finally { await context.close(); }
});

await scenario('A failing voice-settings request is not repeated in a loop', async () => {
  const { page, state, context } = await fixture({ initialFailures: { 'GET /voice': 100000 } });
  try {
    await page.locator('textarea').first().waitFor(); await nap(1200); const n = calls(state, 'GET', '/voice').length;
    assert(n >= 1 && n <= 2, `The microphone asks for the settings once when it appears; got ${n} requests in a second.`); safe(state);
  } finally { await context.close(); }
});

await scenario('Routine keeps its policies for provider failures, overlapping runs and unattended questions', async () => {
  const { page, state, context } = await fixture({ section: 'routines', routinesTab: 'routines' });
  try {
    await page.locator('[data-routine="routine-a"]').click(); const zone = page.locator('.rt-form .hint.zone'); await zone.waitFor();
    assert.match(await zone.innerText(), /time: Asia\/Tbilisi \(UTC\+4\)/, 'The schedule hint names the zone the times are read in.');
    assert.equal(await page.locator('.zone-switch').count(), 0, 'The browser is in the same zone: nothing to switch.');
    const retries = page.locator('.rt-form button.switch.retries'); assert.equal(await retries.getAttribute('aria-pressed'), 'true', 'Retries are on until the owner turns them off.');
    assert.equal(await page.getByLabel('skip the run', { exact: true }).isChecked(), true, 'A routine saved before the policies appeared opens with the defaults.');
    assert.equal(await page.getByLabel('wait for the answer in Inbox', { exact: true }).isChecked(), true);
    assert.equal(await page.locator('.draft-notice').count(), 0, 'Defaults alone do not make the form look edited.');
    // the overlap rule follows the trigger until the owner chooses against it: a webhook brings its own data each time
    const skip = page.getByLabel('skip the run', { exact: true }); const side = page.getByLabel('start next to it', { exact: true });
    await page.locator('.rt-form input[name="trig"][value="webhook"]').check(); assert.equal(await side.isChecked(), true, 'A webhook routine starts runs side by side by default.');
    await page.locator('.rt-form input[name="trig"][value="schedule"]').check(); assert.equal(await skip.isChecked(), true, 'Back on a schedule the next turn is skipped again.');
    assert.equal(await page.locator('.draft-notice').count(), 0, 'Trying a trigger and putting it back leaves no unsaved draft.');
    await side.check(); await page.locator('.rt-form input[name="trig"][value="event"]').check(); await page.locator('.rt-form input[name="trig"][value="schedule"]').check();
    assert.equal(await side.isChecked(), true, 'A rule the owner chose against the default stays as chosen.'); await skip.check();
    await retries.click(); await page.getByLabel('start next to it', { exact: true }).check(); await page.getByLabel('refuse and go on', { exact: true }).check();
    await page.getByRole('button', { name: 'Save', exact: true }).click(); await waitCalls(state, 'PUT', '/routines/routine-a', 1);
    const body = calls(state, 'PUT', '/routines/routine-a')[0].body; assert.equal(body.retries, false); assert.equal(body.overlap, 'parallel'); assert.equal(body.on_question, 'deny'); safe(state);
  } finally { await context.close(); }
});

await scenario('Schedules kept in another time zone offer the zone of this browser', async () => {
  const { page, state, context } = await fixture({ section: 'routines', routinesTab: 'routines', zone: 'Europe/Berlin', zoneOffset: 60 });
  try {
    await page.locator('[data-routine="routine-a"]').click(); const zone = page.locator('.rt-form .hint.zone'); await zone.waitFor();
    assert.match(await zone.innerText(), /time: Europe\/Berlin \(UTC\+1\)/); assert.equal(calls(state, 'PUT', '/settings').length, 0, 'A zone the owner has set is never replaced silently.');
    await page.getByRole('button', { name: 'switch to Asia/Tbilisi', exact: true }).click(); await waitCalls(state, 'PUT', '/settings', 1);
    assert.deepEqual(calls(state, 'PUT', '/settings')[0].body, { timezone: 'Asia/Tbilisi' });
    await page.getByText('time: Asia/Tbilisi (UTC+4)').waitFor(); assert.equal(await page.locator('.zone-switch').count(), 0); safe(state);
  } finally { await context.close(); }
});

await scenario('First visit stores the time zone of the owner while no schedule depends on the server clock', async () => {
  const { page, state, context } = await fixture({ section: 'routines', routinesTab: 'routines', zone: null, zoneOffset: 0, owner: true, routine: { trigger: { kind: 'manual', cron: '0 9 * * 1-5', event: '', contains: '' } } });
  try {
    await waitCalls(state, 'PUT', '/settings', 1); assert.deepEqual(calls(state, 'PUT', '/settings')[0].body, { timezone: 'Asia/Tbilisi' });
    await page.locator('[data-routine="routine-a"]').click(); await page.locator('.rt-form input[name="trig"][value="schedule"]').check(); await page.getByText('time: Asia/Tbilisi (UTC+4)').waitFor(); await nap(300);
    assert.equal(calls(state, 'PUT', '/settings').length, 1, 'The zone is stored once.'); safe(state);
  } finally { await context.close(); }
});

await scenario('Working schedules stay on the server clock until the owner switches the zone', async () => {
  for (const owner of [true, false]) {
    // owner: a schedule is already working; not owner: an automated browser with nothing scheduled
    const { page, state, context } = await fixture({ section: 'routines', routinesTab: 'routines', zone: null, zoneOffset: 0, owner, routine: owner ? {} : { enabled: false } });
    try {
      await page.locator('[data-routine="routine-a"]').click(); const zone = page.locator('.rt-form .hint.zone'); await zone.waitFor(); await nap(400);
      assert.match(await zone.innerText(), /server time \(UTC\+0\)/); assert.equal(calls(state, 'PUT', '/settings').length, 0, owner ? 'A working schedule must not move by itself.' : 'An automated browser never names the zone.');
      if (owner) { await page.getByRole('button', { name: 'switch to Asia/Tbilisi', exact: true }).click(); await waitCalls(state, 'PUT', '/settings', 1); await page.getByText('time: Asia/Tbilisi (UTC+4)').waitFor(); }
      safe(state);
    } finally { await context.close(); }
  }
});

await scenario('Run history tells a retry from the first attempt and when the next retry starts', async () => {
  const run = { routine: 'routine-a', routine_name: 'Review each morning', agent: 'agent-a', agent_name: 'Frontend reviewer', project: 'east', trigger: 'schedule', cost_usd: 0, session: null, summary: null, reason: null };
  const runs = [
    { ...run, id: 'run-wait', status: 'failed', started_at: ts - 240, ended_at: ts - 180, reason: 'the provider is overloaded (503)', kind: 'provider', attempt: 0, retry_at: ts + 120 },
    { ...run, id: 'run-again', status: 'done', started_at: ts - 3000, ended_at: ts - 2900, summary: 'Reviewed the changes.', attempt: 2, retry_of: 'run-first', cost_usd: 0.4 },
    { ...run, id: 'run-skip', status: 'skipped', started_at: ts - 7200, ended_at: ts - 7200, reason: 'the previous run is still going', kind: 'overlap', attempt: 0 },
  ];
  const { page, state, context } = await fixture({ section: 'routines', routinesTab: 'routines', runs });
  try {
    await page.locator('[data-routine="routine-a"]').click(); const waiting = page.locator('[data-run="run-wait"]'); await waiting.waitFor();
    const at = new RegExp(`retry at \\d{1,2}:${String(new Date((ts + 120) * 1000).getMinutes()).padStart(2, '0')}`);
    assert.match(await waiting.innerText(), at, 'A failed run that will be repeated says when.'); assert(!(await waiting.innerText()).includes('retry 0'));
    assert.equal(/retry at \d{1,2}:\d{2}\s?[AP]M/.test(await waiting.innerText()), /[AP]M/.test(await waiting.locator('.when').innerText()), 'The retry time is written in the same clock as the start time.');
    assert((await page.locator('[data-run="run-again"]').innerText()).includes('retry 2'), 'A repeated run is marked with its attempt number.');
    const skipped = await page.locator('[data-run="run-skip"]').innerText(); assert(skipped.includes('skipped') && skipped.includes('the previous run is still going'), 'A skipped run keeps its reason in the history.');
    await page.locator('.rt-top .seg').getByRole('button', { name: 'Runs', exact: true }).click(); await page.locator('[data-run="run-wait"]').waitFor();
    assert.match(await page.locator('[data-run="run-wait"]').innerText(), at, 'The common list of runs shows the same.'); safe(state);
  } finally { await context.close(); }
});

await scenario('Agent card sends only the fields the owner changed', async () => {
  const { page, state, context } = await fixture({ section: 'routines' });
  try {
    await page.locator('[data-agent="agent-a"]').click(); const name = page.locator('.rt-form input').first(); await name.fill('Careful reviewer');
    // a run adds a memory line while the card is open with unsaved edits
    state.agents[0].memory = 'Learned during a run.'; const n = calls(state, 'GET', '/agents').length; await waitCalls(state, 'GET', '/agents', n + 1);
    await page.getByRole('button', { name: 'Save', exact: true }).click(); await waitCalls(state, 'PUT', '/agents/agent-a', 1);
    assert.deepEqual(calls(state, 'PUT', '/agents/agent-a')[0].body, { name: 'Careful reviewer' }, 'Fields the owner did not touch stay out of the request.');
    assert.equal(state.agents[0].memory, 'Learned during a run.', 'The memory written by the run survives the save.'); safe(state);
  } finally { await context.close(); }
});

await scenario('Skill proposed by an agent waits for the owner and is accepted with one button', async () => {
  const { page, state, context } = await fixture({ section: 'routines', routinesTab: 'skills' });
  try {
    state.skills.push({ name: 'check-links', description: 'Check links before publishing', body: 'Open every link.', source: 'agent:agent-a', created_at: ts, updated_at: ts });
    const n = calls(state, 'GET', '/skills').length; await waitCalls(state, 'GET', '/skills', n + 1); await page.locator('button[data-skill="check-links"]').click();
    const notice = page.locator('.draft-notice.proposal'); await notice.waitFor(); assert.match(await notice.innerText(), /Proposed by the agent “Frontend reviewer”\. Until it is accepted, it works only in that agent’s own tasks/, 'The notice names the agent that proposed the skill.');
    await page.locator('button[data-skill="review-ux"]').click(); assert.equal(await page.locator('.draft-notice.proposal').count(), 0, 'The owner’s own skill has nothing to accept.');
    await page.locator('button[data-skill="check-links"]').click(); await page.getByRole('button', { name: 'Accept the skill', exact: true }).click();
    await waitCalls(state, 'POST', '/skills/check-links/accept', 1); await notice.waitFor({ state: 'detached' });
    assert.equal(state.skills.find(k => k.name === 'check-links').source, 'user'); assert.equal(calls(state, 'POST', '/skills/check-links/accept').length, 1); safe(state);
  } finally { await context.close(); }
});

await scenario('Webhook address is replaced only after a confirmation', async () => {
  const { page, state, context } = await fixture({ section: 'routines', routinesTab: 'routines' });
  try {
    state.routines[0].trigger = { kind: 'webhook', cron: '', event: '', contains: '' }; const n = calls(state, 'GET', '/routines').length; await waitCalls(state, 'GET', '/routines', n + 1);
    await page.locator('[data-routine="routine-a"]').click(); const hook = page.locator('.rt-form .hook'); await hook.waitFor(); assert.match(await hook.innerText(), /\/agents\/api\/hooks\/fixture$/);
    let answer = false; page.on('dialog', d => (answer ? d.accept() : d.dismiss()));
    await page.getByRole('button', { name: 'new address', exact: true }).click(); await nap(250);
    assert.equal(calls(state, 'POST', '/routines/routine-a/token').length, 0, 'A declined confirmation leaves the address as it was.');
    answer = true; await page.getByRole('button', { name: 'new address', exact: true }).click(); await waitCalls(state, 'POST', '/routines/routine-a/token', 1);
    await page.getByText(/\/agents\/api\/hooks\/rotated$/).waitFor(); safe(state);
  } finally { await context.close(); }
});

await scenario('Toast with a long error word stays inside a phone screen', async () => {
  // an error text may carry a URL or a metric name longer than the toast is wide (the owner's phone is 412 px)
  const { page, state, context } = await fixture({ section: 'routines', routinesTab: 'routines', width: 412, height: 915, touch: true });
  try {
    await page.locator('[data-routine="routine-a"]').click(); await page.locator('.rt-form textarea').first().fill('Changed task');
    state.failureText = 'core 502: generativelanguage.googleapis.com/generate_content_free_tier_input_token_count_per_model_per_day_for_project — limit: 0';
    state.failures.set('PUT /routines/routine-a', 1); await page.getByRole('button', { name: 'Save', exact: true }).click();
    const toast = page.locator('.toast.bad').filter({ hasText: 'generativelanguage' }); await toast.waitFor();
    const box = await toast.evaluate(el => { const r = el.getBoundingClientRect(); const t = el.lastElementChild.getBoundingClientRect(); return { view: innerWidth, toast: [r.left, r.right], text: [t.left, t.right] }; });
    assert(box.toast[0] >= 0 && box.toast[1] <= box.view, `The toast is inside the screen: ${JSON.stringify(box)}`);
    assert(box.text[1] <= box.toast[1] + .5, `Its text is inside the toast: ${JSON.stringify(box)}`); safe(state);
  } finally { await context.close(); }
});

await scenario('MCP workspace change disables stale writes and ignores older response', async () => {
  const { page, state, context } = await fixture({ settingsTab: 'mcp' });
  try {
    await nav(page, 'Settings'); await page.locator('[data-mcp="east-server"]').waitFor();
    const ws = page.locator('.mcp .frm select').first(); state.delays.set('GET /projects/west/mcp', 600);
    await ws.selectOption('west');
    assert.equal(await page.locator('[data-mcp="east-server"]').count(), 0, 'Old workspace servers must disappear immediately.');
    const visibleEdit = page.locator('.mcp input:not([type="hidden"]):visible'); for (let i = 0; i < await visibleEdit.count(); i++) assert(await visibleEdit.nth(i).isDisabled(), 'MCP inputs must be disabled during scope loading.');
    await ws.selectOption('east'); await page.locator('[data-mcp="east-server"]').waitFor(); await nap(750);
    assert(await page.locator('[data-mcp="east-server"]').isVisible()); assert.equal(await page.locator('[data-mcp="west-server"]').count(), 0, 'The slow previous workspace response must be ignored.');
    assert.equal(state.calls.filter(c => c.method === 'POST' && c.path.endsWith('/mcp')).length, 0); safe(state);
  } finally { await context.close(); }
});

await scenario('Permission rules change disables stale save and ignores previous scope', async () => {
  const { page, state, context } = await fixture({ settingsTab: 'perm' });
  try {
    await nav(page, 'Settings'); const allow = page.locator('.rules textarea').first();
    await page.waitForFunction(() => document.querySelector('.rules textarea')?.value === 'Read(east:user)');
    state.delays.set('GET /projects/east/permissions?scope=project&cwd=%2Fhome%2Fcoder', 600);
    await page.locator('.rules .seg').getByRole('button', { name: 'project', exact: true }).click();
    assert(await page.locator('.rules').getByRole('button', { name: 'Save', exact: true }).isDisabled(), 'Saving cannot use rules from a different scope while loading.');
    await page.locator('.rules .seg').getByRole('button', { name: 'user', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.rules textarea')?.value === 'Read(east:user)'); await nap(750); assert.equal(await allow.inputValue(), 'Read(east:user)'); safe(state);
  } finally { await context.close(); }
});

await scenario('Files ignores an older preview response after selecting another file', async () => {
  const { page, state, context } = await fixture({ width: 1920, height: 1080 });
  try {
    await page.getByRole('tab', { name: 'Files', exact: true }).click(); await page.locator('.files .list button.open').filter({ hasText: 'alpha.txt' }).waitFor();
    state.delays.set('GET /projects/east/fs/file?path=%2Fhome%2Fcoder%2Fapp%2Falpha.txt', 550);
    await page.locator('.files .list button.open').filter({ hasText: 'alpha.txt' }).click(); await page.locator('.files .list button.open').filter({ hasText: 'beta.txt' }).click();
    await page.locator('.preview pre').getByText('Content for /home/coder/app/beta.txt', { exact: true }).waitFor(); await nap(650);
    assert.equal((await page.locator('.preview pre').innerText()).trim(), 'Content for /home/coder/app/beta.txt'); safe(state);
  } finally { await context.close(); }
});

await scenario('Git ignores an older diff response and light theme keeps diff legible', async () => {
  const { page, state, context } = await fixture({ theme: 'light', width: 1920, height: 1080 });
  try {
    await page.getByRole('tab', { name: 'Git', exact: true }).click(); await page.locator('.git .nmb').first().waitFor();
    state.delays.set('GET /projects/east/git/diff?cwd=%2Fhome%2Fcoder%2Fapp&path=alpha.txt&staged=false&untracked=false', 550);
    await page.locator('.git .nmb').filter({ hasText: 'alpha.txt' }).click(); await page.locator('.git .nmb').filter({ hasText: 'beta.txt' }).click();
    await page.locator('.diff .ph b').getByText('beta.txt', { exact: true }).waitFor(); await nap(650); assert.equal(await page.locator('.diff .ph b').innerText(), 'beta.txt');
    const contrasts = await page.locator('.diff .ln.add, .diff .ln.del').evaluateAll(els => els.map(el => {
      const rgba = s => { const n = s.match(/[\d.]+/g).map(Number); return [n[0], n[1], n[2], n[3] ?? 1]; };
      const mix = (fg, bg) => fg.slice(0, 3).map((v, i) => v * fg[3] + bg[i] * (1 - fg[3]));
      const lum = rgb => { const [r, g, b] = rgb.map(x => { x /= 255; return x <= .04045 ? x / 12.92 : ((x + .055) / 1.055) ** 2.4; }); return .2126 * r + .7152 * g + .0722 * b; };
      const layers = []; for (let node = el; node; node = node.parentElement) layers.unshift(node);
      const background = layers.reduce((bg, node) => mix(rgba(getComputedStyle(node).backgroundColor), bg), [255, 255, 255]);
      const bg = lum(background); const fg = lum(mix(rgba(getComputedStyle(el).color), background)); return (Math.max(fg, bg) + .05) / (Math.min(fg, bg) + .05);
    })); for (const contrast of contrasts) assert(contrast >= 4.5, `Diff text should remain readable in light theme (${contrast.toFixed(2)}).`); safe(state);
  } finally { await context.close(); }
});

await scenario('Command palette traps focus and restores its initiating button', async () => {
  const { page, state, context } = await fixture();
  try {
    const trigger = page.getByRole('button', { name: 'Search and commands · Ctrl+K', exact: true }).first(); await trigger.click();
    const dialog = page.getByRole('dialog'); await dialog.waitFor(); assert.equal(await dialog.getAttribute('aria-modal'), 'true');
    assert(await dialog.evaluate(el => el.contains(document.activeElement)), 'Initial focus must enter the palette.');
    await page.keyboard.press('Shift+Tab'); assert(await dialog.evaluate(el => el.contains(document.activeElement)), 'Reverse Tab must remain in the palette.');
    for (let i = 0; i < 14; i++) await page.keyboard.press('Tab'); assert(await dialog.evaluate(el => el.contains(document.activeElement)), 'Tab must remain in the palette.');
    await page.keyboard.press('Escape'); await dialog.waitFor({ state: 'hidden' }); assert(await trigger.evaluate(el => document.activeElement === el), 'Closing must restore focus.'); safe(state);
  } finally { await context.close(); }
});

await scenario('Mobile More sheet traps focus and restores More button', async () => {
  const { page, state, context } = await fixture({ width: 390, height: 844, touch: true });
  try {
    const trigger = page.locator('.tabbar').getByRole('button', { name: 'More', exact: true }); await trigger.click(); const dialog = page.getByRole('dialog'); await dialog.waitFor();
    assert.equal(await dialog.getAttribute('aria-modal'), 'true'); assert(await dialog.evaluate(el => el.contains(document.activeElement)));
    await page.keyboard.press('Shift+Tab'); assert(await dialog.evaluate(el => el.contains(document.activeElement))); for (let i = 0; i < 6; i++) await page.keyboard.press('Tab'); assert(await dialog.evaluate(el => el.contains(document.activeElement)));
    await page.keyboard.press('Escape'); await dialog.waitFor({ state: 'hidden' }); assert(await trigger.evaluate(el => document.activeElement === el)); safe(state);
  } finally { await context.close(); }
});

await scenario('More-to-palette handoff restores focus and releases the app after closing', async () => {
  const { page, state, context } = await fixture({ width: 390, height: 844, touch: true });
  try {
    const more = page.locator('.tabbar').getByRole('button', { name: 'More', exact: true }); await more.click();
    await page.locator('.sheet').getByRole('button', { name: 'Search and commands', exact: true }).click();
    await page.locator('.palette').waitFor(); await page.waitForFunction(() => !!document.activeElement?.closest('.palette'));
    assert(await page.locator('.app').evaluate(el => el.inert), 'The open palette must keep the app inert.');
    await page.keyboard.press('Escape'); await page.locator('.palette').waitFor({ state: 'hidden' });
    assert.equal(await page.locator('.app').evaluate(el => el.inert), false, 'Closing the replacement modal must release the app.');
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
    const focus = await more.evaluate(el => ({ restored: document.activeElement === el, active: document.activeElement?.outerHTML?.slice(0, 700), more: el.outerHTML }));
    assert(focus.restored, `Closing the palette must restore the original More button: ${JSON.stringify(focus)}`); safe(state);
  } finally { await context.close(); }
});

await scenario('Phone keyboard keeps Code and simple-chat composer actions in the visible area', async () => {
  const { page, state, context } = await fixture({ width: 390, height: 844, touch: true });
  try {
    for (const engine of ['code', 'simple']) {
      if (engine === 'simple') await nav(page, 'Chat');
      await page.getByRole('textbox', { name: 'Message', exact: true }).focus();
      await page.evaluate(() => { const vv = window.visualViewport; Object.defineProperty(vv, 'height', { configurable: true, get: () => 360 }); vv.dispatchEvent(new Event('resize')); });
      await page.waitForFunction(() => document.documentElement.classList.contains('kbd')); await nap(100);
      const actions = await page.locator('.composer .composer-send, .composer .composer-attach, .composer .composer-more').evaluateAll(els => els.map(el => { const b = el.getBoundingClientRect(); return { name: el.getAttribute('aria-label'), top: b.top, bottom: b.bottom, right: b.right }; }));
      for (const action of actions) assert(action.top >= 0 && action.bottom <= 361 && action.right <= 391, `${engine}: composer action outside keyboard-visible area: ${JSON.stringify(action)}`);
      await shot(page, `keyboard-${engine}`); await page.evaluate(() => { const vv = window.visualViewport; Object.defineProperty(vv, 'height', { configurable: true, get: () => innerHeight }); vv.dispatchEvent(new Event('resize')); });
    }
    safe(state);
  } finally { await context.close(); }
});

await scenario('Short landscape composer keeps readable history and context details in Parameters', async () => {
  const { page, state, context } = await fixture({ width: 844, height: 390, touch: true });
  try {
    await page.locator('.composer-short-context').getByText('74%', { exact: true }).waitFor();
    assert.equal(await page.locator('.composer-context').isVisible(), false, 'Short landscape uses a compact context summary.');
    const sizing = await page.evaluate(() => ({ log: document.querySelector('.log').getBoundingClientRect().toJSON(), composer: document.querySelector('.composer').getBoundingClientRect().toJSON(), status: document.querySelector('.composer-state').getBoundingClientRect().toJSON() }));
    assert(sizing.log.height >= 60, `History should keep at least two readable lines, got ${sizing.log.height}px.`);
    await shot(page, 'short-landscape-history'); await page.getByRole('button', { name: 'Parameters', exact: true }).click();
    await page.getByRole('menuitem', { name: /^Context:/ }).click(); await page.getByRole('menuitem', { name: '/context', exact: true }).waitFor();
    assert((await page.getByRole('menu').innerText()).includes('74%'), 'Context details remain reachable through Parameters.');
    assert.equal(state.calls.filter(c => c.method === 'POST' && c.path.endsWith('/prompt')).length, 0, 'Opening context details must not send a model request.');
    await shot(page, 'short-landscape-context-menu'); safe(state); return sizing;
  } finally { await context.close(); }
});

// Device UX audit measures actual rendered targets and clipping, including coarse tablets.
async function metrics(page, scope = 'body') {
  return page.locator(scope).evaluate(root => {
    const name = el => (el.getAttribute('aria-label') || (el.getAttribute('aria-labelledby') || '').split(/\s+/).map(id => document.getElementById(id)?.textContent || '').join(' ').trim() || el.textContent?.trim() || el.getAttribute('title') || el.getAttribute('placeholder') || '').replace(/\s+/g, ' ').slice(0, 100);
    const nodes = [...root.querySelectorAll('button, a[href], select, textarea, input:not([type="hidden"]), [role="button"]')];
    const controls = nodes.map(el => {
      const own = el.getBoundingClientRect(); const css = getComputedStyle(el); const target = ['checkbox', 'radio'].includes(el.type) ? el.closest('label') || el : el; const rect = target.getBoundingClientRect();
      if (!own.width || !own.height || css.visibility === 'hidden' || css.display === 'none' || Number(css.opacity) === 0 || own.bottom < 0 || own.top > innerHeight || own.right < 0 || own.left > innerWidth) return null;
      const ancestors = []; let p = el.parentElement; while (p) { const style = getComputedStyle(p); if (style.overflowX !== 'visible' || style.overflowY !== 'visible') ancestors.push({ b: p.getBoundingClientRect(), x: style.overflowX, y: style.overflowY, intentionalX: p.matches('.subnav, .seg, .tab-list, .tw, .crumbs, .gbar') && p.scrollWidth > p.clientWidth + 1 }); p = p.parentElement; }
      const scrollReachable = ancestors.some(a => a.intentionalX && /auto|scroll/.test(a.x) && (own.left < a.b.left - 1 || own.right > a.b.right + 1));
      const clipped = !scrollReachable && (own.left < -1 || own.right > innerWidth + 1 || ancestors.some(a => /hidden|clip/.test(a.x) && (own.left < a.b.left - 1 || own.right > a.b.right + 1)));
      const compactControl = el.matches('.chat-composer .composer-toolbar button, .thread .t-head .thread-menu, .pane .tabs button') && (innerWidth < 600 || matchMedia('(pointer: coarse) and (max-height: 500px) and (max-width: 1099px)').matches);
      const minimumHeight = compactControl ? 32 : 44;
      const label = name(el); const short = el.tagName === 'TEXTAREA' || ['checkbox', 'radio'].includes(el.type) ? false : rect.width < 43.5 || rect.height < minimumHeight - .5;
      return { tag: el.tagName.toLowerCase(), name: label, cls: String(el.className).slice(0, 110), width: Math.round(rect.width * 10) / 10, height: Math.round(rect.height * 10) / 10, minimumHeight, small: short, clipped, scrollReachable, unnamed: el.tagName === 'BUTTON' && !label, disabled: !!el.disabled };
    }).filter(Boolean);
    const composerActions = [...root.querySelectorAll('.composer-send, .composer-more, .composer-attach, .composer textarea')].map(el => { const b = el.getBoundingClientRect(); if (!b.width || !b.height) return null; return { name: name(el), top: b.top, bottom: b.bottom, left: b.left, right: b.right }; }).filter(Boolean);
    return { width: innerWidth, height: innerHeight, coarse: matchMedia('(pointer: coarse)').matches, scrollWidth: document.documentElement.scrollWidth, total: controls.length, small: controls.filter(c => c.small), clipped: controls.filter(c => c.clipped), scrollReachable: controls.filter(c => c.scrollReachable), unnamed: controls.filter(c => c.unnamed), composerActions };
  });
}
const devices = [
  { name: 'phone-320', width: 320, height: 700, touch: true }, { name: 'phone-390', width: 390, height: 844, touch: true },
  { name: 'phone-landscape', width: 844, height: 390, touch: true }, { name: 'tablet-768', width: 768, height: 1024, touch: true },
  { name: 'tablet-1024', width: 1024, height: 768, touch: true }, { name: 'laptop-1280', width: 1280, height: 800, touch: false },
  { name: 'desktop-1920', width: 1920, height: 1080, touch: false },
];
async function measureComposerSpace(page, device) {
  const measure = () => page.locator('.chat-composer').evaluate(composer => {
    const buttons = [...composer.querySelectorAll('.composer-toolbar button')].map(el => { const rect = el.getBoundingClientRect(); return { width: rect.width, height: rect.height }; }).filter(rect => rect.width && rect.height);
    return { height: composer.getBoundingClientRect().height, toolbarHeight: composer.querySelector('.composer-toolbar').getBoundingClientRect().height, historyHeight: composer.parentElement.querySelector('.log, .msgs').getBoundingClientRect().height, buttons };
  });
  const compact = device.width < 600 || (device.touch && device.width <= 1099 && device.height <= 500);
  const current = await measure();
  const expectedHeight = compact || !device.touch ? 32 : 44;
  assert(current.buttons.length >= 4, 'The composer must retain its model, Parameters, attachment and Send actions.');
  assert(current.buttons.every(rect => Math.abs(rect.height - expectedHeight) <= .5), `${device.name}: toolbar buttons should be ${expectedHeight}px high: ${JSON.stringify(current.buttons)}`);
  if (!compact) return current;
  const baseline = await page.addStyleTag({ content: '.chat-composer .composer-toolbar button { height: 44px !important; min-height: 44px !important; }' });
  let previous;
  try { previous = await measure(); } finally { await baseline.evaluate(el => el.remove()); }
  const historyGain = current.historyHeight - previous.historyHeight;
  const formReduction = previous.height - current.height;
  assert(historyGain >= 11.5, `${device.name}: compact buttons should give history at least 12px more space, got ${historyGain}px.`);
  assert(Math.abs(historyGain - formReduction) <= .5, 'The reclaimed composer height should become usable history space.');
  return { ...current, historyGain, previousHeight: previous.height };
}
for (const device of devices) await scenario(`Device UX: ${device.name}`, async () => {
  const { page, state, context } = await fixture({ ...device, lang: 'ru' }); const samples = []; audit.push({ device: device.name, samples });
  try {
    for (const screen of ['thread', 'chat', 'inbox', 'models', 'settings', 'agents']) {
      if (screen === 'chat') await nav(page, 'Чат');
      else if (screen === 'inbox') await nav(page, 'Входящие');
      else if (screen === 'models') await nav(page, 'Модели');
      else if (screen === 'settings' || screen === 'agents') {
        const name = screen === 'settings' ? 'Настройки' : 'Агенты';
        if (device.width < 600) { await page.locator('.tabbar').getByRole('button', { name: 'Ещё', exact: true }).click(); await page.getByRole('dialog').getByRole('button', { name, exact: true }).click(); } else await nav(page, name);
        if (screen === 'settings') await page.locator('.subnav').getByRole('button', { name: 'MCP', exact: true }).click();
        else await page.locator('[data-agent="agent-a"]').click();
      }
      await nap(screen === 'thread' ? 1600 : 200); const measurement = await metrics(page); const composerSpace = ['thread', 'chat'].includes(screen) ? await measureComposerSpace(page, device) : undefined; samples.push({ screen, ...measurement, composerSpace }); await shot(page, `${device.name}-${screen}`);
      if (['inbox', 'models', 'settings', 'agents'].includes(screen)) {
        const scrolled = await page.locator('.main .content').evaluate(el => { const scrolls = el.scrollHeight > el.clientHeight + 8; if (scrolls) el.scrollTop = el.scrollHeight; return scrolls; });
        if (scrolled) { await nap(60); samples.push({ screen: `${screen}-bottom`, ...await metrics(page) }); await shot(page, `${device.name}-${screen}-bottom`); }
      }
    }
    for (const measurement of samples) {
      assert(measurement.scrollWidth <= measurement.width + 1, `${measurement.screen}: page must fit viewport without horizontal scrolling.`);
      assert.equal(measurement.clipped.length, 0, `${measurement.screen}: controls must not be clipped: ${JSON.stringify(measurement.clipped)}`);
      for (const action of measurement.composerActions) assert(action.top >= -1 && action.bottom <= measurement.height + 1 && action.left >= -1 && action.right <= measurement.width + 1, `${measurement.screen}: required composer action must be visible: ${JSON.stringify(action)}`);
      if (process.env.FRONTEND_UX_STRICT === '1') { assert.equal(measurement.unnamed.length, 0, `${measurement.screen}: unnamed controls.`); if (device.touch) assert.equal(measurement.small.filter(c => !c.disabled).length, 0, `${measurement.screen}: touch targets below their minimum size: ${JSON.stringify(measurement.small.filter(c => !c.disabled))}`); }
    }
    safe(state); return { screens: samples.length, smallTargets: samples.reduce((n, s) => n + s.small.length, 0), unnamed: samples.reduce((n, s) => n + s.unnamed.length, 0) };
  } finally { await context.close(); }
});

await scenario('Narrow Code cards use separate Files and Git previews with reachable back actions', async () => {
  const { page, state, context } = await fixture({ view: 'code', width: 1920, height: 1080 });
  try {
    const pane = page.locator('.card').first(); await pane.getByRole('tab', { name: 'Files', exact: true }).click(); await pane.locator('.files .list button.open').filter({ hasText: 'alpha.txt' }).click();
    const back = pane.locator('.preview button.back'); await back.waitFor(); assert(await back.isVisible()); await shot(page, 'narrow-card-files');
    const fileMetrics = await metrics(page, '.card:first-child'); assert.equal(fileMetrics.clipped.length, 0, JSON.stringify(fileMetrics.clipped));
    const split = await pane.locator('.files .split').evaluate(el => { const preview = el.querySelector('.preview').getBoundingClientRect(); const list = el.querySelector('.list').getBoundingClientRect(); return { previewWidth: preview.width, containerWidth: el.clientWidth, listShown: list.width > 0 && list.height > 0 }; });
    assert(split.previewWidth >= split.containerWidth - 2 && !split.listShown, 'A narrow Files pane should devote its width to the active preview.');
    await back.click(); await pane.getByRole('tab', { name: 'Git', exact: true }).click(); await pane.locator('.git .nmb').first().click();
    const diffBack = pane.locator('.diff button.back'); await diffBack.waitFor(); assert(await diffBack.isVisible()); await shot(page, 'narrow-card-git');
    const gitMetrics = await metrics(page, '.card:first-child'); assert.equal(gitMetrics.clipped.length, 0, JSON.stringify(gitMetrics.clipped)); safe(state);
  } finally { await context.close(); }
});

// ---------- notification channels (docs/agents-ux.md §9.12) ----------

const tgChannel = { id: 'ch_tg', kind: 'telegram', name: 'Личный', enabled: true, bot: '@stand_bot', chat_title: 'Alex', chat_linked: true, protect: true, buttons: false, slack_channel: '', host: '', tail: '', internal: false, signed: false, created_at: ts, files: true, bindings: [], sent_today: 0, last: null };
// a checked item carries its mark in the name (generated content counts), so the label is matched from its start
const menuItem = (page, label) => { const name = new RegExp('^' + label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')); return page.getByRole('menu').getByRole('menuitem', { name }).or(page.getByRole('menu').getByRole('menuitemcheckbox', { name })); };
const fitsWidth = page => page.evaluate(() => ({ page: document.documentElement.scrollWidth <= innerWidth + 1, wide: [...document.querySelectorAll('.ntf-card, .ntf-card *, .ntf-dlg, .ntf-dlg *')].filter(el => { const r = el.getBoundingClientRect(); return r.width > 0 && (r.right > innerWidth + 1 || r.left < -1); }).map(el => el.className).slice(0, 5) }));

await scenario('Channels: a Telegram channel is made by a token and a code, and the token is not shown again', async () => {
  const { page, state, context } = await fixture({ section: 'settings', settingsTab: 'notify' });
  try {
    const card = page.locator('.ntf-card'); await card.getByText('No channels yet.').waitFor();
    await card.getByRole('button', { name: 'Channel', exact: true }).click(); await menuItem(page, 'Telegram').click();
    const form = card.locator('.ntf-new'); const token = form.getByLabel('Bot token');
    assert.equal(await token.getAttribute('type'), 'password', 'A token is typed into a password field.');
    assert(await form.getByRole('button', { name: 'Save and check' }).isDisabled(), 'No chat — nothing to save.');
    await token.fill('7312345678:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw-x'); await form.getByRole('button', { name: 'Check the token' }).click();
    await form.getByText('/start 483920').waitFor(); await form.locator('.ntf-bot').getByText('@stand_bot').waitFor();
    assert.deepEqual(calls(state, 'POST', '/notify/telegram/begin').at(-1).body, { token: '7312345678:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw-x' });
    await form.getByRole('button', { name: 'Find the chat' }).click(); await form.getByText('The code has not arrived yet.', { exact: false }).waitFor();
    state.tgFound = true; await form.getByRole('button', { name: 'Find the chat' }).click();
    await form.getByText('chat found: "Alex" (private)', { exact: false }).waitFor();
    assert.equal(await form.getByLabel('Name', { exact: true }).inputValue(), 'Alex', 'The channel is named after the chat unless the owner says otherwise.');
    assert(await form.getByLabel('Forbid forwarding from the chat').isChecked(), 'Forwarding is forbidden unless switched off.');
    assert(!(await form.getByLabel('"Allow / Deny" buttons').isChecked()), 'Buttons are off unless switched on.');
    await form.getByLabel('"Allow / Deny" buttons').check(); await form.getByText('Only a button press is accepted', { exact: false }).waitFor();
    await form.getByRole('button', { name: 'Save and check' }).click();
    const row = card.locator('.ntf-ch'); await row.getByText('@stand_bot → Alex').waitFor();
    const made = calls(state, 'POST', '/notify/channels').at(-1).body;
    assert.deepEqual({ kind: made.kind, pending: made.pending, protect: made.protect, buttons: made.buttons, name: made.name, origin: made.origin }, { kind: 'telegram', pending: 'tg_fixture', protect: true, buttons: true, name: 'Alex', origin });
    assert.equal(made.token, undefined, 'The token went with the handshake; the channel is made by the handshake id.');
    await waitCalls(state, 'POST', '/notify/channels/ch_1/test', 1); await page.getByText('Test message sent to "Alex"').waitFor();
    await row.getByText('no bindings — the channel is silent').waitFor();
    assert(!(await page.locator('body').innerText()).includes('AAHdq'), 'The token is nowhere on the page after saving.');
    await shot(page, 'channels-telegram'); safe(state);
  } finally { await context.close(); }
});

await scenario('Channels: a webhook shows its signing key once; switch, check, journal, replace and delete', async () => {
  const { page, state, context } = await fixture({ section: 'settings', settingsTab: 'notify' });
  try {
    const card = page.locator('.ntf-card');
    await card.getByRole('button', { name: 'Channel', exact: true }).click(); await menuItem(page, 'Webhook (n8n, Mattermost, your script)').click();
    const form = card.locator('.ntf-new'); await form.getByLabel('Webhook address').fill('https://n8n.fixture.invalid/webhook/agents-secret-path');
    await form.getByLabel('Name', { exact: true }).fill('n8n'); await form.getByLabel('An address inside the stand or the tailnet').check();
    assert(await form.getByLabel('Sign the body').isChecked());
    await form.getByRole('button', { name: 'Save and check' }).click();
    const key = card.locator('.ntf-key'); await key.getByText('f'.repeat(64)).waitFor();
    const made = calls(state, 'POST', '/notify/channels').at(-1).body;
    assert.deepEqual({ kind: made.kind, url: made.url, internal: made.internal, signed: made.signed, name: made.name }, { kind: 'webhook', url: 'https://n8n.fixture.invalid/webhook/agents-secret-path', internal: true, signed: true, name: 'n8n' });
    const row = card.locator('.ntf-ch'); await row.getByText('fixture.invalid · …path').waitFor();
    assert(!(await row.innerText()).includes('agents-secret'), 'The address is not shown: the host and a tail only.');
    await key.getByRole('button', { name: 'Cancel' }).click(); assert.equal(await key.count(), 0, 'The key is shown once.');
    // the switch stops the channel, the bindings stay
    await row.getByRole('button', { name: 'Channel is on' }).click(); await waitCalls(state, 'PUT', '/notify/channels/ch_1', 1);
    assert.deepEqual(calls(state, 'PUT', '/notify/channels/ch_1').at(-1).body, { enabled: false });
    await row.locator('.ntf-switch[aria-pressed="false"]').waitFor();
    // a failed check is told with the service's own words and stays on the card
    state.testError = '403 Forbidden: the receiver refused'; await row.getByRole('button', { name: 'Check', exact: true }).click();
    await page.getByText('Not sent: 403 Forbidden: the receiver refused').waitFor(); await row.getByText('last one failed: 403 Forbidden: the receiver refused').waitFor();
    state.notifyLog = [{ at: ts, status: 'failed', event: 'test', title: 'AgentVerse OS', error: '403 Forbidden: the receiver refused', tries: 1, files: 0 }, { at: ts - 60, status: 'sent', event: 'report', title: 'east · Review the interface', error: null, tries: 2, files: 2 }];
    await row.getByRole('button', { name: 'journal' }).click();
    const log = row.locator('.ntf-log-row'); await log.nth(1).getByText('agent report · files: 2').waitFor(); await log.nth(1).getByText('delivered · attempts: 2').waitFor(); await log.first().getByText('not delivered').waitFor();
    // a secret is replaced, never read back
    await row.getByRole('button', { name: 'Channel actions' }).click(); await menuItem(page, 'Replace the address').click();
    const replace = row.locator('.ntf-edit input'); assert.equal(await replace.getAttribute('type'), 'password'); assert.equal(await replace.inputValue(), '', 'The kept address is not put into the field.');
    await replace.fill('https://n8n.fixture.invalid/webhook/new-path'); await row.locator('.ntf-edit').getByRole('button', { name: 'Save' }).click();
    await waitCalls(state, 'PUT', '/notify/channels/ch_1', 2); assert.deepEqual(calls(state, 'PUT', '/notify/channels/ch_1').at(-1).body, { url: 'https://n8n.fixture.invalid/webhook/new-path' });
    await row.getByText('fixture.invalid · …path').waitFor();
    await row.getByRole('button', { name: 'Channel actions' }).click(); await menuItem(page, 'Rename').click();
    await row.locator('.ntf-edit input').fill('Automation'); await row.locator('.ntf-edit input').press('Enter'); await row.getByText('Automation').waitFor();
    // "tell about the stand" is a binding of its own kind
    await row.getByRole('button', { name: 'Channel actions' }).click(); await menuItem(page, 'Tell about the stand (spending limit, memory)').click();
    await row.getByText('all workspaces (the stand)').waitFor();
    assert.deepEqual((({ channel, scope, away_only }) => ({ channel, scope, away_only }))(calls(state, 'POST', '/notify/bindings').at(-1).body), { channel: 'ch_1', scope: 'all', away_only: false });
    await row.getByRole('button', { name: 'Remove the binding' }).click(); await row.getByText('no bindings — the channel is silent').waitFor();
    // deleting asks first
    let accept = false; page.on('dialog', d => (accept ? d.accept() : d.dismiss()));
    await row.getByRole('button', { name: 'Channel actions' }).click(); await menuItem(page, 'Delete the channel').click();
    await nap(150); assert.equal(calls(state, 'DELETE', '/notify/channels/ch_1').length, 0, 'A dismissed question deletes nothing.');
    accept = true; await row.getByRole('button', { name: 'Channel actions' }).click(); await menuItem(page, 'Delete the channel').click();
    await card.getByText('No channels yet.').waitFor(); safe(state);
  } finally { await context.close(); }
});

await scenario('A conversation reports to a channel: choices are saved at once and the header says where it reports', async () => {
  const { page, state, context } = await fixture({ view: 'thread', width: 1920, height: 1080, channels: [tgChannel] });
  try {
    const more = page.locator('.thread-menu'); await more.waitFor();
    assert.equal(await page.locator('.thread-notify').count(), 0); assert(!(await more.evaluate(el => el.classList.contains('ntf-on'))));
    await more.click(); await menuItem(page, 'Report to a channel…').click();
    const dlg = page.getByRole('dialog', { name: 'Report to a channel' }); await dlg.getByText('Conversation "Review the interface" · east').waitFor();
    const row = dlg.locator('.ntf-row'); const when = row.getByLabel('When');
    assert.equal(await when.inputValue(), 'off'); assert.equal(await row.locator('input[type="radio"]').count(), 0, 'Nothing to choose until the channel is bound.');
    await when.selectOption('once'); await waitCalls(state, 'POST', '/notify/bindings', 1);
    const first = calls(state, 'POST', '/notify/bindings').at(-1).body;
    assert.deepEqual({ channel: first.channel, scope: first.scope, project: first.project, session: first.session, when: first.when, content: first.content, origin: first.origin }, { channel: 'ch_tg', scope: 'session', project: 'east', session: 'code-a', when: 'once', content: 'fact', origin });
    assert(await row.getByLabel('the fact only').isChecked(), 'Only the fact leaves the stand unless the owner says otherwise.');
    assert(!(await row.getByLabel('the agent may send reports and files itself').isChecked()));
    assert.equal(await dlg.getByText('The text leaves the stand', { exact: false }).count(), 0);
    await row.getByLabel('the whole result').check(); await waitCalls(state, 'POST', '/notify/bindings', 2); await dlg.getByText('The text leaves the stand', { exact: false }).waitFor();
    await row.getByLabel('the agent may send reports and files itself').check(); await waitCalls(state, 'POST', '/notify/bindings', 3);
    const last = calls(state, 'POST', '/notify/bindings').at(-1).body; assert.deepEqual({ when: last.when, content: last.content, reports: last.reports }, { when: 'once', content: 'full', reports: true });
    await dlg.getByRole('button', { name: 'Done' }).click();
    await page.locator('.thread-notify').getByText('Telegram · Личный · once · reports').waitFor();
    assert(await more.evaluate(el => el.classList.contains('ntf-on')), 'The dot on the menu button marks a conversation that reports.');
    await more.click(); assert.equal(await menuItem(page, 'Report to a channel…').getAttribute('aria-checked'), 'true'); await page.keyboard.press('Escape');
    // the turn has ended elsewhere: the service took the "once" binding away and said so through the hub
    state.channels[0].bindings = []; await page.evaluate(() => window.__frontendFixture.emit({ kind: 'notify' }));
    await page.locator('.thread-notify').waitFor({ state: 'detached' }); assert(!(await more.evaluate(el => el.classList.contains('ntf-on'))));
    // "after every turn", then off again
    await more.click(); await menuItem(page, 'Report to a channel…').click(); await when.selectOption('always'); await page.locator('.ntf-row.on').waitFor();
    const bound = state.channels[0].bindings[0].id;
    await when.selectOption('off'); await waitCalls(state, 'DELETE', `/notify/bindings/${bound}`, 1);
    assert.equal(state.channels[0].bindings.length, 0); await page.locator('.ntf-row.on').waitFor({ state: 'detached' });
    await shot(page, 'channels-bind-dialog'); safe(state);
  } finally { await context.close(); }
});

await scenario('A workspace and a routine are bound from their own places', async () => {
  const { page, state, context } = await fixture({ view: 'thread', channels: [tgChannel] });
  try {
    // the "+" menu of a workspace
    await page.locator('.panel .ws button[aria-label*="east"]').first().click();
    await menuItem(page, 'Workspace notifications…').click();
    const dlg = page.getByRole('dialog', { name: 'Report to a channel' }); await dlg.getByText('Workspace east — all its conversations').waitFor();
    const row = dlg.locator('.ntf-row'); await row.getByLabel('When').selectOption('on'); await waitCalls(state, 'POST', '/notify/bindings', 1);
    assert(await row.getByLabel('only when I am away from the screen').isChecked(), 'A workspace-wide binding waits for the owner to be away unless told otherwise.');
    assert.equal(await row.getByLabel('the agent may send reports and files itself').count(), 0, 'Reports belong to a conversation or a routine.');
    await row.getByLabel('only when I am away from the screen').uncheck(); await waitCalls(state, 'POST', '/notify/bindings', 2);
    assert.equal(calls(state, 'POST', '/notify/bindings').at(-1).body.away_only, false);
    await dlg.getByRole('button', { name: 'Set up channels' }).click();
    // the dialog led to Settings → Notifications, where the binding is listed
    await page.locator('.ntf-card .ntf-ch').getByText('workspace east').waitFor();
    // a routine: "where to" in its form
    await nav(page, 'Agents'); await page.waitForLoadState('networkidle');
    await page.getByRole('tab', { name: /Routines/ }).or(page.getByRole('button', { name: /^Routines/ })).first().click();
    await page.locator('.rt-item').filter({ hasText: 'Review each morning' }).first().click();
    const where = page.locator('.rt-where'); await where.getByText('device').waitFor();
    await where.getByRole('button', { name: 'Channels…' }).click();
    await dlg.getByText('Routine "Review each morning"').waitFor(); await dlg.getByText('When to report is decided by the routine', { exact: false }).waitFor();
    await dlg.locator('.ntf-row').getByLabel('When').selectOption('on'); await waitCalls(state, 'POST', '/notify/bindings', 3);
    const b = calls(state, 'POST', '/notify/bindings').at(-1).body; assert.deepEqual({ scope: b.scope, routine: b.routine, project: b.project }, { scope: 'routine', routine: 'routine-a', project: '' });
    await dlg.locator('.ntf-row').getByLabel('the agent may send reports and files itself').check(); await waitCalls(state, 'POST', '/notify/bindings', 4);
    await dlg.getByRole('button', { name: 'Done' }).click();
    await where.getByText('device · Telegram · Личный · reports').waitFor(); safe(state);
  } finally { await context.close(); }
});

await scenario('Phone: names and errors that are not ours wrap inside the channels card and the dialog', async () => {
  const word = 'generativelanguage.googleapis.com/generate_content_free_tier_input_token_count_'.repeat(2);
  const long = { ...structuredClone(tgChannel), name: 'Канал-с-очень-длинным-названием-без-единого-пробела-в-нём-0123456789', chat_title: `Группа_${word}`, last: { at: ts, status: 'failed', error: `403 ${word}`, event: 'done' },
    bindings: [{ id: 'nb_1', channel: 'ch_tg', scope: 'session', project: 'east', session: 'code-a', routine: '', when: 'once', content: 'full', reports: true, away_only: false, created_at: ts }] };
  const { page, state, context } = await fixture({ section: 'settings', settingsTab: 'notify', width: 412, height: 915, touch: true, lang: 'ru', channels: [long] });
  try {
    state.notifyLog = [{ at: ts, status: 'failed', event: 'done', title: `east · ${word}`, error: `403 ${word}`, tries: 5, files: 0 }];
    const row = page.locator('.ntf-ch'); await row.getByText('последнее не доставлено', { exact: false }).waitFor();
    await row.getByRole('button', { name: 'журнал' }).click(); await row.locator('.ntf-log-row').waitFor();
    assert(await row.locator('.ntf-ch-name b').evaluate(b => b.getClientRects().length === 1), 'The kind of a channel stays in one line next to a long name.');
    let fit = await fitsWidth(page); assert(fit.page, 'The page must not scroll sideways.'); assert.deepEqual(fit.wide, [], 'Nothing in the card may reach past the screen.');
    await shot(page, 'channels-phone-long-words');
    // the form of a new channel with an error of the service in it
    await page.locator('.ntf-card').getByRole('button', { name: 'Канал', exact: true }).click(); await menuItem(page, 'Telegram').click();
    state.failures.set('POST /notify/telegram/begin', 1); state.failureText = `Telegram не принял токен: ${word}`;
    await page.locator('.ntf-new').getByLabel('Токен бота').fill('7312345678:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw-x'); await page.locator('.ntf-new').getByRole('button', { name: 'Проверить токен' }).click();
    await page.locator('.ntf-new .ntf-err').waitFor();
    fit = await fitsWidth(page); assert(fit.page && !fit.wide.length, `The refusal must wrap: ${JSON.stringify(fit.wide)}`);
    await page.locator('.ntf-new').scrollIntoViewIfNeeded(); await shot(page, 'channels-phone-form');
    state.failureText = undefined;
    // the dialog of a conversation whose title is one long word
    state.sessions[0].title = `Сессия_${word}`; state.workspaces.projects[0].sessions[0].title = `Сессия_${word}`;
    await page.evaluate(() => window.__frontendFixture.emit({ kind: 'notify' }));
    await page.locator('.tabbar').getByRole('button', { name: 'Работа', exact: true }).or(page.locator('.tabbar button').first()).first().click();
    await page.locator('.thread-menu').click(); await menuItem(page, 'Сообщать в канал…').click();
    const dlg = page.getByRole('dialog', { name: 'Сообщать в канал' }); await dlg.locator('.ntf-row.on').waitFor();
    fit = await fitsWidth(page); assert(fit.page && !fit.wide.length, `The dialog must stay inside the screen: ${JSON.stringify(fit.wide)}`);
    await page.waitForFunction(() => Number(getComputedStyle(document.querySelector('.ntf-dlg')).opacity) > .99);
    assert(await page.locator('.ntf-card .ntf-ch-name b, .ntf-dlg .ntf-name b').evaluateAll(list => list.every(b => b.getClientRects().length === 1)), 'The kind of a channel stays in one line next to a long name.');
    const box = await dlg.boundingBox(); assert(box.x >= 0 && box.x + box.width <= 412 + 1 && box.y >= 0 && box.y + box.height <= 915 + 1, 'The whole dialog is on the screen.');
    await shot(page, 'channels-phone-dialog'); safe(state);
  } finally { await context.close(); }
});

await browser.close();
await writeFile(join(out, 'report.json'), JSON.stringify({ uiUrl, results, audit }, null, 2));
const failed = results.filter(r => !r.pass);
console.log(`${results.length - failed.length}/${results.length} scenarios passed; report: ${join(out, 'report.json')}`);
if (failed.length) process.exitCode = 1;
