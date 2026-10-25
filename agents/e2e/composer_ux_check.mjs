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
  { seq: 3, ts: ts + 2, kind: 'result', num_turns: 1, duration_ms: 16000, total_cost_usd: .04, usage: { input_tokens: 400, output_tokens: 700, cache_read_input_tokens: 60000 } },
];
const permission = { seq: 3, ts: ts + 2, kind: 'permission_request', request_id: 'approval-demo', tool_name: 'Bash', input: { command: 'npm run check' }, suggestions: [{ rules: [{ toolName: 'Bash', ruleContent: 'npm run check:*' }] }] };
const simpleBase = { id: 'simple-1', kind: 'simple', title: 'Discuss the interface', route: 'gate-demo', model: 'demo-model', system: '', tools: { search: false, fetch: false }, incognito: false, created_at: ts - 600, updated_at: ts, cost_usd: .04, tokens_in: 1200, tokens_out: 700, messages: 2 };
const routes = { routes: [{ id: 'gate-demo', kind: 'gate', title: 'Project gate', provider: 'anthropic', host: 'fixture.invalid', models: [{ id: 'demo-model', name: 'Demo model', context: 200000, provider: 'anthropic', provider_title: 'Claude', kind: 'chat' }, { id: 'alternate-model', name: 'Alternate model', context: 100000, provider: 'anthropic', provider_title: 'Claude', kind: 'chat' }] }], defaults: { route: 'gate-demo', model: 'demo-model' }, search: { ready: true, kind: 'searxng' } };
const simpleMessages = [{ id: 'm1', chat: simpleBase.id, seq: 1, role: 'user', text: 'What makes a composer easy to use?', attachments: [], tools: [], tokens_in: 0, tokens_out: 0, cost_usd: 0, created_at: ts }, { id: 'm2', chat: simpleBase.id, seq: 2, role: 'assistant', text: 'A clear primary action, concise settings and reliable drafts.', attachments: [], tools: [], tokens_in: 1200, tokens_out: 700, cost_usd: .04, created_at: ts + 1 }];

const browser = await chromium.launch({ headless: true, executablePath });
const results = [];

/** Each scenario gets independent state, intercepted requests and a fake session stream. */
async function fixture({ view = 'thread', session = 'demo-1', width = 1280, height = 800, offline = false, lang = 'en' } = {}) {
  const sessions = [structuredClone(baseSession), { ...structuredClone(baseSession), id: 'demo-2', title: 'Improve settings while the agent works', status: 'working' }, { ...structuredClone(baseSession), id: 'demo-3', title: 'Decision required for validation', status: 'waiting', pending: 1, last_seq: 183 }];
  const workspace = { projects: [{ name: 'alpha', description: 'Local browser fixture', status: 'running', reachable: true, cost_today: usage.cost_usd, apps: [], sessions, ptys: [] }], today: usage };
  const state = { sessions, simple: structuredClone(simpleBase), calls: [], unexpected: [], workspaceFailures: offline ? Infinity : 0, promptFail: false, simpleFail: false, queueDelay: 0 };
  const context = await browser.newContext({ viewport: { width, height }, hasTouch: width < 600, serviceWorkers: 'block' });
  await context.addInitScript(({ sessions, view, session, lang }) => {
    localStorage.setItem('agents.lang', lang);
    localStorage.setItem('agents.ui', JSON.stringify({ view, sel: session ? { project: 'alpha', kind: 'session', id: session } : null, cards: sessions.map(s => ({ project: 'alpha', kind: 'session', id: s.id, w: 1, h: 1 })), cols: 0, chatSel: 'simple-1', chatEngine: 'simple' }));
    const streams = [];
    let paused = false;
    class MockWebSocket {
      static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
      readyState = 0;
      constructor(url) {
        this.url = url; streams.push(this);
        const id = new URL(url).pathname.split('/').at(-2);
        setTimeout(() => {
          if (paused || this.readyState === 3) return;
          this.readyState = 1; this.onopen?.({});
          const info = sessions.find(s => s.id === id);
          if (info) this.onmessage?.({ data: JSON.stringify({ kind: 'hello', info, replay: 0 }) });
        }, 30);
      }
      send() {}
      close() { if (this.readyState === 3) return; this.readyState = 3; this.onclose?.({ code: 1006 }); }
    }
    window.WebSocket = MockWebSocket;
    window.__composerFixture = {
      disconnect: () => { paused = true; streams.forEach(s => s.close()); },
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
    else if (path === '/chats/simple-1' && method === 'GET') result = { chat: state.simple, messages: simpleMessages, busy: false };
    else if (path === '/chats/simple-1' && method === 'PUT') { Object.assign(state.simple, body); result = state.simple; }
    else if (path === '/chats/simple-1/messages') {
      if (state.simpleFail) { status = 503; result = { error: 'Fixture send failed' }; }
      else {
        const user = { ...simpleMessages[0], id: 'sent-user', text: body.text };
        const assistant = { ...simpleMessages[1], id: 'sent-agent', text: 'Fixture reply' };
        await route.fulfill({ contentType: 'text/event-stream', body: `data: ${JSON.stringify({ kind: 'user', message: user })}\n\ndata: ${JSON.stringify({ kind: 'done', message: assistant, chat: state.simple })}\n\n` }); return;
      }
    } else if (/^\/projects\/[^/]+\/sessions\/[^/]+\/events$/.test(path)) {
      result = path.includes('/demo-3/') ? [history[0], history[1], permission, ...Array.from({ length: 180 }, (_, i) => ({ seq: i + 4, ts: ts + i + 3, kind: 'assistant', content: [{ type: 'text', text: `Background update ${i + 1}: validating interface behavior.` }] }))] : history;
      const since = Number(u.searchParams.get('since') ?? 0); result = result.filter(e => e.seq > since);
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
