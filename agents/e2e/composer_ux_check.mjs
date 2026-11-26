#!/usr/bin/env node
/**
 * Isolated browser regressions for the shared agent and simple-chat composers.
 * Every API request and WebSocket is replaced by a fixture; no backend/model is used.
 *
 * Start the UI separately: npm --prefix agents/ui run dev -- --host 127.0.0.1 --port 5176
 * Then: UI_URL=http://127.0.0.1:5176/agents/ node agents/e2e/composer_ux_check.mjs
 * Set PLAYWRIGHT_MODULE and CHROME_PATH if the browser tools are not on Node's path.
 * Screenshots and the JSON report go to COMPOSER_UX_OUT (default /tmp/composer-ux-check).
 */
import assert from 'node:assert/strict';
import { mkdir, writeFile, access } from 'node:fs/promises';
import { homedir } from 'node:os';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const originUrl = process.env.UI_URL ?? 'http://127.0.0.1:5176/agents/';
const origin = new URL(originUrl).origin;
const output = resolve(process.env.COMPOSER_UX_OUT ?? '/tmp/composer-ux-check');
const moduleCandidates = [process.env.PLAYWRIGHT_MODULE, 'playwright'].filter(Boolean);
let chromium;
for (const candidate of moduleCandidates) {
  try { ({ chromium } = await import(candidate.startsWith('/') ? pathToFileURL(candidate).href : candidate)); break; } catch {}
}
assert(chromium, 'Install Playwright or set PLAYWRIGHT_MODULE to its index.mjs.');
let executablePath = process.env.CHROME_PATH;
if (!executablePath) {
  for (const candidate of [
    chromium.executablePath(),
    join(homedir(), '.cache/ms-playwright/chromium-1243/chrome-linux64/chrome'),
    join(homedir(), '.cache/puppeteer/chrome/linux-148.0.7778.97/chrome-linux64/chrome'),
  ]) { try { await access(candidate); executablePath = candidate; break; } catch {} }
}
await mkdir(output, { recursive: true });

const ts = Math.floor(Date.now() / 1000);
const usage = { input: 2400, output: 2300, cache_read: 128000, cache_create: 14000, cost_usd: 12.34, turns: 16 };
const baseSession = { id: 'demo-1', agent: 'claude', cwd: '/home/coder/alpha', title: 'Review and improve the interface', created_at: ts - 600, updated_at: ts, model: 'claude-fable-5-1[1m]', permission_mode: 'default', effort: 'max', status: 'idle', alive: true, last_seq: 3, pending: 0, queued: [], usage, last_text: 'Interface review is ready.' };
const caps = { provider: 'claude', mcp: 0, models: [{ value: baseSession.model, label: 'Fable' }, { value: 'claude-sonnet-5-1', label: 'Sonnet' }], commands: [], efforts: ['low', 'medium', 'high', 'xhigh', 'max'], modes: ['default', 'acceptEdits', 'plan', 'auto', 'bypassPermissions'], features: { commands: true, files: true, effort: true, fast: false, compact: true } };
const history = [
  { seq: 1, ts, kind: 'user', content: [{ type: 'text', text: 'Review the frontend and improve the UX.' }] },
  { seq: 2, ts: ts + 1, kind: 'assistant', content: [{ type: 'text', text: '### Interface review\n\nClear actions and readable status indicators help manage several sessions.\n\n- Keep the composer compact.\n- Keep settings accessible.\n- Preserve drafts after network errors.' }] },
  { seq: 3, ts: ts + 2, kind: 'result', num_turns: 1, duration_ms: 16000, total_cost_usd: 455.756, cost_usd: .312, usage: { input_tokens: 400, output_tokens: 700, cache_read_input_tokens: 60000, cache_creation_input_tokens: 4600 } },
];
const permission = { seq: 3, ts: ts + 2, kind: 'permission_request', request_id: 'approval-demo', tool_name: 'Bash', input: { command: 'npm run check' }, suggestions: [{ rules: [{ toolName: 'Bash', ruleContent: 'npm run check:*' }] }] };
const simpleBase = { id: 'simple-1', kind: 'simple', title: 'Discuss the interface', route: 'gate-demo', model: 'demo-model', system: '', tools: { search: false, fetch: false }, incognito: false, created_at: ts - 600, updated_at: ts, cost_usd: .04, tokens_in: 1200, tokens_out: 700, messages: 2 };
const routes = { routes: [{ id: 'gate-demo', kind: 'gate', title: 'Project gate', provider: 'anthropic', host: 'fixture.invalid', models: [{ id: 'demo-model', name: 'Demo model', context: 200000, provider: 'anthropic', provider_title: 'Claude', kind: 'chat' }, { id: 'alternate-model', name: 'Alternate model', context: 100000, provider: 'anthropic', provider_title: 'Claude', kind: 'chat' }] }], defaults: { route: 'gate-demo', model: 'demo-model' }, search: { ready: true, kind: 'searxng' } };
const simpleMessages = [{ id: 'm1', chat: simpleBase.id, seq: 1, role: 'user', text: 'What makes a composer easy to use?', attachments: [], tools: [], tokens_in: 0, tokens_out: 0, cost_usd: 0, created_at: ts }, { id: 'm2', chat: simpleBase.id, seq: 2, role: 'assistant', text: 'A clear primary action, concise settings and reliable drafts.', attachments: [], tools: [], tokens_in: 1200, tokens_out: 700, cost_usd: .04, created_at: ts + 1 }];

const browser = await chromium.launch({ headless: true, executablePath });
const results = [];

