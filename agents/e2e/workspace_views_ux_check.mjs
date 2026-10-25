#!/usr/bin/env node
/**
 * Isolated Files/Git/Terminal regressions. All API requests and sockets use fixtures.
 * Start: npm --prefix agents/ui run dev -- --host 127.0.0.1 --port 5178
 * Run: UI_URL=http://127.0.0.1:5178/agents/ node agents/e2e/workspace_views_ux_check.mjs
 * Optional: PLAYWRIGHT_MODULE, CHROME_PATH, WORKSPACE_UX_OUT.
 */
import assert from 'node:assert/strict';
import { access, mkdir, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
const url = process.env.UI_URL ?? 'http://127.0.0.1:5178/agents/';
const origin = new URL(url).origin;
const output = resolve(process.env.WORKSPACE_UX_OUT ?? '/tmp/workspace-ux-check');
let chromium;
for (const p of [process.env.PLAYWRIGHT_MODULE, 'playwright'].filter(Boolean)) {
  try { ({ chromium } = await import(p.startsWith('/') ? pathToFileURL(p).href : p)); break; } catch {}
}
assert(chromium, 'Install Playwright or set PLAYWRIGHT_MODULE.');
let executablePath = process.env.CHROME_PATH;
if (!executablePath) for (const p of [chromium.executablePath(), join(homedir(), '.cache/ms-playwright/chromium-1243/chrome-linux64/chrome'), join(homedir(), '.cache/puppeteer/chrome/linux-148.0.7778.97/chrome-linux64/chrome')]) {
  try { await access(p); executablePath = p; break; } catch {}
}
const browser = await chromium.launch({ headless: true, executablePath });
await mkdir(output, { recursive: true });
const results = [];
const delay = ms => new Promise(r => setTimeout(r, ms));
const ts = Math.floor(Date.now() / 1000);
const root = '/home/coder/demo';
const usage = { input: 100, output: 200, cache_read: 0, cache_create: 0, cost_usd: .1, turns: 1 };
const sessions = Array.from({ length: 3 }, (_, i) => ({ id: `demo-${i + 1}`, agent: 'claude', cwd: root, title: `Workspace view ${i + 1}`, created_at: ts, updated_at: ts, model: 'fixture', permission_mode: 'default', effort: 'high', status: 'idle', alive: false, last_seq: 0, pending: 0, queued: [], usage, tui: 'pty-demo' }));
const file = (name, kind = 'file') => ({ name, kind, size: 20, mtime: ts, link: false });
const entries = [file('folder', 'dir'), file('slow.txt'), file('fast.txt')];
const gitEntries = ['slow.txt', 'fast.txt'].map(path => ({ path, kind: 'modified', x: '.', y: 'M', staged: false, unstaged: true }));
async function fixture({ tab = 'files', width = 1600, height = 900, view = 'thread', touch = false, light = false } = {}) {
  const state = { requests: [], unexpected: [], pageErrors: [], slowFile: false, slowDir: false, slowDiff: false, slowUpload: false, fileFail: false, diffFail: false };
  const context = await browser.newContext({ viewport: { width, height }, hasTouch: touch, serviceWorkers: 'block' });
  await context.addInitScript(({ sessions, tab, view, light }) => {
    localStorage.setItem('agents.lang', 'ru');
    localStorage.setItem('agents.appearance', JSON.stringify({ theme: light ? 'light' : 'dark' }));
    localStorage.setItem('agents.ui', JSON.stringify({ view, sel: { project: 'demo', kind: 'session', id: 'demo-1' }, cards: sessions.map(s => ({ project: 'demo', kind: 'session', id: s.id, w: 1, h: 1 })), cols: 3 }));
    for (const s of sessions) localStorage.setItem(`cloudos.agents.tab.demo:${s.id}`, tab);
    const sockets = []; let paused = false; const sent = [];
    class MockWebSocket {
      static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
      readyState = 0;
      constructor(url) { this.url = url; sockets.push(this); setTimeout(() => { if (!paused && this.readyState !== 3) { this.readyState = 1; this.onopen?.({}); } }, 30); }
      send(data) { sent.push({ url: this.url, data: typeof data === 'string' ? data : Array.from(data) }); }
      close() { if (this.readyState !== 3) { this.readyState = 3; this.onclose?.({ code: 1006 }); } }
    }
    window.WebSocket = MockWebSocket;
    window.__workspaceFixture = { sent, disconnect: () => { paused = true; sockets.forEach(s => s.close()); }, resume: () => { paused = false; }, output: text => sockets.filter(s => s.readyState === 1).forEach(s => s.onmessage?.({ data: new TextEncoder().encode(text).buffer })) };
  }, { sessions, tab, view, light });
  await context.route('**/*', async route => {
    const req = route.request(); const u = new URL(req.url()); const method = req.method();
    if (u.hostname === 'fonts.googleapis.com') { await route.fulfill({ contentType: 'text/css', body: '' }); return; }
    if (u.origin !== origin) { state.unexpected.push(`${method} ${u.href}`); await route.abort(); return; }
    if (!u.pathname.includes('/api/')) { await route.continue(); return; }
    const path = u.pathname.replace(/^\/agents\/api(?:\/core)?/, '');
    const target = u.searchParams.get('path'); state.requests.push({ method, path, target });
    let json; let status = 200;
    if (path === '/health') json = { ok: true, cloudd: true, version: 'fixture' };
    else if (path === '/workspaces') json = { projects: [{ name: 'demo', status: 'running', reachable: true, cost_today: .1, apps: [], sessions, ptys: [] }], today: usage };
    else if (path === '/agents/layout') json = { cards: sessions.map(s => ({ project: 'demo', kind: 'session', id: s.id, w: 1, h: 1 })), cols: 3, groups: {} };
    else if (path === '/accounts') json = { accounts: [], defaults: {} };
    else if (path === '/settings/desktop') json = { appearance: { language: 'ru' } };
    else if (path === '/settings') json = { prices: {}, defaults: {}, limits: [] };
    else if (path === '/scheduled') json = { scheduled: [], now: ts };
    else if (path.endsWith('/engines')) json = { engines: [] };
    else if (path === '/projects/demo/ptys') json = [{ id: 'pty-demo', session: 'demo-1', tui: true, alive: true, cwd: root }];
    else if (path === '/projects/demo/fs') {
      if (state.slowDir && target === '/') await delay(600);
      json = { path: target, entries: target === root ? entries : [file(`directory-${target.replaceAll('/', '-')}.txt`)] };
    } else if (path === '/projects/demo/fs/file' && method === 'GET') {
      if (state.slowFile && target.endsWith('/slow.txt')) await delay(600);
      if (state.fileFail) { json = { error: 'Fixture preview unavailable' }; status = 503; }
      else { await route.fulfill({ contentType: 'text/plain', body: `Contents of ${target}` }); return; }
    } else if (path === '/projects/demo/fs/file' && method === 'PUT') {
      if (state.slowUpload && target.endsWith('one.txt')) await delay(500);
      json = { path: target.startsWith('/') ? target : `${root}/${target}`, size: 20 };
    } else if (path === '/projects/demo/fs/mkdir') json = { path: '.cloudos/paste' };
    else if (path === '/projects/demo/git/status') json = { repo: true, cwd: root, head: 'main', upstream: 'origin/main', ahead: 0, behind: 0, entries: gitEntries };
    else if (path === '/projects/demo/git/log') json = { entries: [{ hash: 'abcdef', short: 'abcdef', subject: 'Fixture commit', author: 'Fixture', ts }] };
    else if (path === '/projects/demo/git/branches') json = { head: 'main', branches: [{ name: 'main', current: true, upstream: 'origin/main' }], remotes: ['origin'] };
    else if (path === '/projects/demo/git/worktrees') json = { cwd: root, worktrees: [{ name: 'main', path: root, branch: 'main', main: true, current: true, changes: 2, detached: false }, { name: 'review-very-long-worktree-name', path: `${root}/.worktrees/review-very-long-worktree-name`, branch: 'review-very-long-worktree-name', main: false, current: false, changes: 0, detached: false }] };
    else if (path === '/projects/demo/git/diff') {
      if (state.slowDiff && target === 'slow.txt') await delay(600);
      if (state.diffFail) { status = 503; json = { error: 'Fixture diff unavailable' }; }
      else json = { diff: `diff --git a/${target} b/${target}\n@@ -1 +1 @@\n-old ${target}\n+new ${target}`, truncated: false };
    } else if (/^\/projects\/demo\/sessions\/[^/]+$/.test(path)) json = { info: sessions.find(s => s.id === path.split('/').at(-1)), caps: { bypass_allowed: true }, pending: [] };
    else if (path.endsWith('/events')) json = [];
    else if (path.endsWith('/control')) json = { maxTokens: 1000000, totalTokens: 1000, percentage: .1 };
    else { status = 501; json = { error: `Missing fixture ${method} ${path}` }; state.unexpected.push(`${method} ${path}`); }
    await route.fulfill({ status, json });
  });
  const page = await context.newPage(); page.setDefaultTimeout(7000);
  page.on('pageerror', e => state.pageErrors.push(e.message));
  await page.goto(url, { waitUntil: 'networkidle' });
  await page.locator(tab === 'files' ? '.files' : tab === 'git' ? '.git .row' : '.term .xterm').first().waitFor({ state: 'visible' });
  return { context, page, state, panel: page.locator(tab === 'files' ? '.files' : tab === 'git' ? '.git' : '.pane').first() };
}
function clean(state) { assert.deepEqual(state.pageErrors, []); assert.deepEqual(state.unexpected, []); }
async function test(name, fn) {
  if (process.env.TEST_FILTER && !new RegExp(process.env.TEST_FILTER, 'i').test(name)) return;
  try { const detail = await fn(); results.push({ name, pass: true, detail }); console.log(`PASS ${name}`); }
  catch (e) { results.push({ name, pass: false, error: e.stack }); console.error(`FAIL ${name}: ${e.message}`); }
}
try {
  await test('File preview ignores a late response after another file is selected', async () => {
    const { page, panel, state, context } = await fixture(); state.slowFile = true;
    await panel.getByRole('button', { name: /slow.txt/ }).first().click();
    await panel.getByRole('button', { name: /fast.txt/ }).first().click();
    await page.waitForTimeout(800);
    assert.match(await panel.locator('.preview pre').innerText(), /fast.txt/);
    assert.doesNotMatch(await panel.locator('.preview pre').innerText(), /slow.txt/);
    clean(state); await context.close();
  });
  await test('Directory navigation ignores older responses', async () => {
    const { page, panel, state, context } = await fixture(); state.slowDir = true;
    await panel.locator('.crumbs').evaluate(el => { const all = el.querySelectorAll('button'); all[0].click(); all[1].click(); });
    await page.waitForTimeout(800);
    assert.equal(await panel.locator('.crumbs').innerText(), '/\n›\nhome');
    assert.match(await panel.locator('.list').innerText(), /directory--home.txt/);
    clean(state); await context.close();
  });
  await test('Multi-file upload keeps its original directory while navigation changes', async () => {
    const { page, panel, state, context } = await fixture(); state.slowUpload = true;
    await panel.locator('input[type=file]').setInputFiles([{ name: 'one.txt', mimeType: 'text/plain', buffer: Buffer.from('one') }, { name: 'two.txt', mimeType: 'text/plain', buffer: Buffer.from('two') }]);
    await panel.locator('.crumbs button').first().click();
    await page.waitForTimeout(900);
    const writes = state.requests.filter(r => r.method === 'PUT' && r.path.endsWith('/fs/file'));
    assert.deepEqual(writes.map(r => r.target), [`${root}/one.txt`, `${root}/two.txt`]);
    clean(state); await context.close();
  });
  await test('File preview failure has a local retry', async () => {
    const { panel, state, context } = await fixture(); state.fileFail = true;
    await panel.getByRole('button', { name: /fast.txt/ }).first().click();
    await panel.locator('.preview [role=alert]').waitFor(); state.fileFail = false;
    await panel.locator('.preview').getByRole('button', { name: 'Повторить' }).click();
    await panel.locator('.preview pre').waitFor(); assert.match(await panel.locator('.preview pre').innerText(), /fast.txt/);
    clean(state); await context.close();
  });
  await test('Narrow Code card opens Files preview as a full screen and restores keyboard focus', async () => {
    const { page, panel, state, context } = await fixture({ view: 'code', width: 1920 });
    const open = panel.getByRole('button', { name: /fast.txt/ }).first(); await open.click();
    await panel.locator('.preview pre').waitFor();
    await panel.locator('.back').waitFor({ state: 'visible' });
    assert.equal(await panel.locator('.list').isVisible(), false);
    await page.screenshot({ path: join(output, 'files-narrow-card.png') });
    await panel.locator('.back').click(); assert.equal(await open.evaluate(el => el === document.activeElement), true);
    clean(state); await context.close();
  });
  await test('Git diff ignores a late response and has retry', async () => {
    const { page, panel, state, context } = await fixture({ tab: 'git' }); state.slowDiff = true;
    await panel.getByRole('button', { name: /slow.txt/ }).first().click();
    await panel.getByRole('button', { name: /fast.txt/ }).first().click(); await page.waitForTimeout(800);
    assert.match(await panel.locator('.diff pre').innerText(), /fast.txt/); assert.doesNotMatch(await panel.locator('.diff pre').innerText(), /slow.txt/);
    await panel.locator('.diff .ico').click(); state.diffFail = true;
    await panel.getByRole('button', { name: /fast.txt/ }).first().click(); await panel.locator('.diff [role=alert]').waitFor(); state.diffFail = false;
    await panel.locator('.diff').getByRole('button', { name: 'Повторить' }).click(); await panel.locator('.diff pre').waitFor();
    clean(state); await context.close();
  });
  await test('Narrow Code Git diff uses Back and remains readable in light theme', async () => {
    const { page, panel, state, context } = await fixture({ tab: 'git', view: 'code', width: 1920 });
    await page.evaluate(() => document.documentElement.dataset.theme = 'light');
    await panel.getByRole('button', { name: /fast.txt/ }).first().click(); await panel.locator('.diff pre').waitFor();
    assert.equal(await panel.locator('.list').isVisible(), false); await panel.locator('.back').waitFor({ state: 'visible' });
    const colors = await panel.locator('.ln.add').evaluate(el => ({ color: getComputedStyle(el).color, bg: getComputedStyle(el).backgroundColor }));
    assert.equal(colors.color, 'rgb(22, 115, 62)'); assert.match(colors.bg, /31, 157, 90/);
    await page.screenshot({ path: join(output, 'git-light-narrow-card.png') }); await panel.locator('.back').click();
    clean(state); await context.close();
  });
  await test('Phone Files and Git controls have touch targets and no horizontal overflow', async () => {
    for (const tab of ['files', 'git']) {
      const { page, panel, state, context } = await fixture({ tab, width: 390, height: 844, touch: true });
      const controls = await panel.locator('button').evaluateAll(els => els.filter(el => el.offsetParent !== null).map(el => ({ label: el.getAttribute('aria-label') || el.innerText, h: el.getBoundingClientRect().height })));
      assert.deepEqual(controls.filter(c => c.h < 44), [], JSON.stringify(controls));
      if (tab === 'git') assert.equal(await panel.locator('.stage-box').first().evaluate(el => el.getBoundingClientRect().height), 44);
      const bounds = await panel.evaluate(el => ({ client: el.clientWidth, scroll: el.scrollWidth })); assert(bounds.scroll <= bounds.client + 1, JSON.stringify(bounds));
      await panel.getByRole('button', { name: /fast.txt/ }).first().click(); await panel.locator('.back').waitFor({ state: 'visible' });
      await panel.locator(tab === 'files' ? '.preview pre' : '.diff pre').waitFor();
      await page.screenshot({ path: join(output, `${tab}-phone.png`) }); clean(state); await context.close();
    }
  });
  await test('Terminal input is blocked offline, reports real status, and retains uploaded paths', async () => {
    const { page, panel, state, context } = await fixture({ tab: 'cli' });
    await panel.locator('.cstate').getByText('подключено', { exact: false }).waitFor();
    await page.evaluate(() => window.__workspaceFixture.disconnect());
    await panel.locator('.key').first().waitFor(); assert.equal(await panel.locator('.key').first().isDisabled(), true);
    assert.doesNotMatch(await panel.locator('.cstate').innerText(), /подключено/);
    const before = await page.evaluate(() => window.__workspaceFixture.sent.length);
    await panel.locator('.xterm-helper-textarea').press('a');
    assert.equal(await page.evaluate(() => window.__workspaceFixture.sent.length), before);
    await panel.locator('input[type=file]').setInputFiles({ name: 'note.txt', mimeType: 'text/plain', buffer: Buffer.from('attached') });
    await panel.locator('.pending code').waitFor(); assert.match(await panel.locator('.pending code').innerText(), /note.txt/);
    assert.equal(await panel.locator('.pending button').isDisabled(), true);
    await panel.getByRole('tab', { name: 'Файлы', exact: true }).click();
    await page.evaluate(() => window.__workspaceFixture.resume());
    await panel.getByRole('tab', { name: /CLI/ }).click();
    await panel.locator('.pending button:not(:disabled)').waitFor(); await panel.locator('.pending button').click();
    await panel.locator('.pending').waitFor({ state: 'hidden' });
    const sends = await page.evaluate(() => window.__workspaceFixture.sent.filter(s => Array.isArray(s.data)).map(s => String.fromCharCode(...s.data)));
    assert(sends.some(text => text.includes('note.txt'))); clean(state); await context.close();
  });
  await test('Phone terminal controls fit a 320px viewport and remain keyboard accessible', async () => {
    const { page, panel, state, context } = await fixture({ tab: 'cli', width: 320, height: 740, touch: true });
    await panel.locator('.cstate').getByText('подключено', { exact: false }).waitFor();
    const controls = await panel.locator('.tabs button').evaluateAll(els => els.filter(el => el.offsetParent !== null).map(el => ({ h: el.getBoundingClientRect().height, right: el.getBoundingClientRect().right, label: el.getAttribute('aria-label') || el.innerText })));
    assert.deepEqual(controls.filter(c => c.h < 32 || c.right > 320), [], JSON.stringify(controls));
    const before = await page.evaluate(() => window.__workspaceFixture.sent.filter(s => Array.isArray(s.data)).length);
    await panel.getByRole('button', { name: 'Up', exact: true }).focus(); await page.keyboard.press('Enter');
    assert.equal(await page.evaluate(() => window.__workspaceFixture.sent.filter(s => Array.isArray(s.data)).length), before + 1);
    await page.screenshot({ path: join(output, 'terminal-phone-320.png') }); clean(state); await context.close();
  });
  await test('Terminal colors update immediately when light theme is selected', async () => {
    const { page, panel, state, context } = await fixture({ tab: 'cli' });
    await page.evaluate(() => document.documentElement.dataset.theme = 'light'); await page.waitForTimeout(100);
    await page.evaluate(() => window.__workspaceFixture.output('coder@demo:~$ npm run check\r\n'));
    await page.waitForTimeout(100);
    const bg = await panel.locator('.xterm-scrollable-element').evaluate(el => getComputedStyle(el).backgroundColor); assert.equal(bg, 'rgb(255, 255, 255)');
    const fg = await panel.locator('.xterm-rows span').first().evaluate(el => getComputedStyle(el).color); assert.equal(fg, 'rgb(21, 26, 36)');
    await page.screenshot({ path: join(output, 'terminal-light.png') }); clean(state); await context.close();
  });
} finally {
  await writeFile(join(output, 'report.json'), JSON.stringify({ results, passed: results.filter(r => r.pass).length, total: results.length }, null, 2));
  await browser.close();
}
if (results.some(r => !r.pass)) process.exitCode = 1;
