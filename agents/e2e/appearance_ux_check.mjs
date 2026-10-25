#!/usr/bin/env node
/**
 * Browser regressions for global Agents appearance preferences.
 * All API and WebSocket traffic is mocked; no backend or model is used.
 * Start Vite separately, then run with UI_URL and PLAYWRIGHT_MODULE if needed.
 * Optional: APPEARANCE_UX_OUT, CHROME_PATH and TEST_FILTER.
 */
import assert from 'node:assert/strict';
import { access, mkdir, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const url = process.env.UI_URL ?? 'http://127.0.0.1:5179/agents/';
const origin = new URL(url).origin;
const output = resolve(process.env.APPEARANCE_UX_OUT ?? '/tmp/appearance-ux-check');
let chromium;
for (const candidate of [process.env.PLAYWRIGHT_MODULE, 'playwright'].filter(Boolean)) {
  try { ({ chromium } = await import(candidate.startsWith('/') ? pathToFileURL(candidate).href : candidate)); break; } catch {}
}
assert(chromium, 'Install Playwright or set PLAYWRIGHT_MODULE.');
let executablePath = process.env.CHROME_PATH;
if (!executablePath) for (const candidate of [chromium.executablePath(), join(homedir(), '.cache/ms-playwright/chromium-1243/chrome-linux64/chrome'), join(homedir(), '.cache/puppeteer/chrome/linux-148.0.7778.97/chrome-linux64/chrome')]) {
  try { await access(candidate); executablePath = candidate; break; } catch {}
}
await mkdir(output, { recursive: true });
const browser = await chromium.launch({ headless: true, executablePath });
const results = [];
const schemes = ['classic', 'graphite', 'nord', 'dracula', 'monokai', 'solarized', 'contrast', 'matrix', 'cyber'];
const ts = Math.floor(Date.now() / 1000);
const usage = { input: 100, output: 200, cache_read: 0, cache_create: 0, cost_usd: .1, turns: 1 };
const session = { id: 'appearance-session', agent: 'claude', cwd: '/home/coder/demo', title: 'Appearance verification', created_at: ts, updated_at: ts, model: 'fixture', permission_mode: 'default', effort: 'high', status: 'idle', alive: false, last_seq: 2, pending: 0, queued: [], usage, tui: 'pty-appearance' };
const caps = { provider: 'claude', models: [{ value: 'fixture', label: 'Fixture' }], mcp: 0, commands: [], efforts: ['high'], modes: ['default'], bypass_allowed: true, features: { commands: true, files: true, effort: true } };
const history = [{ seq: 1, ts, kind: 'user', content: [{ type: 'text', text: 'Keep preferences independent of the current conversation.' }] }, { seq: 2, ts, kind: 'assistant', content: [{ type: 'text', text: 'Choose a readable interface. **Keep important words emphasized.**\n\n```js\nconst appearance = "global";\n```' }] }];
const draft = 'Keep this draft while changing the appearance.';
const preferenceKeys = ['theme', 'scheme', 'accent', 'uiFont', 'codeFont', 'density', 'textTone', 'textDark', 'textLight', 'textWeight'];

async function navigate(page, name) {
  const buttons = page.locator('nav').getByRole('button', { name, exact: true });
  for (let i = 0; i < await buttons.count(); i++) if (await buttons.nth(i).isVisible()) return buttons.nth(i).click();
  throw Error(`Visible navigation missing: ${name}`);
}
async function openAppearance(page, lang = 'en') {
  const name = lang === 'ru' ? 'Настройки' : 'Settings';
  if (await page.locator('.tabbar').isVisible()) {
    await page.locator('.tabbar').getByRole('button', { name: lang === 'ru' ? 'Ещё' : 'More', exact: true }).click();
    await page.getByRole('dialog').getByRole('button', { name, exact: true }).click();
  } else await navigate(page, name);
  await page.locator('[data-appearance]').waitFor({ state: 'visible' });
}
async function fixture({ width = 1280, height = 900, theme = 'dark', lang = 'en', storage = {}, system = 'dark' } = {}) {
  const state = { calls: [], unexpected: [], pageErrors: [] };
  const context = await browser.newContext({ viewport: { width, height }, hasTouch: width < 600, isMobile: width < 600, serviceWorkers: 'block', colorScheme: system });
  await context.addInitScript(({ session, theme, lang, storage, draft }) => {
    // Seed once: reloads and a second tab must exercise actual persisted preferences.
    if (!localStorage.getItem('__appearanceFixtureSeeded')) {
      localStorage.setItem('__appearanceFixtureSeeded', '1');
      localStorage.setItem('agents.lang', lang); localStorage.setItem('agents.theme', theme);
      localStorage.setItem('agents.ui', JSON.stringify({ view: 'thread', sel: { project: 'demo', kind: 'session', id: session.id }, cards: [{ project: 'demo', kind: 'session', id: session.id, w: 1, h: 1 }], cols: 1, inspector: false, settingsTab: 'look' }));
      localStorage.setItem(`agents.draft.demo:${session.id}`, draft);
      for (const [key, value] of Object.entries(storage)) localStorage.setItem(`agents.${key}`, value);
    }
    const streams = [];
    class MockWebSocket {
      static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
      readyState = 0;
      constructor(url) {
        this.url = url; streams.push(this);
        setTimeout(() => {
          if (this.readyState === 3) return;
          this.readyState = 1; this.onopen?.({});
          if (url.includes(`/sessions/${session.id}/`)) this.onmessage?.({ data: JSON.stringify({ kind: 'hello', info: session, replay: 0 }) });
          if (url.includes('pty-appearance')) this.onmessage?.({ data: new TextEncoder().encode('Appearance terminal fixture\r\n').buffer });
        }, 25);
      }
      send() {} close() { this.readyState = 3; this.onclose?.({ code: 1006 }); }
    }
    window.WebSocket = MockWebSocket;
    window.__appearanceFixture = { terminalOutput: text => streams.filter(s => s.readyState === 1 && s.url.includes('pty-appearance')).forEach(s => s.onmessage?.({ data: new TextEncoder().encode(text).buffer })) };
  }, { session, theme, lang, storage, draft });
  await context.route('**/*', async route => {
    const request = route.request(); const parsed = new URL(request.url()); const method = request.method();
    if (parsed.hostname === 'fonts.googleapis.com') return route.fulfill({ contentType: 'text/css', body: '' });
    if (parsed.origin !== origin) { state.unexpected.push(`${method} ${parsed.href}`); return route.abort(); }
    if (!parsed.pathname.includes('/api/')) return route.continue();
    const path = parsed.pathname.replace(/^\/agents\/api(?:\/core)?/, '');
    state.calls.push({ method, path });
    let json;
    if (path === '/health') json = { ok: true, cloudd: true, version: 'fixture' };
    else if (path === '/workspaces') json = { projects: [{ name: 'demo', description: 'Appearance fixture', status: 'running', reachable: true, cost_today: .1, apps: [], sessions: [session], ptys: [] }], today: usage };
    else if (path === '/agents/layout') json = { cards: [{ project: 'demo', kind: 'session', id: session.id, w: 1, h: 1 }], cols: 1, groups: {} };
    else if (path === '/accounts') json = { accounts: [], defaults: {} };
    else if (path === '/settings/desktop') json = { appearance: { language: lang } };
    else if (path === '/settings') json = { prices: {}, defaults: {}, limits: [] };
    else if (path === '/voice') json = { provider: 'openai', language: '', ready: false, model: 'fixture', elevenlabs: { model: 'fixture', base: '', default_base: '', has_key: false, key_tail: '' }, openai: { model: 'fixture', base: '', default_base: '', has_key: false, key_tail: '' }, models: { elevenlabs: ['fixture'], openai: ['fixture'] } };
    else if (path === '/gate/models') json = { targets: [] };
    else if (path === '/scheduled') json = { scheduled: [], now: ts };
    else if (path.endsWith('/ptys')) json = [{ id: 'pty-appearance', session: session.id, tui: true, alive: true, cwd: session.cwd, title: 'Appearance terminal' }];
    else if (path.endsWith('/engines')) json = { engines: [] };
    else if (path.endsWith('/events')) json = history;
    else if (path === `/projects/demo/sessions/${session.id}`) json = { info: session, caps, pending: [] };
    else if (path.endsWith('/control')) json = { maxTokens: 1000000, totalTokens: 1000, percentage: .1 };
    else { state.unexpected.push(`${method} ${path}`); return route.fulfill({ status: 501, json: { error: `Missing fixture: ${method} ${path}` } }); }
    await route.fulfill({ json });
  });
  function watch(page) { page.setDefaultTimeout(7000); page.setDefaultNavigationTimeout(30000); page.on('pageerror', error => state.pageErrors.push(error.message)); }
  const page = await context.newPage(); watch(page);
  await page.goto(url, { waitUntil: 'domcontentloaded' });
  try { await page.locator('.composer textarea').waitFor({ state: 'visible', timeout: 30000 }); }
  catch (error) {
    console.error('Fixture diagnostics', { calls: state.calls, unexpected: state.unexpected, pageErrors: state.pageErrors });
    await page.screenshot({ path: join(output, 'fixture-failure.png') });
    await context.close(); throw error;
  }
  await openAppearance(page, lang);
  return { page, context, state, watch };
}
function clean(state) {
  assert.deepEqual(state.unexpected, [], 'All requests must stay inside explicit fixtures.');
  assert.deepEqual(state.pageErrors, [], 'Appearance changes must render without browser errors.');
  assert.equal(state.calls.filter(call => call.method !== 'GET' && call.path !== '/agents/layout').length, 0, 'Browser appearance must not mutate backend state.');
}
async function stored(page) { return page.evaluate(keys => Object.fromEntries(keys.map(key => [key, localStorage.getItem(`agents.${key}`)])), preferenceKeys); }
async function colors(page) {
  return page.evaluate(() => {
    const css = getComputedStyle(document.documentElement);
    const resolve = value => {
      const probe = document.createElement('span'); probe.style.color = value; document.body.append(probe);
      const result = getComputedStyle(probe).color; probe.remove(); return result;
    };
    const names = ['--bg-0', '--bg-1', '--bg-2', '--bg-3', '--bg-4', '--fg-1', '--fg-2', '--fg-3', '--accent', '--accent-fg'];
    return { theme: document.documentElement.dataset.theme, scheme: document.documentElement.dataset.scheme, tokens: Object.fromEntries(names.map(name => [name, resolve(css.getPropertyValue(name).trim())])), body: getComputedStyle(document.body).backgroundColor, bodyForeground: getComputedStyle(document.body).color, bodyWeight: getComputedStyle(document.body).fontWeight };
  });
}
function luminance(color) {
  const channels = color.match(/[\d.]+/g)?.slice(0, 3).map(Number);
  assert(channels?.length === 3, `Color must resolve to RGB: ${color}`);
  const linear = channels.map(value => { const normalized = value / 255; return normalized <= .04045 ? normalized / 12.92 : ((normalized + .055) / 1.055) ** 2.4; });
  return linear[0] * .2126 + linear[1] * .7152 + linear[2] * .0722;
}
function contrast(a, b) { const first = luminance(a), second = luminance(b); return (Math.max(first, second) + .05) / (Math.min(first, second) + .05); }
function readable(sample) {
  const ratios = [];
  for (const foreground of ['--fg-1', '--fg-2', '--fg-3']) for (const background of ['--bg-0', '--bg-1', '--bg-2', '--bg-3', '--bg-4']) {
    const ratio = contrast(sample.tokens[foreground], sample.tokens[background]); ratios.push(ratio);
    assert(ratio >= 4.5, `${foreground} on ${background} has ${ratio.toFixed(2)}:1 contrast; regular text requires 4.5:1.`);
  }
  return Math.min(...ratios);
}
async function textTone(page, value) { await page.locator(`button[data-text-tone="${value}"]`).click(); }
async function textHex(page, value) {
  const input = page.locator('input[data-text-hex]'); await input.fill(value); await input.press('Tab');
  await page.waitForFunction(value => localStorage.getItem(`agents.text${document.documentElement.dataset.theme === 'dark' ? 'Dark' : 'Light'}`) === value.toLowerCase(), value);
}
async function chatText(page) {
  await page.locator('.m.agent .md p').first().waitFor();
  return page.locator('.m.agent .md p').first().evaluate(element => {
    const parse = value => { const parts = value.match(/[\d.]+/g)?.map(Number) ?? [0, 0, 0, 0]; return [parts[0], parts[1], parts[2], parts[3] ?? 1]; };
    let background = parse(getComputedStyle(document.body).backgroundColor).slice(0, 3);
    const ancestors = []; for (let parent = element; parent && parent !== document.body; parent = parent.parentElement) ancestors.unshift(parent);
    for (const ancestor of ancestors) { const rgba = parse(getComputedStyle(ancestor).backgroundColor); background = background.map((channel, index) => channel * (1 - rgba[3]) + rgba[index] * rgba[3]); }
    const css = getComputedStyle(element), strong = element.querySelector('strong');
    return { foreground: css.color, weight: Number(css.fontWeight), strongWeight: strong ? Number(getComputedStyle(strong).fontWeight) : null, background: `rgb(${background.map(Math.round).join(', ')})` };
  });
}
async function mode(page, value) { await page.locator('[data-appearance]').getByRole('radio', { name: value === 'system' ? /^(as the system|system)$/i : new RegExp(`^${value}$`, 'i') }).check(); }
async function scheme(page, value) { await page.locator(`[data-scheme-card="${value}"]`).click(); await page.waitForFunction(value => document.documentElement.dataset.scheme === value, value); }
async function select(page, label, value) {
  const appearance = page.locator('[data-appearance]');
  if (label === 'Accent color') {
    const names = { scheme: 'From scheme', blue: 'Blue', violet: 'Violet', green: 'Green', orange: 'Orange' };
    await appearance.getByRole('button', { name: names[value], exact: true }).click();
  } else await appearance.getByRole('combobox', { name: label === 'Code font' ? 'Code and terminal font' : label, exact: true }).selectOption(value);
}
async function scenario(name, run) {
  if (process.env.TEST_FILTER && !new RegExp(process.env.TEST_FILTER, 'i').test(name)) return;
  try { const detail = await run(); results.push({ name, pass: true, detail }); console.log(`PASS ${name}`); }
  catch (error) { results.push({ name, pass: false, error: error.stack }); console.error(`FAIL ${name}: ${error.message}`); }
}

await scenario('Nine palettes apply globally in both modes with readable text', async () => {
  const { page, context, state } = await fixture(); const samples = [];
  try {
    for (const theme of ['dark', 'light']) {
      await mode(page, theme); const signatures = new Set();
      for (const id of schemes) {
        await scheme(page, id); const sample = await colors(page); const prefs = await stored(page);
        assert.equal(sample.theme, theme); assert.equal(prefs.theme, theme, 'Selecting a palette must not change the global mode.');
        assert.equal(prefs.accent, 'scheme', 'Selecting a palette applies its native accent.');
        const ratios = [];
        for (const foreground of ['--fg-1', '--fg-2', '--fg-3']) for (const background of ['--bg-1', '--bg-2', '--bg-3', '--bg-4']) {
          const ratio = contrast(sample.tokens[foreground], sample.tokens[background]); ratios.push({ foreground, background, ratio });
          assert(ratio >= 4.5, `${id}/${theme}: ${foreground} on ${background} has ${ratio.toFixed(2)}:1 contrast; regular text requires 4.5:1.`);
        }
        signatures.add(JSON.stringify([sample.tokens['--bg-0'], sample.tokens['--fg-1'], sample.tokens['--accent']]));
        if (theme === 'dark') assert(Math.max(...sample.tokens['--fg-1'].match(/[\d.]+/g).slice(0, 3).map(Number)) < 230, `${id}: the default text should remain visibly below white brightness.`);
        samples.push({ id, theme, ...sample, minimumContrast: Math.min(...ratios.map(item => item.ratio)) });
      }
      assert.equal(signatures.size, schemes.length, 'Every scheme should have a distinguishable global color combination.');
    }
    for (const id of schemes) {
      const dark = samples.find(item => item.id === id && item.theme === 'dark'); const light = samples.find(item => item.id === id && item.theme === 'light');
      assert(luminance(dark.tokens['--bg-1']) < luminance(light.tokens['--bg-1']), `${id} must respect the selected light/dark mode.`);
    }
    await page.screenshot({ path: join(output, 'appearance-palettes-light.png') }); clean(state); return samples;
  } finally { await context.close(); }
});

await scenario('Accent override is independent and selecting a palette restores its accent', async () => {
  const { page, context, state } = await fixture({ theme: 'light' });
  try {
    await select(page, 'Interface font', 'humanist'); await scheme(page, 'nord'); const before = await colors(page);
    await select(page, 'Accent color', 'orange'); const custom = await colors(page);
    assert.notEqual(custom.tokens['--accent'], before.tokens['--accent']);
    assert.equal(custom.tokens['--bg-1'], before.tokens['--bg-1']);
    await scheme(page, 'dracula'); const saved = await stored(page);
    assert.equal(saved.accent, 'scheme'); assert.equal(saved.theme, 'light'); assert.equal(saved.uiFont, 'humanist'); clean(state);
  } finally { await context.close(); }
});

await scenario('Preferences survive reload without changing language or the conversation draft', async () => {
  const { page, context, state } = await fixture();
  try {
    await mode(page, 'light'); await scheme(page, 'cyber'); await select(page, 'Interface font', 'humanist'); await select(page, 'Code font', 'consolas'); await select(page, 'Accent color', 'violet');
    await page.locator('[data-appearance]').getByRole('radio', { name: 'compact', exact: true }).check();
    const before = await stored(page); const beforeColors = await colors(page);
    await page.reload({ waitUntil: 'networkidle' }); await openAppearance(page);
    assert.deepEqual(await stored(page), before); assert.deepEqual(await colors(page), beforeColors);
    assert.equal(await page.evaluate(() => document.documentElement.lang), 'en');
    await navigate(page, 'Workspaces'); await page.locator('.composer textarea').waitFor();
    assert.equal(await page.getByRole('textbox', { name: 'Message', exact: true }).inputValue(), draft, 'Applying appearance must preserve the actual Code draft.'); clean(state);
  } finally { await context.close(); }
});

await scenario('System mode updates global colors and palette previews live', async () => {
  const { page, context, state } = await fixture({ theme: 'system', system: 'dark' });
  try {
    await scheme(page, 'matrix'); const dark = await colors(page);
    const darkPreview = await page.locator('[data-scheme-card="nord"]').evaluate(el => getComputedStyle(el.querySelector('[style]') ?? el).backgroundColor);
    await page.emulateMedia({ colorScheme: 'light' }); await page.waitForFunction(() => document.documentElement.dataset.theme === 'light');
    const light = await colors(page);
    const lightPreview = await page.locator('[data-scheme-card="nord"]').evaluate(el => getComputedStyle(el.querySelector('[style]') ?? el).backgroundColor);
    assert.notEqual(light.tokens['--bg-1'], dark.tokens['--bg-1']); assert.notEqual(lightPreview, darkPreview, 'Palette previews must follow effective system mode.');
    assert.equal((await stored(page)).theme, 'system'); assert.equal((await stored(page)).scheme, 'matrix');
    await page.emulateMedia({ colorScheme: 'dark' }); await page.waitForFunction(() => document.documentElement.dataset.theme === 'dark');
    assert.deepEqual(await colors(page), dark); clean(state);
  } finally { await context.close(); }
});

await scenario('Interface and code fonts apply to real content and mounted terminal', async () => {
  const { page, context, state, watch } = await fixture();
  try {
    const initialBody = await page.evaluate(() => getComputedStyle(document.body).fontFamily);
    await select(page, 'Interface font', 'humanist'); const changedBody = await page.evaluate(() => getComputedStyle(document.body).fontFamily);
    assert.notEqual(changedBody, initialBody); await select(page, 'Code font', 'consolas');
    await navigate(page, 'Workspaces'); await page.locator('.md pre code').waitFor();
    const codeFont = await page.locator('.md pre code').first().evaluate(el => getComputedStyle(el).fontFamily);
    assert.match(codeFont, /Consolas/i); assert.equal(await page.evaluate(() => getComputedStyle(document.body).fontFamily), changedBody);
    await page.getByRole('tab', { name: 'CLI', exact: true }).click(); await page.locator('.term .xterm-rows').waitFor({ state: 'attached' });
    const terminalFont = await page.locator('.term .xterm-rows').evaluate(el => getComputedStyle(el).fontFamily);
    assert.equal(terminalFont, codeFont, 'The actual terminal renderer must use the selected code font.');
    const other = await context.newPage(); watch(other); await other.goto(url, { waitUntil: 'networkidle' }); await openAppearance(other);
    await select(other, 'Code font', 'system');
    await page.waitForFunction(previous => { const rows = document.querySelector('.term .xterm-rows'); return rows && getComputedStyle(rows).fontFamily !== previous; }, terminalFont);
    const changedTerminal = await page.locator('.term .xterm-rows').evaluate(el => getComputedStyle(el).fontFamily);
    const selectedStack = await other.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--mono').trim());
    assert(!changedTerminal.includes('Consolas')); assert(selectedStack.includes('ui-monospace')); clean(state);
  } finally { await context.close(); }
});

await scenario('Appearance synchronizes across tabs and reset preserves language and draft', async () => {
  const { page, context, state, watch } = await fixture({ lang: 'ru' });
  try {
    const other = await context.newPage(); watch(other); await other.goto(url, { waitUntil: 'networkidle' }); await openAppearance(other, 'ru');
    await scheme(page, 'dracula'); await textTone(page, 'custom'); await textHex(page, '#b3c5d8');
    await page.locator('select[data-text-weight]').selectOption('medium');
    await page.locator('[data-appearance]').getByRole('radio', { name: 'светлая', exact: true }).check(); await textHex(page, '#2c4054');
    await other.waitForFunction(() => document.documentElement.dataset.scheme === 'dracula' && document.documentElement.dataset.theme === 'light');
    assert.deepEqual(await colors(page), await colors(other));
    assert.equal(await other.locator('[data-scheme-card="dracula"]').getAttribute('aria-pressed'), 'true');
    assert(await other.locator('[data-appearance]').getByRole('radio', { name: 'светлая', exact: true }).isChecked(), 'The other tab must update the visible mode control too.');
    assert.equal(await other.locator('button[data-text-tone="custom"]').getAttribute('aria-pressed'), 'true');
    assert.equal(await other.locator('input[data-text-hex]').inputValue(), '#2c4054');
    assert.equal(await other.locator('select[data-text-weight]').inputValue(), 'medium');
    await page.locator('[data-appearance]').getByRole('button', { name: /сброс.*оформ|восстанов.*оформ/i }).click();
    await other.waitForFunction(() => document.documentElement.dataset.scheme === 'classic' && document.documentElement.dataset.theme === 'dark');
    const prefs = await stored(page); assert.deepEqual(prefs, { theme: 'dark', scheme: 'classic', accent: 'scheme', uiFont: 'inter', codeFont: 'jetbrains', density: 'normal', textTone: 'soft', textDark: '#c6ceda', textLight: '#243044', textWeight: 'normal' });
    assert.equal(await page.evaluate(() => localStorage.getItem('agents.lang')), 'ru');
    assert.equal(await other.evaluate(() => document.documentElement.lang), 'ru');
    assert.equal(await page.evaluate(() => localStorage.getItem('agents.draft.demo:appearance-session')), draft); clean(state);
  } finally { await context.close(); }
});

await scenario('Unknown stored appearance values safely recover to usable defaults', async () => {
  const storage = Object.fromEntries(preferenceKeys.map(key => [key, 'unknown value <script>']));
  const { page, context, state } = await fixture({ storage });
  try {
    const applied = await colors(page); assert.equal(applied.theme, 'dark'); assert.equal(applied.scheme, 'classic');
    assert(contrast(applied.tokens['--fg-1'], applied.tokens['--bg-1']) >= 4.5);
    assert.equal(await page.locator('select[data-ui-font]').inputValue(), 'inter');
    assert.equal(await page.locator('select[data-code-font]').inputValue(), 'jetbrains');
    assert.equal(await page.locator('[data-appearance]').getByRole('button', { name: 'From scheme', exact: true }).getAttribute('aria-pressed'), 'true');
    assert(await page.locator('[data-appearance]').getByRole('radio', { name: 'normal', exact: true }).isChecked());
    assert.equal(await page.locator('button[data-text-tone="soft"]').getAttribute('aria-pressed'), 'true');
    assert.equal(await page.locator('select[data-text-weight]').inputValue(), 'normal');
    await scheme(page, 'graphite'); await mode(page, 'light'); const valid = await colors(page);
    assert.equal(valid.theme, 'light'); assert.equal(valid.scheme, 'graphite');
    await page.reload({ waitUntil: 'networkidle' }); await openAppearance(page); assert.deepEqual(await colors(page), valid); clean(state);
  } finally { await context.close(); }
});

await scenario('Palette selection and theme mode work from the keyboard', async () => {
  const { page, context, state } = await fixture();
  try {
    const card = page.locator('[data-scheme-card="nord"]'); await card.focus(); await page.keyboard.press('Enter');
    await page.waitForFunction(() => document.documentElement.dataset.scheme === 'nord');
    assert.equal(await card.getAttribute('aria-pressed'), 'true', 'A keyboard user must receive selected-state semantics.');
    const light = page.locator('[data-appearance]').getByRole('radio', { name: /^light$/i }); await light.focus(); await page.keyboard.press('Space');
    await page.waitForFunction(() => document.documentElement.dataset.theme === 'light'); assert(await light.isChecked());
    const cool = page.locator('button[data-text-tone="cool"]'); await cool.focus(); await page.keyboard.press('Enter');
    assert.equal(await cool.getAttribute('aria-pressed'), 'true'); clean(state);
  } finally { await context.close(); }
});

await scenario('Text color presets change the actual conversation while preserving global mode', async () => {
  const { page, context, state } = await fixture(); const samples = [];
  try {
    for (const theme of ['dark', 'light']) {
      await mode(page, theme); const signatures = new Set();
      for (const tone of ['soft', 'warm', 'cool', 'scheme']) {
        await textTone(page, tone); const applied = await colors(page); readable(applied);
        assert.equal(applied.theme, theme); assert.equal((await stored(page)).textTone, tone);
        assert.equal(applied.bodyForeground, applied.tokens['--fg-1'], 'The body must follow the selected text color.');
        await navigate(page, 'Workspaces'); const actual = await chatText(page);
        assert.equal(actual.foreground, applied.tokens['--fg-1'], 'Rendered conversation paragraphs must follow the text preference.');
        assert(contrast(actual.foreground, actual.background) >= 4.5, 'Actual message text must contrast with the composed message background.');
        signatures.add(actual.foreground); samples.push({ tone, theme, actual });
        await openAppearance(page);
      }
      assert(signatures.size >= 3, 'The text presets should produce visibly different conversation colors.');
    }
    clean(state); return samples;
  } finally { await context.close(); }
});

await scenario('Unreadable custom colors are corrected on every surface without losing the request', async () => {
  const { page, context, state } = await fixture(); const samples = [];
  try {
    await textTone(page, 'custom');
    for (const [theme, requested] of [['dark', '#0c1014'], ['light', '#fefefe']]) {
      await mode(page, theme); await textHex(page, requested); const applied = await colors(page);
      const raw = `rgb(${[1, 3, 5].map(index => parseInt(requested.slice(index, index + 2), 16)).join(', ')})`;
      assert(contrast(raw, applied.tokens['--bg-1']) < 4.5, 'The sample must actually need a readability correction.');
      assert.notEqual(applied.tokens['--fg-1'], raw, 'An unreadable requested color must receive a readable display color.');
      const minimumContrast = readable(applied);
      assert.equal((await stored(page))[theme === 'dark' ? 'textDark' : 'textLight'], requested);
      assert.equal(await page.locator('input[data-text-color]').inputValue(), requested, 'The picker retains the user request.');
      await page.locator('[data-text-adjusted]').waitFor({ state: 'visible' });
      assert(await page.locator('[data-text-contrast]').isVisible(), 'The adjusted contrast is explained beside the picker.');
      await navigate(page, 'Workspaces'); const actual = await chatText(page);
      assert.equal(actual.foreground, applied.tokens['--fg-1']); assert(contrast(actual.foreground, actual.background) >= 4.5);
      samples.push({ theme, requested, displayed: actual.foreground, minimumContrast }); await openAppearance(page);
    }
    await page.screenshot({ path: join(output, 'appearance-custom-adjusted-light.png') }); clean(state); return samples;
  } finally { await context.close(); }
});

await scenario('Explicit white remains available through the native color picker', async () => {
  const { page, context, state } = await fixture();
  try {
    await textTone(page, 'custom');
    await page.locator('input[data-text-color]').evaluate(element => { element.value = '#ffffff'; element.dispatchEvent(new Event('input', { bubbles: true })); });
    await page.waitForFunction(() => localStorage.getItem('agents.textDark') === '#ffffff');
    const applied = await colors(page); readable(applied);
    assert.equal(applied.tokens['--fg-1'], 'rgb(255, 255, 255)', 'The user can deliberately choose white text in dark mode.');
    assert.equal(await page.locator('input[data-text-hex]').inputValue(), '#ffffff');
    assert.equal(await page.locator('[data-text-adjusted]').count(), 0, 'Readable white must not be described as corrected.');
    await navigate(page, 'Workspaces'); const actual = await chatText(page); assert.equal(actual.foreground, 'rgb(255, 255, 255)'); clean(state);
  } finally { await context.close(); }
});

await scenario('Dark and light custom text colors survive mode changes and reload', async () => {
  const { page, context, state } = await fixture();
  try {
    await textTone(page, 'custom'); await textHex(page, '#b5c7dc'); const dark = await colors(page);
    await mode(page, 'light'); await textHex(page, '#2e4155'); const light = await colors(page);
    await page.locator('select[data-text-weight]').selectOption('medium');
    await mode(page, 'dark'); assert.equal((await colors(page)).tokens['--fg-1'], dark.tokens['--fg-1']);
    assert.equal(await page.locator('input[data-text-hex]').inputValue(), '#b5c7dc');
    await mode(page, 'light'); assert.equal((await colors(page)).tokens['--fg-1'], light.tokens['--fg-1']);
    assert.equal(await page.locator('input[data-text-hex]').inputValue(), '#2e4155');
    const before = await stored(page), beforeColors = await colors(page);
    await page.reload({ waitUntil: 'networkidle' }); await openAppearance(page);
    assert.deepEqual(await stored(page), before); assert.deepEqual(await colors(page), beforeColors);
    assert.equal(await page.locator('select[data-text-weight]').inputValue(), 'medium');
    assert.equal(await page.evaluate(() => document.documentElement.lang), 'en');
    await navigate(page, 'Workspaces'); assert.equal(await page.getByRole('textbox', { name: 'Message', exact: true }).inputValue(), draft); clean(state);
  } finally { await context.close(); }
});

await scenario('Invalid HEX editing preserves the last valid color and can recover', async () => {
  const { page, context, state } = await fixture();
  try {
    await textTone(page, 'custom'); await textHex(page, '#b3c5d8'); const before = await colors(page);
    const input = page.locator('input[data-text-hex]'); await input.fill('not-a-color'); await input.press('Tab');
    assert.equal(await input.getAttribute('aria-invalid'), 'true');
    assert(await page.locator('[data-appearance] [role="alert"]').isVisible());
    assert.equal((await stored(page)).textDark, '#b3c5d8'); assert.deepEqual(await colors(page), before);
    await textHex(page, '#c8bfae'); assert.notEqual(await input.getAttribute('aria-invalid'), 'true');
    assert.equal(await page.locator('[data-appearance] [role="alert"]').count(), 0); readable(await colors(page)); clean(state);
  } finally { await context.close(); }
});

await scenario('Text weight changes real paragraphs and keeps emphasized words stronger', async () => {
  const { page, context, state } = await fixture();
  try {
    await page.locator('select[data-text-weight]').selectOption('normal'); await navigate(page, 'Workspaces');
    const normal = await chatText(page); assert.equal(normal.weight, 400); assert(normal.strongWeight > normal.weight);
    await openAppearance(page); await page.locator('select[data-text-weight]').selectOption('medium'); await navigate(page, 'Workspaces');
    const medium = await chatText(page); assert.equal(medium.weight, 500); assert(medium.strongWeight > medium.weight);
    assert(medium.strongWeight >= normal.strongWeight, 'Emphasized text should keep its original strength when regular text becomes medium.');
    assert.equal(await page.evaluate(() => Number(getComputedStyle(document.body).fontWeight)), medium.weight);
    await page.screenshot({ path: join(output, 'appearance-chat-medium-text.png') }); clean(state); return { normal, medium };
  } finally { await context.close(); }
});

for (const width of [320, 390]) await scenario(`Appearance controls fit phone ${width}px with accessible names`, async () => {
  const { page, context, state } = await fixture({ width, height: 844 });
  try {
    await scheme(page, 'nord');
    await textTone(page, 'custom'); await textHex(page, '#b3c6db');
    const selectedAccent = (await colors(page)).tokens['--accent'];
    await page.waitForFunction(accent => { const selected = document.querySelector('button[data-text-tone="custom"]'); return selected && getComputedStyle(selected).borderColor === accent; }, selectedAccent);
    const audit = await page.locator('[data-appearance]').evaluate(root => {
      const name = el => el.getAttribute('aria-label') || el.getAttribute('title') || (el.getAttribute('aria-labelledby') || '').split(/\s+/).map(id => document.getElementById(id)?.textContent || '').join(' ').trim() || [...(el.labels || [])].map(label => label.textContent).join(' ').trim() || el.textContent.trim();
      return { width: innerWidth, scrollWidth: document.documentElement.scrollWidth,
        textSelections: [...root.querySelectorAll('button[data-text-tone]')].map(element => ({ tone: element.dataset.textTone, selected: element.getAttribute('aria-pressed'), border: getComputedStyle(element).borderColor, background: getComputedStyle(element).backgroundColor })),
        controls: [...root.querySelectorAll('button,input,select')].filter(el => el.getBoundingClientRect().width && el.getBoundingClientRect().height).map(el => { const b = el.getBoundingClientRect(); return { name: name(el), tag: el.tagName, type: el.type, left: b.left, right: b.right, width: b.width, height: b.height }; }) };
    });
    assert(audit.scrollWidth <= width + 1, 'Appearance must not force horizontal page scrolling.');
    for (const control of audit.controls) {
      assert(control.name, `${control.tag} must have an accessible name.`);
      assert(control.left >= -1 && control.right <= width + 1, `${control.name} must fit the phone width.`);
      if (control.tag !== 'INPUT' || !['radio', 'checkbox'].includes(control.type)) assert(control.height >= 43.5, `${control.name} needs a usable touch target.`);
    }
    assert.equal(await page.locator('[data-scheme-card]').count(), schemes.length);
    assert.equal(audit.textSelections.filter(item => item.selected === 'true').map(item => item.tone).join(','), 'custom', 'The phone picker must identify exactly the selected text preset.');
    assert(await page.locator('input[data-text-color]').isVisible(), 'A phone user can reach the named native color picker.');
    await page.locator('input[data-text-color]').focus(); await page.keyboard.press('Tab');
    assert.equal(await page.locator('input[data-text-hex]').evaluate(element => document.activeElement === element), true, 'The HEX field is reachable from the color picker by keyboard.');
    await page.screenshot({ path: join(output, `appearance-phone-${width}.png`), fullPage: true });
    await select(page, 'Interface font', 'humanist'); await select(page, 'Code font', 'consolas');
    await page.locator('[data-appearance] .appearance-footer').scrollIntoViewIfNeeded();
    const footer = await page.locator('[data-appearance] .appearance-footer').boundingBox();
    assert(footer && footer.y >= 0 && footer.y + footer.height <= 844, 'Font controls and reset must remain reachable in the phone scroll area.');
    await page.screenshot({ path: join(output, `appearance-phone-${width}-fonts.png`) }); clean(state); return audit;
  } finally { await context.close(); }
});

await browser.close();
await writeFile(join(output, 'report.json'), JSON.stringify({ results }, null, 2));
const failed = results.filter(result => !result.pass);
console.log(`${results.length - failed.length}/${results.length} appearance checks passed. Artifacts: ${output}`);
if (failed.length) process.exitCode = 1;