/** Each scenario gets independent state, intercepted requests and a fake session stream. */
async function fixture({ view = 'thread', session = 'demo-1', width = 1280, height = 800, offline = false, lang = 'en', seed = null, channels = [] } = {}) {
  const sessions = [structuredClone(baseSession), { ...structuredClone(baseSession), id: 'demo-2', title: 'Improve settings while the agent works', status: 'working' }, { ...structuredClone(baseSession), id: 'demo-3', title: 'Decision required for validation', status: 'waiting', pending: 1, last_seq: 183 }];
  const workspace = { projects: [{ name: 'alpha', description: 'Local browser fixture', status: 'running', reachable: true, cost_today: usage.cost_usd, apps: [], sessions, ptys: [] }], today: usage };
  const state = { sessions, simple: structuredClone(simpleBase), simpleMessages: structuredClone(simpleMessages), simpleBusy: null, simpleStatus: 0, calls: [], unexpected: [], workspaceFailures: offline ? Infinity : 0, promptFail: false, simpleFail: false, queueDelay: 0, channels: structuredClone(channels), ...(seed ? { history: structuredClone(seed) } : {}) };
  const context = await browser.newContext({ viewport: { width, height }, hasTouch: width < 600, serviceWorkers: 'block' });
  await context.addInitScript(({ sessions, view, session, lang }) => {
    localStorage.setItem('agents.lang', lang);
    localStorage.setItem('agents.ui', JSON.stringify({ view, sel: session ? { project: 'alpha', kind: 'session', id: session } : null, cards: sessions.map(s => ({ project: 'alpha', kind: 'session', id: s.id, w: 1, h: 1 })), cols: 0, chatSel: 'simple-1', chatEngine: 'simple' }));
    const streams = [];
    let paused = false;
    // what the journal of a session holds beyond what the page has: replayed after "hello" as the adapter does it — every
    // event after `since`, one frame at a time, until the page drops the socket
    const missed = { id: '', events: [] }; const stats = { frames: 0, urls: [] };
    class MockWebSocket {
      static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
      readyState = 0;
      constructor(url) {
        this.url = url; streams.push(this); stats.urls.push(url);
        const id = new URL(url).pathname.split('/').at(-2);
        setTimeout(() => {
          if (paused || this.readyState === 3) return;
          this.readyState = 1; this.onopen?.({});
          const info = sessions.find(s => s.id === id);
          const since = Number(new URL(url).searchParams.get('since') ?? 0);
          const replay = id === missed.id ? missed.events.filter(e => e.seq > since) : [];
          if (info) this.onmessage?.({ data: JSON.stringify({ kind: 'hello', info: replay.length ? { ...info, last_seq: replay.at(-1).seq } : info, replay: replay.length }) });
          if (replay.length) {
            const ch = new MessageChannel(); let i = 0;
            ch.port1.onmessage = () => { if (this.readyState !== 1 || !this.onmessage) return; stats.frames++; this.onmessage({ data: JSON.stringify(replay[i++]) }); if (i < replay.length) ch.port2.postMessage(0); };
            ch.port2.postMessage(0);
          }
        }, 30);
      }
      send() {}
      close() { if (this.readyState === 3) return; this.readyState = 3; this.onclose?.({ code: 1006 }); }
    }
    window.WebSocket = MockWebSocket;
    window.__composerFixture = {
      disconnect: () => { paused = true; streams.forEach(s => s.close()); },
      // the work went on elsewhere: the journal of `id` now also holds `events`; the page comes back to it
      comeBack: (id, events) => { missed.id = id; missed.events = events; stats.frames = 0; paused = false; document.dispatchEvent(new Event('visibilitychange')); window.dispatchEvent(new Event('online')); },
      stats,
      emit: event => streams.filter(s => s.readyState === 1).forEach(s => s.onmessage?.({ data: JSON.stringify(event) })),
    };
  }, { sessions, view, session, lang });
  await context.route('**/*', async route => {
    const request = route.request(); const u = new URL(request.url()); const method = request.method();
    if (u.hostname === 'fonts.googleapis.com') { await route.fulfill({ contentType: 'text/css', body: '' }); return; }
    if (u.origin !== origin) { state.unexpected.push(`${method} ${u.href}`); await route.abort(); return; }
    if (!u.pathname.includes('/api/')) { await route.continue(); return; }
    const path = u.pathname.replace(/^\/agents\/api(?:\/core)?/, '');
    const body = request.postDataJSON();
    state.calls.push({ method, path, query: u.search, body });
    let result; let status = 200;
    if (path === '/health') result = { ok: true, cloudd: true, version: 'fixture' };
    else if (path === '/workspaces') { if (state.workspaceFailures > 0) { state.workspaceFailures--; status = 503; result = { error: 'Workspaces temporarily unavailable' }; } else result = workspace; }
    else if (path === '/agents/layout') result = { cards: sessions.map(s => ({ project: 'alpha', kind: 'session', id: s.id, w: 1, h: 1 })), cols: 0, groups: {} };
    else if (path === '/accounts') result = { accounts: [], defaults: {} };
    else if (path === '/settings/desktop') result = { appearance: { language: lang } };
    else if (path === '/settings') result = { prices: {}, defaults: {}, limits: [] };
    else if (path === '/voice') result = { provider: 'openai', language: '', ready: false, model: 'fixture', elevenlabs: { model: 'fixture', base: '', default_base: '', has_key: false, key_tail: '' }, openai: { model: 'fixture', base: '', default_base: '', has_key: false, key_tail: '' }, models: { elevenlabs: ['fixture'], openai: ['fixture'] } };
    else if (path === '/scheduled') result = { scheduled: [], now: ts };
    else if (path.endsWith('/engines')) result = { engines: [] };
    else if (path.endsWith('/ptys') && method === 'GET') result = [];
    else if (path === '/gate/models') result = { targets: [{ app: 'fixture-gate', title: 'Project gate', models: routes.routes[0].models }] };
    else if (path === '/skills') result = { skills: [{ name: 'interface-review', description: 'Review interface behavior', body: 'Inspect the UI.', source: 'user', created_at: ts, updated_at: ts }] };
    else if (path === '/chat') result = { project: 'scratch', exists: true, status: 'running', reachable: true, sessions: [], busy: false };
    else if (path === '/chat/routes') result = routes;
    else if (path === '/chats') result = { chats: [state.simple] };
    else if (path === '/chat/presets') result = { presets: [{ id: 'brief', name: 'Brief answers', text: 'Answer concisely.' }] };
    else if (path === '/chats/simple-1' && method === 'GET') result = { chat: state.simple, messages: state.simpleMessages, busy: !!state.simpleBusy, ...(state.simpleBusy ? { partial: state.simpleBusy } : {}) };
    else if (path.startsWith('/chats/simple-1/files/')) { await route.fulfill({ contentType: 'text/plain', body: 'fixture file' }); return; }
    else if (path === '/chats/simple-1' && method === 'PUT') { Object.assign(state.simple, body); result = state.simple; }
    else if (path === '/chats/simple-1/messages') {
      if (state.simpleFail) { status = 503; result = { error: 'Fixture send failed' }; }
      else if (state.simpleStatus) { await route.fulfill({ status: state.simpleStatus, contentType: 'text/plain', body: 'Failed to buffer the request body: length limit exceeded' }); return; }
      else {
        // the service's rules (docs/agents-ux.md §6.6): a retry drops what came after the last question, an edit replaces the
        // message and the tail; an edited message keeps the attachments named in keep_files
        const list = state.simpleMessages; const events = [];
        const reply = text => ({ ...simpleMessages[1], id: `agent-${list.length + 1}`, seq: list.length + 1, text });
        if (body.regenerate) {
          let last = -1; list.forEach((m, i) => { if (m.role === 'user') last = i; }); list.length = last + 1;
          list.push(reply('Fixture retry reply'));
        } else {
          let kept = [];
          if (body.replace_from) { const at = list.findIndex(m => m.id === body.replace_from); kept = (list[at]?.attachments ?? []).filter(a => (body.keep_files ?? []).includes(a.file)); list.length = Math.max(0, at); }
          const user = { ...simpleMessages[0], id: body.replace_from ? 'edited-user' : 'sent-user', seq: list.length + 1, text: body.text, attachments: [...kept, ...(body.attachments ?? []).map((a, i) => ({ file: `new-${i}.bin`, name: a.name, mime: a.mime, size: Math.floor(a.data.length * 3 / 4), text: '' }))] };
          list.push(user); events.push({ kind: 'user', message: user });
          list.push(body.replace_from ? reply('Fixture reply to the edit') : { ...simpleMessages[1], id: 'sent-agent', seq: list.length + 1, text: 'Fixture reply' });
        }
        events.push({ kind: 'done', message: list[list.length - 1], chat: state.simple });
        await route.fulfill({ contentType: 'text/event-stream', body: events.map(e => `data: ${JSON.stringify(e)}\n\n`).join('') }); return;
      }
    } else if (/^\/projects\/[^/]+\/sessions\/[^/]+\/events$/.test(path)) {
      result = path.includes('/demo-3/') ? [history[0], history[1], permission, ...Array.from({ length: 180 }, (_, i) => ({ seq: i + 4, ts: ts + i + 3, kind: 'assistant', content: [{ type: 'text', text: `Background update ${i + 1}: validating interface behavior.` }] }))] : (state.history ?? history);
      const since = Number(u.searchParams.get('since') ?? 0); result = result.filter(e => e.seq > since).slice(0, Number(u.searchParams.get('limit') ?? 5000));
    } else if (/^\/projects\/[^/]+\/sessions\/[^/]+$/.test(path)) {
      const info = sessions.find(s => s.id === path.split('/').at(-1)); result = { info, caps, pending: info?.pending ? [permission] : [] };
    } else if (path.endsWith('/control')) result = { maxTokens: 1000000, totalTokens: 740000, percentage: 74 };
    else if (path.endsWith('/queue')) {
      if (state.queueDelay) await new Promise(r => setTimeout(r, state.queueDelay));
      const info = sessions.find(s => path.includes(`/${s.id}/`));
      const item = { id: `q-${info.queued.length + 1}`, content: body.content, created_at: ts }; info.queued.push(item); result = item;
    } else if (path.endsWith('/prompt')) {
      if (state.promptFail) { status = 503; result = { error: 'Fixture send failed' }; }
      else result = { seq: 4, ts: ts + 3, kind: 'user', content: body.content };
    } else if (path.endsWith('/mode')) { const info = sessions.find(s => path.includes(`/${s.id}/`)); info.permission_mode = body.mode; result = {}; }
    else if (path.endsWith('/model')) { const info = sessions.find(s => path.includes(`/${s.id}/`)); info.model = body.model; result = {}; }
    else if (path.endsWith('/effort')) { const info = sessions.find(s => path.includes(`/${s.id}/`)); info.effort = body.effort; result = {}; }
    else if (path.endsWith('/cache')) result = {};
    else if (path === '/notify/channels') result = { channels: structuredClone(state.channels ?? []), rate_per_min: 20 };
    else if (path === '/notify/share' && method === 'POST') result = { ok: true, channel: (state.channels ?? []).find(c => c.id === body.channel)?.name ?? '' };
    else { state.unexpected.push(`${method} ${path}`); status = 501; result = { error: `Missing fixture: ${method} ${path}` }; }
    await route.fulfill({ status, json: result });
  });
  const page = await context.newPage();
  page.setDefaultTimeout(6000);
  state.pageErrors = []; page.on('pageerror', error => state.pageErrors.push(error.message));
  await page.goto(originUrl, { waitUntil: 'networkidle' });
  if (!offline && session) await page.locator('.composer textarea').first().waitFor({ state: 'visible' });
  return { page, state, context };
}

async function test(name, run) {
  if (process.env.TEST_FILTER && !new RegExp(process.env.TEST_FILTER, 'i').test(name)) return;
  try { const detail = await run(); results.push({ name, pass: true, detail }); console.log(`PASS ${name}`); }
  catch (error) { results.push({ name, pass: false, error: error.stack }); console.error(`FAIL ${name}: ${error.message}`); }
}

function checkRequests(state) {
  assert.deepEqual(state.unexpected, [], 'All requests must remain inside explicit fixtures.');
  assert.deepEqual(state.pageErrors, [], 'The page should render without JavaScript errors.');
}
async function screenshot(page, name) {
  if (await page.getByRole('menu').count()) await page.waitForFunction(() => [...document.querySelectorAll('.menu')].every(el => Number(getComputedStyle(el).opacity) > .99));
  await page.screenshot({ path: join(output, `${name}.png`) });
}
async function fitViewport(page) {
  const metrics = await page.evaluate(() => {
    const rect = element => { const b = element.getBoundingClientRect(); return { left: b.left, right: b.right, top: b.top, bottom: b.bottom, width: b.width, height: b.height }; };
    return { width: innerWidth, height: innerHeight, scrollWidth: document.documentElement.scrollWidth, composers: [...document.querySelectorAll('.composer')].filter(e => e.getBoundingClientRect().height).map(rect) };
  });
  assert(metrics.scrollWidth <= metrics.width + 1, 'The page must not scroll horizontally.');
  assert(metrics.composers.length > 0, 'A composer must be visible.');
  for (const rect of metrics.composers) { assert(rect.left >= -1 && rect.right <= metrics.width + 1, 'Composer must stay within viewport width.'); assert(rect.width >= 240, 'Composer must keep a usable input width.'); }
  return metrics;
}

await test('Tab moves focus without changing permission mode', async () => {
  const { page, state, context } = await fixture();
  try {
    const input = page.getByRole('textbox', { name: 'Message', exact: true });
    await input.fill('A retained draft'); await input.focus(); await page.keyboard.press('Tab');
    assert(await input.evaluate(el => document.activeElement !== el), 'Tab should advance keyboard focus.');
    assert.equal(state.calls.filter(c => c.method === 'POST' && c.path.endsWith('/mode')).length, 0, 'Ordinary Tab must not mutate approvals.');
    assert.equal(await input.inputValue(), 'A retained draft');
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Connection loss overrides working status and preserves draft', async () => {
  const { page, state, context } = await fixture({ session: 'demo-2' });
  try {
    await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Keep this while reconnecting');
    await page.evaluate(() => window.__composerFixture.disconnect());
    await page.getByText('Reconnecting. Your draft is saved.', { exact: true }).waitFor();
    await page.waitForFunction(() => [...document.querySelectorAll('.composer [role="status"]')].some(el => /reconnecting/i.test(el.textContent)));
    assert.equal(await page.getByRole('textbox', { name: 'Message', exact: true }).inputValue(), 'Keep this while reconnecting');
    assert(await page.locator('.composer .composer-send').isDisabled(), 'Sending should wait for a confirmed live connection.');
    await screenshot(page, 'thread-reconnecting'); checkRequests(state);
  } finally { await context.close(); }
});

await test('Pending decision jumps to an older actionable request', async () => {
  const { page, state, context } = await fixture({ session: 'demo-3' });
  try {
    const jump = page.getByRole('button', { name: 'View request', exact: true }); await jump.waitFor();
    await jump.click();
    const allow = page.getByRole('button', { name: 'Allow', exact: true }); await allow.waitFor();
    try { await page.waitForFunction(() => !!document.activeElement?.closest('.perm')); }
    catch (error) {
      await screenshot(page, 'decision-focus-failure');
      console.error('Decision diagnostics', await page.evaluate(() => ({ active: document.activeElement?.outerHTML, request: document.querySelector('.perm')?.outerHTML?.slice(0, 600), logTop: document.querySelector('.log')?.scrollTop })));
      throw error;
    }
    try { await page.waitForFunction(() => { const el = document.querySelector('.perm .primary'); if (!el) return false; const b = el.getBoundingClientRect(); const log = el.closest('.log').getBoundingClientRect(); return b.top >= log.top && b.bottom <= log.bottom; }); }
    catch (error) {
      await screenshot(page, 'decision-scroll-failure');
      console.error('Decision scroll diagnostics', await page.evaluate(() => ({ active: document.activeElement?.textContent, rect: document.querySelector('.perm .primary')?.getBoundingClientRect().toJSON(), logRect: document.querySelector('.log')?.getBoundingClientRect().toJSON(), logTop: document.querySelector('.log')?.scrollTop })));
      throw error;
    }
    assert.equal(state.calls.filter(c => c.path.endsWith('/permission')).length, 0, 'Jumping should not answer automatically.');
    await screenshot(page, 'thread-decision'); checkRequests(state);
  } finally { await context.close(); }
});

const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==', 'base64');
await test('Working turn queues text and image exactly once', async () => {
  const { page, state, context } = await fixture({ session: 'demo-2' });
  try {
    state.queueDelay = 350;
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.fill('Queue this follow-up');
    await page.locator('.composer input[type="file"]').first().setInputFiles({ name: 'reference.png', mimeType: 'image/png', buffer: png });
    await page.getByRole('button', { name: 'Remove attachment reference.png', exact: true }).waitFor();
    const queue = page.getByRole('button', { name: 'Queue', exact: true }); assert.equal((await queue.innerText()).trim(), 'Queue');
    await queue.evaluate(el => { el.click(); el.click(); });
    await page.waitForFunction(() => document.querySelector('.composer textarea')?.value === '');
    const requests = state.calls.filter(c => c.path.endsWith('/queue') && c.method === 'POST');
    assert.equal(requests.length, 1, 'Rapid repeat actions must produce one queued message.');
    assert.equal(state.calls.filter(c => c.path.endsWith('/prompt')).length, 0, 'A working turn must not receive an immediate prompt.');
    assert(requests[0].body.content.some(block => block.type === 'text' && block.text === 'Queue this follow-up'));
    assert(requests[0].body.content.some(block => block.type === 'image'));
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Failed agent send retains text and attachment', async () => {
  const { page, state, context } = await fixture();
  try {
    state.promptFail = true;
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.fill('Do not lose this draft');
    await page.locator('.composer input[type="file"]').first().setInputFiles({ name: 'draft.png', mimeType: 'image/png', buffer: png });
    await page.getByRole('button', { name: 'Remove attachment draft.png', exact: true }).waitFor();
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: 'Could not send. Your draft is saved.' }).waitFor();
    assert.equal(await input.inputValue(), 'Do not lose this draft');
    assert(await page.getByRole('button', { name: 'Remove attachment draft.png', exact: true }).isVisible());
    assert.equal(state.calls.filter(c => c.path.endsWith('/prompt')).length, 1);
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Canceling a historical edit restores the existing text and image draft', async () => {
  const { page, state, context } = await fixture();
  try {
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.fill('My current unsent draft');
    await page.locator('.composer input[type="file"]').first().setInputFiles({ name: 'unsent.png', mimeType: 'image/png', buffer: png });
    await page.getByRole('button', { name: 'Remove attachment unsent.png', exact: true }).waitFor();
    await page.locator('.m.user').hover();
    await page.locator('.m.user').getByRole('button', { name: 'Edit and resend', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.composer textarea')?.value === 'Review the frontend and improve the UX.');
    await page.locator('.composer .editbar').getByRole('button', { name: /^cancel$/i }).click();
    assert.equal(await input.inputValue(), 'My current unsent draft');
    assert(await page.getByRole('button', { name: 'Remove attachment unsent.png', exact: true }).isVisible());
    assert.equal(state.calls.filter(c => /\/(prompt|edit)$/.test(c.path)).length, 0, 'Editing and canceling should not submit either draft.');
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Image reading blocks immediate send until the attachment is complete', async () => {
  const { page, state, context } = await fixture();
  try {
    await page.evaluate(() => { const NativeReader = window.FileReader; window.FileReader = class extends NativeReader { readAsDataURL(file) { setTimeout(() => super.readAsDataURL(file), 350); } }; });
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.fill('Include the image in this send');
    await page.locator('.composer input[type="file"]').first().setInputFiles({ name: 'loading.png', mimeType: 'image/png', buffer: png });
    const send = page.getByRole('button', { name: 'Send', exact: true });
    assert(await send.isDisabled(), 'Send should wait for image reading.');
    await input.focus(); await page.keyboard.press('Enter');
    assert.equal(state.calls.filter(c => c.path.endsWith('/prompt')).length, 0, 'Enter must not submit an incomplete attachment.');
    await page.getByRole('button', { name: 'Remove attachment loading.png', exact: true }).waitFor();
    await send.click();
    await page.waitForFunction(() => document.querySelector('.composer textarea')?.value === '');
    const requests = state.calls.filter(c => c.path.endsWith('/prompt'));
    assert.equal(requests.length, 1);
    assert(requests[0].body.content.some(block => block.type === 'image'));
    assert(requests[0].body.content.some(block => block.type === 'text' && block.text === 'Include the image in this send'));
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Session tabs and section rail have usable accessible navigation', async () => {
  const { page, state, context } = await fixture({ width: 1280, height: 720, view: 'code' });
  try {
    const tabs = page.getByRole('tablist', { name: 'Session views' }).first();
    assert.equal(await tabs.getByRole('tab').count(), 5);
    const chatTab = tabs.getByRole('tab', { name: 'Chat', exact: true });
    assert.equal(await chatTab.getAttribute('aria-selected'), 'true');
    assert(await chatTab.locator('span').isVisible(), 'The active tab should keep its visible label even in narrow cards.');
    await chatTab.focus(); await page.keyboard.press('ArrowRight');
    const cliTab = tabs.getByRole('tab', { name: /CLI/ });
    assert.equal(await cliTab.getAttribute('aria-selected'), 'true');
    await page.keyboard.press('ArrowLeft'); assert.equal(await chatTab.getAttribute('aria-selected'), 'true');
    const rail = page.locator('.rail');
    for (const button of await rail.getByRole('button').all()) assert(await button.getAttribute('aria-label'), 'Every section rail control should have an accessible name.');
    assert.equal(await rail.getByRole('button', { name: 'Workspaces', exact: true }).getAttribute('aria-current'), 'page');
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Initial workspace failure is visible in the main phone view and Retry recovers', async () => {
  const { page, state, context } = await fixture({ width: 390, height: 844, offline: true });
  try {
    const main = page.locator('main'); await main.getByRole('alert').filter({ hasText: 'Could not load workspaces' }).waitFor();
    await screenshot(page, 'phone-workspace-error');
    state.workspaceFailures = 0;
    await main.getByRole('button', { name: /^retry$/i }).click();
    await page.getByRole('textbox', { name: 'Message', exact: true }).waitFor();
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Transient refresh failure keeps the mounted chat and draft', async () => {
  const { page, state, context } = await fixture();
  try {
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.fill('Still composing during refresh');
    state.workspaceFailures = 1;
    await page.getByText('Workspaces could not be refreshed. Showing the last loaded sessions.', { exact: true }).waitFor({ timeout: 7000 });
    assert.equal(await input.inputValue(), 'Still composing during refresh');
    await page.locator('main').getByRole('button', { name: /^retry$/i }).click();
    await page.getByText('Workspaces could not be refreshed. Showing the last loaded sessions.', { exact: true }).waitFor({ state: 'hidden' });
    assert.equal(await input.inputValue(), 'Still composing during refresh');
    checkRequests(state);
  } finally { await context.close(); }
});

async function openSimple(page) {
  await page.locator('.rail').getByRole('button', { name: 'Chat', exact: true }).click();
  await page.getByRole('textbox', { name: 'Message', exact: true }).waitFor();
  await page.getByRole('button', { name: 'Demo model', exact: true }).waitFor();
}
function menuAction(page, name) { return page.getByRole('menu').locator('button').filter({ hasText: name }); }

await test('Simple chat Parameters retains tools, system prompt, image command and model selection', async () => {
  const { page, state, context } = await fixture();
  try {
    await openSimple(page);
    const params = page.getByRole('button', { name: 'Parameters', exact: true });
    await params.click(); await menuAction(page, /^search$/).click();
    await page.waitForFunction(() => !!document.querySelector('.composer .tsearch'));
    assert.equal(state.simple.tools.search, true);
    await params.click(); await menuAction(page, /^pages$/).click();
    await page.waitForFunction(() => !!document.querySelector('.composer .tfetch'));
    assert.equal(state.simple.tools.fetch, true);
    await params.click(); await menuAction(page, /^System prompt$/).click();
    const dialog = page.getByRole('dialog', { name: 'System prompt', exact: true }); await dialog.waitFor();
    await dialog.getByRole('textbox').fill('Answer concisely.'); await dialog.getByRole('button', { name: 'Save', exact: true }).click();
    await dialog.waitFor({ state: 'hidden' }); assert.equal(state.simple.system, 'Answer concisely.');
    await params.click(); await menuAction(page, /^image/).click();
    assert.equal(await page.getByRole('textbox', { name: 'Message', exact: true }).inputValue(), '/img ');
    await page.getByRole('button', { name: 'Demo model', exact: true }).click();
    await menuAction(page, /Alternate model/).click();
    await page.getByRole('button', { name: 'Alternate model', exact: true }).waitFor();
    assert.equal(state.simple.model, 'alternate-model');
    await screenshot(page, 'simple-chat-parameters'); checkRequests(state);
  } finally { await context.close(); }
});

await test('Failed simple chat send retains text, attachment and a retryable composer', async () => {
  const { page, state, context } = await fixture();
  try {
    await openSimple(page); state.simpleFail = true;
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.fill('Keep this simple chat draft');
    await page.locator('.composer input[type="file"]').first().setInputFiles({ name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('Unsaved draft attachment') });
    await page.getByRole('button', { name: 'Remove attachment notes.txt', exact: true }).waitFor();
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: 'Message was not sent. Your draft is saved.' }).waitFor();
    assert.equal(await input.inputValue(), 'Keep this simple chat draft');
    assert(await page.getByRole('button', { name: 'Remove attachment notes.txt', exact: true }).isVisible());
    assert.equal(state.calls.filter(c => c.path === '/chats/simple-1/messages').length, 1);
    assert(await page.getByRole('button', { name: 'Send', exact: true }).isEnabled());
    checkRequests(state);
  } finally { await context.close(); }
});

// A journal like the stand's ones: a turn is some 35 events — a request, tool calls with their results, an answer, a result.
function turnEvents(from, n) {
  const out = []; let seq = from;
  while (out.length < n) {
    out.push({ seq: ++seq, ts, kind: 'user', content: [{ type: 'text', text: `Step ${seq}: check the module and fix what is broken.` }] });
    for (let k = 0; k < 16; k++) {
      const tid = `tool-${seq}-${k}`;
      out.push({ seq: ++seq, ts, kind: 'assistant', content: [{ type: 'text', text: `Looking at file ${k}.` }, { type: 'tool_use', id: tid, name: 'Bash', input: { command: `sed -n 1,80p src/module_${k}.rs` } }] });
      out.push({ seq: ++seq, ts, kind: 'tool_result', content: [{ type: 'tool_result', tool_use_id: tid, content: `output of the tool, call ${k}\n`.repeat(12) }] });
    }
    out.push({ seq: ++seq, ts, kind: 'assistant', content: [{ type: 'text', text: `Answer ${seq}: the module was checked, two places were fixed.` }] });
    out.push({ seq: ++seq, ts, kind: 'result', num_turns: 1, duration_ms: 16000, total_cost_usd: 1, cost_usd: .3, usage: { input_tokens: 400, output_tokens: 700 } });
  }
  return out.slice(0, n - (n % 35));   // whole turns: the journal ends with an answer and its result
}
const lastAnswer = events => [...events].reverse().find(e => e.kind === 'assistant' && e.content[0].text.startsWith('Answer')).content[0].text;

await test('Coming back after long work elsewhere, the thread takes the tail of the journal instead of replaying it', async () => {
  // The owner, 2026-11-23: a conversation opened on the phone, hours of work from the desktop, back on the phone — "it loads
  // for long, as if fast-forwarding". The socket replayed every missed event, one frame at a time: 3000 events took 2 min 40 s.
  const { page, state, context } = await fixture({ width: 412, height: 915 });
  try {
    await page.locator('.chat-composer textarea').first().waitFor(); await page.locator('.m.agent').first().waitFor();
    const gone = turnEvents(3, 3010); const last = gone.at(-1).seq;
    state.history = [...history, ...gone]; state.sessions[0].last_seq = last;
    await page.evaluate(() => window.__composerFixture.disconnect());
    const t0 = Date.now();
    await page.evaluate(({ gone }) => window.__composerFixture.comeBack('demo-1', gone), { gone });
    await page.locator('.m.agent').filter({ hasText: lastAnswer(gone) }).waitFor({ timeout: 8000 });
    const took = Date.now() - t0;
    const st = await page.evaluate(() => ({ frames: window.__composerFixture.stats.frames, urls: window.__composerFixture.stats.urls.slice(-2), following: document.querySelector('.log').dataset.following, earlier: document.querySelector('.log .earlier')?.textContent ?? '' }));
    assert(st.frames < 60, `The missed events are not replayed through the socket: ${st.frames} frames got through.`);
    assert(state.calls.some(c => /\/sessions\/demo-1\/events$/.test(c.path) && c.query.includes(`since=${last - 300}`)), 'The last page of the journal is taken with one request.');
    assert(st.urls.at(-1).includes(`since=${last}`), `The new socket continues from the end of that page: ${st.urls.at(-1)}`);
    assert.equal(st.following, 'true', 'The reader is at the fresh end of the conversation.');
    assert(took < 6000, `The fresh tail is on screen in ${took} ms.`);
    // everything in between is still within reach, page by page
    assert(st.earlier.length > 0, 'What was skipped is offered by "show earlier".');
    const rows = () => page.locator('.log .m').count(); const before = await rows();
    await page.locator('.log .earlier').click(); await page.waitForFunction(n => document.querySelectorAll('.log .m').length > n, before);
    const second = await rows(); await page.locator('.log .earlier').click(); await page.waitForFunction(n => document.querySelectorAll('.log .m').length > n, second);
    assert(state.calls.some(c => /\/sessions\/demo-1\/events$/.test(c.path) && c.query.includes(`since=${last - 600}`) && c.query.includes('limit=300')), 'An earlier page comes from the server on request.');
    checkRequests(state);
  } finally { await context.close(); }
});

await test('A conversation reopened with an old cache on the device starts from the fresh tail', async () => {
  // the app was closed on the phone (or unloaded in the background) and opened again after the work went on elsewhere
  const { page, state, context } = await fixture({ width: 412, height: 915 });
  try {
    await page.locator('.m.agent').filter({ hasText: 'Interface review' }).waitFor();
    // the feed goes into the device cache when the page is hidden
    await page.evaluate(() => { Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true }); document.dispatchEvent(new Event('visibilitychange')); });
    await page.waitForFunction(() => new Promise(res => { const r = indexedDB.open('agents-chat', 1); r.onsuccess = () => { try { const q = r.result.transaction('sessions').objectStore('sessions').count(); q.onsuccess = () => res(q.result > 0); q.onerror = () => res(false); } catch { res(false); } }; r.onerror = () => res(false); }), null, { timeout: 5000 });
    const gone = turnEvents(3, 3010); const last = gone.at(-1).seq;
    state.history = [...history, ...gone]; state.sessions[0].last_seq = last;
    const before = state.calls.length;
    await page.reload(); await page.locator('.m.agent').filter({ hasText: lastAnswer(gone) }).waitFor({ timeout: 8000 });
    const asked = state.calls.slice(before).filter(c => /\/sessions\/demo-1\/events$/.test(c.path)).map(c => c.query);
    assert.deepEqual(asked.length, 1, `One request for the last page, nothing else: ${JSON.stringify(asked)}`);
    assert(asked[0].includes(`since=${last - 300}`), asked[0]);
    assert.equal(await page.locator('.m.agent').filter({ hasText: 'Interface review' }).count(), 0, 'The old cached screen is not what the reader gets.');
    const st = await page.evaluate(() => ({ url: window.__composerFixture.stats.urls.at(-1), following: document.querySelector('.log').dataset.following, earlier: !!document.querySelector('.log .earlier') }));
    assert(st.url.includes(`since=${last}`) && st.following === 'true' && st.earlier, JSON.stringify(st));
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Missed events within a page come as one update, not one by one', async () => {
  const { page, state, context } = await fixture({ width: 412, height: 915 });
  try {
    await page.locator('.chat-composer textarea').first().waitFor(); await page.locator('.m.agent').first().waitFor();
    const gone = turnEvents(3, 140); const last = gone.at(-1).seq;
    state.history = [...history, ...gone]; state.sessions[0].last_seq = last;
    await page.evaluate(() => { window.__composerFixture.disconnect(); window.__feedUpdates = 0; new MutationObserver(() => { window.__feedUpdates++; }).observe(document.querySelector('.log'), { childList: true, subtree: true }); });
    await page.evaluate(({ gone }) => window.__composerFixture.comeBack('demo-1', gone), { gone });
    await page.locator('.m.agent').filter({ hasText: lastAnswer(gone) }).waitFor({ timeout: 8000 });
    const st = await page.evaluate(() => ({ frames: window.__composerFixture.stats.frames, updates: window.__feedUpdates }));
    assert.equal(st.frames, gone.length, 'A gap within a page is replayed through the socket as before.');
    assert(st.updates <= 8, `The feed is redrawn a few times, not once per event: ${st.updates} updates for ${st.frames} events.`);
    assert(!state.calls.some(c => /\/sessions\/demo-1\/events$/.test(c.path) && c.query.includes(`since=${last - 300}`)), 'No page is asked for when the gap is small.');
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Scrolling up loads the history in chunks down to its first message', async () => {
  // "the whole history matters — when needed, scroll up to the head, even if in chunks" (the owner, 2026-11-23)
  const { page, state, context } = await fixture({ width: 412, height: 915 });
  try {
    const all = turnEvents(3, 1050); state.history = [...history, ...all]; state.sessions[0].last_seq = all.at(-1).seq;
    await page.reload(); await page.locator('.m.agent').filter({ hasText: lastAnswer(all) }).waitFor();
    assert.equal(await page.locator('.m.user').filter({ hasText: 'Review the frontend and improve the UX.' }).count(), 0, 'The conversation opens at its end, the beginning is not loaded.');
    const box = await page.locator('.log').boundingBox(); await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    let pages = 0;
    for (let i = 0; i < 60; i++) {
      if (await page.locator('.m.user').filter({ hasText: 'Review the frontend and improve the UX.' }).count()) break;
      await page.mouse.wheel(0, -4000); await page.waitForTimeout(250); pages++;
    }
    await page.locator('.m.user').filter({ hasText: 'Review the frontend and improve the UX.' }).waitFor();
    assert.equal(await page.locator('.log .earlier').count(), 0, 'At the first message nothing earlier is left.');
    const asked = state.calls.filter(c => /\/sessions\/demo-1\/events$/.test(c.path) && /limit=\d{1,3}(&|$)/.test(c.query)).map(c => c.query);
    assert(asked.length >= 3 && asked.length <= 5, `The history came in pages of up to 300 events, one request each: ${JSON.stringify(asked)}`);
    const first = await page.evaluate(() => { const log = document.querySelector('.log'); const seqs = [...log.querySelectorAll('[data-seq]')].map(el => Number(el.dataset.seq)); return { min: Math.min(...seqs), sorted: seqs.every((v, i) => i === 0 || v > seqs[i - 1]) }; });
    assert(first.min <= 2 && first.sorted, `The feed runs from the first event in order: ${JSON.stringify(first)}`);
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Simple chat: a long provider error wraps inside a phone screen', async () => {
  // The owner's phone (412 px), 2026-11-23: a Google quota error names its metrics as words of 80 characters without a
  // space; the note did not break them and pushed the chat wider than the screen.
  const quota = '429 You exceeded your current quota, please check your plan and billing details. * Quota exceeded for metric: generativelanguage.googleapis.com/generate_content_free_tier_input_token_count, limit: 0, model: gemini-4-argon * Quota exceeded for metric: generativelanguage.googleapis.com/generate_content_free_tier_requests, limit: 0, model: gemini-4-argon. Please retry in 11h51m1.529268062s. — provider limit or quota: wait or choose another route';
  for (const width of [412, 320]) {
    const { page, state, context } = await fixture({ width, height: 915 });
    try {
      state.simpleMessages = [{ ...simpleMessages[0], id: 'q1' }, { ...simpleMessages[1], id: 'a1', text: '', error: quota }];
      await page.locator('.tabbar').getByRole('button', { name: 'Chat', exact: true }).click();
      const note = page.locator('.m.sys.note'); await note.waitFor();
      const box = await note.evaluate(el => { const r = el.getBoundingClientRect(); const log = el.closest('.msgs, .log') ?? el.parentElement; return { left: r.left, right: r.right, view: innerWidth, page: document.documentElement.scrollWidth, logScroll: log.scrollWidth, logClient: log.clientWidth }; });
      assert(box.left >= 0 && box.right <= box.view + .5, `${width}px: the error note stays inside the screen: ${JSON.stringify(box)}`);
      assert(box.page <= box.view && box.logScroll <= box.logClient, `${width}px: nothing scrolls sideways because of it: ${JSON.stringify(box)}`);
      assert.equal(await note.getByRole('button', { name: 'Retry', exact: true }).isVisible(), true, 'The retry action is within reach under the text.');
    } finally { await context.close(); }
  }
});

await test('Thread: an error row and a service note with long words wrap inside a phone screen', async () => {
  // the same defect as in the simple chat, found by a probe on 2026-11-23: the row of an `error` event and an agent note
  // did not break a long word and made the thread scroll sideways
  const long = '429 You exceeded your current quota. * Quota exceeded for metric: generativelanguage.googleapis.com/generate_content_free_tier_input_token_count, limit: 0, model: gemini-4-argon * Quota exceeded for metric: generativelanguage.googleapis.com/generate_content_free_tier_requests_per_model_per_day_for_project, limit: 0.';
  const { page, state, context } = await fixture({ width: 412, height: 915 });
  try {
    state.history = [
      { seq: 1, ts, kind: 'user', content: [{ type: 'text', text: 'ask ' + long }] },
      { seq: 2, ts: ts + 1, kind: 'assistant', content: [{ type: 'text', text: 'API Error: ' + long }] },
      { seq: 3, ts: ts + 2, kind: 'result', is_error: true, subtype: 'error_during_execution', num_turns: 1, duration_ms: 1000, total_cost_usd: 0, cost_usd: 0, usage: {}, text: 'API Error: ' + long },
      { seq: 4, ts: ts + 3, kind: 'error', message: long },
      { seq: 5, ts: ts + 4, kind: 'system', subtype: 'agent_note', data: { message: 'note ' + long } },
    ];
    await page.reload(); const row = page.locator('.m.sys').filter({ hasText: '429 You exceeded' }).first(); await row.waitFor();
    const box = await page.evaluate(() => { const log = document.querySelector('.log'); return { view: innerWidth, page: document.documentElement.scrollWidth, logScroll: log.scrollWidth, logClient: log.clientWidth, wide: [...document.querySelectorAll('.m')].filter(el => el.getBoundingClientRect().right > innerWidth + .5 || el.scrollWidth > el.clientWidth + 1).map(el => (el.textContent || '').slice(0, 24)) }; });
    assert.deepEqual(box.wide, [], `No message is wider than the screen: ${JSON.stringify(box)}`);
    assert(box.page <= box.view && box.logScroll <= box.logClient, `The thread does not scroll sideways: ${JSON.stringify(box)}`);
  } finally { await context.close(); }
});

await test('Simple chat: a failed answer is retried in place, by the same or another model', async () => {
  // On the stand every fifth answer ended with a provider error, and the question had to be typed again (2026-11-22).
  const { page, state, context } = await fixture();
  try {
    state.simpleMessages = [{ ...simpleMessages[0], id: 'q1' }, { ...simpleMessages[1], id: 'a1', text: '', error: '503 No capacity available for model demo-model' }];
    await openSimple(page);
    const note = page.locator('.m.sys.note'); await note.waitFor();
    assert((await note.textContent()).includes('503 No capacity'), 'The provider error is shown instead of the answer.');
    await note.getByRole('button', { name: 'Retry', exact: true }).click();
    await page.locator('.m.agent').filter({ hasText: 'Fixture retry reply' }).waitFor();
    const sent = state.calls.filter(c => c.path === '/chats/simple-1/messages');
    assert.deepEqual([sent.length, sent[0].body.regenerate, sent[0].body.text], [1, true, ''], 'One request, a retry without a new message.');
    assert.equal(await page.locator('.m.user').count(), 1, 'The question is not repeated in the chat.');
    assert.equal(await note.count(), 0, 'The error gives place to the answer.');
    // the last answer has its own retry among the message actions; an older one does not
    assert.equal(await page.locator('.m.agent .ma.retry').count(), 1);
    assert.equal(await page.locator('.m.agent .ma.copy').count(), 1);
    // another model: the pick becomes the model of the chat, then the answer is asked again
    state.simpleMessages = [{ ...simpleMessages[0], id: 'q1' }, { ...simpleMessages[1], id: 'a1', text: '', error: 'stopped' }];
    await page.reload(); await openSimple(page);
    assert.equal((await page.locator('.m.sys.note').textContent()).trim().startsWith('Stopped'), true, 'A stop is told in words, not as an error.');
    assert.equal(await page.locator('.m.sys.note .bad').count(), 0);
    await page.locator('.m.sys.note').getByRole('button', { name: 'Another model…', exact: true }).click();
    await menuAction(page, 'Alternate model').click();
    await page.locator('.m.agent').filter({ hasText: 'Fixture retry reply' }).waitFor();
    assert.equal(state.simple.model, 'alternate-model');
    assert.equal(state.calls.filter(c => c.path === '/chats/simple-1/messages').at(-1).body.regenerate, true);
    // a question left without any answer offers the same
    state.simpleMessages = [{ ...simpleMessages[0], id: 'q1' }]; state.simple.model = 'demo-model';
    await page.reload(); await openSimple(page);
    await page.locator('.m.sys.note').filter({ hasText: 'No answer was received' }).getByRole('button', { name: 'Retry', exact: true }).waitFor();
    await screenshot(page, 'simple-chat-retry'); checkRequests(state);
  } finally { await context.close(); }
});

await test('Simple chat: an own message is edited and the tail of the chat is replaced', async () => {
  const { page, state, context } = await fixture();
  try {
    state.simpleMessages = [
      { ...simpleMessages[0], id: 'q1', text: 'First question', attachments: [{ file: 'f1.txt', name: 'spec.txt', mime: 'text/plain', size: 1200, text: '' }, { file: 'f2.txt', name: 'notes.txt', mime: 'text/plain', size: 800, text: '' }] },
      { ...simpleMessages[1], id: 'a1', text: 'First answer' }, { ...simpleMessages[0], id: 'q2', seq: 3, text: 'Second question' }, { ...simpleMessages[1], id: 'a2', seq: 4, text: 'Second answer' }];
    await openSimple(page);
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.fill('A draft in progress');
    await page.locator('.m.user').first().hover();
    await page.locator('.m.user').first().getByRole('button', { name: 'Edit and resend', exact: true }).click();
    await page.locator('.editbar').waitFor();
    assert.equal(await input.inputValue(), 'First question', 'The message returns to the field.');
    assert.equal(await page.locator('.thumb.kept').count(), 2, 'Its attachments come with it.');
    // Escape leaves the edit and brings the draft back
    await input.press('Escape');
    assert.equal(await page.locator('.editbar').count(), 0);
    assert.equal(await input.inputValue(), 'A draft in progress');
    await page.locator('.m.user').first().hover();
    await page.locator('.m.user').first().getByRole('button', { name: 'Edit and resend', exact: true }).click();
    await page.getByRole('button', { name: 'Remove notes.txt', exact: true }).click();
    await input.fill('First question, more precise');
    await page.getByRole('button', { name: 'Save edit', exact: true }).click();
    await page.locator('.m.agent').filter({ hasText: 'Fixture reply to the edit' }).waitFor();
    const body = state.calls.filter(c => c.path === '/chats/simple-1/messages').at(-1).body;
    assert.deepEqual([body.text, body.replace_from, body.keep_files], ['First question, more precise', 'q1', ['f1.txt']]);
    assert.deepEqual(await page.locator('.m.user .txt').allTextContents(), ['First question, more precise'], 'The edited message replaced itself and the tail.');
    assert.equal(await page.locator('.m.agent').count(), 1);
    assert.equal(await page.locator('.editbar').count(), 0);
    assert.equal(await input.inputValue(), 'A draft in progress', 'The draft set aside for the edit is back.');
    await screenshot(page, 'simple-chat-edited'); checkRequests(state);
  } finally { await context.close(); }
});

await test('Simple chat: a large photo is reduced before upload, a refusal by size is told in words', async () => {
  // A phone shot of several megabytes did not pass the request limit ("HTTP 413") and was sent again with every later turn.
  const { page, state, context } = await fixture();
  try {
    await openSimple(page);
    // a 3000×2000 photo-like JPEG made in the page: noise does not compress, the file is a few megabytes
    const photo = await page.evaluate(async () => {
      const c = document.createElement('canvas'); c.width = 3000; c.height = 2000; const g = c.getContext('2d'); const d = g.createImageData(c.width, c.height);
      for (let i = 0; i < d.data.length; i += 4) { d.data[i] = Math.random() * 255; d.data[i + 1] = Math.random() * 255; d.data[i + 2] = Math.random() * 255; d.data[i + 3] = 255; }
      g.putImageData(d, 0, 0);
      const blob = await new Promise(res => c.toBlob(res, 'image/jpeg', 0.95));
      return { size: blob.size, b64: await new Promise(res => { const r = new FileReader(); r.onload = () => res(String(r.result).split(',')[1]); r.readAsDataURL(blob); }) };
    });
    assert(photo.size > 2 * 1024 * 1024, `The test photo must be larger than the old limit (${photo.size}).`);
    await page.locator('.composer input[type="file"]').first().setInputFiles({ name: 'IMG_0001.jpg', mimeType: 'image/jpeg', buffer: Buffer.from(photo.b64, 'base64') });
    await page.locator('.thumb img').waitFor();
    await page.getByRole('textbox', { name: 'Message', exact: true }).fill('What is in the photo?');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await page.locator('.m.agent').filter({ hasText: 'Fixture reply' }).waitFor();
    const sent = state.calls.filter(c => c.path === '/chats/simple-1/messages').at(-1).body.attachments[0];
    const bytes = Buffer.from(sent.data, 'base64');
    const dims = await page.evaluate(async b64 => { const img = await createImageBitmap(await (await fetch(`data:image/jpeg;base64,${b64}`)).blob()); return [img.width, img.height]; }, sent.data);
    assert.deepEqual([sent.mime, sent.name, dims], ['image/jpeg', 'IMG_0001.jpg', [2048, 1365]], 'The photo is scaled to 2048 px on the long side.');
    assert(bytes.length < photo.size / 2, `The upload is at least twice smaller (${bytes.length} of ${photo.size}).`);
    // a small screenshot goes as it is
    const small = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=', 'base64');
    await page.locator('.composer input[type="file"]').first().setInputFiles({ name: 'shot.png', mimeType: 'image/png', buffer: small });
    await page.locator('.thumb img').waitFor();
    await page.getByRole('textbox', { name: 'Message', exact: true }).fill('And here?');
    state.simpleStatus = 413;
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: 'The attachments are larger than 8 MB together.' }).waitFor();
    assert.equal(state.calls.filter(c => c.path === '/chats/simple-1/messages').at(-1).body.attachments[0].data, small.toString('base64'), 'A small image is not re-encoded.');
    assert(await page.locator('.thumb img').isVisible(), 'The refused attachment stays in the draft.');
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Simple chat opened in the middle of an answer shows what is written so far', async () => {
  const { page, state, context } = await fixture();
  try {
    state.simpleMessages = [{ ...simpleMessages[0], id: 'q1' }];
    state.simpleBusy = { text: 'The first half of the answer', tools: [{ name: 'fetch_url', args: { url: 'https://example.com' }, result: 'ok', ok: true }], running: null };
    await openSimple(page);
    await page.locator('.m.agent.live').filter({ hasText: 'The first half of the answer' }).waitFor();
    assert.equal(await page.locator('.m.tool').count(), 1, 'The tool calls made so far are shown too.');
    assert.equal(await page.locator('.m.sys.note').count(), 0, 'No "no answer" note while the answer is being written.');
    assert(await page.getByRole('button', { name: 'Stop', exact: true }).isVisible());
    // the text grows with the next poll, and the finished answer replaces it
    state.simpleBusy = { text: 'The first half of the answer and the second', tools: state.simpleBusy.tools, running: null };
    await page.locator('.m.agent.live').filter({ hasText: 'and the second' }).waitFor({ timeout: 8000 });
    state.simpleBusy = null; state.simpleMessages.push({ ...simpleMessages[1], id: 'a1', text: 'The whole answer' });
    await page.locator('.m.agent:not(.live)').filter({ hasText: 'The whole answer' }).waitFor({ timeout: 8000 });
    assert.equal(await page.locator('.m.agent.live').count(), 0);
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Parameters and model menus support keyboard navigation without trapping focus', async () => {
  const { page, state, context } = await fixture();
  try {
    await openSimple(page);
    const params = page.getByRole('button', { name: 'Parameters', exact: true });
    await params.focus(); await page.keyboard.press('Enter');
    const search = page.getByRole('menuitemcheckbox', { name: 'search', exact: true }); await search.waitFor();
    await page.waitForFunction(() => document.activeElement?.textContent?.trim() === 'search');
    assert.equal(await search.getAttribute('aria-checked'), 'false');
    await page.keyboard.press('ArrowDown');
    assert.equal(await page.evaluate(() => document.activeElement.textContent.trim()), 'pages');
    await page.keyboard.press('End');
    assert.match(await page.evaluate(() => document.activeElement.textContent), /^image/);
    await page.keyboard.press('Home');
    assert.equal(await page.evaluate(() => document.activeElement.textContent.trim()), 'search');
    await page.keyboard.press('Escape'); await page.getByRole('menu').waitFor({ state: 'hidden' });
    assert(await params.evaluate(el => el === document.activeElement), 'Escape should restore focus to Parameters.');
    await page.keyboard.press('Enter'); await search.waitFor();
    await page.waitForFunction(() => document.activeElement?.textContent?.trim() === 'search');
    await page.keyboard.press('Tab');
    await page.getByRole('menu').waitFor({ state: 'hidden' });
    assert(await page.locator('.composer-actions button:not(:disabled):visible').first().evaluate(el => el === document.activeElement), 'Tab should move from Parameters to the first available composer action.');
    await params.focus(); await page.keyboard.press('Enter'); await search.waitFor();
    await page.waitForFunction(() => document.activeElement?.textContent?.trim() === 'search');
    await page.keyboard.press('Shift+Tab');
    await page.getByRole('menu').waitFor({ state: 'hidden' });
    assert(await page.getByRole('button', { name: 'Demo model', exact: true }).evaluate(el => el === document.activeElement), 'Shift+Tab should move to the preceding model control.');
    await page.keyboard.press('Enter');
    const modelSearch = page.getByRole('menu').getByRole('searchbox'); await modelSearch.waitFor();
    await page.waitForFunction(() => document.activeElement?.matches('.menu input[type="search"]'));
    assert(await modelSearch.evaluate(el => el === document.activeElement), 'Searchable model picker should focus its search box.');
    await modelSearch.fill('Alternate'); await page.keyboard.press('Enter');
    await page.getByRole('button', { name: 'Alternate model', exact: true }).waitFor();
    assert.equal(state.simple.model, 'alternate-model');
    checkRequests(state);
  } finally { await context.close(); }
});

for (const dimensions of [
  { view: 'code', width: 1920, height: 1080, name: 'code-wide' },
  { view: 'code', width: 1280, height: 720, name: 'code-laptop' },
  { view: 'thread', width: 1280, height: 800, name: 'thread' },
  { view: 'thread', width: 320, height: 700, name: 'phone-small-thread' },
  { view: 'thread', width: 390, height: 844, name: 'phone-thread' },
]) {
  await test(`Composer remains usable at ${dimensions.name}`, async () => {
    const { page, state, context } = await fixture({ ...dimensions, lang: 'ru' });
    try {
      await page.locator(dimensions.width < 600 ? '.composer-short-context' : '.composer-context').first().waitFor();
      const metrics = await fitViewport(page); await screenshot(page, dimensions.name);
      if (dimensions.width < 600) {
        const headerHeight = await page.locator('.t-head').evaluate(el => el.getBoundingClientRect().height);
        assert(headerHeight <= 90, 'Phone session heading should leave room for chat history.');
        const toolbarButtons = await page.locator('.chat-composer .composer-toolbar button').evaluateAll(els => els.map(el => { const rect = el.getBoundingClientRect(); return { width: rect.width, height: rect.height }; }).filter(rect => rect.width && rect.height));
        assert(toolbarButtons.length >= 4, 'Model, Parameters, attachment and Send must remain visible.');
        assert(toolbarButtons.every(rect => rect.height >= 31.5 && rect.height <= 32.5 && rect.width >= 43.5), `Phone toolbar buttons should be 32px high and retain 44px width: ${JSON.stringify(toolbarButtons)}`);
        const tabHeights = await page.locator('.pane .tab').evaluateAll(els => els.map(el => el.getBoundingClientRect().height));
        assert(tabHeights.every(height => height >= 31.5 && height <= 32.5), `Phone session tabs should be 32px high: ${JSON.stringify(tabHeights)}`);
        assert(metrics.composers[0].bottom <= metrics.height, 'The phone composer must stay above the bottom navigation.');
        const meta = await page.locator('meta[name="viewport"]').getAttribute('content');
        assert(!/maximum-scale|user-scalable\s*=\s*no/i.test(meta), 'The phone viewport must allow zoom.');
      }
      checkRequests(state); return metrics;
    } finally { await context.close(); }
  });
}

await test('Simple chat and Parameters remain inside phone viewport', async () => {
  const { page, state, context } = await fixture({ width: 390, height: 844 });
  try {
    await page.locator('.tabbar').getByRole('button', { name: 'Chat', exact: true }).click();
    await page.getByRole('button', { name: 'Demo model', exact: true }).waitFor();
    const metrics = await fitViewport(page); assert(metrics.composers[0].bottom <= metrics.height);
    await screenshot(page, 'phone-simple-chat');
    await page.getByRole('button', { name: 'Parameters', exact: true }).click();
    const menu = page.getByRole('menu'); await menu.waitFor();
    const rect = await menu.boundingBox(); assert(rect.x >= 0 && rect.x + rect.width <= 391 && rect.y >= 0 && rect.y + rect.height <= 845, 'Parameters menu must fit the phone viewport.');
    await screenshot(page, 'phone-simple-parameters'); checkRequests(state); return metrics;
  } finally { await context.close(); }
});

await test('Phone thread: turn summary and message actions', async () => {
  // Found on the owner's phone (2026-11-22): the summary showed the money of the whole conversation ($455.756 after a
  // 12-second turn), its separators had lost a space each ("16.0 s· 1 turn"), and the edit/fork buttons were two black
  // 44-pixel squares inside the owner's message.
  const { page, state, context } = await fixture({ width: 362, height: 796 });
  try {
    await page.locator('.thread .chat .m.sys.turn').first().waitFor();
    const facts = await page.evaluate(() => {
      const plate = el => { const c = getComputedStyle(el); const r = el.getBoundingClientRect(); return { bg: c.backgroundColor, w: r.width, h: r.height }; };
      return {
        coarse: matchMedia('(pointer: coarse)').matches && matchMedia('(hover: none)').matches,
        turn: document.querySelector('.thread .chat .m.sys.turn').textContent,
        user: [...document.querySelectorAll('.thread .chat .m.user .ma')].map(plate),
        agent: [...document.querySelectorAll('.thread .chat .m.agent:not(.typing) .ma')].map(plate),
      };
    });
    assert(facts.coarse, 'The scenario must run as a touch device.');
    assert.equal(facts.turn, '✓ · 16.0 s · 1 turn · 400↑ 700↓ · cache 60k↺ · +4.6k · $0.312', 'The summary shows the turn\'s own cost and evenly spaced separators.');
    assert(facts.user.length >= 1 && facts.agent.length >= 1, 'Both bubbles carry their actions on touch.');
    for (const b of [...facts.user, ...facts.agent]) {
      assert.equal(b.bg, 'rgba(0, 0, 0, 0)', 'A message action on touch is a bare icon, not a plate.');
      assert(b.w >= 44 && b.h >= 44, 'A message action keeps a finger-sized target.');
    }
    checkRequests(state);
    return facts.turn;
  } finally { await context.close(); }
});

await test('Send action looks the same with an empty field and with a draft', async () => {
  // A helper class on the send button once matched the global `.empty` rule of the placeholders: with an empty field the
  // button became a grid, and its label dropped under the icon and out of the button. In a wide composer the action stays
  // in place during a turn as well (only the narrow one swaps it for the stop button).
  const shape = page => page.evaluate(() => {
    const button = document.querySelector('.thread .chat .composer-send'), box = el => el.getBoundingClientRect();
    const own = box(button), icon = box(button.querySelector('svg')), label = box(button.querySelector('.action-label'));
    return { display: getComputedStyle(button).display, width: own.width, height: own.height, text: button.querySelector('.action-label').textContent,
      oneLine: Math.abs((icon.top + icon.bottom) / 2 - (label.top + label.bottom) / 2) < 2, inside: label.width > 0 && label.top >= own.top && label.bottom <= own.bottom && label.right <= own.right };
  });
  const report = {};
  for (const [session, text] of [['demo-1', 'Send'], ['demo-2', 'Queue']]) {
    const { page, state, context } = await fixture({ session });
    try {
      const empty = await shape(page);
      await page.locator('.thread .chat textarea').fill('One more thing');
      const typed = await shape(page);
      assert(typed.oneLine && typed.inside && typed.text === text, `${text}: the icon and the label share a line inside the button.`);
      assert.deepEqual(empty, typed, `${text}: an empty field changes nothing in the button but its availability.`);
      checkRequests(state); report[session] = { empty, typed };
    } finally { await context.close(); }
  }
  return report;
});

await test('Phone composer keeps one row of buttons while the agent works', async () => {
  // Found on the owner's phone (2026-11-22, 412 px): during a turn the action became a labelled "Queue" button next to the new
  // stop button, the actions dropped to a second line and the composer grew by 36 px each time the agent started working.
  const label = caps.models[0].label; caps.models[0].label = 'opus-5-5[1m]';
  const measure = page => page.evaluate(() => {
    const box = el => el.getBoundingClientRect(), shown = el => !!el && el.getClientRects().length > 0;
    const tb = document.querySelector('.thread .chat .composer-toolbar'), send = tb.querySelector('.composer-send'), model = tb.querySelector('.composer-control.model');
    const buttons = [...tb.querySelectorAll('button')].filter(shown).map(box).sort((a, b) => a.top - b.top || a.left - b.left);
    return {
      toolbar: box(tb).height, composer: box(document.querySelector('.thread .chat .composer')).height, model: box(model).width,
      send: shown(send), sendWidth: shown(send) ? box(send).width : 0, sendLabel: shown(send.querySelector('.action-label')), stop: shown(tb.querySelector('.stop')),
      // buttons that leave the row on the right or lie on their neighbour
      overflow: Math.max(...buttons.map(b => b.right)) - box(tb).right,
      overlap: Math.max(0, ...buttons.slice(1).map((b, i) => Math.abs(b.top - buttons[i].top) < 1 ? buttons[i].right - b.left : 0)),
    };
  });
  const states = async (width, session) => {  // the composer of a session: with an empty field and with a typed message
    const { page, state, context } = await fixture({ width, height: 844, session });
    try {
      const empty = await measure(page);
      await page.locator('.thread .chat textarea').fill('One more thing');
      const typed = await measure(page), name = await page.locator('.thread .chat .composer-send').getAttribute('aria-label');
      checkRequests(state); return { empty, typed, name };
    } finally { await context.close(); }
  };
  const report = {};
  try {
    for (const width of [412, 360]) {
      const rest = (await states(width, 'demo-1')).empty, { empty, typed, name } = await states(width, 'demo-2');
      assert(rest.send && !rest.stop, `${width} px: at rest the row ends with the send button.`);
      assert.equal(empty.toolbar, rest.toolbar, `${width} px: a working session keeps the buttons on one row.`);
      assert.equal(empty.composer, rest.composer, `${width} px: the composer does not grow when the agent starts working.`);
      assert(empty.stop && !empty.send, `${width} px: with an empty field the stop button stands in the place of the action.`);
      assert.equal(empty.model, rest.model, `${width} px: the model button is as wide as at rest.`);
      assert.equal(typed.toolbar, rest.toolbar, `${width} px: typing during a turn keeps the buttons on one row.`);
      assert(typed.stop && typed.send && !typed.sendLabel && typed.sendWidth <= 48, `${width} px: the queue action is an icon next to the stop button.`);
      assert.equal(name, 'Queue', 'The icon keeps its name for assistive technology.');
      assert(typed.model >= 44, `${width} px: the model button gives way but stays a touch target (${typed.model}).`);
      for (const [what, m] of Object.entries({ rest, empty, typed })) assert(m.overflow <= .5 && m.overlap <= .5, `${width} px, ${what}: buttons stay in the row and apart (overflow ${m.overflow}, overlap ${m.overlap}).`);
      report[width] = { rest, empty, typed };
    }
    // narrower screens (the cover of a Fold, 280 px of the first one) may wrap, but nothing leaves the composer or overlaps
    for (const width of [344, 280]) {
      const { empty, typed } = await states(width, 'demo-2');
      for (const [what, m] of Object.entries({ empty, typed })) assert(m.overflow <= .5 && m.overlap <= .5, `${width} px, ${what}: buttons stay in the composer and apart (overflow ${m.overflow}, overlap ${m.overlap}).`);
      report[width] = { empty, typed };
    }
    return report;
  } finally { caps.models[0].label = label; }
});

// ---------- notification channels (docs/agents-ux.md §9.12) ----------

await test('The note about the report box is folded in the feed, and an edit starts from the owner\'s own words', async () => {
  const note = '# Отчёты владельцу\nВладелец разрешил этой беседе присылать ему отчёты и файлы в его канал уведомлений. Чтобы прислать: запиши отчёт в Markdown-файл и выполни\n`sh ~/.cloudos/bin/cloudos-report demo-1 <отчёт.md> [файлы…]`';
  const seed = [{ seq: 1, ts, kind: 'user', content: [{ type: 'text', text: `Research the market and send me a report.\n\n---\n${note}` }] }, history[1], history[2]];
  const { page, state, context } = await fixture({ seed });
  try {
    const mine = page.locator('.m.user'); await mine.getByText('Research the market and send me a report.', { exact: true }).waitFor();
    await mine.getByText('the agent was told how to send the owner a report').waitFor();
    assert(!(await mine.innerText()).includes('cloudos-report'), 'The note itself is the service\'s, not a part of the message.');
    await mine.hover(); await mine.getByRole('button', { name: 'Edit and resend', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.composer textarea')?.value === 'Research the market and send me a report.');
    // the input history (arrow up) recalls the owner's words, not the note
    await page.locator('.composer .editbar').getByRole('button', { name: /^cancel$/i }).click();
    const input = page.getByRole('textbox', { name: 'Message', exact: true }); await input.focus(); await input.press('ArrowUp');
    assert.equal(await input.inputValue(), 'Research the market and send me a report.');
    assert.equal(await page.locator('.m.agent .ma.tochan').count(), 0, 'Without a channel there is nothing to send an answer to.');
    checkRequests(state);
  } finally { await context.close(); }
});

await test('An answer goes to a channel by the owner\'s own pick', async () => {
  const tg = { id: 'ch_tg', kind: 'telegram', name: 'Личный', enabled: true, bot: '@stand_bot', chat_title: 'Alex', chat_linked: true, protect: true, buttons: false, slack_channel: '', host: '', tail: '', internal: false, signed: false, created_at: ts, files: true, bindings: [], sent_today: 0, last: null };
  const { page, state, context } = await fixture({ channels: [tg, { ...tg, id: 'ch_off', name: 'Выключенный', enabled: false }] });
  try {
    const answer = page.locator('.m.agent').first(); await answer.hover();
    await answer.getByRole('button', { name: 'Send to a channel', exact: true }).click();
    const menu = page.getByRole('menu'); await menu.getByText('Send to a channel').waitFor();
    assert.equal(await menu.getByRole('menuitem').count(), 1, 'A channel that is switched off is not offered.');
    assert.equal(state.calls.filter(c => c.path === '/notify/share').length, 0, 'Nothing leaves before a channel is picked.');
    await menu.getByRole('menuitem', { name: 'Telegram · Личный' }).click();
    await page.getByText('Sent to "Личный"').waitFor();
    const sent = state.calls.find(c => c.path === '/notify/share').body;
    assert.deepEqual({ channel: sent.channel, project: sent.project, session: sent.session, seq: sent.seq, title: sent.title }, { channel: 'ch_tg', project: 'alpha', session: 'demo-1', seq: 2, title: state.sessions[0].title });
    checkRequests(state);
  } finally { await context.close(); }
});

await test('Empty phone thread opens the workspace drawer', async () => {
  const { page, state, context } = await fixture({ width: 390, height: 844, session: null });
  try {
    await page.locator('main .content').getByRole('button', { name: 'Open workspaces', exact: true }).click();
    assert(await page.locator('.app').evaluate(el => el.classList.contains('popen')));
    checkRequests(state);
  } finally { await context.close(); }
});

await browser.close();
await writeFile(join(output, 'report.json'), JSON.stringify(results, null, 2));
console.log(`${results.filter(result => result.pass).length}/${results.length} scenarios passed. Artifacts: ${output}`);
if (results.some(result => !result.pass)) process.exitCode = 1;
