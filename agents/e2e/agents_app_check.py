import os, re, time, json, ssl, urllib.request, random, string, uuid, base64, struct, zlib, threading, http.server
from playwright.sync_api import sync_playwright
# The "Agents" app as a separate service (cloudos-agents on :7300, the /agents/ route on the edge): the service API (health,
# workplaces, inbox, models, settings, the proxy to the core), then in the browser — the frame, the workspace tree, creating a
# Claude session from the "+" menu, Thread with a chat and a model answer, inbox, models, settings, the light theme. One model
# turn through the gate.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
GATE_APP = os.environ.get("GATE_APP", "llm-external")  # the llm provider app with a Management API (the gate card in Settings → Accounts)
# what lies in the home of the PROJECT workspace: a folder with a text file for the "Files" check, a word from its file and conversation names, a fragment of an existing conversation title
HOME_DIR = os.environ.get("HOME_DIR", "docs"); HOME_FILE = os.environ.get("HOME_FILE", "llms.txt"); KNOWN_WORD = os.environ.get("KNOWN_WORD", "docs"); KNOWN_SESSION = os.environ.get("KNOWN_SESSION", "docs")
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note, f"[{time.strftime('%H:%M:%S')}]", flush=True)
def composer_option(page, label):
    page.locator(".thread .chat .composer .composer-more").click()
    page.wait_for_selector(".menu .mi", timeout=5000)
    page.locator(".menu .mi").filter(has_text=label).first.click()
def composer_modes(page):
    page.locator(".thread .chat .composer-mode-summary:visible, .thread .chat .ctl.mode:visible").first.click()
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def raw(path, method="GET", body=None, ctype="application/json"):
    try:
        r = urllib.request.urlopen(urllib.request.Request(f"https://{HOST}{path}", data=body, method=method, headers={"Content-Type": ctype} if body is not None else {}), context=ctx_ssl, timeout=120); return r.status, r.read()
    except urllib.error.HTTPError as e: return e.code, e.read()
def tree_expand(page, project):
    """The tree shows 8 rows per workspace (TREE_LIMIT): expand "N more" so the owner's old conversations are clickable"""
    m = page.locator(f".panel .ws:has(.wname:text-is('{project}')) .sess-more")
    try:
        if m.count() and "ещё" in m.inner_text(): m.click(); time.sleep(0.4)
    except Exception: pass
def raw_url(url, timeout=30):
    try:
        r = urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "e2e"}), context=ctx_ssl, timeout=timeout); return r.status, r.read()
    except urllib.error.HTTPError as e: return e.code, e.read()
    except Exception as e: return 0, str(e).encode()
def get(path):
    try:
        r = urllib.request.urlopen(urllib.request.Request(f"https://{HOST}{path}"), context=ctx_ssl, timeout=60); t = r.read(); return r.status, (json.loads(t) if t and r.headers.get_content_type() == "application/json" else t[:200])
    except urllib.error.HTTPError as e: return e.code, None
MARK = "app-" + "".join(random.choice(string.ascii_lowercase) for _ in range(6))
st, h = get("/agents/api/health"); ok("service: health via edge route", st == 200 and h and h["ok"] and h["cloudd"], json.dumps(h)[:120] if h else str(st))
st, w = get("/agents/api/workspaces"); ok("service: workspaces composed from the core", st == 200 and w and any(p["name"] == P and p["status"] == "running" for p in w["projects"]) and "today" in w)
st, i = get("/agents/api/inbox"); ok("service: inbox", st == 200 and i and all(k in i for k in ("waiting", "failed", "done")))
st, m = get("/agents/api/models?days=7"); ok("service: models with routes and prices", st == 200 and m and m["routes"] and m["models"] and "claude-fable-5-1" in [x["id"] for x in m["models"]])
st, s = get("/agents/api/settings"); ok("service: settings with default prices", st == 200 and s and "claude-fable-5-1" in s["prices"])
st, pr = get(f"/agents/api/core/projects"); ok("service: core proxy", st == 200 and isinstance(pr, list) and any(p["name"] == P for p in pr))
st, body = get("/agents/"); ok("service: UI served under /agents/", st == 200 and (body or b"").lower().startswith(b"<!doctype html>"))
before = {x["id"] for x in (get(f"/agents/api/core/projects/{P}/sessions")[1] or [])}
sid = None
with sync_playwright() as p:
    b = p.chromium.launch(args=["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"]); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1500, "height": 900}); ctx.grant_permissions(["microphone"], origin=f"https://{HOST}"); ctx.add_init_script("try { if (!localStorage.getItem('agents.lang')) localStorage.setItem('agents.lang', 'ru'); } catch (e) {}"); pg = ctx.new_page()
    errors = []; pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"https://{HOST}/agents/", wait_until="load"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(2)
    ok("ui: shell with six sections", pg.locator(".rail .nav").count() >= 6)
    pg.wait_for_selector(f".panel .ws .wname:has-text('{P}')", timeout=30000); ok("ui: workspaces tree from the core", True)
    pg.screenshot(path="shots/ap1-work.png")
    # a new Claude session from the workspace's
    # "+" menu
    ws = pg.locator(f".panel .ws:has(.wname:has-text('{P}'))"); ws.locator(".wrow").hover(); ws.locator(".wrow button").click(); pg.wait_for_selector(".menu", timeout=5000)
    pg.locator(".menu .mi:has-text('Чат с Claude')").click(); pg.wait_for_selector(".card .chat, .thread .chat", timeout=30000); time.sleep(2)
    new = [x for x in (get(f"/agents/api/core/projects/{P}/sessions")[1] or []) if x["id"] not in before]; sid = new[0]["id"] if new else None
    if sid:
        try: pg.wait_for_selector(f".panel .sess[data-id='{sid}']", timeout=8000)
        except Exception: pass
    ok("ui: session created via the service", bool(sid) and pg.locator(f".panel .sess[data-id='{sid}']").count() == 1)
    pg.locator(".top .seg button:has-text('Thread')").click(); pg.wait_for_selector(".thread .chat", timeout=20000); time.sleep(1)
    ok("ui: thread with named session tabs", [x.get_attribute("aria-label") for x in pg.locator(".thread .pane .tabs .tab").all()] == ["Чат", "CLI", "Терминал", "Файлы", "Git"])
    # context window: a bare id of a native 1M model (fable) through the gate is cut by Claude Code to 200k — agentd starts the CLI with the [1m]
    # tag (window 1 000 000)
    cinfo = (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}); cmods = [m.get("value") for m in ((get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("caps", {}) or {}).get("models", [])]
    ok("ctx: the catalog has a single Fable entry with the [1m] mark (1M window) and the session model, once known, carries it too", cmods.count("claude-fable-5-1[1m]") == 1 and "claude-fable-5-1" not in cmods and (cinfo.get("model") is None or str(cinfo.get("model")).endswith("[1m]")), f"model={cinfo.get('model')} fable entries={[m for m in cmods if m and 'fable' in m]}")
    # focus from the cursor/keyboard on desktop: a click into the field places the cursor,
    # typing lands in the field
    pg.locator(".thread .chat textarea").click(); pg.keyboard.type("x"); ok("ui: textarea takes focus and keystrokes on desktop", pg.evaluate("document.activeElement?.tagName") == "TEXTAREA" and pg.locator(".thread .chat textarea").input_value() == "x")
    # pasting an image from the clipboard (Ctrl+V): a paste event with a PNG in clipboardData → a preview in the attachments; a non-image
    # — a hint, no preview
    PNG1 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
    n_paste = pg.evaluate("""async ([b64]) => { const ta = document.querySelector('.thread .chat textarea'); const bin = atob(b64); const arr = new Uint8Array(bin.length); for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
      const dt = new DataTransfer(); dt.items.add(new File([arr], 'clip.png', { type: 'image/png' })); ta.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }));
      await new Promise(r => setTimeout(r, 400)); return document.querySelectorAll('.thread .chat .thumbs .thumb').length; }""", [PNG1])
    n_text = pg.evaluate("""async () => { const ta = document.querySelector('.thread .chat textarea'); const dt = new DataTransfer(); dt.items.add(new File(['x'], 'note.txt', { type: 'text/plain' })); ta.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }));
      await new Promise(r => setTimeout(r, 400)); return document.querySelectorAll('.thread .chat .thumbs .thumb').length; }""")
    ok("ui: Ctrl+V pastes an image into the composer as an attachment thumbnail; a non-image is ignored", n_paste == 1 and n_text == 1, f"after png={n_paste} after txt={n_text}")
    if pg.locator(".thread .chat .thumbs .thumb .x").count(): pg.locator(".thread .chat .thumbs .thumb .x").first.click(); time.sleep(0.3)
    # UI freshness (cache reset, docs/agent-runtime.md slice 46): "About" shows the build and the reset button; replacing index.html
    # with a different main script hash → the "UI updated" banner → "Reload" goes to an address with ?r= bypassing the cache
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".subnav button", timeout=20000); pg.locator(".subnav button:has-text('О системе')").click(); time.sleep(0.6)
    ui_build = pg.evaluate("(Array.from(document.scripts).map(s => (s.src.match(/assets\\/index-([\\w-]+)\\.js/) || [])[1]).find(Boolean) || '')")
    ok("about: the Application block shows the interface build and the reset-cache button", bool(ui_build) and ui_build in pg.locator(".settings").inner_text() and pg.locator(".settings button.reset-cache").count() == 1, ui_build)
    pg.route("**/agents/index.html?fresh=*", lambda r: r.fulfill(status=200, content_type="text/html", body='<html><script type="module" src="/agents/assets/index-STALE0000.js"></script></html>'))
    pg.locator(".settings button:has-text('проверить обновление')").click()
    try: pg.wait_for_selector(".upd-banner", timeout=10000)
    except Exception: pass
    ok("about: a newer build on the server raises the «interface updated» banner", pg.locator(".upd-banner").count() == 1 and "STALE0000" in pg.locator(".settings").inner_text())
    pg.unroute("**/agents/index.html?fresh=*")
    pg.locator(".upd-banner button.primary").click(); time.sleep(2.5)
    pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(2)
    ok("about: «reload» reloads the app (the banner is gone, the real index.html is current)", pg.locator(".app").count() == 1 and pg.locator(".upd-banner").count() == 0, pg.url[-40:])
    # PWA (slice 47): the "Agents" own manifest with scope /agents/ and 192/512/maskable icons — Chrome Android offers "Install app"
    st_m, man_raw = raw("/agents/manifest.webmanifest"); man = json.loads(man_raw or b"{}") if st_m == 200 else {}
    ok("pwa: the manifest is served as application/manifest+json with id/scope/start_url under /agents/, standalone display and 192/512/maskable icons", st_m == 200 and man.get("scope") == "/agents/" and man.get("start_url") == "/agents/" and man.get("id") == "/agents/" and man.get("display") == "standalone" and {i.get("sizes") for i in man.get("icons", [])} >= {"192x192", "512x512"} and any(i.get("purpose") == "maskable" for i in man.get("icons", [])), str(man)[:160])
    icons_ok = all(raw(i["src"])[0] == 200 for i in man.get("icons", [])) and raw("/agents/apple-touch-icon.png")[0] == 200 and raw("/agents/favicon.svg")[0] == 200
    ok("pwa: every manifest icon, the apple-touch-icon and the favicon answer 200", icons_ok)
    ok("pwa: index.html links the manifest and the apple-touch-icon", pg.locator("link[rel='manifest'][href='/agents/manifest.webmanifest']").count() == 1 and pg.locator("link[rel='apple-touch-icon']").count() == 1)
    # the install button in "About": in headless Chromium beforeinstallprompt never arrives — a per-platform hint is shown; a
    # synthetic event with prompt() → the button appears and calls prompt
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".subnav button", timeout=20000); pg.locator(".subnav button:has-text('О системе')").click(); time.sleep(0.6)
    ok("pwa: without an install prompt the Application block shows manual install hints for Android, iOS and desktop", pg.locator(".settings .install .install-hint").count() == 1 and "Android" in pg.locator(".settings .install").inner_text() and "iOS" in pg.locator(".settings .install").inner_text())
    pg.evaluate("(() => { const e = new Event('beforeinstallprompt', { cancelable: true }); e.prompt = () => { window.__prompted = true; return Promise.resolve(); }; window.dispatchEvent(e); })()"); time.sleep(0.5)
    if pg.locator(".settings .install .install-btn").count(): pg.locator(".settings .install .install-btn").click(); time.sleep(0.4)
    ok("pwa: a captured beforeinstallprompt turns into the «Install the app» button, and the button calls prompt()", pg.locator(".settings .install .install-btn").count() == 1 and pg.evaluate("window.__prompted === true"))
    pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(1.5)
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1); pg.locator(f".panel .sess[data-id='{sid}']").click(); time.sleep(0.5)
    if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click()
    pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab").first.click(); pg.wait_for_selector(".thread .chat .composer", timeout=20000); time.sleep(1)
    pg.locator(".thread .chat textarea").fill(f"Reply with exactly this and nothing else: {MARK}"); pg.locator(".thread .chat button.send").click()
    pg.wait_for_selector(f".thread .chat .m.agent:has-text('{MARK}')", timeout=180000); ok("ui: model reply streamed through service → core → agentd", True)
    pg.screenshot(path="shots/ap2-thread.png")
    ok("ui: inspector shows the session", pg.locator(".inspector").is_visible() and pg.locator(".inspector select").count() >= 3)
    # Chat form: status and context on top, model and options under the field; the cache is available in the
    # options menu.
    composer = pg.locator(".thread .chat .composer"); tools = composer.locator(".composer-controls")
    # result and the exact context arrive a little after the
    # answer text.
    try: pg.wait_for_selector(".thread .chat .m.sys.turn", timeout=30000); pg.wait_for_selector(".thread .chat .composer-context", timeout=20000)
    except Exception: pass
    # agentd sets the `[1m]` tag after init asynchronously (set_model) — the first turn may have gone before it; poll
    # for up to 10 s
    for _ in range(5):
        ctl = json.loads(raw(f"/agents/api/core/projects/{P}/sessions/{sid}/control", "POST", json.dumps({"subtype": "get_context_usage", "detail": "summary"}).encode())[1] or b"{}"); cu = ctl.get("response") or ctl
        if cu.get("maxTokens") == 1000000: break
        time.sleep(2)
    ok("ctx: Claude Code reports a 1M window for the gate session (get_context_usage.maxTokens), not 200k", cu.get("maxTokens") == 1000000 and (cu.get("autoCompactThreshold") or 0) > 900000, f"maxTokens={cu.get('maxTokens')} threshold={cu.get('autoCompactThreshold')} model={cu.get('model')}")
    ok("cmp: turn footer with duration, tokens and cache", pg.locator(".thread .chat .m.sys.turn").count() >= 1 and "кэш" in pg.locator(".thread .chat .m.sys.turn").last.inner_text())
    ok("cmp: named context ring above the input after the first turn", composer.locator(".composer-context .ring").count() == 1 and "контекст" in composer.locator(".composer-context").inner_text() and "%" in composer.locator(".composer-context").inner_text())
    tools.locator(".ctl.more").click(); pg.wait_for_selector(".menu .mi:has-text('кэш')", timeout=5000)
    cache_text = pg.locator(".menu .mi:has-text('кэш')").inner_text()
    ok("cmp: Parameters shows cache usage and TTL countdown", ("5 мин" in cache_text or "1 ч" in cache_text) and bool(re.search(r"\d+:\d{2}|истёк", cache_text)) and composer.locator(".chip.cache").count() == 0, cache_text[:100]); pg.keyboard.press("Escape")
    # the one-hour prompt cache: choose 1h in the popup (apply_flag_settings promptCacheTtl), make a turn — usage must show
    # ephemeral_1h
    composer_modes(pg); pg.wait_for_selector(".thread .chat .modes .seg", timeout=5000); pg.locator(".thread .chat .modes .seg button:has-text('1h')").click(); time.sleep(1.5)
    ok("cache: 1h requested through the popover (meta + journal line)", (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("cache_ttl") == "1h" and pg.locator(".thread .chat .m.sys:has-text('кэш промпта')").count() >= 1 and pg.locator(".thread .chat .modes .seg button.on:has-text('1h')").count() == 1)
    pg.keyboard.press("Escape"); pg.locator(".thread .chat textarea").fill(f"Reply with exactly this and nothing else: cache-{MARK}"); pg.locator(".thread .chat button.send").click()
    pg.wait_for_selector(f".thread .chat .m.agent:has-text('cache-{MARK}')", timeout=180000); time.sleep(1.5)
    evs_c = get(f"/agents/api/core/projects/{P}/sessions/{sid}/events?since=0&limit=500")[1] or []; last_res = [e for e in evs_c if e["kind"] == "result" and e.get("usage") and e.get("num_turns")][-1]
    cc = (last_res["usage"].get("cache_creation") or {})
    tools.locator(".ctl.more").click(); pg.wait_for_selector(".menu .mi:has-text('кэш')", timeout=5000); cache_text = pg.locator(".menu .mi:has-text('кэш')").inner_text(); pg.keyboard.press("Escape")
    ok("cache: the API applied the 1-hour cache (ephemeral_1h in usage), Parameters shows 1 ч", cc.get("ephemeral_1h_input_tokens", 0) > 0 and "1 ч" in cache_text, json.dumps(cc))
    composer_modes(pg); pg.wait_for_selector(".thread .chat .modes .seg", timeout=5000); pg.locator(".thread .chat .modes .seg button:has-text('auto')").click(); time.sleep(1); pg.keyboard.press("Escape")
    ok("cmp: no dollar spend in the composer (header has it)", tools.locator(".chip:has-text('$')").count() == 0 and pg.locator(".thread .t-head .chip:has-text('$')").count() == 1)
    tools.locator(".ctl.model").click(); pg.wait_for_selector(".menu", timeout=5000); mitems = [x.strip() for x in pg.locator(".menu .mi").all_inner_texts()]
    ok("cmp: model menu lists CLI models with context windows; native-1M families (Fable) are one entry with a 1M window, no «Fable · 200k» / «(1M context)» pair", any("Fable" in x and "1M" in x for x in mitems) and not any("Fable (1M context)" in x for x in mitems) and sum(1 for x in mitems if x.startswith("Fable")) == 1 and any("Sonnet" in x for x in mitems) and len(mitems) >= 6, str(mitems)[:200]); pg.keyboard.press("Escape")
    # "Mode and effort" as in Claude Code: modes with descriptions (Manual · Edit automatically · Plan · Auto · Bypass · Don't ask) and the effort
    # slider
    composer_modes(pg); pg.wait_for_selector(".thread .chat .modes", timeout=5000); modes = [x.split("\n")[0].strip() for x in pg.locator(".thread .chat .modes .mode").all_inner_texts()]
    ok("cmp: modes popover lists Claude Code modes with descriptions", modes[:4] == ["Вручную", "Правки автоматически", "План", "Авто"] and "Обход" in modes and "Не спрашивать" not in modes and pg.locator(".thread .chat .modes .mode .muted").count() >= 5, str(modes))
    dots = [pg.locator(".thread .chat .modes .slider .dot").nth(i).get_attribute("title") for i in range(pg.locator(".thread .chat .modes .slider .dot").count())]
    ok("cmp: Claude effort slider as in its own picker (low…max, ultracode on top), original names", dots == ["low", "medium", "high", "xhigh", "max", "ultracode"], str(dots))
    ok("cmp: no Codex-only rows on a Claude session (no tier switch, no separate effort popover)", pg.locator(".thread .chat .modes .switch").count() == 0 and pg.locator(".thread .chat .effpop").count() == 0)
    pg.locator(".thread .chat .modes .slider .dot[title='high']").click(); time.sleep(2)
    einfo = get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}
    ok("cmp: effort applied from the slider (meta + journal event)", einfo.get("info", {}).get("effort") == "high" and pg.locator(".thread .chat .m.sys:has-text('усилие')").count() >= 1 and "high" in tools.locator(".ctl.effort").inner_text() and pg.locator(".thread .chat .modes .slider .dot.on").count() == 1)
    ok("cmp: effort row shows the level with its description like Claude Code's picker", "high" in pg.locator(".thread .chat .modes .eff:not(.sub) .mi-t").inner_text())
    pg.locator(".thread .chat .modes .slider .dot[title='ultracode']").click(); time.sleep(2)
    ok("cmp: ultracode is the top step and applies", (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("effort") == "ultracode" and "workflow" in pg.locator(".thread .chat .modes .eff:not(.sub) .mi-t").inner_text())
    pg.locator(".thread .chat .modes .eff button[title='сбросить на auto']").click(); time.sleep(1.5)
    ok("cmp: reset returns effort to auto", (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("effort") is None and "auto" in pg.locator(".thread .chat .modes .eff:not(.sub) .mi-t").inner_text())
    pg.locator(".thread .chat .modes .mode:has-text('План')").click(); time.sleep(2)
    ok("cmp: mode chosen in the popover applies to the live session", (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("permission_mode") == "plan" and "План" in tools.locator(".ctl.mode").inner_text())
    pg.locator(".thread .chat .modes .mode:has-text('Вручную')").click(); time.sleep(1.5); pg.keyboard.press("Escape"); time.sleep(0.5)
    ok("cmp: popover closes on Escape", pg.locator(".thread .chat .modes").count() == 0)
    ta = pg.locator(".thread .chat textarea"); ta.click(); ta.fill("/"); pg.wait_for_selector(".thread .chat .popup", timeout=5000)
    citems = [x.strip() for x in pg.locator(".thread .chat .popup .pi .pn").all_inner_texts()]
    ok("cmp: slash popup with local and CLI commands", "/model" in citems and "/compact" in citems and "/context" in citems, str(citems)[:200])
    ta.fill("/con"); time.sleep(0.4); citems = [x.strip() for x in pg.locator(".thread .chat .popup .pi .pn").all_inner_texts()]; ok("cmp: slash popup filters by prefix", citems and all(x.startswith("/con") for x in citems), str(citems))
    pg.keyboard.press("Enter"); time.sleep(0.3); ok("cmp: Enter picks the command into the input", ta.input_value().startswith("/context") or ta.input_value().startswith("/config"), ta.input_value())
    ta.fill("/effort xhigh"); pg.keyboard.press("Enter"); time.sleep(2)
    ok("cmp: /effort command goes through the session API", (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("effort") == "xhigh" and ta.input_value() == "")
    ta.fill("/context"); pg.keyboard.press("Escape"); pg.keyboard.press("Enter"); pg.wait_for_selector(".thread .chat .m.cmd", timeout=60000)
    ok("cmp: CLI command output rendered as a command card", "Context" in pg.locator(".thread .chat .m.cmd").last.inner_text() and pg.locator(".thread .chat .m.agent:has-text('Context Usage')").count() == 0)
    cmd_card = pg.locator(".thread .chat .m.cmd").last
    ok("md: markdown rendered — table, bold and headings instead of raw pipes and asterisks", cmd_card.locator("table").count() >= 1 and cmd_card.locator("strong").count() >= 1 and cmd_card.locator("h2, h3").count() >= 1 and "**" not in cmd_card.inner_text() and "|---" not in cmd_card.inner_text(), cmd_card.inner_text()[:80].replace("\n", " "))
    pg.screenshot(path="shots/ap2e-markdown.png")
    ok("md: inspector snippet of the last reply is rendered, not raw markdown", "**" not in pg.locator(".inspector").inner_text() and "##" not in pg.locator(".inspector").inner_text())
    ta.fill("см @net"); time.sleep(2.5)
    fitems = [x.strip() for x in pg.locator(".thread .chat .popup .pi .pn").all_inner_texts()]; ok("cmp: @ popup suggests workspace files", any("net" in x.lower() for x in fitems), str(fitems)[:160])
    if fitems: pg.keyboard.press("Enter"); time.sleep(0.3); ok("cmp: picked file inserted as @path", "@" in ta.input_value() and ta.input_value().startswith("см @"), ta.input_value())
    ta.fill(""); pg.keyboard.press("Escape")
    pg.screenshot(path="shots/ap2d-composer.png")
    st_g, body_g = raw(f"/agents/api/core/projects/{P}/sessions/{sid}/control", "POST", json.dumps({"subtype": "get_settings"}).encode())
    eff = ((json.loads(body_g) or {}).get("effective") or {}).get("effortLevel") if st_g == 200 else None
    ok("cmp: CLI get_settings reports the effort chosen in the UI", eff == "xhigh", f"{st_g} {eff}")
    # session card tabs in the app: Terminal (a shell in cwd), Files (directory, folder, preview), Git
    pane = pg.locator(".thread .pane")
    pane.locator(".tabs .tab:has-text('Терминал')").click(); pg.wait_for_selector(".thread .pane .xterm", timeout=30000); time.sleep(2.5)
    pg.keyboard.type(f"echo tab-{MARK}"); pg.keyboard.press("Enter"); seen = ""
    for _ in range(24):
        seen = pane.locator(".xterm-rows").inner_text() if pane.locator(".xterm-rows").count() else ""
        if seen.count(f"tab-{MARK}") >= 2: break
        time.sleep(0.5)
    ok("tabs: terminal runs a shell and echoes back", seen.count(f"tab-{MARK}") >= 2, " ".join(seen.split())[:120])
    # DECRQM (`CSI ? 2026 $ p` — that is how agy/bubbletea and ratatui ask for mode support) crashed the xterm parser in the
    # build (esbuild dropped `let r` from the dead requestMode enum → ReferenceError), and the terminal froze forever: text
    # after the request and any further output were not rendered. Now the answer `CSI ? 2026 ; 2 $ y` goes to the PTY, the
    # text is visible
    pg.keyboard.type("printf '\\033[?2026$pdecrqm-" + MARK + "\\n'; read -t 3 -s -n 11 REPLY; printf 'reply-%s\\n' \"${REPLY#*[}\" | tr -d '\\033'"); pg.keyboard.press("Enter"); seen = ""
    for _ in range(24):
        seen = pane.locator(".xterm-rows").inner_text() if pane.locator(".xterm-rows").count() else ""
        if f"decrqm-{MARK}" in seen and "reply-" in seen: break
        time.sleep(0.5)
    ok("tabs: a DECRQM query (CSI ? 2026 $ p) neither freezes the terminal nor hides the text after it; xterm answers the PTY", seen.count(f"decrqm-{MARK}") >= 2 and ("reply-?2026;2$" in seen or "reply-?2026;" in seen), " ".join(seen.split())[-160:])
    pane.locator(".tabs .tab:has-text('Файлы')").click(); pg.wait_for_selector(".thread .pane .files .list", timeout=20000); time.sleep(1)
    ok("tabs: files list the workspace home", pane.locator(f".files .list .open:has-text('{HOME_DIR}')").count() == 1)
    pane.locator(f".files .list .open:has-text('{HOME_DIR}')").click(); time.sleep(1.5); pane.locator(f".files .list .open:has-text('{HOME_FILE}')").first.click(); pg.wait_for_selector(".thread .pane .files .preview", timeout=20000); time.sleep(1)
    ok("tabs: files open a folder and preview a text file", pane.locator(f".files .crumbs .crumb:has-text('{HOME_DIR}')").count() == 1 and len(pane.locator(".files .preview").inner_text()) > 20)
    pane.locator(".tabs .tab:has-text('Git')").click(); pg.wait_for_selector(".thread .pane .git", timeout=20000); time.sleep(1.5); gtxt = pane.locator(".git").inner_text()
    ok("tabs: git pane renders for the session directory", len(gtxt.strip()) > 5 and ("/home/coder" in gtxt or pane.locator(".git .row").count() > 0 or "ветк" in gtxt.lower()), " ".join(gtxt.split())[:80])
    pane.locator(".tabs .tab:has-text('Чат')").click(); pg.wait_for_selector(".thread .chat textarea", timeout=10000)
    pg.locator(".thread .t-head button:has-text('Остановить')").click(); pg.wait_for_selector(".thread .chat .composer-status:has-text('завершён')", timeout=30000); time.sleep(1)
    pg.locator(".thread .chat textarea").click(); pg.keyboard.type("Reply with exactly this and nothing else: again-" + MARK)
    ok("ui: finished session still accepts text", pg.locator(".thread .chat textarea").input_value().endswith(MARK) and not pg.locator(".thread .chat textarea").is_disabled())
    # mode and model on a stopped session: saved into the meta (go as flags on resume), no error
    for _ in range(20):
        if (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("alive") is False: break
        time.sleep(0.5)
    composer_modes(pg); pg.wait_for_selector(".thread .chat .modes", timeout=5000); pg.locator(".thread .chat .modes .mode:has-text('План')").click()
    mmode = None
    for _ in range(12):
        time.sleep(0.5); mmode = (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("permission_mode")
        if mmode == "plan": break
    mlabel = ""
    for _ in range(12):
        mlabel = tools.locator(".ctl.mode").inner_text()
        if "План" in mlabel: break
        time.sleep(0.5)
    merr = pg.locator(".thread .chat .muted:has-text('не запущен')").count()
    ok("mode: switch on a stopped session persists without error", mmode == "plan" and "План" in mlabel and merr == 0, f"api={mmode} label={mlabel!r} err={merr}")
    ok("mode: bypass item blocked by the workspace policy", pg.locator(".thread .chat .modes .mode.dis:has-text('Обход')").count() == 1 and "политикой" in pg.locator(".thread .chat .modes .mode.dis").first.inner_text())
    pg.locator(".thread .chat .modes .mode:has-text('Вручную')").click(); time.sleep(1); pg.keyboard.press("Escape")
    pg.locator(".thread .chat button.send").click(); pg.wait_for_selector(f".thread .chat .m.agent:has-text('again-{MARK}')", timeout=180000); ok("ui: send resumes the session and delivers the message", True)
    # terminal mode (CLI): the same conversation in the native TUI; the headless process is stopped, the TUI lives in the
    # session PTY with tui=true
    def tui_pty():
        return [x for x in (get(f"/agents/api/core/projects/{P}/ptys")[1] or []) if x.get("session") == sid and x.get("tui") and x.get("alive")]
    tabs = [x.get_attribute("aria-label") for x in pg.locator(".thread .pane .tabs .tab").all()]; ok("cli: tab in the session pane", tabs == ["Чат", "CLI", "Терминал", "Файлы", "Git"], str(tabs))
    pg.locator(".thread .pane .tabs .tab:has-text('CLI')").click(); pg.wait_for_selector(".thread .pane .start button:has-text('Подключиться')", timeout=10000)
    ok("cli: start panel with bypass switch", pg.locator(".thread .pane .start .switch").count() == 1 and not pg.locator(".thread .pane .start .switch").first.evaluate("e => e.classList.contains('on')"))
    pg.locator(".thread .pane .start button:has-text('Подключиться')").click(); pg.wait_for_selector(".thread .pane .xterm", timeout=30000); time.sleep(2)
    p1 = tui_pty(); s_after = get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}
    ok("cli: attach stops the headless process and opens a TUI pty linked to the session", len(p1) == 1 and s_after.get("info", {}).get("alive") is False and s_after["info"].get("tui") == p1[0]["id"], json.dumps({"ptys": len(p1), "alive": s_after.get("info", {}).get("alive"), "tui": s_after.get("info", {}).get("tui")}))
    ok("cli: launched claude on the same conversation", bool(p1) and p1[0]["cmd"][:2] == ["bash", "-lic"] and " claude " in p1[0]["cmd"][2] and ("--resume" in p1[0]["cmd"][2] or "--session-id" in p1[0]["cmd"][2]), p1[0]["cmd"][2][:160] if p1 else "")
    seen = ""
    for _ in range(60):
        seen = pg.locator(".thread .pane .xterm-rows").inner_text() if pg.locator(".thread .pane .xterm-rows").count() else ""
        if "claude" in seen.lower() or "welcome" in seen.lower() or "trust" in seen.lower(): break
        time.sleep(1)
    ok("cli: TUI renders in xterm", "claude" in seen.lower() or "welcome" in seen.lower() or "trust" in seen.lower(), " ".join(seen.split())[:160])
    pg.screenshot(path="shots/ap2b-cli.png")
    # the "Restart"/"Detach" labels in a narrow Thread (the inspector is open) are hidden, the icons with a
    # title remain
    btns = pg.locator(".thread .pane .tabs button.xs"); head = [btns.nth(0).inner_text().strip()] + [btns.nth(i).get_attribute("title") for i in (1, 2)]
    ok("cli: controls bypass / restart / disconnect", head == ["Обход", "Перезапустить", "Отключиться"], str(head))
    keys = [x.strip() for x in pg.locator(".thread .pane .tabs .keys .key").all_inner_texts()]; ok("cli: phone keys ↑ ↓ ⏎ Esc in the tab strip", keys == ["↑", "↓", "⏎", "Esc"], str(keys))
    if "trust" in seen.lower():   # the folder trust dialog: ↓ + ⏎ pick "Yes, I trust this folder" — the TUI goes on
        # the keys are sent on click (pointerdown only kills the focus) and are disabled until the terminal is connected — wait for
        # an active button
        for _ in range(40):
            if pg.locator(".thread .pane .tabs .keys .key[title='Down']:enabled").count(): break
            time.sleep(0.5)
        pg.locator(".thread .pane .tabs .keys .key[title='Down']").click(); time.sleep(0.4); pg.locator(".thread .pane .tabs .keys .key[title='Enter']").click()
        gone = False
        for _ in range(30):
            time.sleep(0.5); txt = pane.locator(".xterm-rows").inner_text() if pane.locator(".xterm-rows").count() else ""
            if "Quick safety check" not in txt: gone = True; break
        ok("cli: keys drive the TUI (trust dialog answered with ↓ ⏎)", gone, " ".join(txt.split())[:120])
    # an image in the TUI: a paste event with a file → the file in ~/.cloudos/paste, the path is pasted into the terminal, Claude Code
    # makes an [Image #1] attachment
    def png8():
        def chunk(tp, d): return struct.pack(">I", len(d)) + tp + d + struct.pack(">I", zlib.crc32(tp + d) & 0xffffffff)
        raw = b"".join(b"\x00" + b"\xff\x40\x10" * 8 for _ in range(8))
        return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 8, 8, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
    ok("cli: 📎 attach button in the tab strip (button + hidden file input beside it)", pg.locator(".thread .pane .tabs .clipbtn").count() == 1 and pg.locator(".thread .pane .tabs input[type=file]").count() == 1)
    # a real Ctrl+V: xterm must hand it to the browser (otherwise ^V goes to the TUI and Claude Code answers "No image found in
    # clipboard"); an image in the browser clipboard — via the Async Clipboard API if it works in headless
    ctx.grant_permissions(["clipboard-read", "clipboard-write"], origin=f"https://{HOST}")
    png_b64 = base64.b64encode(png8()).decode()
    clip_ok = pg.evaluate("""async ([b64]) => { try { const bin = atob(b64); const arr = new Uint8Array(bin.length); for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i); await navigator.clipboard.write([new ClipboardItem({ 'image/png': new Blob([arr], { type: 'image/png' }) })]); return true; } catch (e) { return String(e); } }""", [png_b64])
    pane.locator(".xterm-helper-textarea").focus(); pg.keyboard.press("Control+v"); got = False; noimg = False; ptxt = ""
    for _ in range(24):
        time.sleep(0.5); ptxt = pane.locator(".xterm-rows").inner_text() if pane.locator(".xterm-rows").count() else ""
        if "[Image #1]" in ptxt: got = True; break
        if "No image found in clipboard" in ptxt: noimg = True; break
    ok("cli: Ctrl+V in the browser is not forwarded to the TUI as ^V", not noimg, " ".join(ptxt.split())[-100:])
    if got: ok("cli: Ctrl+V with an image in the browser clipboard uploads it and the TUI shows [Image #1]", True)
    else: print("SKIP  cli: Ctrl+V with a clipboard image — no paste from the headless clipboard (clipboard.write:", str(clip_ok)[:80], ")", flush=True)
    img_n = 2 if got else 1
    pg.evaluate("""([b64]) => { const bin = atob(b64); const arr = new Uint8Array(bin.length); for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i); const f = new File([arr], 'image.png', { type: 'image/png' }); const dt = new DataTransfer(); dt.items.add(f); const ev = new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }); (document.querySelector('.thread .pane .term .xterm-helper-textarea') || document.querySelector('.thread .pane .term')).dispatchEvent(ev); }""", [png_b64])
    ptxt = ""
    for _ in range(40):
        time.sleep(0.5); ptxt = pane.locator(".xterm-rows").inner_text() if pane.locator(".xterm-rows").count() else ""
        if f"[Image #{img_n}]" in ptxt: break
    plist = get(f"/agents/api/core/projects/{P}/fs?path=.cloudos/paste")[1]
    ok("cli: a pasted image (paste event with a file: drop and 📎 go the same way) lands in ~/.cloudos/paste and the TUI turns its path into an [Image #N] attachment", f"[Image #{img_n}]" in ptxt and ".png" in json.dumps(plist), " ".join(ptxt.split())[-120:] + f" | list={json.dumps(plist)[:80]}")
    pg.screenshot(path="shots/ap2c-cli-image.png")
    raw(f"/agents/api/core/projects/{P}/fs?path=.cloudos/paste&recursive=true", "DELETE")
    hdr = pg.locator(".thread .t-head button:has-text('Отключить CLI')"); ok("cli: thread header treats the session as live in the terminal", hdr.count() == 1 and pg.locator(".thread .t-head:has-text('в терминале')").count() == 1)
    pg.locator(".thread .pane .tabs button.xs:has-text('Перезапустить')").click(); time.sleep(3)
    for _ in range(20):
        p2 = tui_pty()
        if p2 and p2[0]["id"] != p1[0]["id"]: break
        time.sleep(1)
    ok("cli: restart replaces the TUI pty", len(p2) == 1 and p2[0]["id"] != p1[0]["id"] and get(f"/agents/api/core/projects/{P}/ptys/{p1[0]['id']}")[0] == 404)
    # sending from the chat with the TUI attached: agentd closes the TUI and resumes the headless
    # process
    pg.locator(".thread .pane .tabs .tab:has-text('Чат')").click(); pg.wait_for_selector(".thread .chat textarea", timeout=10000)
    ph = ""
    for _ in range(20):
        # slice 45: the hint about returning from the CLI is a notice line above the field (.composer-notice), the
        # placeholder is always short
        ph = " ".join(pg.locator(".thread .chat .composer-notice").all_inner_texts()) + " " + (pg.locator(".thread .chat textarea").get_attribute("placeholder") or "")
        if "CLI" in ph: break
        time.sleep(0.5)
    ok("cli: chat hints that sending returns the session from the CLI", "CLI" in ph, ph.strip()[:120])
    pg.locator(".thread .chat textarea").fill(f"Reply with exactly this and nothing else: cli-{MARK}"); pg.locator(".thread .chat button.send").click()
    pg.wait_for_selector(f".thread .chat .m.agent:has-text('cli-{MARK}')", timeout=180000)
    s_back = get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}
    ok("cli: sending from chat closes the TUI and resumes the headless session", not tui_pty() and s_back.get("info", {}).get("alive") is True and not s_back["info"].get("tui"))
    pg.locator(".thread .pane .tabs .tab:has-text('CLI')").click(); pg.wait_for_selector(".thread .pane .start button:has-text('Подключиться')", timeout=10000); ok("cli: pane shows the start panel again", True)
    # bypass in the CLI: blocked by the workspace policy; after the owner enables the policy — available and goes into the TUI
    # command
    ok("cli: bypass switch blocked by the workspace policy", pg.locator(".thread .pane .start .switch").is_disabled() and "политикой" in pg.locator(".thread .pane .start .opt").inner_text())
    raw("/agents/api/core/agents/policy", "PUT", json.dumps({"bypass": True}).encode())
    pg.locator(".rail .nav[data-tip='Входящие']").click(); time.sleep(1); pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs .tab:has-text('CLI')", timeout=20000); pg.locator(".thread .pane .tabs .tab:has-text('CLI')").click(); pg.wait_for_selector(".thread .pane .start button:has-text('Подключиться')", timeout=10000); time.sleep(1.5)
    ok("cli: bypass switch enabled once the policy allows it", not pg.locator(".thread .pane .start .switch").is_disabled())
    pg.locator(".thread .pane .start .switch").click(); pg.locator(".thread .pane .start button:has-text('Подключиться')").click(); pg.wait_for_selector(".thread .pane .xterm", timeout=30000); time.sleep(2)
    p3 = tui_pty(); ok("cli: bypass switch passes bypassPermissions to the TUI", bool(p3) and "bypassPermissions" in p3[0]["cmd"][2] and pg.locator(".thread .pane .tabs button.xs.on:has-text('Обход')").count() == 1, p3[0]["cmd"][2][-80:] if p3 else "")
    pg.locator(".thread .pane .tabs button.xs:has-text('Отключиться')").click(); pg.wait_for_selector(".thread .pane .start button:has-text('Подключиться')", timeout=20000); time.sleep(1)
    ok("cli: disconnect kills the TUI pty; session resumable", not tui_pty() and (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("alive") is False)
    pg.locator(".thread .pane .start .switch").click(); raw("/agents/api/core/agents/policy", "PUT", json.dumps({"bypass": False}).encode())
    # owner policy: allow bypass → the workspace managed-settings has no bypass ban (only DISABLE_AUTOUPDATER remains), a new session starts in bypassPermissions, live switching works
    st_p, body_p = raw("/agents/api/core/agents/policy"); ok("policy: default forbids bypass", st_p == 200 and json.loads(body_p).get("bypass") is False, body_p[:80])
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Разрешения')").click(); pg.wait_for_selector(".settings .frm .switch", timeout=10000); time.sleep(1)
    sw = pg.locator(".settings .frm .switch").first
    ok("policy: switch in Settings shows the current state", not sw.evaluate("e => e.classList.contains('on')") and "запрещён" in pg.locator(".settings .frm").first.inner_text())
    sw.click(); pg.wait_for_selector("text=Политика применена", timeout=20000); time.sleep(1)
    st_p, body_p = raw("/agents/api/core/agents/policy"); pol = json.loads(body_p) if st_p == 200 else {}
    ok("policy: enabling from Settings applies to running workspaces", sw.evaluate("e => e.classList.contains('on')") and pol.get("bypass") is True and "разрешён" in pg.locator(".settings .frm").first.inner_text(), body_p[:120])
    st_f, body_f = raw(f"/agents/api/core/projects/{P}/fs/file?path=/etc/claude-code/managed-settings.json"); ok("policy: managed-settings rewritten inside the workspace (no bypass ban, auto-updater still off)", st_f == 200 and b"disableBypassPermissionsMode" not in body_f and b"DISABLE_AUTOUPDATER" in body_f, body_f[:80])
    bp = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": "bypass probe", "permission_mode": "bypassPermissions"}).encode())[1])
    cur = None
    for _ in range(40):
        time.sleep(1); evs = get(f"/agents/api/core/projects/{P}/sessions/{bp['id']}/events?since=0&limit=100")[1] or []
        ini = next((e for e in evs if e["kind"] == "initialized"), None)
        if ini: cur = (ini.get("response") or {}).get("current_permission_mode"); break
    ok("policy: new session starts in bypassPermissions once allowed", cur == "bypassPermissions", str(cur))
    st_m, body_m = raw(f"/agents/api/core/projects/{P}/sessions/{bp['id']}/mode", "POST", json.dumps({"mode": "default"}).encode()); st_m2, body_m2 = raw(f"/agents/api/core/projects/{P}/sessions/{bp['id']}/mode", "POST", json.dumps({"mode": "bypassPermissions"}).encode())
    ok("policy: live switch default → bypass accepted by the CLI", st_m == 200 and st_m2 == 200 and json.loads(body_m2).get("mode") == "bypassPermissions", body_m2[:120])
    for _ in range(3):
        if raw(f"/agents/api/core/projects/{P}/sessions/{bp['id']}", "DELETE")[0] == 204: break
        time.sleep(3)
    sw.click(); pg.wait_for_selector("text=Политика применена", timeout=20000); time.sleep(1.5); st_f, body_f = raw(f"/agents/api/core/projects/{P}/fs/file?path=/etc/claude-code/managed-settings.json")
    ok("policy: switched back from Settings, managed-settings restored", not sw.evaluate("e => e.classList.contains('on')") and b"disableBypassPermissionsMode" in body_f, body_f[:80])
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab:has-text('Чат')").click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000)
    # canvas: a card with a long history does not grow — the height is set by the grid row; the canvas
    # on a long conversation from disk (the test session's history is short and may fit into the card)
    pg.locator(".top .seg button:has-text('Code')").click(); time.sleep(1); tree_expand(pg, P); pg.locator(f".panel .ws:has(.wname:has-text('{P}')) .sess:has-text('{KNOWN_SESSION}')").first.click(); time.sleep(2.5)
    card = pg.locator(".canvas .card").filter(has_text=KNOWN_SESSION)
    # another tab may be open in the card — the journal exists only in "Chat"; the owner's foreign live cards may be on the
    # canvas (the layout is shared, from the core), because of them a click on the tab may "drift" — click with force and do not
    # fail the run
    if card.count():
        try: card.first.scroll_into_view_if_needed(); card.first.locator(".tabs .tab").first.click(force=True, timeout=10000)
        except Exception as e: print("note: canvas tab click:", str(e)[:100], flush=True)
        time.sleep(2.5)
    hgt = card.first.evaluate("e => e.getBoundingClientRect().height") if card.count() else 0; scr = card.first.evaluate("e => { const l = e.querySelector('.log'); return l ? l.scrollHeight > l.clientHeight + 4 : false; }") if card.count() else False
    ok("canvas: card height is fixed by the grid row and history scrolls inside", card.count() == 1 and 380 <= hgt <= 470 and scr, f"height {hgt:.0f}, scrolls {scr}")
    # collapsible cards (docs/agents-ux.md §4.5): the chevron collapses to the header (the body and its socket are not mounted), the state
    # survives a reload
    if card.count():
        # collapsible cards on all screens (§4.5, 2026-10-22): on desktop the chevron exists, the card is expanded by default; a click
        # collapses to the header (no body), the state survives a reload (agents.foldd), the phone "collapsed" (agents.fold) does not
        # touch the desktop
        ok("canvas: on the desktop the card has a fold chevron and is expanded by default", card.first.locator(".c-head .fold").count() == 1 and card.first.locator(".c-body").count() == 1 and not card.first.evaluate("e => e.classList.contains('collapsed')"))
        card.first.locator(".c-head .fold").click(); time.sleep(0.6)
        h_f = card.first.evaluate("e => e.getBoundingClientRect().height")
        ok("canvas: the chevron collapses the desktop card to its header — no body, no socket, one grid cell", card.first.evaluate("e => e.classList.contains('collapsed')") and card.first.locator(".c-body").count() == 0 and h_f < 80, f"height {h_f:.0f}")
        pg.reload(wait_until="load"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(2)
        if not pg.locator(".canvas").count(): pg.locator(".top .seg button:has-text('Code')").click(); time.sleep(1)
        card = pg.locator(".canvas .card").filter(has_text=KNOWN_SESSION)
        ok("canvas: the collapsed state survives a reload on the desktop", card.count() == 1 and card.first.evaluate("e => e.classList.contains('collapsed')") and card.first.locator(".c-body").count() == 0)
        card.first.locator(".c-head .fold").click(); time.sleep(0.6)
        ok("canvas: the chevron expands the card back with its body", not card.first.evaluate("e => e.classList.contains('collapsed')") and card.first.locator(".c-body").count() == 1)
        pg.evaluate("() => localStorage.setItem('agents.fold', JSON.stringify(Object.fromEntries([...document.querySelectorAll('.canvas .card')].map(c => [c.dataset.card, true]))))")
        pg.reload(wait_until="load"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(2)
        if not pg.locator(".canvas").count(): pg.locator(".top .seg button:has-text('Code')").click(); time.sleep(1)
        card = pg.locator(".canvas .card").filter(has_text=KNOWN_SESSION)
        ok("canvas: a phone fold state left in localStorage does not fold desktop cards after a reload", card.count() == 1 and not card.first.evaluate("e => e.classList.contains('collapsed')") and card.first.locator(".c-body").count() == 1)
        pg.evaluate("() => localStorage.removeItem('agents.fold')")
        # two columns on an unfolded foldable screen (§4.11, 2026-10-22): at 673 px (Fold 3 unfolded) two cards side by side, at 390 px — in a
        # column
        added = False
        if pg.locator(".canvas .card").count() < 2:
            tree_expand(pg, P); pg.locator(f".panel .sess[data-id='{sid}']").click(); time.sleep(1.5); added = pg.locator(f".canvas .card[data-card$='{sid}']").count() == 1
        if pg.locator(".canvas .card").count() >= 2:
            pg.set_viewport_size({"width": 673, "height": 900}); time.sleep(1.2)
            xs = pg.locator(".canvas .card").evaluate_all("els => els.slice(0, 2).map(e => Math.round(e.getBoundingClientRect().left))")
            pg.set_viewport_size({"width": 390, "height": 844}); time.sleep(1.2)
            xp = pg.locator(".canvas .card").evaluate_all("els => els.slice(0, 2).map(e => Math.round(e.getBoundingClientRect().left))")
            pg.set_viewport_size({"width": 1500, "height": 900}); time.sleep(1.2)
            ok("canvas: at 673 px (unfolded Fold 3) two cards sit side by side, at 390 px they stack", len(xs) == 2 and xs[0] != xs[1] and len(xp) == 2 and xp[0] == xp[1], f"673px lefts={xs} 390px lefts={xp}")
        if added: pg.locator(f".canvas .card[data-card$='{sid}'] .c-head button[aria-label='Снять с холста']").first.click(); time.sleep(0.6)
    # free resize by the corner (docs/agents-ux.md §4.11): rows of 100 px, the card spans columns × rows; drag the corner
    # live, the arrows on the corner ± a row, the "height" preset 4 ↔ 8, the size survives a reload (the layout is in the
    # core)
    rz = pg.locator(f".canvas .card[data-card$=':{sid}']") if sid else pg.locator(".canvas .card").first
    if rz.count():
        span = lambda: rz.first.evaluate("e => [e.style.gridColumn, e.style.gridRow]")
        s0 = span(); r0 = rz.first.bounding_box(); g = rz.first.locator(".grip"); g.hover(); gb = g.bounding_box()
        pg.mouse.move(gb["x"] + gb["width"] / 2, gb["y"] + gb["height"] / 2); pg.mouse.down()
        pg.mouse.move(gb["x"] + 10, gb["y"] + 230, steps=6); time.sleep(0.3)
        hint = rz.first.locator(".size-hint").inner_text() if rz.first.locator(".size-hint").count() else ""
        pg.mouse.up(); time.sleep(0.4); s1 = span(); r1 = rz.first.bounding_box()
        ok("resize: dragging the corner down by two rows grows the card live (span 4 → 6, +228 px) with a size hint while dragging", s0[1] == "span 4" and s1[1] == "span 6" and abs((r1["height"] - r0["height"]) - 228) < 6 and "× 6" in hint and "px" in hint, f"{s0} → {s1} Δh={r1['height'] - r0['height']:.0f} hint={hint!r}")
        g.focus(); pg.keyboard.press("ArrowUp"); time.sleep(0.3); s2 = span()
        rz.first.locator(".c-head .acts button[aria-label='Высота']").click(); time.sleep(0.3); s3 = span()
        rz.first.locator(".c-head .acts button[aria-label='Высота']").click(); time.sleep(0.3); s4 = span()
        ok("resize: ArrowUp on the corner takes one row off, the «Высота» preset toggles 8 ↔ 4 rows", s2[1] == "span 5" and s3[1] == "span 8" and s4[1] == "span 4", f"{s2[1]} {s3[1]} {s4[1]}")
        g.focus(); pg.keyboard.press("ArrowDown"); pg.keyboard.press("ArrowDown"); pg.keyboard.press("ArrowDown"); time.sleep(1.5)
        pg.reload(wait_until="load"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(2.5)
        if not pg.locator(".canvas").count(): pg.locator(".top .seg button:has-text('Code')").click(); time.sleep(1)
        rz = pg.locator(f".canvas .card[data-card$=':{sid}']") if sid else pg.locator(".canvas .card").first
        lay = (get("/agents/api/core/agents/layout")[1] or {}).get("cards", [])
        mine = next((c for c in lay if c.get("id") == sid), None)
        ok("resize: the size survives a reload and the shared layout in the core stores rows, not the old 1|2 flags", rz.count() == 1 and span()[1] == "span 7" and mine is not None and mine.get("h") == 7, f"{span()} core={mine}")
        g = rz.first.locator(".grip"); g.focus(); [pg.keyboard.press("ArrowUp") for _ in range(3)]; time.sleep(1.2)
        card = pg.locator(".canvas .card").filter(has_text=KNOWN_SESSION)
    # session mark and own name (docs/agents-ux.md §4.8)
    if card.count() and sid:
        mk = lambda sel: pg.evaluate("(sel) => { const e = document.querySelector(sel); return e ? (e.dataset.shape + ':' + e.style.getPropertyValue('--mark-h').trim()) : null; }", sel)
        m_card = mk(f".canvas .card[data-card$=':{sid}'] .c-head > .smark"); m_tree = mk(f".panel .sess[data-id='{sid}'] .t .smark")
        hh = card.first.locator(".c-head").evaluate("e => e.getBoundingClientRect().height")
        ok("mark: the session card header and the tree row carry the same deterministic mark, and the header stays one row", m_card is not None and m_card == m_tree and ":" in m_card and hh <= 40, f"mark={m_card} tree={m_tree} header={hh:.0f}px")
        others = pg.evaluate("() => [...document.querySelectorAll('.panel .sess .t .smark')].map(e => e.dataset.shape + ':' + e.style.getPropertyValue('--mark-h').trim())")
        ok("mark: marks are drawn for every session row and vary across sessions", len(others) >= 2 and len(set(others)) >= 2, f"rows={len(others)} distinct={len(set(others))}")
        # renaming from the card's "⋯" menu: the own name over the auto-title, the auto-title is not lost
        auto_t = (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("title", "")
        ecard = pg.locator(f".canvas .card[data-card$=':{sid}']")   # the run's session card (not the owner's cards)
        ecard.first.scroll_into_view_if_needed(); ecard.first.hover(); ecard.first.locator(".c-head .acts button[data-tip='Ещё']").click(force=True); pg.wait_for_selector(".menu .mi", timeout=5000)
        pg.locator(".menu .mi").filter(has_text="Переименовать").click(); pg.wait_for_selector(".rename-dlg .rename-input", timeout=5000); time.sleep(0.3)
        pre = pg.locator(".rename-dlg .rename-input").input_value(); sel_all = pg.evaluate("() => { const i = document.querySelector('.rename-dlg .rename-input'); return i && document.activeElement === i && i.selectionStart === 0 && i.selectionEnd === i.value.length; }")
        ok("rename: the dialog opens from the card menu with the current name prefilled, focused and selected", pre == auto_t and sel_all and "автоназвание" in pg.locator(".rename-dlg .rename-auto").inner_text(), f"prefilled={pre!r}")
        NEW = f"🔥 Своё имя {MARK}"
        pg.locator(".rename-dlg .rename-input").fill(NEW); pg.keyboard.press("Enter")
        try: pg.wait_for_selector(".rename-dlg", state="detached", timeout=10000)
        except Exception: pass
        time.sleep(1.2)
        info_l = (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {})
        ok("rename: the label is stored next to the untouched auto title and shows in the card and the tree", info_l.get("label") == NEW and info_l.get("title") == auto_t and ecard.first.locator(".c-head .ttl b").inner_text().strip() == NEW and NEW in pg.locator(f".panel .sess[data-id='{sid}'] .t").inner_text() and pg.locator(".rename-dlg").count() == 0, f"label={info_l.get('label')!r} title={info_l.get('title')!r}")
        if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.6)
        pg.locator(f".panel .sess[data-id='{sid}']").click(); time.sleep(0.8)
        ok("rename: the Thread heading shows the new name with the mark", pg.locator(".thread-heading h2").count() == 1 and NEW in pg.locator(".thread-heading h2").inner_text() and pg.locator(".thread-heading h2 .smark").count() == 1)
        pg.locator(".thread .t-head .thread-menu").click(); pg.wait_for_selector(".menu .mi", timeout=5000); pg.locator(".menu .mi").filter(has_text="Переименовать").click(); pg.wait_for_selector(".rename-dlg .rename-reset", timeout=5000)
        pg.locator(".rename-dlg .rename-reset").click()
        try: pg.wait_for_selector(".rename-dlg", state="detached", timeout=10000)
        except Exception: pass
        time.sleep(1.2)
        info_l = (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {})
        ok("rename: «автоназвание» from the Thread menu clears the label and the auto title is back everywhere", not info_l.get("label") and info_l.get("title") == auto_t and auto_t in pg.locator(".thread-heading h2").inner_text() and auto_t[:20] in pg.locator(f".panel .sess[data-id='{sid}'] .t").inner_text(), f"label={info_l.get('label')!r}")
        if pg.locator(".top .seg button:has-text('Code')").count(): pg.locator(".top .seg button:has-text('Code')").click(); time.sleep(0.6)
        card = pg.locator(".canvas .card").filter(has_text=KNOWN_SESSION)
    if card.count():
        try: card.first.locator(".acts button[data-tip='Снять с холста']").click(force=True, timeout=10000)   # the owner's foreign live cards may overlap
        except Exception as e: print("note: remove card click:", str(e)[:100], flush=True)
        time.sleep(0.5)
    pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); time.sleep(1)
    # conversations from the workspace disk: `claude -p` in the workspace terminal (outside agentd) → agentd picks up the
    # Claude Code journal with history; Codex — a synthetic rollout. Deleting the session removes the provider file too.
    def sessions(): return get(f"/agents/api/core/projects/{P}/sessions")[1] or []
    DISK = str(uuid.uuid4()); before_ids = {x["id"] for x in sessions()}
    pty = json.loads(raw(f"/agents/api/core/projects/{P}/ptys", "POST", json.dumps({"cmd": ["bash", "-lic", f"claude -p 'Reply with exactly this and nothing else: disk-{MARK}' --session-id {DISK}; exit"], "cwd": "/home/coder", "title": "e2e claude -p"}).encode())[1])
    for _ in range(90):
        time.sleep(2); pi = get(f"/agents/api/core/projects/{P}/ptys/{pty['id']}")[1]
        if pi and not pi["alive"]: break
    ok("disk: claude -p ran in a workspace terminal outside agentd", bool(pi) and not pi["alive"], json.dumps(pi)[:120] if pi else "no pty")
    raw(f"/agents/api/core/projects/{P}/ptys/{pty['id']}", "DELETE")
    CODEX = str(uuid.uuid4()); ROLL = f"/home/coder/.codex/sessions/2026/09/28/rollout-2026-09-28T00-00-00-{CODEX}.jsonl"
    rollout = "\n".join(json.dumps(x, ensure_ascii=False) for x in [
        {"timestamp": "2026-09-28T00:00:00.000Z", "type": "session_meta", "payload": {"id": CODEX, "timestamp": "2026-09-28T00:00:00.000Z", "cwd": "/home/coder", "originator": "codex_cli", "source": "cli"}},
        {"timestamp": "2026-09-28T00:00:01.000Z", "type": "turn_context", "payload": {"model": "gpt-5-codex", "cwd": "/home/coder"}},
        {"timestamp": "2026-09-28T00:00:02.000Z", "type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": f"Codex с диска {MARK}"}]}},
        {"timestamp": "2026-09-28T00:00:03.000Z", "type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Ответ Codex из rollout"}]}},
    ]) + "\n"
    st_w, _ = raw(f"/agents/api/core/projects/{P}/fs/mkdir", "POST", json.dumps({"path": "/home/coder/.codex/sessions/2026/09/28"}).encode())
    st_w, body_w = raw(f"/agents/api/core/projects/{P}/fs/file?path={ROLL}", "PUT", rollout.encode(), "application/octet-stream"); ok("disk: codex rollout written into the workspace", st_w == 200, f"{st_w} {body_w[:100]}")
    disk = codex = None
    for _ in range(30):
        time.sleep(2); cur = [x for x in sessions() if x["id"] not in before_ids]
        disk = next((x for x in cur if x.get("provider_session_id") == DISK), None); codex = next((x for x in cur if x.get("provider_session_id") == CODEX), None)
        if disk and codex: break
    ok("disk: claude transcript adopted as a session with the prompt as title", bool(disk) and disk["imported"] and disk["agent"] == "claude" and disk["title"].startswith("Reply with exactly") and not disk["alive"], json.dumps(disk, ensure_ascii=False)[:200] if disk else "not found")
    ok("disk: codex rollout adopted with title, model and cwd", bool(codex) and codex["imported"] and codex["agent"] == "codex" and codex["title"] == f"Codex с диска {MARK}" and codex["model"] == "gpt-5-codex" and codex["cwd"] == "/home/coder", json.dumps(codex, ensure_ascii=False)[:200] if codex else "not found")
    evs = get(f"/agents/api/core/projects/{P}/sessions/{disk['id']}/events?since=0&limit=100")[1] if disk else []
    kinds = [e["kind"] for e in evs]
    ok("disk: history imported into the agentd journal", "user" in kinds and "assistant" in kinds and any(e["kind"] == "assistant" and f"disk-{MARK}" in json.dumps(e, ensure_ascii=False) for e in evs), str(kinds)[:160])
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1)
    # the tree shows 8 rows per workspace (TREE_LIMIT), conversations from disk with old activity go behind "N more" — expand
    more = pg.locator(f".panel .ws:has(.wname:text-is('{P}')) .sess-more")
    if more.count() and "ещё" in more.inner_text(): more.click(); time.sleep(0.5)
    try: pg.wait_for_selector(f".panel .sess[data-id='{disk['id']}']", timeout=20000); pg.wait_for_selector(f".panel .sess[data-id='{codex['id']}']", timeout=20000)
    except Exception: pass
    ok("disk: adopted sessions appear in the tree", pg.locator(f".panel .sess[data-id='{disk['id']}']").count() == 1 and pg.locator(f".panel .sess[data-id='{codex['id']}']").count() == 1)
    # scrolling on a long picked-up conversation: opens right at the end, the history in a window, reading upwards is not
    # disturbed by new events
    tree_expand(pg, P); t_open = time.time(); pg.locator(f".panel .sess:has-text('{KNOWN_SESSION}')").first.click(); pg.wait_for_selector(".thread .chat .log .m", timeout=20000)
    for _ in range(80):
        if pg.locator(".thread .chat .log .m").count() >= 40 and pg.locator(".thread .chat .log").evaluate("l => l.scrollHeight - l.scrollTop - l.clientHeight < 60"): break
        time.sleep(0.25)
    t_open = time.time() - t_open; time.sleep(1)
    ok("scroll: long conversation is fully rendered and at the end within seconds, not tens", t_open < 6, f"{t_open:.1f} s")
    at_end = pg.locator(".thread .chat .log").evaluate("l => l.scrollHeight - l.scrollTop - l.clientHeight < 60"); has_more = pg.locator(".thread .chat .log .earlier").count() == 1
    pg.locator(".thread .chat .log").evaluate("l => { l.scrollTop = 0; }"); time.sleep(1.2); stays_top = pg.locator(".thread .chat .log").evaluate("l => l.scrollTop < 40")
    ok("scroll: long conversation opens at the end, older events behind 'show more', reading up is not yanked down", at_end and has_more and stays_top, f"end {at_end} more {has_more} top {stays_top}")
    # "N earlier" — a page of 300 events (§4.3.16): after the click there are more messages; the button stays while the journal has older pages
    n_before = pg.locator(".thread .chat .log .m").count(); pg.locator(".thread .chat .log .earlier").click(); time.sleep(2); n_after = pg.locator(".thread .chat .log .m").count()
    ok("scroll: 'show more' reveals earlier history", n_after > n_before, f"{n_before} → {n_after}, more={pg.locator('.thread .chat .log .earlier').count()}")
    pg.locator(f".panel .sess[data-id='{disk['id']}']").click(); pg.wait_for_selector(".thread .chat", timeout=20000); time.sleep(2)
    ok("disk: chat shows the imported conversation", pg.locator(f".thread .chat .m.agent:has-text('disk-{MARK}')").count() >= 1 and pg.locator(".thread .chat .m.user").count() >= 1)
    ok("disk: inspector marks the source", pg.locator(".inspector:has-text('беседа из workspace')").count() == 1)
    pg.screenshot(path="shots/ap2c-disk.png")
    pg.locator(".thread .chat textarea").fill(f"Reply with exactly this and nothing else: cont-{MARK}"); pg.locator(".thread .chat button.send").click()
    pg.wait_for_selector(f".thread .chat .m.agent:has-text('cont-{MARK}')", timeout=180000)
    after = get(f"/agents/api/core/projects/{P}/sessions/{disk['id']}")[1] or {}
    ok("disk: continued from chat on the same provider session", after.get("info", {}).get("provider_session_id") == DISK and after["info"]["usage"]["turns"] >= 1)
    for sid2 in (disk["id"], codex["id"]):
        for _ in range(3):
            st_d, _ = raw(f"/agents/api/core/projects/{P}/sessions/{sid2}", "DELETE")
            if st_d == 204: break
            time.sleep(3)
    left = {x["id"] for x in sessions()}
    st_f, _ = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.claude/projects/-home-coder/{DISK}.jsonl"); st_r, _ = raw(f"/agents/api/core/projects/{P}/fs/file?path={ROLL}")
    ok("disk: deleting removes the record and the provider file, nothing comes back", disk["id"] not in left and codex["id"] not in left and st_f == 404 and st_r == 404, f"claude file {st_f}, rollout {st_r}")
    time.sleep(16); ok("disk: no re-adoption after the next scan", not any(x.get("provider_session_id") in (DISK, CODEX) for x in sessions()))
    # Codex adapter: its own models, efforts (without max), approval modes and only local commands — without model
    # turns
    cx = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "codex", "title": "codex adapter"}).encode())[1])
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(3); pg.locator(f".panel .sess[data-id='{cx['id']}']").click(); pg.wait_for_selector(".thread .chat .tools", timeout=20000); time.sleep(2)
    ctools = pg.locator(".thread .chat .tools"); ctools.locator(".ctl.model").click(); pg.wait_for_selector(".menu", timeout=5000); cm = [x.split("\n")[0].strip() for x in pg.locator(".menu .mi").all_inner_texts()]; pg.keyboard.press("Escape")
    # through the gate the route catalog is everything the proxy returns in the OpenAI format (since 2026-10 Claude models too) —
    # Sonnet in the list is acceptable
    ok("codex: model picker like Codex's own — Default (recommended set) then models from model/list by display name", cm[0] == "Default" and any(x.startswith("GPT-6") for x in cm) and len(cm) >= 5, str(cm)[:200])
    composer_modes(pg); pg.wait_for_selector(".thread .chat .modes", timeout=5000)
    cmo = [x.split("\n")[0].strip() for x in pg.locator(".thread .chat .modes .mode").all_inner_texts()]
    ok("codex: approval modes on-request / untrusted / never, no effort rows inside", len(cmo) == 3 and not any("План" in x for x in cmo) and pg.locator(".thread .chat .modes .slider").count() == 0, str(cmo)[:160]); pg.keyboard.press("Escape")
    composer_option(pg, "reasoning effort"); pg.wait_for_selector(".thread .chat .effpop", timeout=5000)
    ce = [x.strip() for x in pg.locator(".thread .chat .effpop .ep-step").all_inner_texts()]
    ok("codex: effort popover like Codex's own — gradient slider, levels incl. ultra, model name", ce == ["low", "medium", "high", "xhigh", "max", "ultra"] and pg.locator(".thread .chat .effpop input.range").count() == 1 and len(pg.locator(".thread .chat .effpop .ep-cur .muted").inner_text().strip()) > 3, str(ce) + " " + pg.locator(".thread .chat .effpop .ep-cur").inner_text().replace("\n", " "))
    pg.locator(".thread .chat .effpop .ep-step:has-text('ultra')").click(); time.sleep(1.5)
    ok("codex: choosing ultra applies and shows in the effort control", (get(f"/agents/api/core/projects/{P}/sessions/{cx['id']}")[1] or {}).get("info", {}).get("effort") == "ultra" and "ultra" in ctools.locator(".ctl.effort").inner_text())
    ok("codex: Fast tier from the model catalog", pg.locator(".thread .chat .effpop .ep-tier:has-text('Fast')").count() == 1)
    pg.locator(".thread .chat .effpop .ep-tier .switch").click(); time.sleep(1.5)
    ok("codex: fast tier toggles service tier (priority) for the next turn", (get(f"/agents/api/core/projects/{P}/sessions/{cx['id']}")[1] or {}).get("info", {}).get("service_tier") == "priority" and pg.locator(".thread .chat .effpop .switch.on").count() == 1)
    pg.locator(".thread .chat .effpop .ep-tier .switch").click(); time.sleep(1); pg.keyboard.press("Escape")
    cta = pg.locator(".thread .chat textarea"); cta.click(); cta.fill("/"); pg.wait_for_selector(".thread .chat .popup", timeout=5000); cc = [x.strip() for x in pg.locator(".thread .chat .popup .pi .pn").all_inner_texts()]
    ok("codex: slash popup has only local commands", set(cc) == {"/model", "/effort", "/mode"}, str(cc)); cta.fill("")
    cta.fill("см @net"); time.sleep(2.5); cf = [x.strip() for x in pg.locator(".thread .chat .popup .pi .pn").all_inner_texts()]
    ok("codex: @ popup via agentd file search", any(KNOWN_WORD in x for x in cf), str(cf)[:120]); pg.keyboard.press("Escape"); cta.fill("")
    composer_option(pg, "reasoning effort"); pg.wait_for_selector(".thread .chat .effpop", timeout=5000); pg.locator(".thread .chat .effpop .ep-step:has-text('low')").click(); time.sleep(1.5); pg.keyboard.press("Escape")
    ok("codex: effort stored for the next turn", (get(f"/agents/api/core/projects/{P}/sessions/{cx['id']}")[1] or {}).get("info", {}).get("effort") == "low")
    # a live Codex turn through the gate (the OpenAI-family route there is unstable): wait for an answer
    # or an error up to 150 s
    cta.fill(f"Reply with exactly this and nothing else: cx-{MARK}. Do not run any commands."); pg.locator(".thread .chat button.send").click(); verdict = None
    for _ in range(150):
        time.sleep(1); evs = get(f"/agents/api/core/projects/{P}/sessions/{cx['id']}/events?since=0&limit=300")[1] or []
        err = next((e for e in evs if e["kind"] == "error" or (e["kind"] == "system" and e.get("subtype") == "provider_error") or (e["kind"] == "result" and e.get("is_error"))), None)
        if err: verdict = ("error", json.dumps(err, ensure_ascii=False)[:220]); break
        if any(e["kind"] == "assistant" and not e.get("local") and any(b.get("type") == "text" and "cx-" in (b.get("text") or "") for b in (e.get("content") or [])) for e in evs): verdict = ("ok", ""); break
    if verdict is None or (verdict[0] == "error" and ("503" in verdict[1] or "auth_unavailable" in verdict[1] or "502" in verdict[1] or "429" in verdict[1])):
        print("SKIP  codex: live turn — gate upstream unavailable or silent for 150 s, not our code:", (verdict[1][:160] if verdict else "timeout"), flush=True)
    else:
        ok("codex: live turn with effort=low completes through the gate", verdict is not None and verdict[0] == "ok", str(verdict)[:220])
    for _ in range(3):
        if raw(f"/agents/api/core/projects/{P}/sessions/{cx['id']}", "DELETE")[0] == 204: break
        time.sleep(3)
    # model prices (docs/agents-ux.md §9.6): the button on the "Models" screen and in Settings → Routes, synchronization with the
    # public source
    pg.locator(".rail .nav[data-tip='Модели']").click(); pg.wait_for_selector(".kpis", timeout=20000); time.sleep(0.8)
    ok("prices: the Models screen carries the sync button next to the list title", pg.locator(".h-sec .prices-sync").count() == 1)
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000)
    pg.locator(".settings .subnav button:has-text('Маршруты')").click(); pg.wait_for_selector(".prices-card .prices-sync", timeout=20000); time.sleep(0.4)
    before_rows = pg.locator(".tbl.prices tbody tr").count(); pg.locator(".prices-card .prices-sync").click()
    try: pg.wait_for_selector(".prices-card .prices-sync:enabled", timeout=60000)
    except Exception: pass
    time.sleep(0.8)
    ps = (get("/agents/api/settings")[1] or {})
    pr = ps.get("prices", {}); sync = ps.get("prices_sync") or {}
    ok("prices: sync pulls current prices — Opus 5.5 $4/$20, Sonnet 5.5 and Haiku 5.5 present, Fable 5.1 cache read $0.25, GPT-6 Astra $10/$50", pr.get("claude-opus-5-5", {}).get("input") == 4.0 and pr.get("claude-opus-5-5", {}).get("output") == 20.0 and "claude-sonnet-5-5" in pr and "claude-haiku-5-5" in pr and pr.get("claude-fable-5-1", {}).get("cache_read") == 0.25 and pr.get("gpt-6-astra", {}).get("input") == 10.0 and pr.get("gpt-6-astra", {}).get("source") == "openrouter", f"opus={pr.get('claude-opus-5-5')} sync={ {k: v for k, v in sync.items() if k != 'missing'} }")
    ok("prices: the table shows the synced rows and the source line with the sync time", pg.locator(".tbl.prices tbody tr").count() >= max(before_rows, 10) and "синхронизировано" in pg.locator(".prices-synced").inner_text() and "openrouter" in pg.locator(".prices-synced").inner_text(), pg.locator(".prices-synced").inner_text()[:120])
    ok("prices: the toast reports what changed", pg.locator(".toasts .toast").filter(has_text="Цены синхронизированы").count() >= 1, pg.locator(".toasts .toast").all_inner_texts()[:2].__str__()[:160])
    pg.screenshot(path="shots/ap16-prices.png")
    # dev server preview (docs/agents-ux.md §4.9 step 1 and §4.10 sidebar): off by default; enabling in Settings → Browser gives a tab in the sidebar on desktop; the port is published through
    # the edge
    raw("/agents/api/settings", "PUT", json.dumps({"browser": {P: {"preview": False, "port": 3000, "agent": False}}}).encode()); raw(f"/agents/api/core/projects/{P}/browser", "POST", json.dumps({"enabled": False}).encode())
    for r_ in (get(f"/agents/api/core/projects/{P}/previews")[1] or {}).get("previews", []): raw(f"/agents/api/core/projects/{P}/previews/{r_['port']}", "DELETE")
    pg.evaluate("() => localStorage.removeItem('agents.side')")
    pg.reload(wait_until="load"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(2)   # the browser flags are read at app start
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6)
    if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.4)
    pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); time.sleep(0.6)
    if not pg.locator(".app.insp").count(): pg.locator(".top button[data-tip='Инспектор'], .top button[aria-label='Инспектор']").first.click(); time.sleep(0.5)
    w0 = pg.locator(".inspector").evaluate("e => e.getBoundingClientRect().width")
    ok("sidebar: the right panel opens as a sidebar with the «Инспектор» tab and no «Превью» tab while the preview is off; the session pane has no «Превью» tab on the desktop", pg.locator(".inspector .side-tabs [role=tab]").count() == 1 and pg.locator(".inspector .side-tab-preview").count() == 0 and pg.locator(".thread .pane .tabs .tab").filter(has_text="Превью").count() == 0 and (get("/agents/api/settings")[1] or {}).get("browser", {}).get(P, {}).get("preview") is not True, f"tabs={pg.locator('.inspector .side-tabs [role=tab]').count()} width={w0:.0f}")
    ok("sidebar: the default width is the old inspector width (320 px, 340 on wide screens) and the «уже» button is disabled", w0 in (320.0, 340.0) and pg.locator(".inspector .side-narrower").is_disabled(), f"width={w0:.0f}")
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Браузер')").click(); pg.wait_for_selector(f".browser-card tr[data-project='{P}'] .browser-preview", timeout=20000); time.sleep(0.4)
    pg.locator(f".browser-card tr[data-project='{P}'] .browser-preview").click(); time.sleep(1.0)
    bs = (get("/agents/api/settings")[1] or {}).get("browser", {}).get(P, {})
    ok("preview: the Browser tab in Settings switches the preview on per workspace (port 3000 by default)", bs.get("preview") is True and pg.locator(f".browser-card tr[data-project='{P}'] .browser-preview.on").count() == 1, str(bs))
    pg.screenshot(path="shots/ap17-browser-settings.png")
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); time.sleep(0.6)
    if not pg.locator(".app.insp").count(): pg.locator(".top button[data-tip='Инспектор'], .top button[aria-label='Инспектор']").first.click(); time.sleep(0.5)
    ptab = pg.locator(".inspector .side-tab-preview")
    ok("sidebar: the «Превью» tab appears in the sidebar once enabled (still none in the session pane on the desktop)", ptab.count() == 1 and pg.locator(".thread .pane .tabs .tab").filter(has_text="Превью").count() == 0)
    ptab.first.click()
    try: pg.wait_for_selector(".inspector .preview iframe", timeout=30000)
    except Exception: pass
    time.sleep(0.8)
    prv = (get(f"/agents/api/core/projects/{P}/previews")[1] or {}).get("previews", [])
    src = pg.locator(".inspector .preview iframe").first.get_attribute("src") if pg.locator(".inspector .preview iframe").count() else None
    ok("preview: opening the sidebar tab publishes port 3000 through an edge port from the app range and shows it in a frame beside the conversation", len(prv) == 1 and prv[0]["port"] == 3000 and 8450 <= prv[0]["edge_port"] <= 8499 and bool(src) and src.startswith(f"https://{HOST}:{prv[0]['edge_port']}/") and pg.locator(".thread .chat .chat-composer").count() == 1, f"previews={prv} src={src}")
    if prv:
        est, ebody = raw_url(f"https://{HOST}:{prv[0]['edge_port']}/")
        ok("preview: the edge route answers — the dev server is not running, so the AgentVerse error page explains it (not a bare 502)", est in (502, 503, 504) and b"\xd0\xbf\xd1\x80\xd0\xb5\xd0\xb2\xd1\x8c\xd1\x8e" in ebody, f"status={est} len={len(ebody)}")
    # a real dev server inside the workspace: a terminal with python http.server on :3000 → the edge serves it, the iframe shows
    # the page
    pst, pbody = raw(f"/agents/api/core/projects/{P}/ptys", "POST", json.dumps({"cmd": ["sh", "-lc", f"mkdir -p /tmp/pv-{MARK} && echo preview-{MARK} > /tmp/pv-{MARK}/index.html && cd /tmp/pv-{MARK} && python3 -m http.server 3000"], "title": f"preview server {MARK}"}).encode())
    pv_pty = (json.loads(pbody or b"{}") if pst in (200, 201) else {}).get("id")
    served = False; est2 = None
    if prv and pv_pty:
        for _ in range(30):
            est2, ebody2 = raw_url(f"https://{HOST}:{prv[0]['edge_port']}/")
            if est2 == 200 and f"preview-{MARK}".encode() in ebody2: served = True; break
            time.sleep(1)
    ok("preview: a dev server started in the workspace is served through the edge route", served, f"pty={pv_pty} status={est2}")
    pg.locator(".inspector .preview button[aria-label='обновить']").first.click(); time.sleep(2.0)
    fr = next((f for f in pg.frames if prv and f.url.startswith(f"https://{HOST}:{prv[0]['edge_port']}/")), None)
    ftext = ""
    if fr:
        try: ftext = fr.locator("body").inner_text(timeout=10000)
        except Exception: ftext = ""
    ok("preview: the frame shows the dev server page", f"preview-{MARK}" in ftext, repr(ftext[:80]))
    # sidebar width: only by buttons — "wider" +180, remembered after a reload, "narrower" brings back the
    # default
    pg.locator(".inspector .side-wider").click(); time.sleep(0.4)
    w1 = pg.locator(".inspector").evaluate("e => e.getBoundingClientRect().width")
    pg.reload(wait_until="load"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(2)
    if not pg.locator(".app.insp").count(): pg.locator(".top button[data-tip='Инспектор'], .top button[aria-label='Инспектор']").first.click(); time.sleep(0.5)
    w2 = pg.locator(".inspector").evaluate("e => e.getBoundingClientRect().width") if pg.locator(".inspector").count() else 0
    ok("sidebar: «шире» adds 180 px and the width survives a reload; nothing changes the width on its own", abs(w1 - (w0 + 180)) < 2 and abs(w2 - w1) < 2, f"w0={w0:.0f} w1={w1:.0f} after reload={w2:.0f}")
    pg.locator(".inspector .side-narrower").click(); time.sleep(0.4)
    w3 = pg.locator(".inspector").evaluate("e => e.getBoundingClientRect().width")
    ok("sidebar: «уже» returns to the default width", abs(w3 - w0) < 2, f"w3={w3:.0f}")
    ok("sidebar: the preview tab is remembered across the reload", pg.locator(".inspector .preview").count() == 1)
    pg.locator(".inspector .preview .pv-sizes button:has-text('телефон')").click(); time.sleep(0.4)
    fw = pg.locator(".inspector .preview iframe").first.evaluate("e => e.getBoundingClientRect().width") if pg.locator(".inspector .preview iframe").count() else 0
    ok("preview: the «телефон» preset narrows the frame to the sidebar width or 390 px, whichever is smaller", 0 < fw <= 390.5, f"width={fw}")
    pg.screenshot(path="shots/ap17-preview.png")
    # turning off from the "⋯" menu of the Thread header: the tab disappears, the route is removed, the sidebar
    # returns to the inspector
    pg.locator(".thread .t-head .thread-menu").click(); pg.wait_for_selector(".menu .mi", timeout=5000)
    mi = pg.locator(".menu .mi").filter(has_text="Превью dev-сервера")
    ok("preview: the Thread «⋯» menu carries the same switch, checked while on", mi.count() == 1 and mi.first.get_attribute("aria-checked") == "true")
    mi.first.click(); time.sleep(1.2)
    prv2 = (get(f"/agents/api/core/projects/{P}/previews")[1] or {}).get("previews", [])
    ok("preview: switching off from the menu removes the sidebar tab and the edge route, the sidebar falls back to the inspector", pg.locator(".inspector .side-tab-preview").count() == 0 and pg.locator(".inspector .preview").count() == 0 and not prv2 and (get("/agents/api/settings")[1] or {}).get("browser", {}).get(P, {}).get("preview") is False, f"previews={prv2}")
    if pv_pty: raw(f"/agents/api/core/projects/{P}/ptys/{pv_pty}", "DELETE")
    pg.locator(".inspector .side-head button[aria-label='Закрыть']").click(); time.sleep(0.3)
    # agent browser (docs/agents-ux.md §4.9 step 2): off by default; enabling in Settings → Browser brings up Chromium in the workspace,
    # registers the browser MCP server with the engines, the sidebar shows the screencast; clicks and input reach the page; disabling removes
    # everything
    raw("/agents/api/settings", "PUT", json.dumps({"browser": {P: {"preview": False, "port": 3000, "agent": False}}}).encode()); raw(f"/agents/api/core/projects/{P}/browser", "POST", json.dumps({"enabled": False}).encode())
    bst = get(f"/agents/api/core/projects/{P}/browser")[1] or {}
    ok("abrowser: off by default — Chromium not running, no MCP server, no «Браузер» tab in the sidebar", bst.get("running") is False and bst.get("mcp") is False and pg.locator(".inspector .side-tab-browser").count() == 0, str({k: bst.get(k) for k in ('installed', 'running', 'mcp')}))
    if bst.get("installed"):
        pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Браузер')").click(); pg.wait_for_selector(f".browser-card tr[data-project='{P}'] .browser-agent", timeout=20000); time.sleep(0.4)
        pg.locator(f".browser-card tr[data-project='{P}'] .browser-agent").click()
        try: pg.wait_for_selector(f".browser-card tr[data-project='{P}'] .browser-agent.on", timeout=90000)
        except Exception: pass
        time.sleep(0.8)
        bst = get(f"/agents/api/core/projects/{P}/browser")[1] or {}; mcp_l = (get(f"/agents/api/core/projects/{P}/mcp")[1] or {}).get("servers", [])
        ok("abrowser: the switch starts headless Chromium in the workspace and registers the browser MCP server (Playwright MCP on our CDP) for the engines", bst.get("running") is True and bst.get("mcp") is True and any(s_.get("name") == "browser" and "--cdp-endpoint" in s_.get("args", []) for s_ in mcp_l), f"status={ {k: bst.get(k) for k in ('running', 'mcp', 'own')} } mcp={[s_.get('name') for s_ in mcp_l]}")
        pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); time.sleep(0.6)
        if not pg.locator(".app.insp").count(): pg.locator(".top button[data-tip='Инспектор'], .top button[aria-label='Инспектор']").first.click(); time.sleep(0.5)
        pg.locator(".inspector .side-tab-browser").click()
        got_frame = False
        for _ in range(60):
            if pg.locator(".inspector .ab-frame").count() and int(pg.locator(".inspector .ab-frame").get_attribute("data-frames") or 0) > 0: got_frame = True; break
            time.sleep(0.5)
        ok("abrowser: the sidebar «Браузер» tab streams the shared browser tab (first JPEG frame arrived)", got_frame and (pg.locator(".inspector .ab-frame").get_attribute("src") or "").startswith("data:image/jpeg"), f"frames={pg.locator('.inspector .ab-frame').get_attribute('data-frames') if pg.locator('.inspector .ab-frame').count() else None}")
        page_html = f"data:text/html,<title>e2e-{MARK}</title><button style='position:fixed;left:0;top:0;width:200px;height:100px' onclick=\"document.title='clicked-{MARK}'\">b</button><input id=i oninput='document.title=this.value' style='position:fixed;left:0;top:200px;width:300px;height:40px;font-size:20px'>"
        pg.locator(".inspector .ab-addr").fill(page_html); pg.locator(".inspector .ab-addr").press("Enter")
        def title_is(t_, secs=12):
            for _ in range(int(secs * 2)):
                if any(t_ == o for o in pg.locator(".inspector .ab-tabs option").all_inner_texts()): return True
                time.sleep(0.5)
            return False
        ok("abrowser: the address bar navigates the shared tab (title in the tabs list)", title_is(f"e2e-{MARK}"))
        sb = pg.locator(".inspector .ab-surface").bounding_box()
        pg.mouse.click(sb["x"] + 20, sb["y"] + 20)
        ok("abrowser: a click on the frame reaches the page (button handler changed the title)", title_is(f"clicked-{MARK}"))
        pg.mouse.click(sb["x"] + 20, sb["y"] + 220); time.sleep(0.4); pg.keyboard.type(f"typed-{MARK}")
        ok("abrowser: typing on the focused frame reaches the page input", title_is(f"typed-{MARK}"))
        pg.screenshot(path="shots/ap18-agent-browser.png")
        # Design Mode (§4.9 step 3): design mode → a click on a page button puts its HTML, styles and crop into the
        # conversation composer
        ta_d = pg.locator(".thread .chat .composer textarea"); ta_d.fill("")
        n_img0 = pg.locator(".thread .chat .thumbs .thumb").count()
        dm_log = []; seen_toasts = []
        pg.on("websocket", lambda w: w.on("framereceived", lambda f: dm_log.append((f if isinstance(f, str) else str(f))[:160])) if "/browser/ws" in w.url else None)
        pg.locator(".inspector .ab-design").click(); time.sleep(0.8)
        ok("design: the Design toggle arms the mode (pressed, crosshair surface, hint)", pg.locator(".inspector .ab-design[aria-pressed='true']").count() == 1 and pg.locator(".inspector .ab-view.design").count() == 1 and pg.locator(".inspector .ab-design-hint").count() == 1)
        sb = pg.locator(".inspector .ab-surface").bounding_box()   # the mode hint shifts the image — the coordinates anew
        pg.mouse.move(sb["x"] + 40, sb["y"] + 40); time.sleep(0.3); pg.mouse.click(sb["x"] + 40, sb["y"] + 40)
        got_el = False
        for _ in range(40):
            for tx in pg.locator(".toasts .toast").all_inner_texts():
                if tx not in seen_toasts: seen_toasts.append(tx)
            if "<button" in ta_d.input_value(): got_el = True; break
            time.sleep(0.3)
        dtext = ta_d.input_value()
        ok("design: a click on the page button puts the element into the composer — selector line, html block, styles line", got_el and "```html" in dtext and "button" in dtext and "Стили:" in dtext and ("width" in dtext or "height" in dtext), f"{dtext[:120]!r} toasts={seen_toasts[:3]} ws={[m for m in dm_log if 'element' in m or 'error' in m][:3]}")
        n_img1 = pg.locator(".thread .chat .thumbs .thumb").count()
        ok("design: the element's screenshot crop is attached to the draft as an image", n_img1 == n_img0 + 1, f"thumbs {n_img0} → {n_img1}")
        ok("design: the page itself did not receive the click (title unchanged)", not title_is("clicked-" + MARK + "-2", 1) and pg.locator(".inspector .ab-tabs option").all_inner_texts()[0] == f"typed-{MARK}")
        pg.screenshot(path="shots/ap18-design-mode.png")
        pg.locator(".inspector .ab-surface").focus(); pg.keyboard.press("Escape"); time.sleep(0.3)
        ok("design: Esc leaves the mode", pg.locator(".inspector .ab-design[aria-pressed='true']").count() == 0)
        ta_d.fill("")
        if pg.locator(".thread .chat .thumbs .thumb button").count(): pg.locator(".thread .chat .thumbs .thumb button").first.click(); time.sleep(0.3)
        pg.locator(".thread .t-head .thread-menu").click(); pg.wait_for_selector(".menu .mi", timeout=5000)
        mi = pg.locator(".menu .mi").filter(has_text="Браузер агента"); mi_checked = mi.count() == 1 and mi.first.get_attribute("aria-checked") == "true"
        mi.first.click(); time.sleep(2.0)
        bst = get(f"/agents/api/core/projects/{P}/browser")[1] or {}; mcp_l = (get(f"/agents/api/core/projects/{P}/mcp")[1] or {}).get("servers", [])
        ok("abrowser: the Thread «⋯» menu switch (checked while on) turns it off — Chromium stopped, MCP server removed, sidebar tab gone", mi_checked and bst.get("running") is False and not any(s_.get("name") == "browser" for s_ in mcp_l) and pg.locator(".inspector .side-tab-browser").count() == 0, f"status={ {k: bst.get(k) for k in ('running', 'mcp')} }")
    else: print("SKIP  abrowser: Chromium not installed in the workspace — install flow only", flush=True)

    # ---- agents, routines, runs (routines.rs, the "Agents" section)
    # ----
    def wait_run(rid, secs=150):
        r = None
        for _ in range(secs // 3):
            time.sleep(3); st_r, rr = get(f"/agents/api/runs?limit=50"); r = next((x for x in (rr or {}).get("runs", []) if x["id"] == rid), None)
            if r and r["status"] != "running": break
        return r
    st_a, ag = raw("/agents/api/agents", "POST", json.dumps({"name": f"e2e agent {MARK}", "brief": "Ты тестовый агент e2e. Отвечай коротко, инструменты не используй, если не просят.", "provider": "claude", "workspace": P, "mode": "auto", "remember": True}).encode()); ag = json.loads(ag or b"{}")
    ok("agents: agent created via the service", st_a == 200 and ag.get("id", "").startswith("ag_") and ag["mode"] == "auto", f"{st_a} {str(ag)[:100]}")
    st_r, rt = raw("/agents/api/routines", "POST", json.dumps({"name": f"e2e routine {MARK}", "agent": ag.get("id"), "prompt": f"Reply with exactly: routine-{MARK}. Then add a separate last line: ПАМЯТЬ: e2e memory {MARK}. Today is {{date}}. Do not run tools.", "trigger": {"kind": "schedule", "cron": "0 3 * * *"}, "budget": {"run_usd": 1, "day_usd": 5}, "notify": "never"}).encode()); rt = json.loads(rt or b"{}")
    st_l, rl = get("/agents/api/routines"); mine = next((x for x in (rl or {}).get("routines", []) if x["id"] == rt.get("id")), {})
    ok("agents: routine with webhook token, schedule text and next run", st_r == 200 and rt.get("token", "").startswith("rt_") and mine.get("schedule_text") == "каждый день в 03:00" and (mine.get("next_at") or 0) > time.time() and (rl or {}).get("scheduler") is True, f"{st_r} {str(mine)[:160]}")
    st_bad, bad = raw("/agents/api/routines", "POST", json.dumps({"name": "bad", "agent": ag.get("id"), "prompt": "x", "trigger": {"kind": "schedule", "cron": "0 25 * * *"}}).encode())
    ok("agents: invalid cron is rejected with a clear message", st_bad >= 400 and "расписание" in (bad or b"").decode("utf-8", "ignore"), f"{st_bad} {(bad or b'')[:100]}")
    st_run, rr = raw(f"/agents/api/routines/{rt.get('id')}/run", "POST", b"{}"); rr = json.loads(rr or b"{}").get("run", {})
    ok("agents: run now starts a session in the agent's workspace", st_run == 200 and rr.get("status") == "running" and bool(rr.get("session")) and rr.get("project") == P and rr.get("trigger") == "manual", f"{st_run} {str(rr)[:160]}")
    r1 = wait_run(rr.get("id")) or {}
    s_run = get(f"/agents/api/core/projects/{P}/sessions/{rr.get('session')}")[1] or {}
    ok("agents: run finishes when the turn ends — status done, cost, the reply as summary, session stopped but kept, titled after the routine", r1.get("status") == "done" and f"routine-{MARK}" in (r1.get("summary") or "") and (r1.get("cost_usd") or 0) > 0 and s_run.get("info", {}).get("title", "").startswith(f"e2e routine {MARK}") and s_run["info"]["permission_mode"] == "auto" and s_run["info"]["alive"] is False, f"{str(r1)[:200]}")
    # first events: status/spawn/queued/init/unqueued — the routine message leaves the queue after init (the resume+prompt race is closed by
    # the queue)
    ev1 = get(f"/agents/api/core/projects/{P}/sessions/{rr.get('session')}/events?since=0&limit=40")[1] or []
    first_user = next((e for e in ev1 if e["kind"] == "user"), {}); utext = " ".join(b.get("text", "") for b in (first_user.get("content") or []))
    ok("agents: the task carries the brief, memory section and today's date", "# Агент «e2e agent" in utext and "# Память" in utext and time.strftime("%Y-%m-%d") in utext and "ПАМЯТЬ:" in utext, utext[:160])
    ag2 = next((x for x in (get("/agents/api/agents")[1] or {}).get("agents", []) if x["id"] == ag.get("id")), {})
    ok("agents: memory picked up the ПАМЯТЬ line and the run summary", f"e2e memory {MARK}" in ag2.get("memory", "") and "итог" in ag2.get("memory", "") and (ag2.get("last_run") or {}).get("status") == "done", ag2.get("memory", "")[:160])
    st_u, _ = raw(f"/agents/api/routines/{rt.get('id')}", "PUT", json.dumps({"trigger": {"kind": "webhook", "cron": "", "event": "", "contains": ""}, "prompt": f"Reply with exactly: hook-{MARK} and the value of field x from this payload: {{payload}}. Do not run tools."}).encode())
    st_h, hk = raw(f"/agents/api/hooks/{rt.get('token')}", "POST", json.dumps({"x": f"payload-{MARK}"}).encode()); hk = json.loads(hk or b"{}")
    ok("agents: webhook with the routine token starts a run", st_u == 200 and st_h == 202 and hk.get("status") == "running", f"{st_u} {st_h} {str(hk)[:100]}")
    ok("agents: unknown webhook token is 404", raw("/agents/api/hooks/rt_nope", "POST", b"{}")[0] == 404)
    r2 = wait_run(hk.get("run")) or {}
    ok("agents: webhook run finished and the payload reached the task", r2.get("status") == "done" and f"payload-{MARK}" in (r2.get("summary") or "") and r2.get("trigger") == "webhook", str(r2)[:200])
    # scheduler: the "every minute" cron must fire at the nearest minute (15 s tick); the run is disabled and stopped right away
    st_c, _ = raw(f"/agents/api/routines/{rt.get('id')}", "PUT", json.dumps({"trigger": {"kind": "schedule", "cron": "* * * * *", "event": "", "contains": ""}}).encode())
    r3 = None
    for _ in range(32):
        time.sleep(3); rr3 = get(f"/agents/api/runs?routine={rt.get('id')}&limit=10")[1] or {}; r3 = next((x for x in rr3.get("runs", []) if x["trigger"] == "schedule"), None)
        if r3: break
    raw(f"/agents/api/routines/{rt.get('id')}", "PUT", json.dumps({"enabled": False}).encode())
    ok("agents: the scheduler fires a cron routine on the next minute", st_c == 200 and bool(r3) and r3.get("status") in ("running", "done") and bool(r3.get("session")), str(r3)[:160] if r3 else "no scheduled run in 96 s")
    if r3 and r3.get("session"):
        for _ in range(3):
            if raw(f"/agents/api/core/projects/{P}/sessions/{r3['session']}", "DELETE")[0] in (204, 404): break
            time.sleep(2)
        r3f = wait_run(r3["id"], 45) or {}
        ok("agents: a stopped scheduled run is closed by the watcher", r3f.get("status") in ("done", "failed"), str(r3f.get("status")))
    pg.locator(".rail .nav[data-tip='Агенты']").click(); pg.wait_for_selector(".rt", timeout=20000); time.sleep(1.5)
    pg.locator(f".rt-item[data-agent='{ag.get('id')}']").click(); time.sleep(0.6)
    ok("agents: section lists the agent and opens its card with brief and memory", pg.locator(f".rt-item[data-agent='{ag.get('id')}']").count() == 1 and pg.locator(".rt-form input").first.input_value() == f"e2e agent {MARK}" and f"e2e memory {MARK}" in pg.locator(".rt-form textarea.memory").input_value(), pg.locator(".rt-form input").first.input_value()); pg.screenshot(path="shots/ap9-agents.png")
    pg.locator(".top .seg button:has-text('Рутины')").click(); time.sleep(0.6); pg.locator(f".rt-item[data-routine='{rt.get('id')}']").click(); time.sleep(0.6)
    cron_shown = pg.locator(".rt-form input.mono").input_value() if pg.locator(".rt-form input.mono").count() else ""
    ok("agents: routine card shows the schedule (after the cron check) and the run history", cron_shown == "* * * * *" and pg.locator(".rt-form .runs .tr").count() >= 3 and pg.locator(".rt-form .switch").count() == 1 and not pg.locator(".rt-form .switch").evaluate("e => e.classList.contains('on')"), f"cron={cron_shown} rows={pg.locator('.rt-form .runs .tr').count()}"); pg.screenshot(path="shots/ap9b-routine.png")
    pg.locator(".rt-form input[name='trig'][value='webhook']").check(); time.sleep(0.4)
    hook_shown = pg.locator(".rt-form .hook").inner_text() if pg.locator(".rt-form .hook").count() else ""
    ok("agents: switching the trigger to webhook shows the address with the routine token", rt.get("token", "") in hook_shown, hook_shown[-40:])
    # the peek at the address must not leave an unsaved draft: a dirty draft outlives API changes (by design), so put the radio back
    pg.locator(".rt-form input[name='trig'][value='schedule']").check(); time.sleep(0.4)
    ok("agents: putting the trigger back clears the unsaved-draft notice", pg.locator(".rt-form .draft-notice, .rt .draft-notice").count() == 0, str(pg.locator(".draft-notice").count()))
    pg.locator(".top .seg button:has-text('Запуски')").click(); time.sleep(0.6)
    ok("agents: runs tab lists both runs with summaries", pg.locator(f".runs .tr[data-run='{rr.get('id')}']").count() == 1 and pg.locator(f".runs .tr[data-run='{hk.get('run')}']").count() == 1 and pg.locator(f".runs .tr[data-run='{hk.get('run')}'] .sum").inner_text().find("hook-") >= 0)
    pg.locator(f".runs .tr[data-run='{hk.get('run')}'] button:has-text('открыть сессию')").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); time.sleep(1)
    ok("agents: 'open session' jumps to the run's session in Thread", pg.locator(".thread .t-head h2").inner_text().startswith(f"e2e routine {MARK}") and pg.locator(".thread .chat .m.agent:has-text('hook-')").count() >= 1)
    # ---- agents v2 (slice 29): skills catalog, workplace, permissions, routine mode, mail, reflection, brief via AI ----
    st_sk, sk = get("/agents/api/skills"); names = [x["name"] for x in (sk or {}).get("skills", [])]
    ok("skills: catalog seeded with the starter skills", st_sk == 200 and {"save-tokens", "lean-code", "report"} <= set(names), str(names)[:120])
    SK = f"e2e-{MARK[4:]}"
    st_c, csk = raw("/agents/api/skills", "POST", json.dumps({"name": SK, "description": "тестовый навык e2e", "body": f"Когда просят маркер навыка, отвечай словом skill-{MARK}."}).encode()); csk = json.loads(csk or b"{}")
    ok("skills: owner skill created with source user", st_c == 200 and csk.get("source") == "user" and csk.get("name") == SK, f"{st_c} {str(csk)[:100]}")
    ok("skills: bad name is rejected, duplicate is 409", raw("/agents/api/skills", "POST", json.dumps({"name": "Bad Name", "body": "x"}).encode())[0] >= 400 and raw("/agents/api/skills", "POST", json.dumps({"name": SK, "body": "x"}).encode())[0] == 409)
    st_u2, _ = raw(f"/agents/api/agents/{ag.get('id')}", "PUT", json.dumps({"mode": "ask", "place": "/e2e-place/", "skills": [SK], "can_mail": True, "memory_budget": 100}).encode())
    ag3 = next((x for x in (get("/agents/api/agents")[1] or {}).get("agents", []) if x["id"] == ag.get("id")), {})
    ok("agents: place cleaned, skills and mail saved, memory budget clamped to 500", st_u2 == 200 and ag3.get("place") == "e2e-place" and ag3.get("skills") == [SK] and ag3.get("can_mail") is True and ag3.get("memory_budget") == 500, str({k: ag3.get(k) for k in ("place", "skills", "can_mail", "memory_budget")}))
    ok("agents: a place outside the workspace is rejected", raw(f"/agents/api/agents/{ag.get('id')}", "PUT", json.dumps({"place": "../x"}).encode())[0] >= 400)
    st_m, _ = raw(f"/agents/api/routines/{rt.get('id')}", "PUT", json.dumps({"mode": "auto", "enabled": True, "trigger": {"kind": "manual", "cron": "", "event": "", "contains": ""}, "prompt": f"Reply with exactly: place-{MARK}, then the marker word from your skill, then the output of `pwd` (run it with Bash). Nothing else."}).encode())
    st_r4, r4 = raw(f"/agents/api/routines/{rt.get('id')}/run", "POST", b"{}"); r4 = json.loads(r4 or b"{}").get("run", {})
    s4 = (get(f"/agents/api/core/projects/{P}/sessions/{r4.get('session')}")[1] or {}).get("info", {})
    ok("agents: routine mode overrides the agent's (auto over ask); the run starts in the place", st_m == 200 and st_r4 == 200 and s4.get("permission_mode") == "auto" and s4.get("cwd") == "/home/coder/e2e-place", f"{s4.get('permission_mode')} {s4.get('cwd')}")
    r4f = wait_run(r4.get("id")) or {}
    ev4 = get(f"/agents/api/core/projects/{P}/sessions/{r4.get('session')}/events?since=0&limit=40")[1] or []
    u4 = " ".join(b.get("text", "") for e in ev4 if e["kind"] == "user" for b in (e.get("content") or []))
    ok("agents: the task carries the skill text, the place rule and the ПОЧТА rule", "# Навыки" in u4 and f"## {SK}" in u4 and "`e2e-place`" in u4 and "ПОЧТА:" in u4, u4[:120])
    st_sf, sfile = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.claude/skills/{SK}/SKILL.md")
    ok("skills: Claude gets the skill as ~/.claude/skills/<name>/SKILL.md in the workspace", st_sf == 200 and f"name: {SK}".encode() in (sfile or b"") and f"skill-{MARK}".encode() in (sfile or b""), f"{st_sf} {(sfile or b'')[:80]}")
    ok("agents: the run answered from the place with the skill marker", r4f.get("status") == "done" and f"skill-{MARK}" in (r4f.get("summary") or "") and "e2e-place" in (r4f.get("summary") or ""), str(r4f.get("summary"))[:160])
    # mail: pause → 409; the owner's letter to a recipient; a "ПОЧТА:" chain from agent to agent and the answer into
    # the sender's memory
    st_b, agB = raw("/agents/api/agents", "POST", json.dumps({"name": f"e2e pen {MARK}", "brief": "Ты тестовый агент-адресат e2e. Отвечай одной строкой, инструменты не используй.", "provider": "claude", "workspace": P, "mode": "auto", "can_mail": True, "remember": False, "can_routine": False}).encode()); agB = json.loads(agB or b"{}")
    raw("/agents/api/settings", "PUT", json.dumps({"mail_paused": True}).encode()); st_pz, _ = raw(f"/agents/api/agents/{agB.get('id')}/mail", "POST", json.dumps({"text": "x"}).encode()); raw("/agents/api/settings", "PUT", json.dumps({"mail_paused": False}).encode())
    ok("mail: paused mail is refused with 409", st_b == 200 and agB.get("can_mail") is True and st_pz == 409, f"{st_b} {st_pz}")
    st_nr, nr = raw("/agents/api/routines", "POST", json.dumps({"name": "no", "agent": agB.get("id"), "prompt": "x", "trigger": {"kind": "manual"}}).encode())
    ok("agents: an agent without the routine power cannot be put on a routine", st_nr >= 400 and "запрещены рутины" in (nr or b"").decode("utf-8", "ignore"), f"{st_nr} {(nr or b'')[:100]}")
    st_ml, ml = raw(f"/agents/api/agents/{agB.get('id')}/mail", "POST", json.dumps({"text": f"Reply with exactly: mail-{MARK}"}).encode()); ml = json.loads(ml or b"{}").get("run", {})
    ok("mail: a message from the owner starts a mail run for the addressee", st_ml == 200 and ml.get("trigger") == "mail" and ml.get("status") == "running" and ml.get("from_agent") is None and ml.get("hops") == 0, f"{st_ml} {str(ml)[:140]}")
    mlf = wait_run(ml.get("id")) or {}
    ok("mail: the owner's mail run finished with the reply", mlf.get("status") == "done" and f"mail-{MARK}" in (mlf.get("summary") or ""), str(mlf.get("summary"))[:120])
    raw(f"/agents/api/routines/{rt.get('id')}", "PUT", json.dumps({"prompt": f"Reply with exactly: chain-{MARK}. Then add a separate last line exactly like this: ПОЧТА: e2e pen {MARK}: Reply with exactly: pong-{MARK}", "mode": "auto"}).encode())
    st_r5, r5 = raw(f"/agents/api/routines/{rt.get('id')}/run", "POST", b"{}"); r5 = json.loads(r5 or b"{}").get("run", {}); r5f = wait_run(r5.get("id")) or {}
    rB = None
    for _ in range(20):
        time.sleep(3); rB = next((x for x in (get("/agents/api/runs?limit=50")[1] or {}).get("runs", []) if x.get("agent") == agB.get("id") and x.get("reply_to") == r5.get("id")), None)
        if rB: break
    ok("mail: a «ПОЧТА:» line in the run result starts a mail run for the other agent (hop 1, reply_to the source run)", st_r5 == 200 and r5f.get("status") == "done" and bool(rB) and rB.get("trigger") == "mail" and rB.get("from_agent") == ag.get("id") and rB.get("hops") == 1, str(rB)[:160] if rB else str(r5f)[:160])
    rBf = wait_run((rB or {}).get("id", "none")) or {}
    agA = {}
    for _ in range(10):
        time.sleep(2); agA = next((x for x in (get("/agents/api/agents")[1] or {}).get("agents", []) if x["id"] == ag.get("id")), {})
        if f"pong-{MARK}" in agA.get("memory", ""): break
    ok("mail: the addressee's reply lands in the sender's memory", rBf.get("status") == "done" and f"pong-{MARK}" in (rBf.get("summary") or "") and f"pong-{MARK}" in agA.get("memory", "") and "на почту" in agA.get("memory", ""), agA.get("memory", "")[-160:])
    raw(f"/agents/api/agents/{agB.get('id')}", "PUT", json.dumps({"can_mail": False}).encode())
    ok("mail: an agent without mail cannot be written to", raw(f"/agents/api/agents/{agB.get('id')}/mail", "POST", json.dumps({"text": "x"}).encode())[0] >= 400)
    # reflection: after "Assign" the memory is rewritten by a cheap model (reflected_at), stays within the budget and
    # dated
    raw(f"/agents/api/agents/{ag.get('id')}", "PUT", json.dumps({"reflect": True, "memory_budget": 1500, "memory": "2026-01-01: проект использует npm test\n2026-01-02: проект использует npm test\n2026-01-03: проект использует npm test"}).encode())
    t_refl = time.time(); st_r6, r6 = raw(f"/agents/api/agents/{ag.get('id')}/run", "POST", json.dumps({"prompt": f"Reply with exactly: reflect-{MARK}. Also state that the project's tests are green today. Do not run tools."}).encode()); r6 = json.loads(r6 or b"{}").get("run", {}); r6f = wait_run(r6.get("id")) or {}
    agR = {}
    for _ in range(30):
        time.sleep(3); agR = next((x for x in (get("/agents/api/agents")[1] or {}).get("agents", []) if x["id"] == ag.get("id")), {})
        if (agR.get("reflected_at") or 0) >= int(t_refl): break
    mlines = [l for l in agR.get("memory", "").splitlines() if l.strip()]
    ok("agents: reflection rewrote the memory after the run — dated lines within the budget", r6f.get("status") == "done" and (agR.get("reflected_at") or 0) >= int(t_refl) and 0 < len(mlines) and all(re.match(r"^\d{4}-\d{2}-\d{2}:", l) for l in mlines) and len(agR.get("memory", "")) <= 1500, f"reflected_at={agR.get('reflected_at')} lines={len(mlines)} {agR.get('memory', '')[:120]}")
    st_d, dr = raw("/agents/api/agents/draft", "POST", json.dumps({"description": "нужен сотрудник, который каждое утро прогоняет тесты проекта, чинит мелкие падения и не трогает миграции", "workspace": P}).encode()); dr = json.loads(dr or b"{}")
    ok("agents: draft with AI returns a name and a brief", st_d == 200 and len(dr.get("name", "")) > 2 and len(dr.get("brief", "")) > 40 and dr.get("workspace") == P, f"{st_d} {str(dr)[:140]}")
    ok("agents: draft without a description is rejected", raw("/agents/api/agents/draft", "POST", b'{"description": ""}')[0] >= 400)
    pg.locator(".rail .nav[data-tip='Агенты']").click(); pg.wait_for_selector(".top .seg button:has-text('Навыки')", timeout=20000); time.sleep(1)
    pg.locator(".top .seg button:has-text('Агенты')").click(); pg.wait_for_selector(".rt", timeout=20000); time.sleep(0.5); pg.locator(f".rt-item[data-agent='{ag.get('id')}']").click(); time.sleep(0.8)
    ok("agents: the card shows the place, powers, memory budget, reflection and the skill checklist", pg.locator(".rt-form input.place").input_value() == "e2e-place" and pg.locator(".rt-form input.can-mail").is_checked() and pg.locator(f".rt-form .skills input[data-skill='{SK}']").is_checked() and pg.locator(".rt-form .draft").count() == 1 and pg.locator(".top .mailsw .switch.on").count() == 1, pg.locator(".rt-form input.place").input_value()); pg.screenshot(path="shots/ap9b-agent-v2.png")
    pg.locator(".top .seg button:has-text('Рутины')").click(); time.sleep(0.5); pg.locator(f".rt-item[data-routine='{rt.get('id')}']").click(); time.sleep(0.6)
    ok("agents: the routine card shows its own permission mode", pg.locator(".rt-form select.rmode").input_value() == "auto", pg.locator(".rt-form select.rmode").input_value())
    pg.locator(".top .seg button:has-text('Навыки')").click(); time.sleep(0.6); pg.locator(f".rt-item[data-skill='{SK}']").click(); time.sleep(0.6)
    ok("skills: the tab lists the skill with its users and opens the card", pg.locator(f".rt-item[data-skill='{SK}']").inner_text().find(f"e2e agent {MARK}") >= 0 and pg.locator(".rt-form textarea").input_value().find(f"skill-{MARK}") >= 0 and pg.locator(".rt-form input").first.is_disabled()); pg.screenshot(path="shots/ap9c-skills.png")
    pg.locator(".top .seg button:has-text('Запуски')").click(); time.sleep(0.6)
    ok("mail: the runs tab shows the mail run with its sender", pg.locator(f".runs .tr[data-run='{(rB or {}).get('id')}']").count() == 1 and pg.locator(f".runs .tr[data-run='{(rB or {}).get('id')}']").inner_text().find(f"почта от e2e agent {MARK}") >= 0 and pg.locator(f".runs .tr[data-run='{ml.get('id')}']").inner_text().find("почта от владельца") >= 0)
    raw(f"/agents/api/skills/{SK}", "DELETE"); raw(f"/agents/api/agents/{agB.get('id')}", "DELETE")
    ok("skills: deleting a skill removes it from the catalog and from the agents", all(x["name"] != SK for x in (get("/agents/api/skills")[1] or {}).get("skills", [])) and SK not in (next((x for x in (get("/agents/api/agents")[1] or {}).get("agents", []) if x["id"] == ag.get("id")), {}).get("skills") or []))
    raw(f"/agents/api/core/projects/{P}/fs?path=.claude/skills/{SK}&recursive=true", "DELETE"); raw(f"/agents/api/core/projects/{P}/fs?path=e2e-place&recursive=true", "DELETE")
    for x in (r4, ml, r5, rB or {}, r6):
        if x.get("id"): raw(f"/agents/api/runs/{x['id']}", "DELETE")
        if x.get("session"):
            for _ in range(3):
                if raw(f"/agents/api/core/projects/{P}/sessions/{x['session']}", "DELETE")[0] in (204, 404): break
                time.sleep(2)
    raw(f"/agents/api/routines/{rt.get('id')}", "DELETE"); raw(f"/agents/api/agents/{ag.get('id')}", "DELETE"); d1 = raw(f"/agents/api/runs/{rr.get('id')}", "DELETE")[0]; d2 = raw(f"/agents/api/runs/{hk.get('run')}", "DELETE")[0]
    if r3: raw(f"/agents/api/runs/{r3['id']}", "DELETE")
    ok("agents: routine, agent and their runs removed", d1 == 204 and d2 == 204 and get(f"/agents/api/routines")[1] and all(x["id"] != rt.get("id") for x in get("/agents/api/routines")[1]["routines"]) and all(x["id"] != ag.get("id") for x in get("/agents/api/agents")[1]["agents"]), f"{d1} {d2}")
    for sid_r in (rr.get("session"), r2.get("session"), (r3 or {}).get("session")):
        if sid_r:
            for _ in range(3):
                if raw(f"/agents/api/core/projects/{P}/sessions/{sid_r}", "DELETE")[0] in (204, 404): break
                time.sleep(3)
    # ---- permissions by rules: workspace settings via agentd perms.rs, CLI rule suggestions, "remember" with a scope ----
    PQ = f"/agents/api/core/projects/{P}/permissions?scope=project&cwd=/home/coder"
    st_pg, pg0 = raw(PQ); pg0 = json.loads(pg0 or b"{}")
    st_pp, pp = raw(PQ, "PUT", json.dumps({"allow": ["Bash(mkdir:*)"], "deny": [], "ask": []}).encode()); pp = json.loads(pp or b"{}")
    ok("perm: project rules written to .claude/settings.local.json and read back", st_pg == 200 and st_pp == 200 and pp.get("allow") == ["Bash(mkdir:*)"] and pp.get("path", "").endswith("/home/coder/.claude/settings.local.json") and (get(PQ)[1] or {}).get("allow") == ["Bash(mkdir:*)"], f"{st_pg} {st_pp} {str(pp)[:120]}")
    ps = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"perm probe {MARK}", "permission_mode": "default"}).encode())[1] or b"{}")
    time.sleep(4); raw(f"/agents/api/core/projects/{P}/sessions/{ps['id']}/prompt", "POST", json.dumps({"content": f"Run this exact shell command with the Bash tool: mkdir -p perm-allow-{MARK}   (relative path, nothing else). Then reply with exactly: perm-allow-{MARK}"}).encode())
    for _ in range(40):
        time.sleep(3); pinfo = get(f"/agents/api/core/projects/{P}/sessions/{ps['id']}")[1] or {}
        if pinfo.get("info", {}).get("status") in ("idle", "waiting", "error"): break
    pev = get(f"/agents/api/core/projects/{P}/sessions/{ps['id']}/events?since=0&limit=200")[1] or []
    ok("perm: a command matching an allow rule runs without a permission request", pinfo.get("info", {}).get("status") == "idle" and not any(e["kind"] == "permission_request" for e in pev) and f"perm-allow-{MARK}" in (pinfo.get("info", {}).get("last_text") or "") and get(f"/agents/api/core/projects/{P}/fs?path=/home/coder/perm-allow-{MARK}")[0] == 200, f"status={pinfo.get('info', {}).get('status')} last={(pinfo.get('info', {}).get('last_text') or '')[:60]} dir={get(f'/agents/api/core/projects/{P}/fs?path=/home/coder/perm-allow-{MARK}')[0]}")
    raw(f"/agents/api/core/projects/{P}/sessions/{ps['id']}/prompt", "POST", json.dumps({"content": f"Run this exact shell command with the Bash tool: touch perm-ask-{MARK}.txt   (relative path, nothing else). Then reply with exactly: perm-ask-{MARK}"}).encode())
    for _ in range(30):
        time.sleep(3); pinfo = get(f"/agents/api/core/projects/{P}/sessions/{ps['id']}")[1] or {}
        if pinfo.get("info", {}).get("status") in ("waiting", "idle", "error"): break
    pend = (pinfo.get("pending") or [{}])[0] if pinfo.get("pending") else {}
    ok("perm: a command outside the rules asks, and the request carries the CLI's rule suggestions", pinfo.get("info", {}).get("status") == "waiting" and pend.get("tool_name") == "Bash" and bool(pend.get("suggestions")), f"status={pinfo.get('info', {}).get('status')} sugg={str(pend.get('suggestions'))[:100]}")
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.8); pg.locator(f".panel .sess[data-id='{ps['id']}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab").first.click()
    try: pg.wait_for_selector(".thread .chat .m.perm:not(.done)", timeout=20000)
    except Exception: pass
    card = pg.locator(".thread .chat .m.perm:not(.done)").last
    ok("perm: the chat card shows the suggested rule and the 'allow and remember' menu button", card.count() == 1 and card.locator(".chip.rule").count() == 1 and "Bash(" in card.locator(".chip.rule").inner_text() and card.locator("button:has-text('Разрешить и запомнить')").count() == 1, card.locator(".chip.rule").inner_text() if card.count() and card.locator(".chip.rule").count() else "no rule chip")
    if card.count() and card.locator("button:has-text('Разрешить и запомнить')").count():
        card.locator("button:has-text('Разрешить и запомнить')").click(); pg.wait_for_selector(".menu .mi", timeout=5000)
        ok("perm: remember menu offers session, project and user scopes", pg.locator(".menu .mi:has-text('только в этой сессии')").count() == 1 and pg.locator(".menu .mi:has-text('в проекте')").count() == 1 and pg.locator(".menu .mi:has-text('для пользователя')").count() == 1)
        pg.locator(".menu .mi:has-text('только в этой сессии')").click()
    else: ok("perm: remember menu offers session, project and user scopes", False, "no remember button")
    try: pg.wait_for_selector(f".thread .chat .m.agent:has-text('perm-ask-{MARK}')", timeout=120000)
    except Exception: pass
    pres = [e for e in (get(f"/agents/api/core/projects/{P}/sessions/{ps['id']}/events?since=0&limit=300")[1] or []) if e["kind"] == "permission_result"]
    ok("perm: the answer went out with scope 'session' and the remembered rule", bool(pres) and pres[-1].get("scope") == "session" and pres[-1].get("remember") is True and any("Bash(" in r for r in (pres[-1].get("rules") or [])) and pg.locator(f".thread .chat .m.agent:has-text('perm-ask-{MARK}')").count() >= 1, str(pres[-1] if pres else None)[:160])
    n_req_before = len([e for e in (get(f"/agents/api/core/projects/{P}/sessions/{ps['id']}/events?since=0&limit=300")[1] or []) if e["kind"] == "permission_request"])
    pg.locator(".thread .chat textarea").fill(f"Run this exact shell command with the Bash tool again: touch perm-ask-{MARK}.txt   (relative path, nothing else). Then reply with exactly: perm-ask-{MARK}-again"); pg.locator(".thread .chat button.send").click()
    try: pg.wait_for_selector(f".thread .chat .m.agent:has-text('perm-ask-{MARK}-again')", timeout=120000)
    except Exception: pass
    n_req_after = len([e for e in (get(f"/agents/api/core/projects/{P}/sessions/{ps['id']}/events?since=0&limit=300")[1] or []) if e["kind"] == "permission_request"])
    ok("perm: the remembered session rule lets the next matching command run without asking", n_req_after == n_req_before and pg.locator(f".thread .chat .m.agent:has-text('perm-ask-{MARK}-again')").count() >= 1, f"requests before={n_req_before} after={n_req_after}"); pg.screenshot(path="shots/ap12-perm.png")
    raw(PQ, "PUT", json.dumps({"allow": pg0.get("allow", []), "deny": pg0.get("deny", []), "ask": pg0.get("ask", [])}).encode())
    ok("perm: project rules restored", (get(PQ)[1] or {}).get("allow") == pg0.get("allow", []))
    raw(f"/agents/api/core/projects/{P}/fs?path=/home/coder/perm-allow-{MARK}&recursive=1", "DELETE"); raw(f"/agents/api/core/projects/{P}/fs?path=/home/coder/perm-ask-{MARK}.txt", "DELETE")
    # cleanup of the "permissions" block's session right here: in a parallel run the "background after a turn"
    # block goes as a separate part
    for _ in range(3):
        if raw(f"/agents/api/core/projects/{P}/sessions/{ps.get('id')}", "DELETE")[0] in (204, 404): break
        time.sleep(3)
    # ---- background after a turn: task_started … task_notification, the task strip, stop, subagents under the
    # Agent card ----
    bs = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"bg probe {MARK}", "permission_mode": "auto"}).encode())[1] or b"{}")
    time.sleep(4); pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.8); pg.locator(f".panel .sess[data-id='{bs['id']}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab").first.click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000)
    pg.locator(".thread .chat textarea").fill(f"Use the Bash tool with run_in_background set to true to start this command: sleep 45; echo bg-{MARK}-done. Do not wait for it. Reply immediately with exactly: started"); pg.locator(".thread .chat button.send").click()
    try: pg.wait_for_selector(".thread .chat .tasks .chip.task", timeout=90000)
    except Exception: pass
    binfo = {}
    for _ in range(12):
        time.sleep(2); binfo = get(f"/agents/api/core/projects/{P}/sessions/{bs['id']}")[1] or {}
        if binfo.get("info", {}).get("background") == 1 and pg.locator(f".panel .sess[data-id='{bs['id']}'] .dot.background").count() == 1: break
    ok("bg: a backgrounded Bash shows up in the task strip, the session reports background work and the tree dot turns purple", pg.locator(".thread .chat .tasks .chip.task").count() == 1 and binfo.get("info", {}).get("background") == 1 and pg.locator(f".panel .sess[data-id='{bs['id']}'] .dot.background").count() == 1, f"chips={pg.locator('.thread .chat .tasks .chip.task').count()} background={binfo.get('info', {}).get('background')}"); pg.screenshot(path="shots/ap13-bg.png")
    try: pg.wait_for_selector(".thread .chat .m.task:has-text('завершена')", timeout=120000)
    except Exception: pass
    binfo2 = get(f"/agents/api/core/projects/{P}/sessions/{bs['id']}")[1] or {}
    ok("bg: the task's completion lands in the journal and the strip empties", pg.locator(".thread .chat .m.task:has-text('завершена')").count() >= 1 and pg.locator(".thread .chat .tasks .chip.task").count() == 0 and binfo2.get("info", {}).get("background") == 0, f"rows={pg.locator('.thread .chat .m.task').count()} background={binfo2.get('info', {}).get('background')}")
    # in headless mode the CLI sends task_notification as an event, and puts the <task-notification> user message only into the
    # transcript (visible in conversations from the TUI); check the render if such a message is in the journal
    time.sleep(6); bev = get(f"/agents/api/core/projects/{P}/sessions/{bs['id']}/events?since=0&limit=400")[1] or []
    has_note = any(e["kind"] == "user" and any(str(b.get("text", "")).lstrip().startswith("<task-notification>") for b in (e.get("content") or []) if isinstance(b, dict)) for e in bev)
    ok("bg: a CLI <task-notification> user message (if present) renders as a task row, not raw XML", pg.locator(".thread .chat .m.user:has-text('<task-notification>')").count() == 0 and (not has_note or pg.locator(".thread .chat .m.task.note").count() >= 1), f"in journal={has_note} raw={pg.locator('.thread .chat .m.user:has-text(\'<task-notification>\')').count()} notes={pg.locator('.thread .chat .m.task.note').count()}")
    for _ in range(20):
        if (get(f"/agents/api/core/projects/{P}/sessions/{bs['id']}")[1] or {}).get("info", {}).get("status") == "idle": break
        time.sleep(3)
    pg.locator(".thread .chat textarea").fill(f"Use the Bash tool with run_in_background set to true to start: sleep 600; echo never-{MARK}. Do not wait. Reply with exactly: started2"); pg.locator(".thread .chat button.send").click()
    try: pg.wait_for_selector(".thread .chat .tasks .chip.task", timeout=90000)
    except Exception: pass
    if pg.locator(".thread .chat .tasks .chip.task .x").count(): pg.locator(".thread .chat .tasks .chip.task .x").first.click()
    try: pg.wait_for_selector(".thread .chat .m.task:has-text('остановлена'), .thread .chat .m.task:has-text('упала')", timeout=45000)
    except Exception: pass
    ok("bg: the stop button kills the task through control stop_task", pg.locator(".thread .chat .tasks .chip.task").count() == 0 and pg.locator(".thread .chat .m.task:has-text('остановлена'), .thread .chat .m.task:has-text('упала')").count() >= 1, str(pg.locator('.thread .chat .m.task').all_inner_texts()[-2:])[:200])
    for _ in range(20):
        if (get(f"/agents/api/core/projects/{P}/sessions/{bs['id']}")[1] or {}).get("info", {}).get("status") == "idle": break
        time.sleep(3)
    pg.locator(".thread .chat textarea").fill("Use the Agent tool with subagent_type Explore to count how many files in the current directory end with .md (non-recursive is fine). Then reply with exactly: md=<number>"); pg.locator(".thread .chat button.send").click()
    try: pg.wait_for_selector(".thread .chat .m.tool .sub", timeout=180000)
    except Exception: pass
    # the Agent card is collapsed, the nested <details> is invisible — read text_content, then expand for the
    # screenshot
    sub_txt = (pg.locator(".thread .chat .m.tool .sub summary").first.text_content() or "") if pg.locator(".thread .chat .m.tool .sub").count() else "no sub group"
    ok("bg: subagent messages are grouped under the Agent tool card", pg.locator(".thread .chat .m.tool .sub").count() >= 1 and "сабагент" in sub_txt, sub_txt[:80])
    if pg.locator(".thread .chat .m.tool:has(.sub)").count(): pg.locator(".thread .chat .m.tool:has(.sub) > summary").first.click(); time.sleep(0.4); pg.locator(".thread .chat .m.tool .sub > summary").first.click(); time.sleep(0.4)
    pg.screenshot(path="shots/ap14-subagent.png")
    for sid_x in [x.get("id") for x in (globals().get("ps"), bs) if isinstance(x, dict) and x.get("id")]:   # ps — the session of the "permissions" block, if it was in this part
        for _ in range(3):
            if raw(f"/agents/api/core/projects/{P}/sessions/{sid_x}", "DELETE")[0] in (204, 404): break
            time.sleep(3)
    # ---- editing and forking messages, the queue, scheduled sending, input history, draft, Esc/Tab (agentd fork.rs + queue, the service's
    # scheduled.rs) ----
    es = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"edit probe {MARK}", "permission_mode": "auto"}).encode())[1] or b"{}")
    def open_sess(sid_o):
        pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.8); pg.locator(f".panel .sess[data-id='{sid_o}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab").first.click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000); time.sleep(1)
    def turn(text, expect=None, timeout=90000):
        n = pg.locator(".thread .chat .m.sys.turn").count() + 1 if expect is None else expect
        pg.locator(".thread .chat textarea").fill(text); pg.locator(".thread .chat button.send").click()
        try: pg.wait_for_function("n => document.querySelectorAll('.thread .chat .m.sys.turn').length >= n", arg=n, timeout=timeout)
        except Exception: pass
        time.sleep(2)
    def sinfo(sid_i): return (get(f"/agents/api/core/projects/{P}/sessions/{sid_i}")[1] or {}).get("info", {})
    def sevents(sid_i): return get(f"/agents/api/core/projects/{P}/sessions/{sid_i}/events?since=0&limit=600")[1] or []
    def utexts(evs): return [" ".join(str(b.get("text", "")) for b in (e.get("content") or []) if isinstance(b, dict)) for e in evs if e["kind"] == "user" and not e.get("parent_tool_use_id")]
    time.sleep(4); open_sess(es["id"])
    turn(f"Remember the word: apple-{MARK}. Reply with exactly: ok")
    turn(f"Remember the word: pear-{MARK}. Reply with exactly: ok")
    psid0 = sinfo(es["id"]).get("provider_session_id"); u0 = utexts(sevents(es["id"]))
    ok("edit: two user turns recorded on one provider session", len(u0) == 2 and bool(psid0), f"users={len(u0)} psid={psid0}")
    # a fork "from here" at the second message: a new conversation with the history up to it, its own
    # provider_session_id, opens by itself
    u2 = pg.locator(".thread .chat .m.user").nth(1); u2.hover(); u2.locator(".ma[title='Форк отсюда']").click(force=True)
    try: pg.wait_for_selector(".thread .chat .m.sys:has-text('форк беседы')", timeout=30000)
    except Exception: pass
    time.sleep(2)
    fsess = [x for x in (get(f"/agents/api/core/projects/{P}/sessions")[1] or []) if str(x.get("title", "")).startswith("⑂") and MARK in str(x.get("title", ""))]; fid = fsess[0]["id"] if fsess else None
    finfo = sinfo(fid) if fid else {}; fev = sevents(fid) if fid else []; fu = utexts(fev); tree_expand(pg, P)
    ok("fork: ⑂ on a user bubble creates a session with the history before it, its own provider session and a «forked» row", bool(fid) and finfo.get("provider_session_id") not in (None, psid0) and len(fu) == 1 and any(e["kind"] == "forked" for e in fev) and pg.locator(f".panel .sess[data-id='{fid}']").count() == 1 and pg.locator(".thread .chat .m.sys:has-text('форк беседы')").count() == 1, f"fid={fid} users={len(fu)} psid={finfo.get('provider_session_id')}")
    pg.screenshot(path="shots/ap15-fork.png")
    turn("Which words did I ask you to remember? List them all.", timeout=120000)
    ftxt = " ".join(pg.locator(".thread .chat .m.agent").all_inner_texts())
    ok("fork: the forked CLI session knows only the history before the fork point", f"apple-{MARK}" in ftxt and f"pear-{MARK}" not in ftxt, ftxt[-160:].replace("\n", " "))
    # editing the second message: the journal is cut, the CLI resumes from the anchor (--resume-session-at) on the same
    # provider session
    open_sess(es["id"])
    u2 = pg.locator(".thread .chat .m.user").nth(1); u2.hover(); u2.locator(".ma[title='Править и переотправить']").click(force=True); time.sleep(0.5)
    ok("edit: ✎ on a user bubble switches the composer into edit mode with the original text", pg.locator(".thread .chat .editbar").count() == 1 and f"pear-{MARK}" in pg.locator(".thread .chat textarea").input_value(), pg.locator(".thread .chat textarea").input_value()[:60])
    pg.locator(".thread .chat textarea").fill("Which words did I ask you to remember? List them all."); pg.locator(".thread .chat button.send").click()
    # wait on the backend: the truncation mark, a new answer after it, the session is free (an edit = stop the process + restart
    # from the anchor + a turn)
    for _ in range(75):
        ev1 = sevents(es["id"]); cut = [e for e in ev1 if e["kind"] == "history_truncated"]
        if cut and any(e["kind"] == "result" and e["seq"] > cut[0]["seq"] for e in ev1) and sinfo(es["id"]).get("status") == "idle": break
        time.sleep(2)
    time.sleep(2); ev1 = sevents(es["id"]); u1 = utexts(ev1); cut = [e for e in ev1 if e["kind"] == "history_truncated"]; psid1 = sinfo(es["id"]).get("provider_session_id")
    atxt = " ".join(pg.locator(".thread .chat .m.agent").all_inner_texts())
    ok("edit: the journal is cut at the edited message, the CLI resumes at the anchor on the same provider session and answers the new text", len(cut) == 1 and bool(cut[0].get("anchor")) and psid1 == psid0 and len(u1) == 2 and not any(f"pear-{MARK}" in x for x in u1) and pg.locator(".thread .chat .m.sys.cut").count() == 1 and f"apple-{MARK}" in atxt and f"pear-{MARK}" not in atxt, f"cut={len(cut)} anchor={bool(cut and cut[0].get('anchor'))} same_psid={psid1 == psid0} users={len(u1)} {atxt[-140:]}".replace("\n", " "))
    pg.screenshot(path="shots/ap16-edit.png")
    # queue: messages during a turn go to the agentd queue, × removes, after the turn the first one goes
    # by itself
    pg.locator(".thread .chat textarea").fill(f"Use the Bash tool to run: sleep 30; echo q-{MARK}. Then reply with exactly: slept"); pg.locator(".thread .chat button.send").click(); time.sleep(5)
    pg.locator(".thread .chat textarea").fill(f"Reply with exactly: queued-one-{MARK}"); pg.locator(".thread .chat button.send").click(); time.sleep(1.5)
    pg.locator(".thread .chat textarea").fill(f"Reply with exactly: queued-two-{MARK}"); pg.locator(".thread .chat button.send").click(); time.sleep(1.5)
    qi = sinfo(es["id"])
    ok("queue: messages sent during a turn are queued in agentd and shown as cards", pg.locator(".thread .chat .qcard:not(.sched)").count() == 2 and len(qi.get("queued") or []) == 2 and qi.get("status") == "working", f"cards={pg.locator('.thread .chat .qcard:not(.sched)').count()} queued={len(qi.get('queued') or [])} status={qi.get('status')}")
    pg.screenshot(path="shots/ap17-queue.png")
    pg.locator(".thread .chat .qcard:not(.sched):has-text('queued-two')").locator(".x").click()
    for _ in range(10):
        time.sleep(1)
        if pg.locator(".thread .chat .qcard:not(.sched)").count() == 1 and len(sinfo(es["id"]).get("queued") or []) == 1: break
    ok("queue: × removes a queued message", pg.locator(".thread .chat .qcard:not(.sched)").count() == 1 and len(sinfo(es["id"]).get("queued") or []) == 1, f"cards={pg.locator('.thread .chat .qcard:not(.sched)').count()} queued={len(sinfo(es['id']).get('queued') or [])}")
    for _ in range(90):
        qi2 = sinfo(es["id"])
        if not (qi2.get("queued") or []) and qi2.get("status") == "idle" and any(f"queued-one-{MARK}" in x for x in utexts(sevents(es["id"]))): break
        time.sleep(2)
    time.sleep(3); u2t = utexts(sevents(es["id"]))
    ok("queue: after the turn the first queued message goes out by itself, the removed one never does", any(f"queued-one-{MARK}" in x for x in u2t) and not any("queued-two" in x for x in u2t) and pg.locator(".thread .chat .qcard:not(.sched)").count() == 0 and f"queued-one-{MARK}" in " ".join(pg.locator(".thread .chat .m.agent").all_inner_texts()), f"users={len(u2t)} status={sinfo(es['id']).get('status')}")
    # Esc interrupts the turn
    pg.locator(".thread .chat textarea").fill(f"Use the Bash tool to run: sleep 90; echo esc-{MARK}. Then reply with exactly: slept2"); pg.locator(".thread .chat button.send").click(); time.sleep(6)
    pg.locator(".thread .chat textarea").press("Escape"); est = ""
    for _ in range(15):
        est = sinfo(es["id"]).get("status"); 
        if est == "idle": break
        time.sleep(2)
    ok("keys: Esc in the composer interrupts the running turn", est == "idle" and "slept2" not in " ".join(pg.locator(".thread .chat .m.agent").all_inner_texts()), f"status={est}")
    # ↑/↓ browse your messages, Tab moves the focus without changing permissions, the draft survives a reload
    ta_e = pg.locator(".thread .chat textarea"); ta_e.fill(""); ta_e.press("ArrowUp"); time.sleep(0.3); v_up = ta_e.input_value(); ta_e.press("ArrowUp"); time.sleep(0.3); v_up2 = ta_e.input_value(); ta_e.press("ArrowDown"); ta_e.press("ArrowDown"); time.sleep(0.3); v_dn = ta_e.input_value()
    ok("keys: ↑ recalls previous messages, ↓ returns to the empty field", f"esc-{MARK}" in v_up and f"queued-one-{MARK}" in v_up2 and v_dn == "", f"up={v_up[:40]!r} up2={v_up2[:40]!r} down={v_dn!r}")
    m0 = sinfo(es["id"]).get("permission_mode"); ta_e.press("Tab"); time.sleep(2.5); m1 = sinfo(es["id"]).get("permission_mode")
    ok("keys: Tab moves focus without changing the permission mode", m1 == m0 and ta_e.evaluate("e => document.activeElement !== e"), f"{m0} → {m1}")
    ta_e.fill(f"draft-{MARK}"); time.sleep(0.6); pg.reload(wait_until="load"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(2); open_sess(es["id"])
    ok("keys: an unsent draft survives a reload (localStorage per session)", pg.locator(".thread .chat textarea").input_value() == f"draft-{MARK}", pg.locator(".thread .chat textarea").input_value()[:40])
    # scheduled sending: via the API at +1 h (card + cancel), via the 🕒 menu "custom time…" at the nearest minute — goes by itself through the
    # service scheduler
    st_s, sm1 = raw("/agents/api/scheduled", "POST", json.dumps({"project": P, "session": es["id"], "content": f"Reply with exactly: never-sched-{MARK}", "at": int(time.time()) + 3600}).encode()); sm1 = json.loads(sm1 or b"{}")
    ok("sched: POST /agents/api/scheduled stores a pending message", st_s == 200 and sm1.get("status") == "pending" and str(sm1.get("id", "")).startswith("sm_"), f"{st_s} {json.dumps(sm1)[:100]}")
    # the nearest minute boundary not earlier than 60 s away (minute precision of the field: +61 s near the end of a minute gave "in 1 s", and the scheduler beat the
    # cancel check)
    when_s = time.strftime("%Y-%m-%d %H:%M", time.localtime((int(time.time() + 60) // 60 + 1) * 60)); pg.once("dialog", lambda d: d.accept(when_s))
    pg.locator(".thread .chat textarea").fill(f"Reply with exactly: sched-{MARK}"); composer_option(pg, "Отправить позже"); pg.wait_for_selector(".menu .mi:has-text('через 15 мин')", timeout=5000)
    ok("sched: the 🕒 menu offers presets and a custom time", pg.locator(".menu .mi:has-text('через 15 мин')").count() == 1 and pg.locator(".menu .mi:has-text('завтра в 09:00')").count() == 1 and pg.locator(".menu .mi:has-text('своё время')").count() == 1)
    pg.locator(".menu .mi:has-text('своё время')").click(); time.sleep(2.5)
    ok("sched: a custom time schedules the message; both pending ones show as cards and the field clears", pg.locator(".thread .chat .qcard.sched").count() == 2 and pg.locator(".thread .chat textarea").input_value() == "", f"cards={pg.locator('.thread .chat .qcard.sched').count()} when={when_s}")
    pg.screenshot(path="shots/ap18-sched.png")
    pg.locator(".thread .chat .qcard.sched:has-text('never-sched')").locator(".x").click(); time.sleep(1.5)
    lst = (get(f"/agents/api/scheduled?project={P}&session={es['id']}")[1] or {}).get("scheduled", [])
    ok("sched: × cancels a scheduled message", pg.locator(".thread .chat .qcard.sched").count() == 1 and len(lst) == 1 and f"sched-{MARK}" in lst[0].get("content", ""), f"pending={len(lst)}")
    try: pg.wait_for_selector(f".thread .chat .m.user:has-text('sched-{MARK}')", timeout=150000)
    except Exception: pass
    try: pg.wait_for_function("m => [...document.querySelectorAll('.thread .chat .m.agent')].some(x => x.textContent.includes(m))", arg=f"sched-{MARK}", timeout=90000)
    except Exception: pass
    for _ in range(20):
        if pg.locator(".thread .chat .qcard.sched").count() == 0: break
        time.sleep(2)
    time.sleep(1); lst2 = (get(f"/agents/api/scheduled?project={P}&session={es['id']}&all=true")[1] or {}).get("scheduled", [])
    s_user = pg.locator(f".thread .chat .m.user:has-text('sched-{MARK}')").count(); s_agent = f"sched-{MARK}" in " ".join(pg.locator(".thread .chat .m.agent").all_inner_texts()); s_cards = pg.locator(".thread .chat .qcard.sched").count()
    ok("sched: the due message is sent by the service into the live session and the card disappears", s_user == 1 and any(x.get("status") == "sent" and f"sched-{MARK}" in x.get("content", "") for x in lst2) and s_cards == 0 and s_agent, f"statuses={[x.get('status') for x in lst2]} notes={[x.get('note') for x in lst2]} user_rows={s_user} agent_has={s_agent} cards={s_cards}")
    for sid_x in (es.get("id"), fid):
        if not sid_x: continue
        for _ in range(3):
            if raw(f"/agents/api/core/projects/{P}/sessions/{sid_x}", "DELETE")[0] in (204, 404): break
            time.sleep(3)
    # ---- git panel and worktrees (agentd fs.rs git_*, WsGit.svelte, docs/agents-ux.md §4.3.6): a repository and a bare remote in temporary
    # directories ----
    GR = f"e2e-git-{MARK}"; GRP = f"/home/coder/{GR}"; REM = f"/home/coder/{GR}-remote.git"; WTP = f"{GRP}/.worktrees/feat-{MARK}"
    gpost = lambda op, body: raw(f"/agents/api/core/projects/{P}/git/{op}", "POST", json.dumps(body).encode())
    st_g, _ = gpost("init", {"cwd": GRP}); st_r, _ = gpost("init", {"cwd": REM, "bare": True})
    raw(f"/agents/api/core/projects/{P}/fs/file?path={GRP}/README.md", "PUT", f"# {GR}\n".encode(), "text/plain")
    ok("git: init of a repo and a bare remote through the panel API", st_g == 200 and st_r == 200 and (get(f"/agents/api/core/projects/{P}/git/status?cwd={GRP}")[1] or {}).get("repo") is True, f"{st_g} {st_r}")
    gs = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"git probe {MARK}", "cwd": GRP}).encode())[1] or b"{}")
    gpane = pg.locator(".thread .pane"); gp = pg.locator(".thread .pane .git")
    def open_git(sid_o):
        pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.8); pg.locator(f".panel .sess[data-id='{sid_o}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000)
        gpane.locator(".tabs .tab:has-text('Git')").click(); pg.wait_for_selector(".thread .pane .git .gbar", timeout=20000); time.sleep(1.2)
    time.sleep(3); open_git(gs["id"])
    ok("git: the panel shows the branch, the untracked file and no sync buttons without a remote", "main" in gp.locator(".gbar .branch").inner_text() and gp.locator(".row:has-text('README.md') .st.q").count() == 1 and gp.locator(".gbar .push").count() == 0, gp.locator(".gbar").inner_text()[:60])
    gp.locator(".row:has-text('README.md') .ck").check(); time.sleep(1.5)
    ok("git: the checkbox stages the file", gp.locator(".row:has-text('README.md') .ck").is_checked() and gp.locator(".row:has-text('README.md') .st.a").count() == 1)
    gp.locator(".cm textarea").fill(f"init {MARK}"); gp.locator("button.commit").click(); time.sleep(3)
    glog = (get(f"/agents/api/core/projects/{P}/git/log?cwd={GRP}")[1] or {}).get("entries", [])
    ok("git: commit from the panel lands in the log and the tree is clean", gp.locator(f".commit:has-text('init {MARK}')").count() == 1 and any(c["subject"] == f"init {MARK}" for c in glog) and gp.locator(".row").count() == 0, f"log={[c['subject'] for c in glog][:3]} rows={gp.locator('.row').count()}")
    gpost("remote", {"cwd": GRP, "name": "origin", "url": REM}); gp.locator(".gbar button:has-text('⟳')").click(); time.sleep(1.5)
    pg.once("dialog", lambda d: d.accept()); gp.locator(".gbar .push").click(); time.sleep(3)
    ok("git: push -u to the bare remote sets the upstream", "origin/main" in gp.locator(".gbar").inner_text(), gp.locator(".gbar").inner_text()[:80])
    pg.once("dialog", lambda d: d.accept(f"feat-{MARK}")); gp.locator("button.wt-add").click(); time.sleep(3)
    wl = (get(f"/agents/api/core/projects/{P}/git/worktrees?cwd={GRP}")[1] or {}).get("worktrees", [])
    ok("git: + worktree creates .worktrees/<name> on a branch with the same name and lists it", gp.locator(f".wt .wrow[data-name='feat-{MARK}']").count() == 1 and any(w["name"] == f"feat-{MARK}" and w["branch"] == f"feat-{MARK}" and w["path"] == WTP for w in wl), json.dumps(wl)[:200])
    gp.locator(f".wt .wrow[data-name='feat-{MARK}'] .act-session").click(); time.sleep(4)
    wsess = [x for x in (get(f"/agents/api/core/projects/{P}/sessions")[1] or []) if x.get("cwd") == WTP]
    ok("git: «session» opens a new Claude session with cwd in the worktree and the tree shows a ⎇ chip", len(wsess) == 1 and pg.locator(f".panel .sess[data-id='{wsess[0]['id']}'] .wt").count() == 1, f"sessions={len(wsess)} chip={pg.locator('.panel .sess .wt').count()}")
    raw(f"/agents/api/core/projects/{P}/fs/file?path={WTP}/feature.txt", "PUT", f"feature {MARK}\n".encode(), "text/plain")
    if wsess: open_git(wsess[0]["id"])
    ok("git: the worktree session's Git tab is on the feature branch with the new file and marks the worktree as current", f"feat-{MARK}" in gp.locator(".gbar .branch").inner_text() and gp.locator(".row:has-text('feature.txt')").count() == 1 and gp.locator(f".wt .wrow[data-name='feat-{MARK}'].cur").count() == 1, gp.locator(".gbar").inner_text()[:60])
    gp.locator(".lnk.stage-all").click(); time.sleep(1.5); gp.locator(".lnk.suggest").click()
    for _ in range(90):
        time.sleep(1)
        if gp.locator(".cm textarea").input_value().strip(): break
    sug = gp.locator(".cm textarea").input_value()
    ok("git: ✨ asks headless Claude (haiku via gate) for a commit message from the staged diff", len(sug.strip()) > 5 and "feature" in sug.lower(), sug[:100].replace("\n", " "))
    gp.locator(".cm textarea").fill(f"feature {MARK}"); gp.locator("button.commit").click(); time.sleep(3)
    ok("git: commit inside the worktree", gp.locator(f".commit:has-text('feature {MARK}')").count() == 1 and gp.locator(".row").count() == 0)
    pg.screenshot(path="shots/ap19-git-worktree.png")
    open_git(gs["id"])
    pg.once("dialog", lambda d: d.accept()); gp.locator(f".wt .wrow[data-name='feat-{MARK}'] .act-merge").click(); pg.wait_for_selector(".menu .mi", timeout=5000); pg.locator(".menu .mi:has-text('удалить worktree и ветку')").click(); time.sleep(5)
    glog2 = (get(f"/agents/api/core/projects/{P}/git/log?cwd={GRP}")[1] or {}).get("entries", []); fl = get(f"/agents/api/core/projects/{P}/fs?path={GRP}")[1]
    wl2 = (get(f"/agents/api/core/projects/{P}/git/worktrees?cwd={GRP}")[1] or {}).get("worktrees", []); br2 = (get(f"/agents/api/core/projects/{P}/git/branches?cwd={GRP}")[1] or {}).get("branches", [])
    ok("git: squash-merge brings the feature file into main as one commit; the worktree and its branch are gone", any(f"feat-{MARK}" in c["subject"] for c in glog2) and "feature.txt" in json.dumps(fl) and not any(w["name"] == f"feat-{MARK}" for w in wl2) and not any(b["name"] == f"feat-{MARK}" for b in br2) and gp.locator(f".wt .wrow[data-name='feat-{MARK}']").count() == 0, f"log={[c['subject'] for c in glog2][:3]} wts={len(wl2)} branches={[b['name'] for b in br2]}")
    pg.screenshot(path="shots/ap20-git-merged.png")
    raw(f"/agents/api/core/projects/{P}/fs/file?path={GRP}/README.md", "PUT", b"changed\n", "text/plain"); gp.locator(".gbar button:has-text('⟳')").click(); time.sleep(1.5)
    pg.once("dialog", lambda d: d.accept()); gp.locator(".row:has-text('README.md') .disc").click(); time.sleep(2.5)
    ok("git: ↶ discards a file's changes after confirmation", gp.locator(".row").count() == 0 and b"changed" not in (get(f"/agents/api/core/projects/{P}/fs/file?path={GRP}/README.md")[1] or b""), f"rows={gp.locator('.row').count()}")
    for sid_x in [gs.get("id")] + [x["id"] for x in wsess]:
        for _ in range(3):
            if raw(f"/agents/api/core/projects/{P}/sessions/{sid_x}", "DELETE")[0] in (204, 404): break
            time.sleep(3)
    raw(f"/agents/api/core/projects/{P}/fs?path={GRP}&recursive=true", "DELETE"); raw(f"/agents/api/core/projects/{P}/fs?path={REM}&recursive=true", "DELETE")
    # Claude Code transcript directories for the temporary cwds (empty after the
    # sessions are deleted)
    for d in (f"-home-coder-e2e-git-{MARK}", f"-home-coder-e2e-git-{MARK}--worktrees-feat-{MARK}"): raw(f"/agents/api/core/projects/{P}/fs?path=~/.claude/projects/{d}&recursive=true", "DELETE")
    # ---- scratch chat (chat.rs, the "Chat" section): the service workspace, a chat in its own directory,
    # moving to a project ----
    st_c, ch = raw("/agents/api/chat/ensure", "POST", b"{}"); ch = json.loads(ch or b"{}")
    for _ in range(80):
        if ch.get("status") == "running" and ch.get("reachable"): break
        time.sleep(3); ch = get("/agents/api/chat")[1] or {}
    if not (ch.get("status") == "running" and ch.get("reachable")):
        print("SKIP  chat: scratch is not running yet (", ch.get("status"), ch.get("error", ""), ") — the container takes minutes on first creation; rerun later", flush=True)
    else:
        ok("chat: scratch is a hidden core project (absent from /projects, present with ?all=1)", all(p["name"] != "scratch" for p in (get("/agents/api/core/projects")[1] or [])) and any(p["name"] == "scratch" and p["spec"].get("hidden") for p in (get("/agents/api/core/projects?all=1")[1] or [])))
        # auto mode: a command in scratch runs without a question (otherwise the turn stalls on a
        # permission card)
        st_n, cs = raw("/agents/api/chat", "POST", json.dumps({"agent": "claude", "permission_mode": "auto"}).encode()); cs = json.loads(cs or b"{}")
        ok("chat: new chat is a session in its own directory of scratch", st_n == 200 and cs.get("cwd", "").startswith("/home/coder/chats/") and cs.get("agent") == "claude" and cs.get("permission_mode") == "auto", f"{st_n} {cs.get('cwd')}")
        pg.locator(".rail .nav[data-tip='Чат']").click(); pg.wait_for_selector(".top .engine button[data-engine='advanced']", timeout=20000); pg.locator(".top .engine button[data-engine='advanced']").click(); time.sleep(0.5); pg.wait_for_selector(f".panel .sess[data-id='{cs.get('id')}']", timeout=20000); pg.locator(f".panel .sess[data-id='{cs.get('id')}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab").first.click(); time.sleep(1)
        ok("chat: section lists the chat and opens it without a Git tab", [x.get_attribute("aria-label") for x in pg.locator(".thread .pane .tabs .tab").all()] == ["Чат", "CLI", "Терминал", "Файлы"], str(pg.locator(".thread .pane .tabs .tab").all_inner_texts()))
        ok("chat: scratch isn't in the workspaces tree", pg.locator(".rail .nav[data-tip='Рабочие места']").count() == 1 and (get("/agents/api/workspaces")[1] or {}).get("projects") and any(p["name"] == "scratch" and p.get("hidden") for p in get("/agents/api/workspaces")[1]["projects"]))
        pg.locator(".thread .chat textarea").fill(f"Using a shell command, create a file named chat-{MARK}.txt in the current directory containing the text chat-{MARK}. Then reply with exactly: chat-{MARK}"); pg.locator(".thread .chat button.send").click()
        try: pg.wait_for_selector(f".thread .chat .m.agent:has-text('chat-{MARK}')", timeout=150000)
        except Exception:
            if pg.locator(".thread .chat button:has-text('Разрешить')").count(): pg.locator(".thread .chat button:has-text('Разрешить')").first.click(); pg.wait_for_selector(f".thread .chat .m.agent:has-text('chat-{MARK}')", timeout=120000)
        fl = get(f"/agents/api/core/projects/scratch/fs?path={cs.get('cwd')}")[1] or {}
        ok("chat: model turn in scratch wrote the file into the chat directory", pg.locator(f".thread .chat .m.agent:has-text('chat-{MARK}')").count() >= 1 and any(e["name"] == f"chat-{MARK}.txt" for e in fl.get("entries", [])), str([e["name"] for e in fl.get("entries", [])])[:120])
        title_before = pg.locator(".thread .t-head h2").inner_text(); pg.screenshot(path="shots/ap10-chat.png")
        pg.once("dialog", lambda d: d.accept()); pg.locator(".top button:has-text('Перенести в workspace')").click(); pg.wait_for_selector(".menu .mi", timeout=5000); pg.locator(f".menu .mi:has-text('{P}')").click()
        try: pg.wait_for_selector(".toast:has-text('перенесён')", timeout=60000)
        except Exception: pass
        time.sleep(2)
        moved = get(f"/agents/api/core/projects/{P}/sessions/{cs.get('id')}")[1] or {}
        gone = get(f"/agents/api/core/projects/scratch/sessions/{cs.get('id')}")[0]
        mfiles = get(f"/agents/api/core/projects/{P}/fs?path={cs.get('cwd')}")[1] or {}
        mev = get(f"/agents/api/core/projects/{P}/sessions/{cs.get('id')}/events?since=0&limit=500")[1] or []
        ok("chat: move keeps id, title and history; cwd becomes the project home; files travel to the same path; source removed", moved.get("info", {}).get("id") == cs.get("id") and moved["info"]["title"] == title_before and moved["info"]["cwd"] == "/home/coder" and gone == 404 and any(e["name"] == f"chat-{MARK}.txt" for e in mfiles.get("entries", [])) and any(e["kind"] == "assistant" and f"chat-{MARK}" in json.dumps(e, ensure_ascii=False) for e in mev), f"cwd={moved.get('info', {}).get('cwd')} gone={gone} files={[e['name'] for e in mfiles.get('entries', [])][:3]}")
        ok("chat: after the move the UI lands on the session in Work / Thread", pg.locator(".thread .t-head h2").count() == 1 and pg.locator(".thread .t-head").inner_text().find(P) >= 0 and pg.locator(f".panel .sess[data-id='{cs.get('id')}']").count() == 1)
        pg.locator(".thread .pane .tabs .tab").first.click(); pg.locator(".thread .chat textarea").fill(f"Reply with exactly: moved-{MARK}"); pg.locator(".thread .chat button.send").click()
        try: pg.wait_for_selector(f".thread .chat .m.agent:has-text('moved-{MARK}')", timeout=150000)
        except Exception: pass
        after_m = {}
        for _ in range(12):
            after_m = get(f"/agents/api/core/projects/{P}/sessions/{cs.get('id')}")[1] or {}
            if (after_m.get("info", {}).get("usage") or {}).get("turns", 0) >= 2: break
            time.sleep(3)
        ok("chat: the moved conversation resumes in the project on the same provider session", pg.locator(f".thread .chat .m.agent:has-text('moved-{MARK}')").count() >= 1 and after_m.get("info", {}).get("provider_session_id") == cs.get("id") and (after_m.get("info", {}).get("usage") or {}).get("turns", 0) >= 2, f"pid={after_m.get('info', {}).get('provider_session_id')} turns={(after_m.get('info', {}).get('usage') or {}).get('turns')}")
        for _ in range(4):
            if raw(f"/agents/api/core/projects/{P}/sessions/{cs.get('id')}", "DELETE")[0] in (204, 404): break
            time.sleep(4)
        raw(f"/agents/api/core/projects/{P}/fs?path={cs.get('cwd')}&recursive=1", "DELETE")
        ok("chat: cleanup — moved session and its directory removed from the project", get(f"/agents/api/core/projects/{P}/sessions/{cs.get('id')}")[0] == 404 and get(f"/agents/api/core/projects/{P}/fs?path={cs.get('cwd')}")[0] >= 400)
    # OAuth sign-in from Settings: the sign-in terminal opens and prints a link; an import before the sign-in completes
    # — a clear error
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Аккаунты')").click(); time.sleep(0.5)
    # Claude "by link" = an OAuth link (return to localhost:54545, the address is pasted back); "by code" = the code from
    # the page
    pg.locator(".settings button:has-text('войти в подписку')").click(); pg.wait_for_selector(".settings button:has-text('Получить ссылку')", timeout=5000); pg.locator(".settings button:has-text('Получить ссылку')").click()
    pg.wait_for_selector(".settings a:has-text('Открыть ссылку')", timeout=15000); href = pg.locator(".settings a:has-text('Открыть ссылку')").get_attribute("href") or ""
    ok("login: Claude by link is an OAuth link with PKCE and the localhost:54545 return plus a field for the address", href.startswith("https://claude.com/cai/oauth/authorize?") and "client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e" in href and "code_challenge=" in href and "redirect_uri=http%3A%2F%2Flocalhost%3A54545%2Fcallback" in href and pg.locator(".settings input.cbaddr").count() == 1, href[:100])
    pg.locator(".settings input.cbaddr").fill("http://localhost:54545/callback?code=bogus&state=other"); pg.locator(".settings button:has-text('Завершить вход')").click(); time.sleep(3)
    lerr = pg.locator(".settings .err").first.inner_text() if pg.locator(".settings .err").count() else ""
    ok("login: an address from another sign-in (state mismatch) is rejected with a clear message", "state" in lerr, lerr[:120])
    pg.locator(".settings button:has-text('Отмена')").click(); time.sleep(0.5)
    pg.locator(".settings button:has-text('войти в подписку')").click(); pg.wait_for_selector(".settings input[name='lw'][value='code']", timeout=5000); pg.locator(".settings input[name='lw'][value='code']").check(); pg.locator(".settings button:has-text('Получить ссылку')").click()
    pg.wait_for_selector(".settings a:has-text('Открыть ссылку')", timeout=15000); href = pg.locator(".settings a:has-text('Открыть ссылку')").get_attribute("href") or ""
    ok("login: Claude by code builds the PKCE link with the code page (platform.claude.com) and a code#state field", "redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback" in href and pg.locator(".settings input[placeholder='xxxxxxxx#yyyyyyyy']").count() == 1, href[:100])
    pg.locator(".settings input[placeholder='xxxxxxxx#yyyyyyyy']").fill("bogus-code#bogus-state"); pg.locator(".settings button:has-text('Завершить вход')").click(); time.sleep(3)
    lerr = pg.locator(".settings .err").first.inner_text() if pg.locator(".settings .err").count() else ""
    ok("login: a wrong code is rejected with a clear message", "state" in lerr or "обмен кода" in lerr or "недоступен" in lerr, lerr[:120])
    pg.locator(".settings button:has-text('Отмена')").click(); time.sleep(0.5)
    # ChatGPT (Codex) "by link" = an auth.openai.com OAuth link with a return to localhost:1455; "by code" =
    # device-code
    pg.locator(".settings button:has-text('войти в подписку')").click(); pg.wait_for_selector(".settings select", timeout=5000); pg.locator(".settings .frm select").first.select_option("codex"); time.sleep(0.3)
    ok("login: the link way is available for ChatGPT (Codex), not only for Claude", pg.locator(".settings input[name='lw'][value='link']").is_enabled() and pg.locator(".settings input[name='lw'][value='link']").is_checked())
    pg.locator(".settings button:has-text('Получить ссылку')").click(); pg.wait_for_selector(".settings input.cbaddr", timeout=15000); ohref = pg.locator(".settings a:has-text('Открыть ссылку')").get_attribute("href") or ""
    ok("login: ChatGPT by link is the OAuth link with PKCE and the localhost:1455 return plus a field for the address", ohref.startswith("https://auth.openai.com/oauth/authorize?client_id=app_EMoamEEZ73f0CkXaXp7hrann") and "redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback" in ohref and "code_challenge=" in ohref and "codex_cli_simplified_flow=true" in ohref, ohref[:100])
    pg.locator(".settings input.cbaddr").fill("http://localhost:1455/auth/callback?code=bogus&state=" + (ohref.split("state=")[1].split("&")[0] if "state=" in ohref else "x")); pg.locator(".settings button:has-text('Завершить вход')").click(); time.sleep(6)
    lerr = pg.locator(".settings .err").first.inner_text() if pg.locator(".settings .err").count() else ""
    ok("login: a bogus code in a matching address goes to the token endpoint and is rejected there", "обмен кода" in lerr or "недоступен" in lerr, lerr[:120])
    pg.locator(".settings button:has-text('Отмена')").click(); time.sleep(0.5)
    pg.locator(".settings button:has-text('войти в подписку')").click(); pg.wait_for_selector(".settings input[name='lw'][value='code']", timeout=5000); pg.locator(".settings .frm select").first.select_option("codex"); pg.locator(".settings input[name='lw'][value='code']").check()
    pg.locator(".settings button:has-text('Получить ссылку')").click(); pg.wait_for_selector(".settings .devcode .code", timeout=30000); time.sleep(0.5)
    dhref = pg.locator(".settings a:has-text('Открыть ссылку')").get_attribute("href") or ""; dcode = pg.locator(".settings .devcode .code").inner_text().strip()
    ok("login: Codex by code shows the device page link and a one-time code, and waits for the confirmation", dhref == "https://auth.openai.com/codex/device" and re.fullmatch(r"[A-Z0-9]{4}-[A-Z0-9]{5}", dcode) is not None and pg.locator(".settings .waiting").count() == 1, f"{dhref} code={dcode}")
    pg.locator(".settings button:has-text('Отмена')").click(); time.sleep(0.5)
    # providers through the gate: a card with the sign-ins of the local LLM proxy (management API), the "sign in ▾" and "plugins ▾" menus, a Kimi
    # device sign-in gives a link and a code
    pg.wait_for_selector(".settings .gate .gtarget", timeout=30000); gtg = pg.locator(f".settings .gate .gtarget[data-app='{GATE_APP}']"); time.sleep(0.5)
    gprov = [x.strip() for x in gtg.locator(".glogin-row .gp").all_inner_texts()]
    ok("gate: the providers card lists the local LLM proxy with its sign-ins (auth files) read through the Management API", gtg.count() == 1 and gtg.locator(".dot.ok").count() == 1 and "antigravity" in gprov, str(gprov)[:120])
    gtg.locator("button.glogin").click(); pg.wait_for_selector(".menu .mi", timeout=5000); gm = [x.split("\n")[0].strip() for x in pg.locator(".menu .mi").all_inner_texts()]
    ok("gate: the sign-in menu offers Antigravity, Kimi, xAI Grok, ChatGPT and Claude", all(any(n in x for x in gm) for n in ("Antigravity", "Kimi", "xAI", "ChatGPT", "Claude")), str(gm)[:160])
    # ChatGPT and Claude in the gate — the proxy's own OAuth links (return to localhost:1455 / :54545, the address is pasted back), as in the
    # the proxy panel
    pg.locator(".menu .mi:has-text('ChatGPT (Codex)')").first.click(); pg.wait_for_selector(".settings .gflow input.mono, .settings .gflow .err", timeout=45000); time.sleep(0.5)
    curl = pg.locator(".settings .gflow a:has-text('Открыть ссылку')").get_attribute("href") if pg.locator(".settings .gflow a:has-text('Открыть ссылку')").count() else ""
    ok("gate: ChatGPT (Codex) sign-in into the proxy is the proxy's OAuth link (auth.openai.com, return to localhost:1455) with a field for the address", (curl or "").startswith("https://auth.openai.com/oauth/authorize?") and "localhost%3A1455" in (curl or "") and pg.locator(".settings .gflow input.mono").count() == 1 and "1455" in pg.locator(".settings .gflow").inner_text(), (curl or "")[:90])
    pg.locator(".settings .gflow button:has-text('Отмена')").click(); time.sleep(0.3)
    gtg.locator("button.glogin").click(); pg.wait_for_selector(".menu .mi", timeout=5000); pg.locator(".menu .mi:has-text('Claude')").first.click(); pg.wait_for_selector(".settings .gflow input.mono, .settings .gflow .err", timeout=30000); time.sleep(0.5)
    clurl = pg.locator(".settings .gflow a:has-text('Открыть ссылку')").get_attribute("href") if pg.locator(".settings .gflow a:has-text('Открыть ссылку')").count() else ""
    ok("gate: Claude sign-in into the proxy is the proxy's OAuth link (return to localhost:54545) with a field for the address", "oauth/authorize" in (clurl or "") and "localhost%3A54545" in (clurl or "") and pg.locator(".settings .gflow input.mono").count() == 1, (clurl or "")[:90])
    pg.locator(".settings .gflow button:has-text('Отмена')").click(); time.sleep(0.3)
    # fallback ways through the core's flows: a device code (ChatGPT) and the page code (Claude), the sign-in file
    # would go into the proxy
    gtg.locator("button.glogin").click(); pg.wait_for_selector(".menu .mi", timeout=5000); pg.locator(".menu .mi:has-text('код устройства')").click(); pg.wait_for_selector(".settings .gflow .devcode .code, .settings .gflow .err", timeout=45000); time.sleep(0.5)
    ccode = pg.locator(".settings .gflow .devcode .code").inner_text().strip() if pg.locator(".settings .gflow .devcode .code").count() else ""
    ok("gate: the device-code fallback for ChatGPT shows the device page link and a one-time code", (pg.locator(".settings .gflow a:has-text('Открыть ссылку')").get_attribute("href") or "") == "https://auth.openai.com/codex/device" and re.fullmatch(r"[A-Z0-9]{4}-[A-Z0-9]{5}", ccode) is not None, f"code={ccode}")
    pg.locator(".settings .gflow button:has-text('Отмена')").click(); time.sleep(0.3)
    gtg.locator("button.glogin").click(); pg.wait_for_selector(".menu .mi", timeout=5000); pg.locator(".menu .mi:has-text('код со страницы')").click(); pg.wait_for_selector(".settings .gflow input[placeholder='xxxxxxxx#yyyyyyyy'], .settings .gflow .err", timeout=30000); time.sleep(0.5)
    pg.locator(".settings .gflow input.mono").fill("bogus-code#bogus-state"); pg.locator(".settings .gflow button:has-text('Завершить вход')").click(); time.sleep(4)
    ok("gate: the page-code fallback for Claude rejects a wrong code with a clear error", pg.locator(".settings .gflow .err").count() == 1, pg.locator(".settings .gflow .err").inner_text()[:100] if pg.locator(".settings .gflow .err").count() else "no error")
    pg.locator(".settings .gflow button:has-text('Отмена')").click(); time.sleep(0.3)
    gtg.locator("button.glogin").click(); pg.wait_for_selector(".menu .mi", timeout=5000); pg.locator(".menu .mi:has-text('Kimi (Moonshot)')").click(); pg.wait_for_selector(".settings .gflow .devcode .code, .settings .gflow .err", timeout=45000); time.sleep(0.5)
    kurl = pg.locator(".settings .gflow a:has-text('Открыть ссылку')").get_attribute("href") if pg.locator(".settings .gflow a:has-text('Открыть ссылку')").count() else ""; kcode = pg.locator(".settings .gflow .devcode .code").inner_text().strip() if pg.locator(".settings .gflow .devcode .code").count() else ""
    ok("gate: Kimi device sign-in through the proxy returns a verification link and a code", re.match(r"https://(www\.|auth\.)?kimi\.com/", kurl or "") is not None and len(kcode) >= 4 and pg.locator(".settings .gflow .waiting").count() == 1, f"{kurl} code={kcode} err={pg.locator('.settings .gflow .err').inner_text() if pg.locator('.settings .gflow .err').count() else ''}"[:160])
    pg.screenshot(path="shots/ap21-gate-login.png"); pg.locator(".settings .gflow button:has-text('Отмена')").click(); time.sleep(0.3)
    pg.locator(".menu").count() and pg.keyboard.press("Escape")
    gtg.locator("button.glogin").click(); pg.wait_for_selector(".menu .mi", timeout=5000); pg.locator(".menu .mi:has-text('Antigravity')").click(); pg.wait_for_selector(".settings .gflow input.mono, .settings .gflow .err", timeout=45000); time.sleep(0.5)
    aurl = pg.locator(".settings .gflow a:has-text('Открыть ссылку')").get_attribute("href") if pg.locator(".settings .gflow a:has-text('Открыть ссылку')").count() else ""
    ok("gate: Antigravity (callback flow) shows the Google link and a field for the localhost:51121 redirect address", (aurl or "").startswith("https://accounts.google.com/") and pg.locator(".settings .gflow input.mono").count() == 1 and "51121" in pg.locator(".settings .gflow").inner_text(), (aurl or "")[:80])
    pg.locator(".settings .gflow input.mono").fill("http://localhost:51121/oauth-callback?code=bogus&state=bogus"); pg.locator(".settings .gflow button:has-text('Завершить')").click(); time.sleep(6)
    ok("gate: a callback address with a foreign state is rejected with a clear error", pg.locator(".settings .gflow .err").count() == 1 and "state" in pg.locator(".settings .gflow .err").inner_text(), pg.locator(".settings .gflow .err").inner_text()[:100] if pg.locator(".settings .gflow .err").count() else "no error shown")
    pg.locator(".settings .gflow button:has-text('Отмена')").click(); time.sleep(0.3)
    gtg.locator("button.gplugins").click(); pg.wait_for_selector(".menu .mi", timeout=5000); pm = [x.split("\n")[0].strip() for x in pg.locator(".menu .mi").all_inner_texts()]; pg.keyboard.press("Escape")
    ok("gate: the plugins menu lists provider plugins from the proxy's store (Gemini CLI, GitHub Copilot) and notes that Qwen is gone", any("Gemini CLI" in x for x in pm) and any("Copilot" in x for x in pm) and any("Qwen" in x for x in pm), str(pm)[:160])
    # the ChatGPT subscription if the owner has already signed in: a Codex session with the account (CODEX_HOME) answers
    # through ChatGPT, not the gate
    cacc = [a for a in (get("/agents/api/core/accounts")[1] or {}).get("accounts", []) if a.get("provider") == "codex" and a.get("kind") == "subscription"]
    if cacc:
        cs2 = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "codex", "title": f"codex sub probe {MARK}"}).encode())[1] or b"{}")
        time.sleep(2); raw(f"/agents/api/core/projects/{P}/sessions/{cs2['id']}/account", "POST", json.dumps({"account": cacc[0]["id"]}).encode()); time.sleep(6)
        raw(f"/agents/api/core/projects/{P}/sessions/{cs2['id']}/prompt", "POST", json.dumps({"content": f"Reply with exactly: sub-{MARK}"}).encode())
        for _ in range(40):
            time.sleep(3); cst = (get(f"/agents/api/core/projects/{P}/sessions/{cs2['id']}")[1] or {}).get("info", {}).get("status")
            if cst in ("idle", "exited", "error"): break
        cev = get(f"/agents/api/core/projects/{P}/sessions/{cs2['id']}/events?since=0&limit=300")[1] or []
        croute = next((e.get("route") for e in reversed(cev) if e.get("kind") == "init"), None); ctext = " ".join(str(b.get("text", "")) for e in cev if e.get("kind") == "assistant" for b in (e.get("content") or []) if isinstance(b, dict))
        cerr = next((str(e.get("data", {}).get("message") or e.get("text") or "")[:160] for e in cev if e.get("kind") in ("error", "result") and (e.get("is_error") or e.get("kind") == "error")), "")
        ok("codex: a session pinned to the ChatGPT subscription account answers through the account route", croute == "account" and f"sub-{MARK}" in ctext, f"route={croute} status={cst} text={ctext[-60:]!r} err={cerr}")
        # slice 61: ChatGPT limits (account/rateLimits/updated) → a rate_limit_event in the Claude format → the limit chip
        # in the composer
        crl = [e for e in cev if e.get("kind") == "rate_limit_event"]
        cinfo = (crl[-1].get("data") or {}).get("rate_limit_info") if crl else None
        ok("codex: the journal carries a normalized rate_limit_event with unified windows and a status", bool(cinfo) and (crl[-1].get("data") or {}).get("source") == "codex" and cinfo.get("status") in ("allowed", "allowed_warning", "rejected") and isinstance(cinfo.get("unifiedWindows"), dict) and len(cinfo["unifiedWindows"]) >= 1, str(cinfo)[:160])
        pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.8)
        if pg.locator(f".panel .sess[data-id='{cs2['id']}']").count():
            pg.locator(f".panel .sess[data-id='{cs2['id']}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab:has-text('Чат')").click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000); time.sleep(1.5)
            chip = pg.locator(".thread .chat .composer-state .composer-limit")
            chip_txt = chip.inner_text() if chip.count() else ""
            if chip.count(): chip.click(); time.sleep(0.5)
            rows = [x.strip() for x in pg.locator(".menu .mi").all_inner_texts()] if pg.locator(".menu").count() else []
            ok("codex: the composer shows the live limit chip (busiest window with a percentage) and its menu lists the windows and a way to the accounts", chip.count() == 1 and "%" in chip_txt and any("%" in r for r in rows) and any("Аккаунты" in r for r in rows), f"chip={chip_txt!r} rows={rows[:4]}")
            pg.keyboard.press("Escape")
        else: ok("codex: the composer shows the live limit chip (busiest window with a percentage) and its menu lists the windows and a way to the accounts", False, "codex session not in the tree")
        # the block continues working in Settings → Accounts
        pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Аккаунты')").click(); time.sleep(0.8)
        for _ in range(3):
            if raw(f"/agents/api/core/projects/{P}/sessions/{cs2['id']}", "DELETE")[0] in (204, 404): break
            time.sleep(3)
    else: print("SKIP  codex: no ChatGPT subscription account on the stand — account route not exercised", flush=True)
    st_gm, gmod = get(f"/agents/api/core/gate/models?project={P}")
    ok("gate: /gate/models lists the proxy models of the project's llm provider (or all targets)", st_gm == 200 and isinstance(gmod, dict) and "targets" in gmod, json.dumps(gmod)[:120] if gmod else str(st_gm))
    st_ga, gall = get("/agents/api/core/gate/models")
    gms = [m for tg in (gall or {}).get("targets", []) for m in tg.get("models", [])]
    provs = {m.get("provider") for m in gms}
    ok("gate: model ids are real (no claude-fable-5-dd- cloaking) and every model carries its channel; Claude Code and Codex channels are present", st_ga == 200 and gms and not any(m["id"].startswith("claude-fable-5-dd-") for m in gms) and all(m.get("provider") and m.get("provider_title") for m in gms) and {"claude", "codex"} <= provs and any(m["id"] == "gpt-6-astra" and m["provider"] == "codex" for m in gms), str(sorted(p for p in provs if p))[:160])
    ext = next((tg["models"] for tg in (gall or {}).get("targets", []) if any(m.get("provider") == "claude" for m in tg.get("models", [])) and any(m.get("provider") == "antigravity" for m in tg.get("models", []))), [])
    if ext: ok("gate: a model served by two channels is listed under both (Sonnet 4.6 under Claude Code and Antigravity)", {m["provider"] for m in ext if m["id"] == "claude-sonnet-4-6"} >= {"claude", "antigravity"}, str([m["provider"] for m in ext if m["id"] == "claude-sonnet-4-6"]))
    pg.locator(".settings button:has-text('войти в подписку')").click(); pg.wait_for_selector(".settings input[name='lw'][value='term']", timeout=5000); pg.locator(".settings .frm select").first.select_option("claude"); pg.locator(".settings input[name='lw'][value='term']").check(); pg.wait_for_selector(".settings button:has-text('Открыть терминал входа')", timeout=5000); pg.locator(".settings button:has-text('Открыть терминал входа')").click()
    pg.wait_for_selector(".settings .login-term .xterm", timeout=30000); seen = ""
    for _ in range(40):
        seen = pg.locator(".settings .login-term .xterm-rows").inner_text() if pg.locator(".settings .login-term .xterm-rows").count() else ""
        if "http" in seen or "code" in seen.lower(): break
        time.sleep(1)
    ok("login: terminal runs claude auth login and prints the sign-in link", "http" in seen or "claude.ai" in seen, " ".join(seen.split())[:160]); pg.screenshot(path="shots/ap8-login.png")
    pg.locator(".settings input[placeholder='код из браузера']").fill(f"paste-{MARK}"); pg.locator(".settings button:has-text('Вставить в терминал')").click(); time.sleep(2)
    seen2 = pg.locator(".settings .login-term .xterm-rows").inner_text() if pg.locator(".settings .login-term .xterm-rows").count() else ""
    ok("login: the paste field delivers the code into the login terminal", f"paste-{MARK}" in seen2 or "Invalid code" in seen2, " ".join(seen2.split())[-120:])
    pg.locator(".settings button:has-text('Импортировать вход')").click(); time.sleep(2)
    ok("login: importing before sign-in reports a clear error", pg.locator(".settings .err:has-text('.credentials.json')").count() == 1)
    pg.locator(".settings button:has-text('Отмена')").click(); time.sleep(1.5)
    ok("login: cancel closes the login terminal", pg.locator(".settings .login-term").count() == 0)
    pg.locator(".rail .nav[data-tip='Входящие']").click(); pg.wait_for_selector(".inbox", timeout=20000); ok("ui: inbox renders", pg.locator(".inbox .h-sec").count() == 2); pg.screenshot(path="shots/ap3-inbox.png")
    pg.locator(".rail .nav[data-tip='Модели']").click(); pg.wait_for_selector(".kpis", timeout=20000); time.sleep(1); ok("ui: models with KPIs, routes and table", pg.locator(".kpi").count() == 4 and pg.locator(".routes .route").count() >= 1 and pg.locator(".tbl.models tbody tr").count() >= 1); pg.screenshot(path="shots/ap4-models.png")
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); ok("ui: settings with ten tabs (accounts … MCP … browser … about)", pg.locator(".settings .subnav button").count() == 10, str(pg.locator(".settings .subnav button").count()))
    pg.locator(".settings .subnav button:has-text('Внешний вид')").click(); pg.locator("input[name='appearance-mode'][value='light']").check(force=True); time.sleep(0.5)
    ok("ui: light theme applied", pg.evaluate("document.documentElement.dataset.theme") == "light"); pg.screenshot(path="shots/ap5-settings-light.png")
    # multilingual UI (i18n.ts + i18n/messages/app.ts): switching in "Appearance" is reactive, without a reload; radio order: auto, en, ru, uk, es
    pg.locator("input[name='appearance-language'][value='en']").check(force=True); time.sleep(0.8)
    ok("i18n: English — rail tooltips, title, settings tabs and <html lang> switch without a reload", pg.locator(".rail .nav[data-tip='Settings']").count() == 1 and pg.locator(".settings .subnav button:has-text('Appearance')").count() == 1 and pg.locator(".top h1").inner_text().strip() == "Settings" and pg.evaluate("document.documentElement.lang") == "en", pg.locator(".top h1").inner_text())
    pg.locator(".rail .nav[data-tip='Agents']").click(); time.sleep(0.8); pg.locator(".top .seg button:has-text('Agents')").click(); time.sleep(0.5)  # the section remembers the tab — go back to "Agents"
    ok("i18n: Agents section in English (tabs, button, empty state)", pg.locator(".top .seg button:has-text('Routines')").count() == 1 and pg.locator(".top h1").inner_text().strip() == "Agents" and pg.locator(".top button.primary:has-text('New agent')").count() == 1 and (pg.locator(".content:has-text('No agents yet')").count() == 1 or pg.locator(".rt-item").count() >= 1), pg.locator(".content").inner_text()[:80]); pg.screenshot(path="shots/ap11-en-agents.png")
    pg.locator(".rail .nav[data-tip='Workspaces']").click(); time.sleep(0.8)
    ok("i18n: workspaces panel and thread header in English", pg.locator(".panel .p-head h2").inner_text().strip() == "Workspaces" and pg.locator(".top .seg button:has-text('Thread')").count() == 1)
    pg.locator(".rail .nav[data-tip='Settings']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Appearance')").click(); pg.locator("input[name='appearance-language'][value='uk']").check(force=True); time.sleep(0.8)
    ok("i18n: Ukrainian dictionary", pg.locator(".rail .nav[data-tip='Налаштування']").count() == 1 and pg.locator(".top h1").inner_text().strip() == "Налаштування" and pg.evaluate("document.documentElement.lang") == "uk")
    pg.locator("input[name='appearance-language'][value='es']").check(force=True); time.sleep(0.8)
    ok("i18n: Spanish dictionary", pg.locator(".rail .nav[data-tip='Ajustes']").count() == 1 and pg.locator(".top h1").inner_text().strip() == "Ajustes" and pg.locator(".settings .subnav button:has-text('Apariencia')").count() == 1)
    pg.reload(wait_until="load"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(1.2)
    ok("i18n: the chosen language survives a reload", pg.locator(".rail .nav[data-tip='Ajustes']").count() == 1 and pg.evaluate("document.documentElement.lang") == "es")
    pg.locator(".rail .nav[data-tip='Ajustes']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Apariencia')").click(); pg.locator("input[name='appearance-language'][value='auto']").check(force=True); time.sleep(0.8)
    ok("i18n: auto follows the browser locale (ru-RU) again", pg.locator(".rail .nav[data-tip='Настройки']").count() == 1 and pg.evaluate("document.documentElement.lang") == "ru" and pg.locator("input[name='appearance-language'][value='auto']").is_checked())
    pg.locator("input[name='appearance-language'][value='ru']").check(force=True); time.sleep(0.4)
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab:has-text('Чат')").click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000); time.sleep(1)
    lum = "e => { const c = (getComputedStyle(e).backgroundColor.match(/[\\d.]+/g) || [0,0,0]).map(Number); return (0.2126*c[0] + 0.7152*c[1] + 0.0722*c[2]) / 255; }"
    pg.locator(".thread .chat textarea").fill("/"); time.sleep(0.6); pl = pg.locator(".thread .chat .popup").evaluate(lum); pg.keyboard.press("Escape"); pg.locator(".thread .chat textarea").fill("")
    pg.locator(".thread .chat .composer .ctl.model").click(); time.sleep(0.6); ml = pg.locator(".menu").evaluate(lum); pg.keyboard.press("Escape"); cl = pg.locator(".thread .chat .composer").evaluate(lum)
    composer_modes(pg); time.sleep(0.5); pl2 = pg.locator(".thread .chat .modes").evaluate(lum); pg.keyboard.press("Escape")
    ok("ui: light theme styles the popup, menu, modes popover and composer", pl > 0.7 and ml > 0.7 and cl > 0.7 and pl2 > 0.7, f"popup {pl:.2f} menu {ml:.2f} modes {pl2:.2f} composer {cl:.2f}"); pg.screenshot(path="shots/ap5b-light-chat.png")
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Внешний вид')").click(); pg.locator("input[name='appearance-mode'][value='dark']").check(force=True); time.sleep(0.5)
    # the account switch in the session's "⋯" menu (card, tree, Thread, chat): the project gate and the provider accounts, the
    # current one marked; choosing restarts the session with the same history (even a finished one)
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.8); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .t-head", timeout=20000)
    pg.locator(".thread .t-head button.icon.sm.ghost").last.click(); pg.wait_for_selector(".menu", timeout=5000)
    acc_items = [x.strip() for x in pg.locator(".menu .mi").all_inner_texts()]
    ok("account: session menu has an account section with the project gate checked and provider accounts listed", pg.locator(".menu .mh:has-text('Аккаунт')").count() == 1 and pg.locator(".menu .mi.on:has-text('gate проекта')").count() == 1 and all("codex" not in x.lower() for x in acc_items), str(acc_items)[:160])
    pg.locator(".menu .mi:has-text('gate проекта')").click(); time.sleep(6)
    acc_after = get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}
    ok("account: choosing an account restarts the session with the same history", acc_after.get("info", {}).get("alive") is True and acc_after["info"].get("account") is None and acc_after["info"].get("provider_session_id") == sid and pg.locator(".toast:has-text('перезапущена')").count() >= 0, str({k: acc_after.get("info", {}).get(k) for k in ("alive", "account")}))
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); pg.locator(".top .seg button:has-text('Code')").click(); time.sleep(0.8)
    card_more = pg.locator(f".card[data-card='{P}:{sid}'] .c-head .acts button").nth(3)
    # the canvas is shared with the owner: foreign cards overlap ours, a regular click is intercepted — the event is sent to the
    # button directly
    if card_more.count(): card_more.dispatch_event("click"); pg.wait_for_selector(".menu", timeout=5000); ok("account: the canvas card menu carries the same account section", pg.locator(".menu .mh:has-text('Аккаунт')").count() == 1); pg.keyboard.press("Escape")
    else: ok("account: the canvas card menu carries the same account section", False, "card not on canvas")
    pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5)
    # account pool and auto-switching on limits (docs/agents-ux.md §9.7): two proxy accounts from the project gate in
    # the pool, a session on A, a simulated limit hit (the agentd test endpoint) → the core blocks A until resets_at,
    # moves the session to B, writes a note into the conversation and repeats the last request; in the settings — the
    # "in pool" toggle and the block line
    def pinfo_(x): return (get(f"/agents/api/core/projects/{P}/sessions/{x}")[1] or {}).get("info", {})
    def pevents_(x, since=0): return get(f"/agents/api/core/projects/{P}/sessions/{x}/events?since={since}&limit=600")[1] or []
    pa = json.loads(raw("/agents/api/core/accounts/import", "POST", json.dumps({"provider": "gate", "project": P, "label": f"e2e pool A {MARK}"}).encode())[1] or b"{}")
    pb = json.loads(raw("/agents/api/core/accounts/import", "POST", json.dumps({"provider": "gate", "project": P, "label": f"e2e pool B {MARK}"}).encode())[1] or b"{}")
    pool_ids = [x.get("id") for x in (pa, pb) if x.get("id")]
    pst = [raw(f"/agents/api/core/accounts/{x}", "PATCH", json.dumps({"pool": True}).encode())[0] for x in pool_ids]
    pacc = {a["id"]: a for a in (get("/agents/api/core/accounts")[1] or {}).get("accounts", [])}
    ok("pool: two proxy accounts imported from the project gate and switched into the pool via PATCH", len(pool_ids) == 2 and pst == [200, 200] and all(pacc.get(x, {}).get("pool") is True for x in pool_ids), f"{pst} {[pacc.get(x, {}).get('pool') for x in pool_ids]}")
    if len(pool_ids) == 2:
        raw(f"/agents/api/core/projects/{P}/sessions/{sid}/account", "POST", json.dumps({"account": pa["id"]}).encode())
        for _ in range(30):
            time.sleep(1)
            if pinfo_(sid).get("alive") and pinfo_(sid).get("account") == pa["id"] and pinfo_(sid).get("status") == "idle": break
        raw(f"/agents/api/core/projects/{P}/sessions/{sid}/prompt", "POST", json.dumps({"content": f"Reply with exactly: pool-{MARK}"}).encode())
        for _ in range(90):
            time.sleep(1)
            if pinfo_(sid).get("status") == "idle" and f"pool-{MARK}" in (pinfo_(sid).get("last_text") or ""): break
        seq0 = pinfo_(sid).get("last_seq", 0)
        ok("pool: the session runs on pool account A and answered the prompt that will be retried", pinfo_(sid).get("account") == pa["id"] and f"pool-{MARK}" in (pinfo_(sid).get("last_text") or ""), str({k: pinfo_(sid).get(k) for k in ("account", "status", "last_text")})[:160])
        reset_at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() + 240))
        hst, hb = raw(f"/agents/api/core/projects/{P}/sessions/{sid}/limit_hit", "POST", json.dumps({"window": "five_hour", "resets_at": reset_at}).encode())
        ok("pool: the test hook marks the session with a limit hit", hst == 200 and (json.loads(hb or b"{}").get("limit_hit") or {}).get("window") == "five_hour", f"{hst} {(hb or b'')[:120]}")
        for _ in range(60):
            time.sleep(1)
            if pinfo_(sid).get("account") == pb["id"] and pinfo_(sid).get("alive"): break
        pblocked = (get("/agents/api/core/accounts")[1] or {}).get("blocked", {})
        ok("pool: the core blocked account A until resets_at and moved the session to pool account B (same history)", pinfo_(sid).get("account") == pb["id"] and pinfo_(sid).get("alive") is True and pinfo_(sid).get("provider_session_id") == sid and pblocked.get(pa["id"], "")[:16] == reset_at[:16] and not pinfo_(sid).get("limit_hit"), f"account={pinfo_(sid).get('account') == pb['id']} blocked={pblocked.get(pa['id'])} hit={pinfo_(sid).get('limit_hit')}")
        for _ in range(90):
            time.sleep(1)
            pev = pevents_(sid, seq0)
            if any(e["kind"] == "result" and f"pool-{MARK}" in (e.get("text") or "") for e in pev) and pinfo_(sid).get("status") == "idle": break
        pev = pevents_(sid, seq0)
        pnotes = [e.get("data", {}).get("message", "") for e in pev if e["kind"] == "system" and e.get("subtype") == "agent_note"]
        pusers = [" ".join(str(b.get("text", "")) for b in (e.get("content") or []) if isinstance(b, dict)) for e in pev if e["kind"] == "user" and not e.get("parent_tool_use_id")]
        presults = [e for e in pev if e["kind"] == "result" and not e.get("is_error")]
        ok("pool: the chat got the limit note, the switch note «аккаунт: A → B, лимит five_hour исчерпан, сброс через …» and the last request was resent and answered on B", any("лимит подписки исчерпан" in n for n in pnotes) and any(n.startswith(f"аккаунт: e2e pool A {MARK} → e2e pool B {MARK}, лимит five_hour исчерпан, сброс через ") and "UTC)" in n for n in pnotes) and len(pusers) >= 1 and f"pool-{MARK}" in pusers[-1] and any(f"pool-{MARK}" in (e.get("text") or "") for e in presults), f"notes={pnotes[:3]} users={len(pusers)} results={len(presults)}")
        pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.8); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab:has-text('Чат')").click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000); time.sleep(1.5)
        ok("pool: the switch note is visible in the chat", pg.locator(f".thread .chat :text('аккаунт: e2e pool A {MARK} → e2e pool B {MARK}')").count() >= 1)
        pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Аккаунты')").click(); pg.wait_for_selector(".settings .acct-card", timeout=20000); time.sleep(1)
        ca = pg.locator(f".settings .acct-card[data-acc='{pa['id']}']"); cb = pg.locator(f".settings .acct-card[data-acc='{pb['id']}']")
        ok("pool: account cards show the «в пуле» switch on for both and the block line on A until the reset time", ca.locator(".pool-switch.on").count() == 1 and cb.locator(".pool-switch.on").count() == 1 and ca.locator(".pool-blocked").count() == 1 and "заблокирован до" in ca.locator(".pool-blocked").inner_text() and cb.locator(".pool-blocked").count() == 0, (ca.locator(".pool-blocked").inner_text() if ca.locator(".pool-blocked").count() else "no block line"))
        cb.locator(".pool-switch").click(); time.sleep(1.2)
        pacc2 = {a["id"]: a for a in (get("/agents/api/core/accounts")[1] or {}).get("accounts", [])}
        ok("pool: the switch takes account B out of the pool", pg.locator(".toast:has-text('вне пула')").count() >= 1 and pacc2.get(pb["id"], {}).get("pool") is False and cb.locator(".pool-switch.on").count() == 0)
        pg.screenshot(path="shots/ap-pool.png")
        # proactive substitution at start: the project default is the blocked A, a new session starts on B with a note (B returned to
        # the pool)
        raw(f"/agents/api/core/accounts/{pb['id']}", "PATCH", json.dumps({"pool": True}).encode())
        pdef0 = (get("/agents/api/core/accounts")[1] or {}).get("defaults", {}).get(P)
        raw(f"/agents/api/core/projects/{P}/account", "PUT", json.dumps({"account": pa["id"]}).encode())
        pro = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"pool probe {MARK}"}).encode())[1] or b"{}")
        time.sleep(3); pro_i = pinfo_(pro.get("id", "")); pro_notes = [e.get("data", {}).get("message", "") for e in pevents_(pro.get("id", "")) if e["kind"] == "system" and e.get("subtype") == "agent_note"]
        ok("pool: a new session whose project default is the blocked account A starts on B with a note", pro_i.get("account") == pb["id"] and pro_i.get("alive") is True and any(n.startswith(f"аккаунт: e2e pool A {MARK} → e2e pool B {MARK}, лимит исчерпан, сброс через ") for n in pro_notes), f"account=B:{pro_i.get('account') == pb['id']} notes={pro_notes[:2]}")
        raw(f"/agents/api/core/projects/{P}/account", "PUT", json.dumps({"account": pdef0}).encode())
        if pro.get("id"):
            for _ in range(5):   # the first DELETE stops the process, the second removes the record
                if raw(f"/agents/api/core/projects/{P}/sessions/{pro['id']}", "DELETE")[0] == 204: break
                time.sleep(2)
        raw(f"/agents/api/core/projects/{P}/sessions/{sid}/account", "POST", json.dumps({"account": None}).encode()); time.sleep(4)
    for x in pool_ids: raw(f"/agents/api/core/accounts/{x}", "DELETE")
    ok("pool: cleanup — the session is back on the project gate, pool accounts deleted and A unblocked", pinfo_(sid).get("account") is None and not any(a["id"] in pool_ids for a in (get("/agents/api/core/accounts")[1] or {}).get("accounts", [])) and pa.get("id") not in ((get("/agents/api/core/accounts")[1] or {}).get("blocked", {})), str({k: pinfo_(sid).get(k) for k in ("account", "alive")}))
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.8); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .t-head", timeout=20000); pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5)
    ok("ui: no page errors", not errors, "; ".join(errors)[:200])
    dp = b.new_page(); dp.goto(f"https://{HOST}/?app=agents", wait_until="domcontentloaded"); dp.wait_for_selector(".agents .sess", timeout=30000); time.sleep(3)
    ok("desktop: Agents window lists workspace conversations too", dp.locator(".agents .sess").count() >= 3 and dp.locator(f".agents .sess:has-text('{KNOWN_WORD}')").count() >= 1, str(dp.locator(".agents .sess").count())); dp.screenshot(path="shots/ap7-desktop-agents.png"); dp.close()
    ph = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 390, "height": 844}, device_scale_factor=2, is_mobile=True, has_touch=True); ph.add_init_script("try { if (!localStorage.getItem('agents.lang')) localStorage.setItem('agents.lang', 'ru'); } catch (e) {}"); pp = ph.new_page()
    pp.goto(f"https://{HOST}/agents/", wait_until="load"); pp.wait_for_selector(".tabbar", timeout=30000); time.sleep(2)
    ok("phone: bottom tab bar and thread view", pp.locator(".tabbar .tb").count() == 5 and pp.locator(".thread, .empty").count() >= 1); pp.screenshot(path="shots/ap6-phone.png")
    pp.locator(".tabbar .tb").nth(0).click(); time.sleep(0.8)
    if pp.locator(".burger").count(): pp.locator(".burger").first.click(); time.sleep(0.8)
    pp.locator(f".panel .sess[data-id='{sid}']").click(force=True); pp.wait_for_selector(".thread .pane .tabs", timeout=20000); pp.locator(".thread .pane .tabs .tab").first.click(); pp.wait_for_selector(".thread .chat .tools", timeout=20000); time.sleep(2.5)
    pcomposer = pp.locator(".thread .chat .composer"); ptools = pcomposer.locator(".composer-controls")
    try: pp.wait_for_selector(".thread .chat .composer-context", timeout=20000)
    except Exception: pass
    # On the phone the model and "Options" stay in the bottom strip (the row is compacted to 32 px, targets ≥ 44 px wide —
    # slice 45); status, mode and context are text above the field, their clickable duplicates are hidden, details in the
    # "Options" menu.
    th = ptools.evaluate("e => e.getBoundingClientRect().height"); vis_ctl = [i for i in range(ptools.locator(".ctl").count()) if ptools.locator(".ctl").nth(i).is_visible()]
    more_w = (ptools.locator(".ctl.more").bounding_box() or {"width": 0})["width"]
    ok("phone: model and Parameters fit one compact row (32 px, targets ≥ 44 px wide); permission mode and context stay above the field as text", pcomposer.locator(".composer-compact-mode").is_visible() and pcomposer.locator(".composer-compact-mode").inner_text().strip() != "" and pcomposer.locator(".composer-short-context").is_visible() and "контекст" in pcomposer.locator(".composer-short-context").inner_text() and not pcomposer.locator(".composer-mode-summary").is_visible() and not pcomposer.locator(".composer-state .composer-context").is_visible() and ptools.locator(".ctl.model").is_visible() and ptools.locator(".ctl.more").is_visible() and pcomposer.locator(".chip.cache").count() == 0 and 30 <= th <= 36 and len(vis_ctl) == 2 and more_w >= 44, f"tools height {th:.0f} visible controls {len(vis_ctl)} more width {more_w:.0f}")
    ptools.locator(".ctl.more").click(); time.sleep(0.5); more_items = [x.split("\n")[0].strip() for x in pp.locator(".menu .mi").all_inner_texts()]
    ok("phone: Parameters offers the permission mode, skills, MCP, the context and the cache line", any("Режим разрешений" in x for x in more_items) and any("навыки" in x for x in more_items) and any("MCP" in x for x in more_items) and any("Контекст" in x for x in more_items) and any("кэш" in x for x in more_items), str(more_items)[:200]); pp.keyboard.press("Escape"); time.sleep(0.3)
    ta_h = pp.evaluate("(() => { const t = document.querySelector('.thread .chat textarea'); const b = document.querySelector('.thread .chat button.send'); const r = b.getBoundingClientRect(); return { fs: parseFloat(getComputedStyle(t).fontSize), bh: r.height, bw: r.width }; })()")
    ok("phone: the field is 16px (no iOS auto-zoom) and the send button is as tall as the compact row and ≥ 44 px wide", ta_h["fs"] >= 16 and 30 <= ta_h["bh"] <= 36 and ta_h["bw"] >= 44, str(ta_h))
    pp.locator(".thread .chat textarea").click(); pp.locator(".thread .chat textarea").fill("/"); time.sleep(0.8)
    ok("phone: slash popup opens above the input", pp.locator(".thread .chat .popup .pi").count() >= 3 and pp.locator(".thread .chat .popup").evaluate("e => e.getBoundingClientRect().bottom <= innerHeight")); pp.keyboard.press("Escape"); pp.locator(".thread .chat textarea").fill("")
    ptools.locator(".ctl.model").click(); time.sleep(0.8)
    ok("phone: model menu opens within the viewport", pp.locator(".menu").is_visible() and pp.locator(".menu").evaluate("e => { const r = e.getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth + 1 && r.bottom <= innerHeight + 1; }")); pp.keyboard.press("Escape"); pp.screenshot(path="shots/ap6b-phone-composer.png")
    # phone, terminal (docs/agents-ux.md §4.3.7): 16px fields against iOS auto-zoom, 11px font, input, fitting with the "keyboard" (a reduced
    # viewport)
    pp.keyboard.press("Escape"); time.sleep(0.4)
    ok("phone: inputs are 16px so iOS does not zoom on focus", pp.locator(".thread .chat textarea").evaluate("e => getComputedStyle(e).fontSize") == "16px")
    pp.locator(".thread .pane .tabs .tab").nth(2).click(); pp.wait_for_selector(".thread .pane .term .xterm-rows", timeout=30000); time.sleep(2)
    fs0 = pp.locator(".thread .pane .term .xterm-rows").evaluate("e => getComputedStyle(e).fontSize")
    ok("phone: terminal font is 11px on the phone and the hidden xterm textarea is 16px", fs0 == "11px" and pp.locator(".thread .pane .term .xterm-helper-textarea").evaluate("e => getComputedStyle(e).fontSize") == "16px", fs0)
    rows0 = pp.locator(".thread .pane .term .xterm-rows > div").count()
    pp.locator(".thread .pane .term .xterm").click(); time.sleep(0.4); pp.keyboard.type(f"echo ph-{MARK}"); pp.keyboard.press("Enter"); time.sleep(3)
    txt = " ".join(pp.locator(".thread .pane .term .xterm-rows").inner_text().split())
    ok("phone: typing into the terminal reaches the shell and the output is on screen", txt.count(f"ph-{MARK}") >= 2, txt[-120:])
    pp.set_viewport_size({"width": 390, "height": 520}); time.sleep(1.5)
    rows1 = pp.locator(".thread .pane .term .xterm-rows > div").count(); tb = pp.locator(".thread .pane .term").evaluate("e => e.getBoundingClientRect().bottom")
    ok("phone: a shorter viewport (keyboard) refits the terminal — fewer rows, bottom stays on screen", rows1 < rows0 and tb <= 521, f"rows {rows0}→{rows1} bottom={tb:.0f}"); pp.screenshot(path="shots/ap14-phone-term.png")
    pp.set_viewport_size({"width": 390, "height": 844}); time.sleep(1)
    # ---- subscription limits (slice 40, docs/agents-ux.md §9.2): the core asks the providers with the account tokens (refreshing expired ones), windows 0–100 with a reset in the future; a line under the
    # account in "Settings"
    st_l, lim = get("/agents/api/core/accounts/limits?refresh=1"); lims = (lim or {}).get("limits", [])
    subs = [a for a in (get("/agents/api/core/accounts")[1] or {}).get("accounts", []) if a.get("kind") == "subscription"]
    def lim_ok(l):
        if l.get("error"): return False
        ws = l.get("windows") or []
        return bool(ws) and all(0 <= float(w.get("used_pct", -1)) <= 100 and (not w.get("resets_at") or w["resets_at"] > time.strftime("%Y-%m-%dT%H:%M", time.gmtime())) for w in ws)
    good = [l for l in lims if not l.get("error")]; bad = [l for l in lims if l.get("error")]
    by_prov = {l["provider"]: l for l in good}
    ok("limits: the core returns an entry for every subscription account — provider-shaped windows (Claude: five_hour/seven_day, ChatGPT: primary/secondary) in 0–100 with a future reset and a plan; a revoked login gets an honest «sign in again» error instead",
       st_l == 200 and len(lims) == len(subs) and len(good) >= 1 and all(lim_ok(l) and l.get("plan") for l in good) and all("войдите" in l["error"] for l in bad)
       and (not by_prov.get("claude") or {w["id"] for w in by_prov["claude"]["windows"]} >= {"five_hour", "seven_day"}) and (not by_prov.get("codex") or {w["id"] for w in by_prov["codex"]["windows"]} >= {"primary", "secondary"})
       and not any(w["id"].startswith("extra") for l in good for w in l["windows"]),
       json.dumps([{k: v for k, v in l.items() if k not in ("account", "credits")} for l in lims], ensure_ascii=False)[:400])
    st_c, lim_c = get("/agents/api/core/accounts/limits")
    ok("limits: the second call is served from the 60 s cache with the same fetched_at", st_c == 200 and lim_c and [l.get("fetched_at") for l in lim_c.get("limits", [])] == [l.get("fetched_at") for l in lims])
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Аккаунты')").first.click(); time.sleep(1)
    try: pg.wait_for_selector(".settings .acct-card .lrow, .settings .acct-card .lerr", timeout=45000)
    except Exception: pass
    n_cards = pg.locator(".settings .acct-card").count(); n_bars = pg.locator(".settings .acct-card .lrow .bar").count(); txt = " ".join(pg.locator(".settings .acct-card .lrow").all_inner_texts())
    n_err = pg.locator(".settings .acct-card .lerr").count(); n_plan = pg.locator(".settings .acct-card .badges .chip.plan").count()
    ok("limits: Settings → Accounts is a card per account (§9.3) — plan badge, a row per window «label · bar · % · reset», an error with «sign in again» for a revoked login, per-card refresh and delete buttons",
       n_cards == len((get("/agents/api/core/accounts")[1] or {}).get("accounts", [])) and n_bars >= 2 and ("сброс" in txt or "сбрасывается" in txt) and "%" in txt and n_err == len(bad) and n_plan >= len(good) and pg.locator(".settings .acct-card button[title='обновить лимиты']").count() >= len(subs) and (n_err == 0 or pg.locator(".settings .acct-card .lerr button:has-text('войти заново')").count() == n_err),
       f"cards={n_cards} bars={n_bars} err={n_err} plans={n_plan} text={txt[:140]}")
    pg.screenshot(path="shots/ap15-limits.png")
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1)
    # ---- spending limits in action (slice 67, docs/agents-ux.md §8.3): the service mirrors settings.limits into the core, the core refuses a turn on "stop", the bar in the composer, a routine skips
    # the run ----
    s0 = get("/agents/api/settings")[1] or {}; lim0 = s0.get("limits", [])
    spent_p = (get(f"/agents/api/core/usage?days=1")[1] or {}).get("by_project", []); spent_p = next((b.get("cost_usd", 0) for b in spent_p if b.get("key") == P), 0)
    tiny = max(0.0001, round(float(spent_p) / 2, 4)) if spent_p else 0.0001   # the limit is deliberately below the 24 h usage
    raw("/agents/api/settings", "PUT", json.dumps({"limits": [l for l in lim0 if l.get("scope") != P] + [{"scope": P, "per_day": tiny, "action": "stop"}]}).encode())
    bst = get("/agents/api/core/agents/budgets/state")[1] or []; bmine = next((b for b in bst if b.get("scope") == P), {})
    ok("budget: a stop limit saved in «Модели» reaches the core as a budget and its state shows the 24 h spend and «blocked»", bmine.get("action") == "stop" and bmine.get("blocked") is True and bmine.get("spent", 0) >= tiny, str(bmine)[:160])
    st_b, body_b = raw(f"/agents/api/core/projects/{P}/sessions/{sid}/prompt", "POST", json.dumps({"content": f"Reply with exactly: budget-{MARK}"}).encode())
    berr = (json.loads(body_b or b"{}").get("error") or "") if body_b else ""
    ok("budget: a turn is refused with 402 and a clear message naming the limit, the spend and «Модели»", st_b == 402 and "дневной лимит" in berr and "исчерпан" in berr and "Моделях" in berr, f"{st_b} {berr[:120]}")
    st_q, body_q = raw(f"/agents/api/core/projects/{P}/sessions/{sid}/queue", "POST", json.dumps({"content": f"queued-budget-{MARK}"}).encode())
    ok("budget: the queue is refused the same way", st_q == 402, f"{st_q}")
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab:has-text('Чат')").click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000); time.sleep(1.2)
    bn = pg.locator(".thread .chat .composer-notice.budget")
    ok("budget: the composer shows the red stop notice with the spend, the limit and a button to «Модели»", bn.count() == 1 and bn.first.evaluate("e => e.classList.contains('bad')") and "исчерпан" in bn.first.inner_text() and bn.first.locator("button:has-text('Модели')").count() == 1, bn.first.inner_text()[:140] if bn.count() else "no notice")
    pg.locator(".thread .chat textarea").fill(f"budget-ui-{MARK}"); pg.keyboard.press("Enter"); time.sleep(2)
    ok("budget: sending from the composer surfaces the core's refusal as the composer error", pg.locator(".thread .chat .composer-error:has-text('дневной лимит')").count() == 1 and f"budget-ui-{MARK}" not in " ".join(x for x in utexts(sevents(sid))) if "utexts" in globals() else pg.locator(".thread .chat .composer-error:has-text('дневной лимит')").count() == 1, (pg.locator(".thread .chat .composer-error").first.inner_text()[:100] if pg.locator(".thread .chat .composer-error").count() else "no error"))
    pg.locator(".thread .chat textarea").fill("")
    # "warn": the turn goes through, an event about the excess in the core feed,
    # the bar is yellow
    raw("/agents/api/settings", "PUT", json.dumps({"limits": [l for l in lim0 if l.get("scope") != P] + [{"scope": P, "per_day": tiny, "action": "warn"}]}).encode()); time.sleep(0.5)
    st_w, _ = raw(f"/agents/api/core/projects/{P}/sessions/{sid}/prompt", "POST", json.dumps({"content": f"Reply with exactly: budget-warn-{MARK}"}).encode())
    for _ in range(90):
        time.sleep(1); i_w = (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {})
        if i_w.get("status") == "idle" and f"budget-warn-{MARK}" in (i_w.get("last_text") or ""): break
    evs_w = get("/agents/api/core/events?limit=40")[1] or []
    bev = [e for e in (evs_w if isinstance(evs_w, list) else evs_w.get("events", [])) if "дневной лимит превышен" in json.dumps(e, ensure_ascii=False)]
    time.sleep(1.5); bn2 = pg.locator(".thread .chat .composer-notice.budget")
    ok("budget: with «предупредить» the turn goes through, the core writes a feed event once and the composer notice turns yellow", st_w == 200 and f"budget-warn-{MARK}" in (i_w.get("last_text") or "") and len(bev) >= 1 and bn2.count() == 1 and not bn2.first.evaluate("e => e.classList.contains('bad')"), f"status={st_w} answered={f'budget-warn-{MARK}' in (i_w.get('last_text') or '')} events={len(bev)} notice={bn2.count()}")
    # a routine on "stop": the run is skipped with a reason (no
    # turn)
    raw("/agents/api/settings", "PUT", json.dumps({"limits": [l for l in lim0 if l.get("scope") != P] + [{"scope": P, "per_day": tiny, "action": "stop"}]}).encode()); time.sleep(0.5)
    bag = json.loads(raw("/agents/api/agents", "POST", json.dumps({"name": f"budget agent {MARK}", "brief": "Отвечай одним словом.", "provider": "claude", "workspace": P, "mode": "auto"}).encode())[1] or b"{}")
    brun = (json.loads(raw(f"/agents/api/agents/{bag['id']}/run", "POST", json.dumps({"prompt": "скажи: привет"}).encode())[1] or b"{}").get("run") or {}) if bag.get("id") else {}
    time.sleep(2); brun2 = next((x for x in (get("/agents/api/runs?limit=50")[1] or {}).get("runs", []) if x.get("id") == brun.get("id")), brun)
    ok("budget: an agent run under a stop limit is skipped with the limit as the reason instead of failing on its first turn", brun2.get("status") == "skipped" and "дневной лимит" in (brun2.get("reason") or ""), f"{brun2.get('status')} {str(brun2.get('reason'))[:100]}")
    if brun.get("id"): raw(f"/agents/api/runs/{brun['id']}", "DELETE")
    if bag.get("id"): raw(f"/agents/api/agents/{bag['id']}", "DELETE")
    raw("/agents/api/settings", "PUT", json.dumps({"limits": lim0}).encode())
    ok("budget: cleanup restores the owner's limits in the service and the core", (get("/agents/api/settings")[1] or {}).get("limits") == lim0 and [b.get("scope") for b in (get("/agents/api/core/agents/budgets/state")[1] or [])] == [l.get("scope") for l in lim0])
    # ---- simple chat (simplechat.rs, the "Chat" section, the "Simple" engine): core routes, the SSE stream, an image, the pages tool, incognito, usage, to advanced
    # ----
    st_rt, cr = get("/agents/api/chat/routes"); routes = [r for r in (cr or {}).get("routes", []) if r.get("models")]
    ok("schat: routes with models come from the core (gate and keys, no secrets)", st_rt == 200 and any(r["kind"] == "gate" for r in routes) and all("key" not in r and "base" not in r for r in routes), str([(r["id"], len(r["models"])) for r in routes])[:160])
    def pick_model():
        for r in routes:
            ids = [m["id"] for m in r["models"]]
            for cand in ("claude-haiku-4-5-20251001", "claude-haiku-4-5", "anthropic/claude-haiku-4.5"):
                if cand in ids: return r["id"], cand
        for r in routes:
            for m in r["models"]:
                if "haiku" in m["id"]: return r["id"], m["id"]
        return (routes[0]["id"], routes[0]["models"][0]["id"]) if routes else ("", "")
    RT, MD = pick_model()
    st_c, sc = raw("/agents/api/chats", "POST", json.dumps({"route": RT, "model": MD, "tools": {"search": False, "fetch": True}}).encode()); sc = json.loads(sc or b"{}")
    ok("schat: chat created on the chosen route and model", st_c == 200 and sc.get("id", "").startswith("ch_") and sc.get("model") == MD and sc.get("kind") == "simple" and sc.get("incognito") is False, f"{st_c} {RT} {MD} {str(sc)[:100]}")
    def sse(chat_id, body):
        st, b = raw(f"/agents/api/chats/{chat_id}/messages", "POST", json.dumps(body).encode())
        evs = []
        for l in (b or b"").decode("utf-8", "ignore").splitlines():
            if l.startswith("data:"):
                try: evs.append(json.loads(l[5:].strip()))
                except Exception: pass
        return st, evs
    st_s, evs = sse(sc["id"], {"text": f"Reply with exactly: simple-{MARK}"})
    done = next((e for e in evs if e["kind"] == "done"), None); deltas = "".join(e.get("text", "") for e in evs if e["kind"] == "delta")
    ok("schat: SSE stream — user echo, deltas, done; the reply is stored with tokens, cost and an automatic title", st_s == 200 and bool(done) and f"simple-{MARK}" in deltas and f"simple-{MARK}" in done["message"]["text"] and done["message"]["tokens_out"] > 0 and done["chat"]["title"].startswith("Reply with exactly") and any(e["kind"] == "user" for e in evs), f"{st_s} n={len(evs)} {deltas[:60]} {str(done)[:100] if done else ''}")
    st_g, cg = get(f"/agents/api/chats/{sc['id']}")
    ok("schat: history persisted (user + assistant) and the chat is listed", st_g == 200 and [m["role"] for m in cg["messages"]] == ["user", "assistant"] and any(c["id"] == sc["id"] for c in (get("/agents/api/chats")[1] or {}).get("chats", [])))
    def png_red():
        def chunk(tp, d): return struct.pack(">I", len(d)) + tp + d + struct.pack(">I", zlib.crc32(tp + d) & 0xffffffff)
        rows = b"".join(b"\x00" + b"\xdc\x1e\x1e" * 16 for _ in range(16))
        return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 16, 16, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
    st_i, evi = sse(sc["id"], {"text": "What is the dominant color of this image? Answer with exactly one English word in lowercase.", "attachments": [{"name": "red.png", "mime": "image/png", "data": base64.b64encode(png_red()).decode()}]})
    di = next((e for e in evi if e["kind"] == "done"), None); ui_ev = next((e for e in evi if e["kind"] == "user"), {}); afile = ((ui_ev.get("message") or {}).get("attachments") or [{}])[0].get("file", "")
    st_f, fbody = raw(f"/agents/api/chats/{sc['id']}/files/{afile}")
    ok("schat: an image attachment reaches the model (vision) and is served back as a file", st_i == 200 and bool(di) and "red" in di["message"]["text"].lower() and st_f == 200 and (fbody or b"").startswith(b"\x89PNG"), f"{st_i} {(di or {}).get('message', {}).get('text', '')[:60]} file={st_f}")
    st_t, evt = sse(sc["id"], {"text": "Use the fetch_url tool to fetch https://example.com and reply with the exact text of its main heading, nothing else."})
    dt = next((e for e in evt if e["kind"] == "done"), None); tools_ev = [e for e in evt if e["kind"] == "tool"]
    ok("schat: the fetch_url tool runs in the loop (tool event with the page text) and the answer uses it", st_t == 200 and bool(dt) and tools_ev and tools_ev[0]["name"] == "fetch_url" and "Example Domain" in tools_ev[0]["result"] and "example domain" in dt["message"]["text"].lower() and dt["message"]["tools"][0]["ok"] is True, f"{st_t} tools={[(e['name'], e.get('ok')) for e in tools_ev]} {(dt or {}).get('message', {}).get('text', '')[:60]}")
    cx = next((r["id"] for r in routes if any(m["id"] == "gpt-6-astra" and m.get("provider") == "codex" for m in r["models"])), None)
    if cx:
        st_cx, ccx = raw("/agents/api/chats", "POST", json.dumps({"route": cx, "model": "gpt-6-astra", "tools": {"search": False, "fetch": False}}).encode()); ccx = json.loads(ccx or b"{}")
        st_cs, evx = sse(ccx.get("id", "x"), {"text": "What is 3+4? Answer with just the number."}); dx = next((e for e in evx if e["kind"] == "done"), None)
        ok("schat: a Codex subscription model (gpt-6-astra through the gate) answers in the simple chat", st_cs == 200 and bool(dx) and "7" in dx["message"]["text"], (dx or {}).get("message", {}).get("text", "")[:80] or str([e.get("error") for e in evx if e["kind"] == "error"])[:160])
        raw(f"/agents/api/chats/{ccx.get('id')}", "DELETE")
    else: print("SKIP  schat: no gate route offers gpt-6-astra under the Codex channel", flush=True)
    ok("schat: empty message and unknown chat are rejected", raw(f"/agents/api/chats/{sc['id']}/messages", "POST", b'{"text": "  "}')[0] == 400 and raw("/agents/api/chats/ch_nope/messages", "POST", b'{"text": "hi"}')[0] == 404)
    us = get("/agents/api/core/usage?days=1")[1] or {}
    ok("schat: spend recorded in the core ledger under project «chat»", any(b.get("key") == "chat" and (b.get("cost_usd") or 0) >= 0 and (b.get("turns") or b.get("output") or 0) > 0 for b in us.get("by_project", [])), str([b.get("key") for b in us.get("by_project", [])])[:120])
    # second pass (slice 37, §6.5): system prompt presets, a PDF with a text layer, an image via /img
    st_p0, pr0 = get("/agents/api/chat/presets"); p0 = (pr0 or {}).get("presets", [])
    ok("schat: default system-prompt presets are served", st_p0 == 200 and len(p0) >= 3 and any(p["name"] == "Код-ревьюер" for p in p0) and all(p.get("id") and p.get("text") for p in p0), str([p["name"] for p in p0])[:120])
    st_p1, pr1 = raw("/agents/api/chat/presets", "PUT", json.dumps({"presets": p0 + [{"name": f"e2e {MARK}", "text": f"Always end your reply with the word preset-{MARK}."}]}).encode()); pr1 = json.loads(pr1 or b"{}")
    ok("schat: a preset saved through the API gets an id and comes back in the list", st_p1 == 200 and any(p["name"] == f"e2e {MARK}" and p["id"].startswith("p_") for p in pr1.get("presets", [])) and raw("/agents/api/chat/presets", "PUT", json.dumps({"presets": [{"name": "", "text": "x"}]}).encode())[0] >= 400, f"{st_p1} n={len(pr1.get('presets', []))}")
    def mini_pdf(text):
        objs = [b"<< /Type /Catalog /Pages 2 0 R >>", b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>", b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 144] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>", None, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
        stream = f"BT /F1 18 Tf 20 80 Td ({text}) Tj ET".encode(); objs[3] = b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"\nendstream"
        out = b"%PDF-1.4\n"; offs = []
        for i, o in enumerate(objs, 1): offs.append(len(out)); out += f"{i} 0 obj\n".encode() + o + b"\nendobj\n"
        xref = len(out); out += f"xref\n0 {len(objs) + 1}\n0000000000 65535 f \n".encode() + b"".join(f"{o:010d} 00000 n \n".encode() for o in offs)
        out += f"trailer\n<< /Size {len(objs) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode(); return out
    st_pdf, evp = sse(sc["id"], {"text": "The attached PDF contains a codeword. Reply with just that codeword.", "attachments": [{"name": "codeword.pdf", "mime": "application/pdf", "data": base64.b64encode(mini_pdf(f"codeword pdf-{MARK}")).decode()}]})
    dp = next((e for e in evp if e["kind"] == "done"), None); up = next((e for e in evp if e["kind"] == "user"), {}); patt = ((up.get("message") or {}).get("attachments") or [{}])[0]
    ok("schat: a PDF attachment gets its text layer extracted on upload and the model reads it", st_pdf == 200 and f"pdf-{MARK}" in (patt.get("text") or "") and bool(dp) and f"pdf-{MARK}" in dp["message"]["text"], f"{st_pdf} text={str(patt.get('text'))[:40]!r} reply={(dp or {}).get('message', {}).get('text', '')[:40]!r}")
    img_models = [m["id"] for r in routes if r["id"] == RT for m in r["models"] if m.get("kind") == "image" and m["id"].startswith("gpt-image")]
    if img_models:
        st_im, evi2 = sse(sc["id"], {"text": "/img a small red circle on a white background, flat icon"})
        di2 = next((e for e in evi2 if e["kind"] == "done"), None); ti2 = [e for e in evi2 if e["kind"] == "tool" and e.get("name") == "image"]
        iatt = (((di2 or {}).get("message") or {}).get("attachments") or [{}])[0]
        st_if, ibody = raw(f"/agents/api/chats/{sc['id']}/files/{iatt.get('file', 'none')}")
        ok("schat: /img generates an image on the route — tool event, assistant message with an image attachment served as a file", st_im == 200 and bool(ti2) and ti2[0].get("ok") and bool(di2) and iatt.get("mime", "").startswith("image/") and st_if == 200 and len(ibody or b"") > 10000 and (ibody or b"")[:4] in (b"\x89PNG", b"\xff\xd8\xff\xe0", b"RIFF") , f"{st_im} tool={bool(ti2)} att={iatt} file={st_if} {len(ibody or b'')}B")
    else: print("SKIP  schat: no gpt-image model on the simple-chat route — image generation not exercised", flush=True)
    raw("/agents/api/chat/presets", "PUT", json.dumps({"presets": p0}).encode())
    # incognito: in memory, with expires_at, not in the SQLite list; after "keep" → becomes
    # regular
    st_ic, ic = raw("/agents/api/chats", "POST", json.dumps({"route": RT, "model": MD, "incognito": True, "tools": {"search": False, "fetch": False}}).encode()); ic = json.loads(ic or b"{}")
    st_is, evs2 = sse(ic["id"], {"text": "What is 2+2? Answer with just the number."}); d2 = next((e for e in evs2 if e["kind"] == "done"), None)
    lst = (get("/agents/api/chats")[1] or {}).get("chats", []); mine = next((c for c in lst if c["id"] == ic["id"]), {})
    ok("schat: incognito chat answers, is listed with an expiry and keeps its history in memory", st_ic == 200 and ic.get("incognito") is True and st_is == 200 and bool(d2) and "4" in d2["message"]["text"] and mine.get("incognito") is True and (mine.get("expires_at") or 0) > time.time() and mine.get("messages") == 2, f"{st_ic} {st_is} d2={bool(d2)} marker={bool(d2) and '4' in d2['message']['text']} text={(d2 or {}).get('message', {}).get('text', '')[:40]!r} incog={mine.get('incognito')} exp={mine.get('expires_at')} now={int(time.time())} msgs={mine.get('messages')}")
    st_k, kp = raw(f"/agents/api/chats/{ic['id']}/keep", "POST", b"{}"); kp = json.loads(kp or b"{}"); kg = get(f"/agents/api/chats/{ic['id']}")[1] or {}
    ok("schat: «keep» turns the incognito chat into a saved one with its messages", st_k == 200 and kp.get("incognito") is False and [m["role"] for m in kg.get("messages", [])] == ["user", "assistant"] and raw(f"/agents/api/chats/{ic['id']}/keep", "POST", b"{}")[0] == 404, f"{st_k} {str(kp)[:80]}")
    st_h, ho = raw(f"/agents/api/chats/{sc['id']}/handoff", "POST", json.dumps({"agent": "claude"}).encode()); ho = json.loads(ho or b"{}"); hsid = (ho.get("session") or {}).get("id")
    ok("schat: handoff opens a Claude Code session in scratch with the history as the first message", st_h == 200 and bool(hsid) and ho.get("project") == "scratch", f"{st_h} {str(ho)[:100]}")
    if hsid:
        for _ in range(30):
            time.sleep(3); evh = get(f"/agents/api/core/projects/scratch/sessions/{hsid}/events?since=0&limit=60")[1] or []
            if any(e["kind"] == "user" for e in evh): break
        uh = " ".join(b.get("text", "") for e in evh if e["kind"] == "user" for b in (e.get("content") or []))
        ok("schat: the handoff prompt carries the simple chat transcript", f"simple-{MARK}" in uh and "Пользователь" in uh, uh[:100])
    # UI: the engine switch, the simple chat list, an answer in the chat, the incognito mark on the kept one — gone, on a new one —
    # present
    pg.keyboard.press("Escape"); time.sleep(0.4)
    for _ in range(3):
        pg.locator(".rail .nav[data-tip='Чат']").click(); time.sleep(1)
        if pg.locator(".top .engine button[data-engine='simple']").count(): break
    pg.wait_for_selector(".top .engine button[data-engine='simple']", timeout=20000); pg.locator(".top .engine button[data-engine='simple']").click(); time.sleep(0.8)
    pg.wait_for_selector(f".panel .sess[data-chat='{sc['id']}']", timeout=20000); pg.locator(f".panel .sess[data-chat='{sc['id']}']").click(); pg.wait_for_selector(".thread .chat .m.agent", timeout=20000); time.sleep(1)
    ok("schat: the panel switch shows simple chats and the chat renders its messages, tools and model chip", pg.locator(".thread .chat .m.agent").count() >= 2 and pg.locator(f".thread .chat .m.agent:has-text('simple-{MARK}')").count() == 1 and pg.locator(".thread .chat .m.tool").count() >= 1 and pg.locator(".thread .chat .atts img").count() >= 1 and (MD.split('/')[-1][:12] in (pg.locator(".thread .chat .composer-model").get_attribute("title") or "") or MD.split('/')[-1][:12] in pg.locator(".thread .chat .composer-model").inner_text()) and pg.locator(".thread .chat .ctl.tfetch.on").count() == 1, f"agent={pg.locator('.thread .chat .m.agent').count()} tool={pg.locator('.thread .chat .m.tool').count()}"); pg.screenshot(path="shots/ap12-simple-chat.png")
    pg.locator(".thread .chat .tools .ctl.mono").click(); time.sleep(0.6)
    mitems = pg.locator(".menu .mi").all_inner_texts(); subs = [x.strip() for x in pg.locator(".menu .mh.sub").all_inner_texts()]
    ok("schat: the model menu is grouped by provider (Claude Code and Codex subscriptions first) with real model ids", any(x.startswith("Claude Code") for x in subs) and any(x.startswith("Codex") for x in subs) and not any("-dd-" in x for x in mitems) and any("gpt-6-astra" in x for x in mitems), f"subs={subs[:8]}")
    pg.locator(".menu input.ms").fill("opus 4.6"); time.sleep(0.4)
    oitems = pg.locator(".menu .mi").all_inner_texts()
    ok("schat: search matches words separately and treats '-' as '.' (opus 4.6 finds claude-opus-4-6 and «Claude 4.6 Opus»)", any("claude-opus-4-6" in x and "thinking" not in x and "devin/" not in x for x in oitems) and any("devin/claude-opus-4-6" in x for x in oitems), str([x.replace(chr(10), ' ')[:40] for x in oitems][:6]))
    pg.locator(".menu input.ms").fill("astra"); time.sleep(0.4)
    fitems = pg.locator(".menu .mi").all_inner_texts()
    ok("schat: the search field filters models and keeps their provider headers", fitems and all("astra" in x.lower() for x in fitems) and pg.locator(".menu .mh.sub").count() >= 1 and len(fitems) < len(mitems), f"{len(fitems)} of {len(mitems)}"); pg.keyboard.press("Escape"); time.sleep(0.3)
    n_agent = pg.locator(".thread .chat .m.agent").count()
    pg.locator(".thread .chat textarea").fill(f"Reply with exactly: ui-{MARK}"); pg.locator(".thread .chat .composer .send").click()
    for _ in range(90):
        time.sleep(1)
        if pg.locator(".thread .chat .m.agent").count() > n_agent and pg.locator(".thread .chat .composer .send").count() == 1: break
    ok("schat: a message sent from the composer streams back into the chat", pg.locator(".thread .chat .m.agent").count() == n_agent + 1 and pg.locator(".thread .chat .composer .send").count() == 1 and pg.locator(f".thread .chat .m.user:has-text('ui-{MARK}')").count() == 1, f"agents {n_agent}→{pg.locator('.thread .chat .m.agent').count()}")
    # chat rendering (slice 33, docs/agents-ux.md §4.3.9): a mermaid diagram as a card with SVG, KaTeX formulas, a copy button on code, the "code"
    # toggle
    composer_option(pg, "Системный промпт"); pg.wait_for_selector(".thread .chat .syspop", timeout=10000); time.sleep(0.6)
    n_pre = pg.locator(".thread .chat .syspop .preset").count()
    pg.locator(".thread .chat .syspop .preset:has-text('Код-ревьюер') button.apply").click(); time.sleep(0.3)
    applied_txt = pg.locator(".thread .chat .syspop textarea.sys").input_value()
    pg.locator(".thread .chat .syspop button.save").click(); time.sleep(1.5)
    sc_after = (get(f"/agents/api/chats/{sc['id']}")[1] or {}).get("chat", {})
    system_checked = False
    for _ in range(3):  # right after saving the popup is still closing — open "Options" again if the wrong menu opened
        pg.keyboard.press("Escape"); time.sleep(0.4)
        pg.locator(".thread .chat .composer .composer-more").click(); pg.wait_for_selector(".menu .mi", timeout=5000); time.sleep(0.3)
        it_sys = pg.locator(".menu .mi").filter(has_text="Системный промпт")
        if it_sys.count(): system_checked = it_sys.first.get_attribute("aria-checked") == "true"; break
    pg.keyboard.press("Escape")
    ok("schat: the system-prompt popover lists presets, «apply» fills the text and «save» writes it to the chat", n_pre >= 3 and "код-ревьюер" in applied_txt.lower() and sc_after.get("system", "").startswith("Ты строгий код-ревьюер") and pg.locator(".thread .chat .syspop").count() == 0 and system_checked, f"presets={n_pre} system={sc_after.get('system', '')[:40]!r}")
    composer_option(pg, "картинка")
    ok("schat: Parameters → image prefixes /img into the composer and the PDF chip shows the extracted-text mark", pg.locator(".thread .chat textarea").input_value().startswith("/img ") and pg.locator(".thread .chat .m.user .atts .chip.file:has-text('codeword.pdf')").count() == 1 and "текст извлечён" in pg.locator(".thread .chat .m.user .atts .chip.file:has-text('codeword.pdf')").inner_text()); pg.locator(".thread .chat textarea").fill("")
    pg.screenshot(path="shots/ap19-schat2.png")
    n_agent = pg.locator(".thread .chat .m.agent").count()
    # after the answer the chat is re-read (chatGet) and the input field may be recreated — type until the "Send" button becomes active
    for _ in range(4):
        pg.locator(".thread .chat textarea").fill("Answer with three things in markdown and nothing else: (1) a mermaid flowchart in a ```mermaid code block with two nodes Start and Done connected by an arrow; (2) the formula for the area of a circle as a display math block delimited by $$ on its own lines; (3) one short sentence that mentions x squared as inline math $x^2$; (4) a one-line python code block printing hello."); time.sleep(0.5)
        if pg.locator(".thread .chat .composer .send:not([disabled])").count(): break
        time.sleep(1.5)
    pg.locator(".thread .chat .composer .send:not([disabled])").click(timeout=15000)
    for _ in range(120):
        time.sleep(1)
        if pg.locator(".thread .chat .m.agent").count() > n_agent and pg.locator(".thread .chat .composer .send").count() == 1: break
    try: pg.wait_for_selector(".thread .chat .m.agent .mermaid-src .diagram svg", timeout=30000)
    except Exception: pass
    last = pg.locator(".thread .chat .m.agent").last
    n_svg = last.locator(".mermaid-src .diagram svg").count(); n_ktx = last.locator(".katex").count(); n_disp = last.locator(".katex-display").count(); n_cp = pg.locator(".thread .chat .m.agent pre .cp").count()
    ok("md: a mermaid block renders as a diagram card with an SVG, TeX renders through KaTeX (display and inline), code blocks get a copy button", n_svg >= 1 and n_ktx >= 2 and n_disp >= 1 and n_cp >= 1, f"svg={n_svg} katex={n_ktx} display={n_disp} copy-buttons={n_cp} text={last.inner_text()[:80]!r}")
    if n_svg:
        last.locator(".diagram .dh .tg").first.click(); time.sleep(0.4)
        src_shown = last.locator(".mermaid-src.src > pre").first.is_visible() and not last.locator(".mermaid-src.src .dg").first.is_visible()
        last.locator(".diagram .dh .tg").first.click(); time.sleep(0.4)
        ok("md: the «code» toggle shows the mermaid source instead of the SVG and back", src_shown and last.locator(".mermaid-src .dg svg").first.is_visible(), f"src_shown={src_shown}")
    try: last.scroll_into_view_if_needed(timeout=5000)
    except Exception: pass
    pg.screenshot(path="shots/ap15-md.png")
    pg.locator(".panel .p-head button[data-tip='Инкогнито-чат']").click(); pg.wait_for_selector(".thread .chat .incog", timeout=20000); time.sleep(0.5)
    ok("schat: a new incognito chat shows the «not saved» bar and the eye-off badge", pg.locator(".thread .chat .incog").count() == 1 and pg.locator(".thread .t-head .av.incog").count() == 1 and pg.locator(".top .keep").count() == 1); pg.screenshot(path="shots/ap13-incognito.png")
    inc_id = pg.locator(".panel .sess.active").get_attribute("data-chat") or ""
    pg.locator(".top .engine button[data-engine='advanced']").click(); time.sleep(0.6)
    ok("schat: the advanced engine lists scratch sessions again", pg.locator(".panel .sess[data-id]").count() >= 1 and pg.locator(".panel .sess[data-chat]").count() == 0)
    for cid in (sc.get("id"), ic.get("id"), inc_id):
        if cid: raw(f"/agents/api/chats/{cid}", "DELETE")
    if hsid:
        for _ in range(3):
            if raw(f"/agents/api/core/projects/scratch/sessions/{hsid}", "DELETE")[0] in (204, 404): break
            time.sleep(3)
    ok("schat: chats removed", all(c["id"] not in (sc.get("id"), ic.get("id")) for c in (get("/agents/api/chats")[1] or {}).get("chats", [])))
    # ---- conversation search and export (slice 34, docs/agents-ux.md §4.6): the service's /search over the agentd of all workspaces and simple chats, jump to a message, Markdown/JSON export, the
    # expense ledger CSV ----
    if "RT" not in globals():   # a part without the "simple chat" block: the route and model for the search chat are chosen here
        routes = [r for r in (get("/agents/api/chat/routes")[1] or {}).get("routes", []) if r.get("models")]; RT, MD = pick_model()
    # own data: a Claude session with a code word and a simple chat with another — the previous blocks have
    # already removed their conversations
    ss = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"search probe {MARK}"}).encode())[1] or b"{}"); ssid = ss.get("id")
    raw(f"/agents/api/core/projects/{P}/sessions/{ssid}/prompt", "POST", json.dumps({"content": f"Remember the word apple-{MARK}. Reply with exactly: ok"}).encode())
    for _ in range(60):
        time.sleep(3)
        if any(e["kind"] == "result" for e in (get(f"/agents/api/core/projects/{P}/sessions/{ssid}/events?since=0&limit=100")[1] or [])): break
    st_c2, sc2 = raw("/agents/api/chats", "POST", json.dumps({"route": RT, "model": MD, "tools": {"search": False, "fetch": False}}).encode()); sc2 = json.loads(sc2 or b"{}")
    sse(sc2.get("id", "x"), {"text": f"The codeword is needle-{MARK}. Repeat the codeword back to me."})
    time.sleep(2)
    st_sr, sr = get(f"/agents/api/search?q=apple-{MARK}")
    s_hits = [h for h in (sr or {}).get("hits", []) if h["kind"] == "session"]
    ok("search: a word from a turn is found across workspaces — session hit with project, title, seq, role and a snippet around the word", st_sr == 200 and s_hits and s_hits[0]["project"] == P and s_hits[0]["id"] == ssid and f"apple-{MARK}" in s_hits[0]["snippet"] and s_hits[0]["role"] == "user" and (s_hits[0].get("seq") or 0) > 0 and "words" in sr, f"{st_sr} hits={len((sr or {}).get('hits', []))} {str(s_hits[:1])[:160]}")
    st_sc, src_ = get(f"/agents/api/search?q=Needle-{MARK.upper()}")
    c_hits = [h for h in (src_ or {}).get("hits", []) if h["kind"] == "chat"]
    ok("search: simple chats are searched too, case-insensitively — chat hit with message id and snippet; one row per conversation", st_sc == 200 and c_hits and c_hits[0]["id"] == sc2.get("id") and c_hits[0].get("mid") and f"needle-{MARK}".lower() in c_hits[0]["snippet"].lower() and len([h for h in c_hits if h["id"] == sc2.get("id")]) == 1, f"{st_sc} {str(c_hits[:1])[:160]}")
    st_x, xmd = raw(f"/agents/api/core/projects/{P}/sessions/{ssid}/download?format=md"); xmd_s = (xmd or b"").decode("utf-8", "ignore")
    st_xj, xjs = raw(f"/agents/api/core/projects/{P}/sessions/{ssid}/download?format=json")
    try: xj = json.loads(xjs or b"{}")
    except Exception: xj = {}
    ok("export: a session downloads as Markdown (header with engine and turns, owner/agent messages) and as JSON (session + events)", st_x == 200 and xmd_s.startswith("# ") and "**Владелец**" in xmd_s and f"apple-{MARK}" in xmd_s and "Ходов:" in xmd_s and st_xj == 200 and xj.get("session", {}).get("id") == ssid and any(e.get("kind") == "user" for e in xj.get("events", [])), f"{st_x} {len(xmd_s)}B json={st_xj} events={len(xj.get('events', []))}")
    st_cx, cmd_ = raw(f"/agents/api/chats/{sc2.get('id', 'x')}/export?format=md"); cmd_s = (cmd_ or b"").decode("utf-8", "ignore")
    ok("export: a simple chat downloads as Markdown with its model and messages", st_cx == 200 and cmd_s.startswith("# ") and f"needle-{MARK}" in cmd_s.lower() and "**Модель**" in cmd_s and MD in cmd_s, f"{st_cx} {len(cmd_s)}B")
    st_csv, csv_ = raw("/agents/api/core/usage/export.csv?days=1"); csv_s = (csv_ or b"").decode("utf-8-sig", "ignore"); csv_lines = [l for l in csv_s.splitlines() if l.strip()]
    ok("export: the usage ledger downloads as CSV — header row and one row per turn with ISO time, workspace, session, model, tokens and cost", st_csv == 200 and csv_lines and csv_lines[0].startswith("time,workspace,session,agent,model,input,output") and len(csv_lines) >= 2 and csv_lines[1].count(",") >= 11 and "T" in csv_lines[1][:25], f"{st_csv} rows={len(csv_lines) - 1} first={csv_lines[1][:60] if len(csv_lines) > 1 else ''}")
    # UI: the panel search field shows matches in messages, a click opens the conversation and highlights the message
    pg.keyboard.press("Escape"); time.sleep(0.3); pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1)
    pg.locator(".panel .search input").fill(f"apple-{MARK}")
    try: pg.wait_for_selector(".panel .hits .hit", timeout=20000)
    except Exception: pass
    hit_rows = pg.locator(".panel .hits .hit"); n_hits = hit_rows.count(); hit_txt = hit_rows.first.inner_text() if n_hits else ""
    ok("search: typing three or more characters in the panel search lists message hits with a highlighted snippet", n_hits >= 1 and f"apple-{MARK}" in hit_txt and pg.locator(".panel .hits .hit mark").count() >= 1, f"hits={n_hits} {hit_txt[:80]!r}")
    if n_hits:
        hit_rows.first.click()
        try: pg.wait_for_selector(".thread .chat .m.hit", timeout=20000)
        except Exception: pass
        # the panel tree during search is filtered by titles — the session row may be absent there; the conversation is
        # open in Thread
        head_ok = "search probe" in pg.locator(".thread .t-head h2").inner_text()
        ok("search: a click opens the session in Thread and scrolls to the matching message, highlighting it", pg.locator(".thread .chat .m.hit").count() == 1 and f"apple-{MARK}" in pg.locator(".thread .chat .m.hit").first.inner_text() and head_ok, f"hit={pg.locator('.thread .chat .m.hit').count()} head={head_ok}")
        pg.screenshot(path="shots/ap16-search.png")
    pg.locator(".panel .search input").fill(""); time.sleep(0.5)
    ok("search: clearing the field removes the hits block", pg.locator(".panel .hits").count() == 0)
    if sc2.get("id"): raw(f"/agents/api/chats/{sc2['id']}", "DELETE")
    if ssid:
        for _ in range(3):
            if raw(f"/agents/api/core/projects/{P}/sessions/{ssid}", "DELETE")[0] in (204, 404): break
            time.sleep(3)
    # ---- MCP servers and skills in the conversation (slice 36, docs/agents-ux.md §4.3.10): the agentd list and the rollout to three CLIs, the "Settings → MCP" tab, the skills popup in the composer,
    # the skills block in a message ----
    MCP = f"/agents/api/core/projects/{P}/mcp"
    st_m0, m0 = get(MCP); base_srv = [x for x in (m0 or {}).get("servers", []) if not x["name"].startswith("e2e-")]
    ok("mcp: the workspace list and per-CLI counts are readable", st_m0 == 200 and isinstance((m0 or {}).get("servers"), list) and "engines" in (m0 or {}), str(m0)[:160])
    MCPN = f"e2e-mcp-{MARK[4:]}"; MCPH = f"e2e-http-{MARK[4:]}"
    st_ms, ms = raw(MCP, "POST", json.dumps({"servers": base_srv + [{"name": MCPN, "transport": "stdio", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-everything"], "env": {"E2E_MARK": MARK}, "url": "", "headers": {}}]}).encode()); ms = json.loads(ms or b"{}")
    ok("mcp: a stdio server is saved and applied to all three CLIs (each reports ok)", st_ms == 200 and any(x["name"] == MCPN for x in ms.get("servers", [])) and all(ms.get("applied", {}).get(k) == "ok" for k in ("claude", "codex", "gemini")), f"{st_ms} applied={ms.get('applied')} engines={ms.get('engines')}")
    st_cj, cj = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.claude.json"); st_ct, ct = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.codex/config.toml"); st_gj, gj = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.gemini/settings.json")
    ok("mcp: the server lands in ~/.claude.json, ~/.codex/config.toml and ~/.gemini/settings.json", st_cj == 200 and MCPN.encode() in (cj or b"") and st_ct == 200 and f"[mcp_servers.{MCPN}]".encode() in (ct or b"") and st_gj == 200 and MCPN.encode() in (gj or b""), f"claude={st_cj} codex={st_ct} gemini={st_gj}")
    ok("mcp: a bad name and a stdio server without a command are rejected", raw(MCP, "POST", json.dumps({"servers": [{"name": "bad name", "transport": "stdio", "command": "x"}]}).encode())[0] >= 400 and raw(MCP, "POST", json.dumps({"servers": [{"name": "ok-name", "transport": "stdio", "command": ""}]}).encode())[0] >= 400)
    pg.keyboard.press("Escape"); time.sleep(0.3); pg.locator(".rail .nav[data-tip='Настройки']").click(); time.sleep(0.8); pg.locator(".subnav button:has-text('MCP')").click(); time.sleep(1.5)
    msel = pg.locator(".card-s.mcp select").first
    if msel.count() and msel.input_value() != P: msel.select_option(P); time.sleep(1.5)
    try: pg.wait_for_selector(f".mcp-row[data-mcp='{MCPN}']", timeout=15000)
    except Exception: pass
    row_t = pg.locator(f".mcp-row[data-mcp='{MCPN}']").inner_text() if pg.locator(f".mcp-row[data-mcp='{MCPN}']").count() else ""
    ok("mcp: the Settings tab lists the workspace servers with transport and command, plus the per-CLI counts", pg.locator(f".mcp-row[data-mcp='{MCPN}']").count() == 1 and "stdio" in row_t and "server-everything" in row_t and "Claude Code" in pg.locator(".card-s.mcp .engines").inner_text(), row_t[:100])
    pg.locator(".mcp-form input.name").fill(MCPH); pg.locator(".mcp-form select.transport").select_option("http"); time.sleep(0.3); pg.locator(".mcp-form input.url").fill("https://mcp.example.com/mcp"); pg.locator(".mcp-form textarea.headers").fill("Authorization: Bearer e2e"); pg.locator(".mcp-form button.add").click()
    try: pg.wait_for_selector(f".mcp-row[data-mcp='{MCPH}']", timeout=90000)
    except Exception: pass
    m2 = get(MCP)[1] or {}; hs = next((x for x in m2.get("servers", []) if x["name"] == MCPH), {})
    ok("mcp: an http server added from the form is saved with its URL and header and applied", pg.locator(f".mcp-row[data-mcp='{MCPH}']").count() == 1 and hs.get("url") == "https://mcp.example.com/mcp" and hs.get("headers", {}).get("Authorization") == "Bearer e2e", str(hs)[:160])
    # OAuth sign-in to an MCP server from the tab (slice 66): the button on the http row opens a card with a `claude mcp login <name>
    # --no-browser` terminal and a field for the address; for a made-up server the CLI exits with an error — the card shows it
    pg.locator(f".mcp-row[data-mcp='{MCPH}'] .mcp-login").click()
    try: pg.wait_for_selector(".mcp-auth .login-term", timeout=20000)
    except Exception: pass
    got_done = False
    for _ in range(120):
        if pg.locator(".mcp-auth:has-text('Команда завершилась')").count(): got_done = True; break
        time.sleep(0.5)
    term_txt = pg.locator(".mcp-auth .login-term").inner_text() if pg.locator(".mcp-auth .login-term").count() else ""
    ok("mcp: «Войти (OAuth)» on an http server opens the sign-in card with a terminal running the CLI login and a paste field; the CLI's outcome for a fake server shows up and the card reports the exit", pg.locator(".mcp-auth").count() == 1 and got_done, f"done={got_done} term={term_txt[-120:]!r}")
    pg.locator(".mcp-auth button:has-text('Закрыть')").click(); time.sleep(0.3)
    ok("mcp: the stdio server row has no OAuth button, the http row does", pg.locator(f".mcp-row[data-mcp='{MCPN}'] .mcp-login").count() == 0 and pg.locator(f".mcp-row[data-mcp='{MCPH}'] .mcp-login").count() == 1)
    # server health (slice 70): the button calls `claude mcp list`, a made-up http server gets a "not connected" badge with a reason, stdio — the CLI
    # result
    pg.locator(".mcp-health-check").click()
    try: pg.wait_for_selector(f".mcp-row[data-mcp='{MCPH}'] .mcp-health", timeout=170000)
    except Exception: pass
    hb = pg.locator(f".mcp-row[data-mcp='{MCPH}'] .mcp-health"); hs_api = (get(f"/agents/api/core/projects/{P}/mcp/health")[1] or {}).get("servers", [])
    ok("mcp: «Проверить подключения» shows a badge per server from `claude mcp list` — the fake http server is «не подключён» with the CLI's reason, the API returns the same rows", hb.count() == 1 and "не подключён" in hb.inner_text() and "mcp.example.com" in (hb.get_attribute("title") or "") and any(x.get("name") == MCPH and x.get("status") == "fail" for x in hs_api) and pg.locator(f".mcp-row[data-mcp='{MCPN}'] .mcp-health").count() == 1, f"badge={hb.inner_text() if hb.count() else 'none'} api={[(x.get('name'), x.get('status')) for x in hs_api][:3]}")
    pg.screenshot(path="shots/ap17-mcp.png")
    st_rm, mr = raw(MCP, "POST", json.dumps({"servers": base_srv}).encode())
    cj2 = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.claude.json")[1] or b""; ct2 = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.codex/config.toml")[1] or b""; gj2 = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.gemini/settings.json")[1] or b""
    ok("mcp: removed servers disappear from all three CLI configs", st_rm == 200 and all(n.encode() not in cj2 and n.encode() not in ct2 and n.encode() not in gj2 for n in (MCPN, MCPH)), f"{st_rm}")
    # skills in the conversation: chip → a popup with the catalog, the "report" mark, the first message goes with the skills block, the second —
    # without; for Claude a SKILL.md appears
    ks = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"skills probe {MARK}"}).encode())[1] or b"{}"); ksid = ks.get("id")
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1)
    try: pg.wait_for_selector(f".panel .sess[data-id='{ksid}']", timeout=20000)
    except Exception: pass
    if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.4)
    pg.locator(f".panel .sess[data-id='{ksid}']").click(); pg.wait_for_selector(".thread .chat .composer-more", timeout=20000); time.sleep(0.8)
    composer_option(pg, "навыки"); pg.wait_for_selector(".thread .chat .skills-pop label.sk", timeout=10000); time.sleep(0.4)
    pg.locator(".thread .chat .skills-pop label.sk[data-skill='report'] input").check(); time.sleep(0.5)
    ok("skills: the composer chip opens the catalog popover and a checked skill shows in the chip", pg.locator(".thread .chat .skills-pop label.sk").count() >= 3 and pg.locator(".thread .chat .skills-pop label.sk[data-skill='report'] input").is_checked() and "· 1" in pg.locator(".thread .chat .tools .ctl.skills").inner_text(), pg.locator(".thread .chat .tools .ctl.skills").inner_text())
    pg.keyboard.press("Escape"); time.sleep(0.3)
    pg.locator(".thread .chat textarea").fill(f"Reply with exactly: skill-{MARK}"); pg.locator(".thread .chat button.send").click()
    try: pg.wait_for_selector(f".thread .chat .m.agent:has-text('skill-{MARK}')", timeout=180000)
    except Exception: pass
    kev = get(f"/agents/api/core/projects/{P}/sessions/{ksid}/events?since=0&limit=200")[1] or []
    ku = [" ".join(str(b.get("text", "")) for b in (e.get("content") or []) if isinstance(b, dict)) for e in kev if e["kind"] == "user" and not e.get("parent_tool_use_id")]
    ok("skills: the first message carries the skills block before the question (same shape as routines) and the chat shows it collapsed", len(ku) >= 1 and ku[0].startswith("# Навыки (как делать повторяющуюся работу)") and "## report —" in ku[0] and ku[0].rstrip().endswith(f"Reply with exactly: skill-{MARK}") and pg.locator(".thread .chat .m.user .sk:has-text('report')").count() == 1 and pg.locator(f".thread .chat .m.user .txt:has-text('skill-{MARK}')").count() == 1, f"users={len(ku)} head={ku[0][:60] if ku else ''!r}")
    st_sk, skf = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.claude/skills/report/SKILL.md")
    ok("skills: for Claude the chosen skill is also laid out as ~/.claude/skills/report/SKILL.md", st_sk == 200 and b"name: report" in (skf or b""), f"{st_sk}")
    pg.locator(".thread .chat textarea").fill(f"Reply with exactly: again-{MARK}"); pg.locator(".thread .chat button.send").click()
    try: pg.wait_for_selector(f".thread .chat .m.agent:has-text('again-{MARK}')", timeout=180000)
    except Exception: pass
    kev2 = get(f"/agents/api/core/projects/{P}/sessions/{ksid}/events?since=0&limit=200")[1] or []
    ku2 = [" ".join(str(b.get("text", "")) for b in (e.get("content") or []) if isinstance(b, dict)) for e in kev2 if e["kind"] == "user" and not e.get("parent_tool_use_id")]
    ok("skills: the second message goes without the block — skills are sent once per session", len(ku2) == 2 and not ku2[1].startswith("# Навыки") and f"again-{MARK}" in ku2[1], f"users={len(ku2)}")
    pg.screenshot(path="shots/ap18-skills.png")
    if ksid:
        for _ in range(3):
            if raw(f"/agents/api/core/projects/{P}/sessions/{ksid}", "DELETE")[0] in (204, 404): break
            time.sleep(3)
    # ---- agentd restart without losing conversations (slice 38, docs/agents-ux.md §4.3.11): live Claude and Gemini conversations come back by themselves
    # after systemctl restart ----
    rs = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"restart probe {MARK}"}).encode())[1] or b"{}"); rsid = rs.get("id")
    rg = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "gemini", "title": f"restart gemini {MARK}"}).encode())[1] or b"{}"); rgid = rg.get("id")
    raw(f"/agents/api/core/projects/{P}/sessions/{rsid}/prompt", "POST", json.dumps({"content": f"Remember the word restart-{MARK}. Reply with exactly: ok"}).encode())
    raw(f"/agents/api/core/projects/{P}/sessions/{rgid}/prompt", "POST", json.dumps({"content": f"Remember the word gemini-{MARK}. Reply with exactly: ok"}).encode())
    for _ in range(60):
        time.sleep(3)
        ev_a = get(f"/agents/api/core/projects/{P}/sessions/{rsid}/events?since=0&limit=100")[1] or []; ev_g = get(f"/agents/api/core/projects/{P}/sessions/{rgid}/events?since=0&limit=100")[1] or []
        if any(e["kind"] == "result" for e in ev_a) and any(e["kind"] == "result" for e in ev_g): break
    psid_before = (get(f"/agents/api/core/projects/{P}/sessions/{rsid}")[1] or {}).get("info", {}).get("provider_session_id")
    st_rr, rr_ = raw(f"/agents/api/core/projects/{P}/agentd/restart", "POST", b"{}")
    ok("restart: the core restarts agentd in the workspace on request", st_rr == 200 and json.loads(rr_ or b"{}").get("restarted") is True, f"{st_rr} {(rr_ or b'')[:80]}")
    back = {}
    for _ in range(40):
        time.sleep(3)
        ia = (get(f"/agents/api/core/projects/{P}/sessions/{rsid}")[1] or {}).get("info", {}); ig = (get(f"/agents/api/core/projects/{P}/sessions/{rgid}")[1] or {}).get("info", {})
        if ia.get("status") == "idle" and ig.get("status") == "idle": back = {"a": ia, "g": ig}; break
    ev_a2 = get(f"/agents/api/core/projects/{P}/sessions/{rsid}/events?since=0&limit=200")[1] or []; ev_g2 = get(f"/agents/api/core/projects/{P}/sessions/{rgid}/events?since=0&limit=200")[1] or []
    # Gemini CLI 0.62 cannot find its ACP sessions in session/load ("Invalid session identifier") — it comes up as a new session with a history retelling; loaded is not
    # required of it
    ok("restart: both sessions come back alive by themselves with a restart note in the journal and the same provider session for Claude", bool(back) and back["a"].get("alive") and back["g"].get("alive") and any(e["kind"] == "system" and e.get("subtype") == "agentd_restart" for e in ev_a2) and any(e["kind"] == "system" and e.get("subtype") == "agentd_restart" for e in ev_g2) and back["a"].get("provider_session_id") == psid_before and len([e for e in ev_g2 if e["kind"] == "init"]) >= 2, f"a={(back.get('a') or {}).get('status')} g={(back.get('g') or {}).get('status')} psid_same={(back.get('a') or {}).get('provider_session_id') == psid_before} gemini_loaded={any(e['kind'] == 'init' and e.get('loaded') is True for e in ev_g2)}")
    raw(f"/agents/api/core/projects/{P}/sessions/{rsid}/prompt", "POST", json.dumps({"content": "What word did I ask you to remember? Reply with just that word."}).encode())
    raw(f"/agents/api/core/projects/{P}/sessions/{rgid}/prompt", "POST", json.dumps({"content": "What word did I ask you to remember? Reply with just that word."}).encode())
    for _ in range(60):
        time.sleep(3)
        ev_a3 = get(f"/agents/api/core/projects/{P}/sessions/{rsid}/events?since=0&limit=200")[1] or []; ev_g3 = get(f"/agents/api/core/projects/{P}/sessions/{rgid}/events?since=0&limit=200")[1] or []
        if len([e for e in ev_a3 if e["kind"] == "result"]) >= 2 and len([e for e in ev_g3 if e["kind"] == "result"]) >= 2: break
    ta = " ".join(str(b.get("text", "")) for e in ev_a3 if e["kind"] == "assistant" for b in (e.get("content") or []) if isinstance(b, dict)); tg = " ".join(str(b.get("text", "")) for e in ev_g3 if e["kind"] == "assistant" for b in (e.get("content") or []) if isinstance(b, dict))
    recap = any(e["kind"] == "system" and e.get("subtype") == "recap_sent" for e in ev_g3)
    # Gemini: either session/load found the session (native context) or the journal retelling as the first prompt (system recap_sent)
    # — the result matters
    ok("restart: the resumed sessions keep their context — Claude natively, Gemini natively or through the journal recap sent with the first prompt after the restart — both recall the word", f"restart-{MARK}" in ta and f"gemini-{MARK}" in tg, f"claude={ta[-60:]!r} gemini={tg[-60:]!r} recap={recap}")
    for sdel in (rsid, rgid):
        if sdel:
            for _ in range(3):
                if raw(f"/agents/api/core/projects/{P}/sessions/{sdel}", "DELETE")[0] in (204, 404): break
                time.sleep(3)
    # ---- ACP engines (slice 32: agentd acp.rs + engines.rs, docs/agents-ux.md §4.3.8): the engine catalog, Gemini CLI and pi through the gate, permissions, model and mode on the fly,
    # the "+" menu ----
    st_e, eng = get(f"/agents/api/core/projects/{P}/engines"); engs = {e["id"]: e for e in (eng or {}).get("engines", [])}
    ok("engines: catalog — Claude Code, Codex, Gemini CLI, pi, Antigravity and Oh My Pi listed; the first four installed with versions; Gemini, pi and omp speak ACP, Antigravity — its own agy protocol without an npm adapter", st_e == 200 and set(engs) == {"claude", "codex", "gemini", "pi", "antigravity", "omp"} and engs["omp"]["protocol"] == "acp" and engs["omp"]["bin"] == "omp" and not (engs["omp"].get("npm") or []) and (engs["omp"]["installed"] is False or bool(engs["omp"].get("version"))) and all(engs[k]["installed"] and engs[k].get("version") for k in ("claude", "codex", "gemini", "pi")) and engs["gemini"]["protocol"] == "acp" and engs["pi"]["protocol"] == "acp" and engs["antigravity"]["protocol"] == "agy" and engs["antigravity"]["bin"] == "agy" and not (engs["antigravity"].get("npm") or []) and (engs["antigravity"]["installed"] is False or bool(engs["antigravity"].get("version"))), f"{st_e} {{k: (v.get('version'), v['protocol'], v['installed']) for k, v in engs.items()}}")
    # catalog version in the hub (slice 80): the install stamp → an agentd snapshot with engines_v → a project frame → the client
    # re-reads the catalog; simulate the end of an install by touching the stamp from a terminal and wait for a new version in
    # /workspaces
    def ws_engines_v():
        w = get("/agents/api/workspaces")[1] or {}; return next((p.get("engines_v") for p in w.get("projects", []) if p.get("name") == P), None)
    v0 = ws_engines_v()
    st_tp, tpb = raw(f"/agents/api/core/projects/{P}/ptys", "POST", json.dumps({"cmd": ["bash", "-lc", "mkdir -p ~/.cache/cloudos && sleep 1 && touch ~/.cache/cloudos/engines.stamp && sleep 2"], "title": "stamp"}).encode()); tp = json.loads(tpb or b"{}")
    v1 = v0
    for _ in range(20):
        time.sleep(3); v1 = ws_engines_v()
        if v1 and v0 is not None and v1 > v0: break
    ok("engines: touching the install stamp in the workspace bumps engines_v in the hub snapshot within 30 s (clients re-read the catalog, the «+» menu switches «Install…» → «Chat with …» on its own)", st_tp == 200 and v0 is not None and v1 is not None and v1 > v0, f"before={v0} after={v1}")
    if tp.get("id"): raw(f"/agents/api/core/projects/{P}/ptys/{tp['id']}", "DELETE")
    # ?refresh=1 (slice 77): the catalog is re-read bypassing the agentd cache — the same set of
    # engines and versions
    st_r, eng_r = get(f"/agents/api/core/projects/{P}/engines?refresh=1"); engs_r = {e["id"]: e for e in (eng_r or {}).get("engines", [])}
    ok("engines: ?refresh=1 re-detects the catalog past the agentd cache — same engines and versions", st_r == 200 and set(engs_r) == set(engs) and all(engs_r[k].get("installed") == engs[k].get("installed") and engs_r[k].get("version") == engs[k].get("version") for k in engs), f"{st_r} {[(k, v.get('installed'), v.get('version')) for k, v in engs_r.items()]}")
    def e_sess(sid_e): return get(f"/agents/api/core/projects/{P}/sessions/{sid_e}")[1] or {}
    # Oh My Pi (§4.3.18): an ACP engine like pi — the gate provider in ~/.omp/agent/models.json, the model as the --model flag
    # before acp; modes and models from configOptions; a command in the "ask" mode → a permission request → "allow" → a Bash card
    # with output
    if engs.get("omp", {}).get("installed"):
        om = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "omp", "title": f"omp probe {MARK}"}).encode())[1] or b"{}"); omid = om.get("id")
        oev = []; oinit = {}
        for _ in range(40):
            time.sleep(3); oev = get(f"/agents/api/core/projects/{P}/sessions/{omid}/events?since=0&limit=60")[1] or [] if omid else []
            oinit = next((e for e in oev if e.get("kind") == "init"), {})
            if oinit or any(e.get("kind") in ("error", "exit") for e in oev): break
        oc = (e_sess(omid).get("caps") or {}) if omid else {}; omods = [(m.get("value"), m.get("label")) for m in oc.get("models", [])]; omodes = oc.get("modes") or []
        ok("omp: the session starts over omp's own ACP through the gate — init with route gate and a gate model, no provider errors", bool(omid) and oinit.get("protocol") == "acp" and oinit.get("route") == "gate" and str(oinit.get("model", "")).startswith("agentverse-gate/") and not any(e.get("kind") == "system" and e.get("subtype") == "provider_error" for e in oev), f"init={ {k: oinit.get(k) for k in ('protocol', 'route', 'model')} } events={[e.get('kind') for e in oev][:8]}")
        ok("omp: caps — only gate models with bare labels (from configOptions), modes default/plan from configOptions", bool(omods) and all(str(v).startswith("agentverse-gate/") and not str(l).startswith("agentverse-gate/") for v, l in omods) and set(omodes) >= {"default", "plan"}, f"models={omods[:3]} modes={omodes}")
        if omid:
            n_o = len(get(f"/agents/api/core/projects/{P}/sessions/{omid}/events?since=0&limit=400")[1] or [])
            raw(f"/agents/api/core/projects/{P}/sessions/{omid}/prompt", "POST", json.dumps({"content": f"Run the shell command `echo omp-{MARK}` and reply with its output only."}).encode())
            perm = None
            for _ in range(50):
                time.sleep(3); oev = (get(f"/agents/api/core/projects/{P}/sessions/{omid}/events?since=0&limit=400")[1] or [])[n_o:]
                perm = next((e for e in oev if e.get("kind") == "permission_request"), None)
                if perm or any(e.get("kind") == "result" for e in oev): break
            ok("omp: a shell command in the «ask» mode raises a permission request through ACP (Bash card with the command)", perm is not None and "echo omp-" in json.dumps(perm.get("input") or perm, ensure_ascii=False), f"perm={json.dumps(perm, ensure_ascii=False)[:160] if perm else None} kinds={[e.get('kind') for e in oev][:8]}")
            if perm:
                rid = perm.get("request_id") or (perm.get("data") or {}).get("request_id") or perm.get("id")
                st_a, _ = raw(f"/agents/api/core/projects/{P}/sessions/{omid}/permission", "POST", json.dumps({"request_id": rid, "behavior": "allow"}).encode())
                for _ in range(50):
                    time.sleep(3); oev = (get(f"/agents/api/core/projects/{P}/sessions/{omid}/events?since=0&limit=400")[1] or [])[n_o:]
                    if any(e.get("kind") == "result" for e in oev): break
                ores = [b for e in oev if e.get("kind") == "tool_result" for b in (e.get("content") or []) if isinstance(b, dict) and b.get("type") == "tool_result"]
                otext = " ".join(str(b.get("text", "")) for e in oev if e.get("kind") == "assistant" for b in (e.get("content") or []) if isinstance(b, dict) and b.get("type") == "text")
                ok("omp: «allow» runs the command — tool_result with its output and the answer carries the marker; result success", st_a == 200 and any(f"omp-{MARK}" in str(r.get("content", "")) and not r.get("is_error") for r in ores) and f"omp-{MARK}" in otext and any(e.get("kind") == "result" and e.get("subtype") == "success" for e in oev), f"allow={st_a} results={[str(r.get('content', ''))[:40] for r in ores]} text={otext[-60:]!r}")
            for _ in range(3):
                if raw(f"/agents/api/core/projects/{P}/sessions/{omid}", "DELETE")[0] in (204, 404): break
                time.sleep(3)
    # Antigravity account (§9.11): a Google sign-in in an agy terminal under a separate HOME; the core watches the sign-in
    # itself. Without a Google account we check the start of the flow: a terminal with HOME=…/logins/antigravity-…, a Google link
    # in the terminal text, status waiting, closing the terminal → cancelled and the sign-in directory removed
    if engs.get("antigravity", {}).get("installed"):
        st_l, lb = raw("/agents/api/core/accounts/login", "POST", json.dumps({"provider": "antigravity", "project": P}).encode()); lj = json.loads(lb or b"{}")
        lpty = lj.get("pty"); ldir = lj.get("dir") or ""
        ltext = ""; lurl = ""
        if lpty:
            time.sleep(5); raw(f"/agents/api/core/projects/{P}/ptys/{lpty}/input", "POST", json.dumps({"data": "\r"}).encode())   # 1. Google OAuth
            for _ in range(20):
                time.sleep(2); ltext = (get(f"/agents/api/core/projects/{P}/ptys/{lpty}/text")[1] or {}).get("text", "")
                i = ltext.rfind("https://accounts.google.com/")
                if i >= 0:
                    lurl = "".join(ltext[i:].split("Copy and paste")[0].split()); break
        lst = (get(f"/agents/api/core/accounts/login/{lpty}")[1] or {}) if lpty else {}
        ok("accounts: an Antigravity sign-in opens the agy terminal under its own HOME, the Google link is readable from the terminal text and the core is waiting for the sign-in", st_l == 200 and bool(lpty) and "/home/coder/.cloudos/logins/antigravity-" in ldir and "redirect_uri" in lurl and "oauth-callback" in lurl and lst.get("status") == "waiting", f"{st_l} dir={ldir} url={lurl[:60]} status={lst.get('status')} text={ltext[-80:]!r}")
        if lpty:
            raw(f"/agents/api/core/projects/{P}/ptys/{lpty}", "DELETE"); lst2 = {}
            for _ in range(10):
                time.sleep(3); lst2 = get(f"/agents/api/core/accounts/login/{lpty}")[1] or {}
                if lst2.get("status") in ("cancelled", "timeout", "done", "error"): break
            st_d, _ = raw(f"/agents/api/core/projects/{P}/fs/file?path={ldir}/.gemini/antigravity-cli/installation_id")
            ok("accounts: closing the sign-in terminal cancels the Antigravity sign-in and removes the login directory", lst2.get("status") == "cancelled" and st_d == 404, f"status={lst2.get('status')} dir_file={st_d}")
        # by link (§9.11): the core itself drives agy in a hidden terminal and returns the Google link; the code from the
        # page goes to oauth/finish
        st_o, ob = raw("/agents/api/core/accounts/oauth/start", "POST", json.dumps({"provider": "antigravity", "project": P}).encode()); oj = json.loads(ob or b"{}")
        ourl = oj.get("url") or ""; opty = oj.get("pty")
        ok("accounts: an Antigravity sign-in by link returns agy's Google link and a state without showing a terminal to the owner", st_o == 200 and ourl.startswith("https://accounts.google.com/") and "redirect_uri" in ourl and "\n" not in ourl and bool(oj.get("state")) and bool(opty) and oj.get("hint") == "agy-code", f"{st_o} url={ourl[:60]} hint={oj.get('hint')}")
        if opty:
            raw(f"/agents/api/core/projects/{P}/ptys/{opty}", "DELETE"); ost = {}
            for _ in range(10):
                time.sleep(3); ost = get(f"/agents/api/core/accounts/login/{opty}")[1] or {}
                if ost.get("status") in ("cancelled", "timeout", "done", "error"): break
            st_f, fb = raw("/agents/api/core/accounts/oauth/finish", "POST", json.dumps({"state": oj.get("state"), "code": "4/0nope"}).encode())
            ok("accounts: after the hidden terminal is gone the sign-in is cancelled and finishing with a code reports that the link must be requested again", ost.get("status") == "cancelled" and st_f >= 400 and ("заново" in (fb or b"").decode("utf8", "replace")), f"status={ost.get('status')} finish={st_f} {(fb or b'')[:80]!r}")
    # Antigravity (§4.3.17): through the gate — agy's Gemini API key mode (modelProvider gemini, GOOGLE_GEMINI_BASE_URL, model
    # gemini-api://<gate id>); without a model agentd takes the best Gemini from the gate catalog; a live turn through the
    # adapter answers with text
    if engs.get("antigravity", {}).get("installed"):
        # account: "" — no account (through the gate): after the owner signs in to Google an Antigravity session without an explicit account
        # takes it by itself (§9.11)
        ag = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "antigravity", "title": f"agy probe {MARK}", "account": ""}).encode())[1] or b"{}"); agid = ag.get("id")
        ag_text = ""; ag_note = ""; ag_st = ""; ag_model = ""
        if agid and raw(f"/agents/api/core/projects/{P}/sessions/{agid}/prompt", "POST", json.dumps({"content": f"Reply with exactly: agy-{MARK}"}).encode())[0] == 200:
            for _ in range(60):
                time.sleep(3); agi = e_sess(agid); ag_st = (agi.get("info") or {}).get("status", ""); ag_model = (agi.get("info") or {}).get("model") or ""
                evs = get(f"/agents/api/core/projects/{P}/sessions/{agid}/events?since=0&limit=80")[1] or []
                ag_text = " ".join(str(b.get("text", "")) for e in evs if e.get("kind") == "assistant" for b in (e.get("content") or []) if isinstance(b, dict) and b.get("type") == "text")
                ag_note = " | ".join((e.get("data") or {}).get("message", "") for e in evs if e.get("kind") == "system" and e.get("subtype") in ("agent_note", "provider_error"))
                if f"agy-{MARK}" in ag_text or ag_st in ("error", "exited") or "Antigravity" in ag_note: break
        ok("engines: an Antigravity session answers through the gate (agy in Gemini API-key mode, default model = best Gemini of the gate, no Google login)", agid is not None and f"agy-{MARK}" in ag_text and ag_model.startswith("gemini-"), f"status={ag_st} model={ag_model} text={ag_text[-80:]!r} note={ag_note[:160]}")
        # permissions (§4.3.17, path 2): in the "ask" mode headless agy declines the command — a note "Antigravity declined…" in the journal; in
        # the "auto" mode (a process restart with --conversation at the next message) the command runs — a Bash card with the command and output
        def ag_turn(text, secs=150):
            n0 = len(get(f"/agents/api/core/projects/{P}/sessions/{agid}/events?since=0&limit=400")[1] or [])
            if raw(f"/agents/api/core/projects/{P}/sessions/{agid}/prompt", "POST", json.dumps({"content": text}).encode())[0] != 200: return []
            for _ in range(secs // 3):
                time.sleep(3); evs = get(f"/agents/api/core/projects/{P}/sessions/{agid}/events?since=0&limit=400")[1] or []
                if any(e.get("kind") == "result" for e in evs[n0:]): return evs[n0:]
            return get(f"/agents/api/core/projects/{P}/sessions/{agid}/events?since=0&limit=400")[1][n0:]
        if agid:
            ev1 = ag_turn(f"Run the shell command `echo agy-cmd-{MARK}` and reply with its output only.")
            note1 = " | ".join((e.get("data") or {}).get("message", "") for e in ev1 if e.get("kind") == "system" and e.get("subtype") == "agent_note")
            tools1 = [b for e in ev1 if e.get("kind") == "assistant" for b in (e.get("content") or []) if isinstance(b, dict) and b.get("type") == "tool_use"]
            res1 = [b for e in ev1 if e.get("kind") == "tool_result" for b in (e.get("content") or []) if isinstance(b, dict) and b.get("type") == "tool_result"]
            ok("engines: Antigravity in the «ask» mode cannot prompt — the command card is marked declined and the journal says so (switch to auto or add a permissions.allow rule)", "Antigravity отклонил" in note1 and "авто" in note1 and all(t.get("name") == "Bash" and "echo agy-cmd" in str(t.get("input", {}).get("command", "")) for t in tools1) and any(r.get("is_error") and "отклонено" in str(r.get("content", "")) for r in res1), f"note={note1[:120]!r} tools={[(t.get('name'), t.get('input', {}).get('command')) for t in tools1]} results={[(str(r.get('content', ''))[:40], r.get('is_error')) for r in res1]}")
            st_m, _ = raw(f"/agents/api/core/projects/{P}/sessions/{agid}/mode", "POST", json.dumps({"mode": "auto"}).encode())
            ev2 = ag_turn(f"Run the shell command `echo agy-cmd-{MARK}` and reply with its output only.")
            tools2 = [b for e in ev2 if e.get("kind") == "assistant" for b in (e.get("content") or []) if isinstance(b, dict) and b.get("type") == "tool_use"]
            res2 = [b for e in ev2 if e.get("kind") == "tool_result" for b in (e.get("content") or []) if isinstance(b, dict) and b.get("type") == "tool_result"]
            spawned = any(e.get("kind") == "spawn" and "--dangerously-skip-permissions" in (e.get("cmd") or []) and "--conversation" in (e.get("cmd") or []) for e in ev2)
            ok("engines: switching Antigravity to «auto» restarts agy with the same conversation (--conversation) and the command runs — Bash card with the command, result with its output", st_m == 200 and spawned and any(t.get("name") == "Bash" for t in tools2) and any(f"agy-cmd-{MARK}" in str(r.get("content", "")) and not r.get("is_error") for r in res2), f"mode={st_m} spawned={spawned} tools={[t.get('name') for t in tools2]} results={[str(r.get('content', ''))[:60] for r in res2]}")
            info_e = e_sess(agid).get("info") or {}
            ok("engines: the Antigravity session keeps its conversation id across the restart and reports the context from the last step", bool(info_e.get("provider_session_id")) and info_e.get("alive") is True, f"psid={str(info_e.get('provider_session_id'))[:8]} alive={info_e.get('alive')}")
            for _ in range(3):
                if raw(f"/agents/api/core/projects/{P}/sessions/{agid}", "DELETE")[0] in (204, 404): break
                time.sleep(3)
    def e_events(sid_e): return get(f"/agents/api/core/projects/{P}/sessions/{sid_e}/events?since=0&limit=600")[1] or []
    def e_wait(sid_e, pred, secs=120):
        for _ in range(max(1, secs // 3)):
            time.sleep(3); evw = e_events(sid_e)
            if any(pred(e) for e in evw): return evw
        return e_events(sid_e)
    def e_results(evs): return [e for e in evs if e["kind"] == "result"]
    def e_atext(evs): return " ".join(str(b.get("text", "")) for e in evs if e["kind"] == "assistant" for b in (e.get("content") or []) if isinstance(b, dict) and b.get("type") == "text")
    def e_prompt(sid_e, text): return raw(f"/agents/api/core/projects/{P}/sessions/{sid_e}/prompt", "POST", json.dumps({"content": text}).encode())[0]
    gs = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "gemini", "title": f"gemini probe {MARK}"}).encode())[1] or b"{}"); gid = gs.get("id")
    gev = e_wait(gid, lambda e: e["kind"] == "init", 90) if gid else []; ginit = next((e for e in gev if e["kind"] == "init"), {})
    ok("gemini: the session starts over ACP through the gate — init with protocol acp, route gate, a gate Gemini model picked by agentd and the CLI's four approval modes", bool(gid) and ginit.get("protocol") == "acp" and ginit.get("route") == "gate" and str(ginit.get("model", "")).startswith("gemini-") and [m.get("id") for m in ginit.get("modes", [])] == ["default", "autoEdit", "yolo", "plan"], f"{str(gs)[:80] if not gid else ''} model={ginit.get('model')} modes={[m.get('id') for m in ginit.get('modes', [])]}")
    gfull = e_sess(gid); gc = gfull.get("caps") or {}; gmods = [m.get("value") for m in gc.get("models", [])]
    ok("gemini: caps — gate Gemini models instead of the CLI's own list (no «auto», none the CLI would rename), four modes with labels, mode default, no effort/edit/fork features", gc.get("protocol") == "acp" and bool(gmods) and all(str(m).startswith("gemini-") for m in gmods) and "auto" not in gmods and "gemini-3-flash" not in gmods and gc.get("model") == ginit.get("model") and len(gc.get("mode_options") or []) == 4 and gc.get("current_mode") == "default" and (gc.get("features") or {}).get("effort") is False and (gc.get("features") or {}).get("edit") is False, f"{gmods[:4]} model={gc.get('model')} mode={gc.get('current_mode')}")
    e_prompt(gid, f"Reply with exactly: gem-{MARK}")
    gev = e_wait(gid, lambda e: e["kind"] == "result", 150); gr = e_results(gev)
    ok("gemini: a plain turn — user, assistant text and a result with the model, stop reason, duration and tokens from _meta", len(gr) == 1 and gr[0]["subtype"] == "success" and not gr[0]["is_error"] and f"gem-{MARK}" in e_atext(gev) and gr[0].get("model") == ginit.get("model") and gr[0].get("stop_reason") == "end_turn" and any(e["kind"] == "user" for e in gev) and ((gr[0].get("usage") or {}).get("input_tokens") or 0) > 0, f"{(gr or [{}])[-1].get('subtype')} {e_atext(gev)[:60]} usage={(gr or [{}])[-1].get('usage')}")
    # a tool in the default mode: Gemini lets `echo` through as a safe command, `touch` — asks → session/request_permission → a card; "allow" → allow_once → tool_result
    e_prompt(gid, f"Run the shell command `touch /tmp/gem-{MARK}.txt && echo tool-{MARK}` and tell me its output.")
    gev = e_wait(gid, lambda e: e["kind"] == "permission_request", 120); preq = [e for e in gev if e["kind"] == "permission_request"]
    popts = [o.get("kind") for o in ((preq[-1].get("options") or []) if preq else [])]
    ok("gemini: a file-changing shell command in default mode asks for permission — request with the agent's own options and a pending entry", bool(preq) and preq[-1].get("tool_name") == "Bash" and preq[-1].get("provider") == "acp" and "allow_once" in popts and "allow_always" in popts and "reject_once" in popts and len(e_sess(gid).get("pending") or []) == 1, f"{(preq or [{}])[-1].get('tool_name')} {popts}")
    st_pa, pa = raw(f"/agents/api/core/projects/{P}/sessions/{gid}/permission", "POST", json.dumps({"request_id": (preq or [{}])[-1].get("request_id", "x"), "behavior": "allow"}).encode())
    gseq = gr[0]["seq"] if gr else 0
    gev = e_wait(gid, lambda e: e["kind"] == "result" and e["seq"] > gseq, 150); gr2 = [e for e in e_results(gev) if e["seq"] > gseq]
    tres = [e for e in gev if e["kind"] == "tool_result"]; tuse = [b for e in gev if e["kind"] == "assistant" for b in (e.get("content") or []) if isinstance(b, dict) and b.get("type") == "tool_use"]
    ok("gemini: after «allow» the command runs — tool_use Bash, tool_result with the output, the turn ends with success and nothing stays pending", st_pa == 200 and any(b.get("name") == "Bash" for b in tuse) and any(f"tool-{MARK}" in json.dumps(e.get("content")) for e in tres) and bool(gr2) and gr2[-1]["subtype"] == "success" and not (e_sess(gid).get("pending") or []), f"{st_pa} uses={len(tuse)} results={len(tres)} {str((gr2 or [{}])[-1].get('text'))[:50]}")
    st_m, mres = raw(f"/agents/api/core/projects/{P}/sessions/{gid}/mode", "POST", json.dumps({"mode": "yolo"}).encode()); mres = json.loads(mres or b"{}"); gc2 = e_sess(gid).get("caps") or {}
    time.sleep(2); gst = (e_sess(gid).get("info") or {}).get("status")
    ok("gemini: the approval mode switches live (session/set_mode → yolo), caps follow and the session stays idle (the CLI's text notice becomes a system note, not a turn)", st_m == 200 and mres.get("mode") == "yolo" and gc2.get("current_mode") == "yolo" and gst == "idle", f"{st_m} {str(mres)[:80]} caps={gc2.get('current_mode')} status={gst}")
    alt = next((m for m in gmods if m != ginit.get("model")), None)
    st_md, mdres = (raw(f"/agents/api/core/projects/{P}/sessions/{gid}/model", "POST", json.dumps({"model": alt}).encode()) if alt else (0, b"{}")); mdres = json.loads(mdres or b"{}"); gi3 = e_sess(gid)
    if not alt: print("SKIP  gemini: the gate lists a single Gemini model — the live model switch is not exercised", flush=True)
    else: ok("gemini: the model switches live (session/set_model with a gate id the CLI does not know) — caps and session info follow", bool(alt) and st_md == 200 and mdres.get("applied") == "live" and (gi3.get("caps") or {}).get("model") == alt and (gi3.get("info") or {}).get("model") == alt, f"{st_md} {alt} {str(mdres)[:60]}")
    # pi via pi-acp: the gate provider in ~/.pi/agent/models.json, the model via configOptions, tools without permissions, bash output from the pi session file, model errors —
    # from the same file
    ps = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "pi", "title": f"pi probe {MARK}", "model": "claude-sonnet-4-6"}).encode())[1] or b"{}"); pid = ps.get("id")
    pev = e_wait(pid, lambda e: e["kind"] == "init", 90) if pid else []; pinit = next((e for e in pev if e["kind"] == "init"), {})
    pc = e_sess(pid).get("caps") or {}; pmods = [(m.get("value"), m.get("label")) for m in pc.get("models", [])]
    ok("pi: the session starts over pi-acp through the gate — init with route gate and the requested model under the gate provider, no provider errors", bool(pid) and pinit.get("protocol") == "acp" and pinit.get("route") == "gate" and pinit.get("model") == "agentverse-gate/claude-sonnet-4-6" and not any(e["kind"] == "system" and e.get("subtype") == "provider_error" for e in pev), f"{pinit.get('model')} {[e.get('data') for e in pev if e['kind'] == 'system'][:1]}")
    ok("pi: caps — only gate models (labels without the provider prefix), pi's thinking levels as modes, model switching via configOptions", bool(pmods) and all(str(v).startswith("agentverse-gate/") and not str(l).startswith("agentverse-gate/") for v, l in pmods) and ("agentverse-gate/claude-sonnet-4-6", "claude-sonnet-4-6") in pmods and pc.get("protocol") == "acp" and pc.get("model") == "agentverse-gate/claude-sonnet-4-6" and any(o.get("id") == "model" for o in (pc.get("config_options") or [])), f"{len(pmods)} {pmods[:2]} model={pc.get('model')}")
    e_prompt(pid, f"Run the shell command `echo pi-{MARK}` and tell me its output.")
    pev = e_wait(pid, lambda e: e["kind"] == "result", 150); pr = e_results(pev)
    time.sleep(3); pev = e_events(pid); ptres = [e for e in pev if e["kind"] == "tool_result"]
    ok("pi: a shell tool runs without a permission request — tool_use Bash, tool_result with the output taken from pi's session file, result success with the gate model, context usage and tokens", bool(pr) and pr[-1]["subtype"] == "success" and not any(e["kind"] == "permission_request" for e in pev) and any(f"pi-{MARK}" in json.dumps(e.get("content")) for e in ptres) and f"pi-{MARK}" in e_atext(pev) and pr[-1].get("model") == "agentverse-gate/claude-sonnet-4-6" and (pr[-1].get("context") or {}).get("size") and ((pr[-1].get("usage") or {}).get("output_tokens") or 0) > 0, f"{(pr or [{}])[-1].get('subtype')} tool={str([e.get('content') for e in ptres])[:60]} {e_atext(pev)[:40]} usage={(pr or [{}])[-1].get('usage')}")
    # model error: a pi provider without a key (anthropic/…) — pi answers with an error, pi-acp stays silent, agentd takes errorMessage
    # from the session file
    st_bad, _ = raw(f"/agents/api/core/projects/{P}/sessions/{pid}/model", "POST", json.dumps({"model": "anthropic/claude-haiku-4-5"}).encode())
    pseq = pr[-1]["seq"] if pr else 0; e_prompt(pid, "Reply with exactly: never")
    pev = e_wait(pid, lambda e: e["kind"] == "result" and e["seq"] > pseq, 120); pr2 = [e for e in e_results(pev) if e["seq"] > pseq]
    ok("pi: a model pi cannot reach (its anthropic provider without a key) ends the turn as an error with pi's message (pi-acp alone reports end_turn with no text)", st_bad == 200 and bool(pr2) and pr2[-1]["is_error"] and pr2[-1]["subtype"] == "error_during_execution" and len(str(pr2[-1].get("text") or "")) > 5, f"{st_bad} {str((pr2 or [{}])[-1].get('text'))[:120]}")
    # stale pi default (slice 39): our provider's defaultModel disappeared from the gate catalog — agentd checks and changes it at session start without an
    # explicit model
    raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.pi/agent/settings.json", "PUT", b'{"defaultProvider": "agentverse-gate", "defaultModel": "claude-sonnet-0-0-stale", "quietStartup": true}', "application/octet-stream")
    ps2 = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "pi", "title": f"pi stale {MARK}"}).encode())[1] or b"{}"); pid2 = ps2.get("id")
    e_wait(pid2, lambda e: e["kind"] == "init", 90) if pid2 else []
    e_prompt(pid2, f"Reply with exactly: pi2-{MARK}")
    pev2 = e_wait(pid2, lambda e: e["kind"] == "result", 150) if pid2 else []; pr2b = e_results(pev2); p2info = e_sess(pid2).get("info") or {}
    gate_ids = [m.get("value") for m in (e_sess(pid2).get("caps") or {}).get("models", [])]
    try: s2 = json.loads(raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.pi/agent/settings.json")[1] or b"{}")
    except Exception: s2 = {}
    ok("pi: a stale default model (gone from the gate catalog) is replaced before the session starts — settings.json gets a served gate model, the session uses it and the turn succeeds",
       bool(pid2) and str(p2info.get("model", "")).startswith("agentverse-gate/") and "stale" not in str(p2info.get("model")) and (p2info.get("model") in gate_ids) and s2.get("defaultModel") not in (None, "claude-sonnet-0-0-stale") and f"agentverse-gate/{s2.get('defaultModel')}" in gate_ids and bool(pr2b) and pr2b[-1]["subtype"] == "success" and f"pi2-{MARK}" in e_atext(pev2),
       f"model={p2info.get('model')} default={s2.get('defaultModel')} results={[(r['subtype'], r.get('text', '')[:40]) for r in pr2b]}")
    if pid2:
        for _ in range(3):
            if raw(f"/agents/api/core/projects/{P}/sessions/{pid2}", "DELETE")[0] in (204, 404): break
            time.sleep(3)
    # engine TUI from the "+" menu (a PTY without a session): agentd prepares the engine configuration — pi gets the gate provider and the default model, otherwise it starts with
    # its own model without a key (401)
    raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.pi/agent/settings.json", "PUT", b'{"quietStartup": true}', "application/octet-stream")
    st_pt, ptb = raw(f"/agents/api/core/projects/{P}/ptys", "POST", json.dumps({"cmd": ["bash", "-lic", "exec pi"], "title": "pi", "engine": "pi"}).encode()); ptb = json.loads(ptb or b"{}")
    time.sleep(3); st_ps, pset = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.pi/agent/settings.json"); st_pm, pmod = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.pi/agent/models.json")
    try: pset = json.loads(pset or b"{}"); pmod = json.loads(pmod or b"{}")
    except Exception: pset, pmod = {}, {}
    gate_prov = (pmod.get("providers") or {}).get("agentverse-gate") or {}
    ok("engines: a pi TUI from the «+» menu gets the gate provider and a default model written before pi starts (models.json + settings.json)", st_pt == 200 and ptb.get("id") and st_ps == 200 and pset.get("defaultProvider") == "agentverse-gate" and str(pset.get("defaultModel", "")).startswith("claude-sonnet") and pset.get("quietStartup") is True and st_pm == 200 and gate_prov.get("api") == "openai-completions" and gate_prov.get("apiKey") == "$OPENAI_API_KEY" and len(gate_prov.get("models") or []) > 20, f"{st_pt} settings={str(pset)[:90]} models={len(gate_prov.get('models') or [])}")
    if ptb.get("id"): raw(f"/agents/api/core/projects/{P}/ptys/{ptb['id']}", "DELETE")
    # UI: the "+" menu with engines, the Gemini session card — the engine avatar, an answer, a tool card, turn totals
    pg.keyboard.press("Escape"); time.sleep(0.3); pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1)
    wse = pg.locator(f".panel .ws:has(.wname:has-text('{P}'))"); wse.locator(".wrow").hover(); wse.locator(".wrow button").click(); pg.wait_for_selector(".menu", timeout=5000); time.sleep(0.5)
    mi = [x.strip() for x in pg.locator(".menu .mi").all_inner_texts()]
    ok("engines: the «+» menu offers a chat and a TUI for every installed engine", any(x.startswith("Чат с Gemini CLI") for x in mi) and any(x.startswith("Чат с pi") for x in mi) and any(x.startswith("Gemini CLI TUI") for x in mi) and any(x.startswith("pi TUI") for x in mi) and (not engs.get("omp", {}).get("installed") or (any(x.startswith("Чат с Oh My Pi") for x in mi) and any(x.startswith("Oh My Pi TUI") for x in mi))) and any(x.startswith("Чат с Claude") for x in mi) and any(x.startswith("Чат с Codex") for x in mi), str(mi)[:220]); pg.keyboard.press("Escape"); time.sleep(0.3)
    if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5)
    pg.locator(f".panel .sess[data-id='{gid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab").first.click(); time.sleep(1)
    for sel in (f".thread .chat .m.agent:has-text('gem-{MARK}')", f".thread .chat .m.tool:has-text('tool-{MARK}')"):
        try: pg.wait_for_selector(sel, timeout=30000)
        except Exception: pass
    n_ag = pg.locator(f".thread .chat .m.agent:has-text('gem-{MARK}')").count(); n_tl = pg.locator(f".thread .chat .m.tool:has-text('tool-{MARK}')").count(); n_tn = pg.locator(".thread .chat .m.sys.turn").count(); ttxt = pg.locator(".thread .chat .tools").inner_text()
    ok("engines: the Gemini session renders like any other — engine avatar in the header, assistant text, the Bash tool card with its output, turn footers, the yolo mode in the composer and no «thinking» indicator while idle", pg.locator(".thread .t-head .av.gemini").count() >= 1 and n_ag == 1 and n_tl >= 1 and n_tn >= 2 and "YOLO" in ttxt and "думает" not in ttxt, f"av={pg.locator('.thread .t-head .av.gemini').count()} agent={n_ag} tool={n_tl} turns={n_tn} tools-text={ttxt[:80]!r}")
    pg.screenshot(path="shots/ap14-gemini.png")
    # ---- feed on the device (docs/agents-ux.md §4.3.16): a lightweight replay, "show whole", a cache in IndexedDB, reconciliation by since,
    # reset ----
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6); tree_expand(pg, P); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab:has-text('Чат')").click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000); time.sleep(1)
    # a large tool result: a Read of a 2000-line file (the CLI itself cuts Bash output to a couple of
    # kilobytes)
    raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/big-{MARK}.txt", "PUT", ("\n".join(f"line {i:05d} " + "x" * 60 for i in range(2000)) + "\n").encode(), "application/octet-stream")
    raw(f"/agents/api/core/projects/{P}/sessions/{sid}/prompt", "POST", json.dumps({"content": f"Use the Read tool to read the whole file /home/coder/big-{MARK}.txt (all lines, no offset or limit). Then reply with exactly: big-{MARK}"}).encode())
    cinfo = {}
    for _ in range(150):
        time.sleep(1); cinfo = (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {})
        st_c = cinfo.get("info", {}).get("status")
        if st_c == "waiting":
            for pr in cinfo.get("pending", []): raw(f"/agents/api/core/projects/{P}/sessions/{sid}/permission", "POST", json.dumps({"request_id": pr["request_id"], "behavior": "allow"}).encode())
        if st_c == "idle" and f"big-{MARK}" in (cinfo.get("info", {}).get("last_text") or ""): break
    lite_ev = get(f"/agents/api/core/projects/{P}/sessions/{sid}/events?since=0&limit=600&lite=1")[1] or []
    big = next((e for e in lite_ev if e.get("kind") == "tool_result" and any(b.get("truncated") for b in (e.get("content") or []) if isinstance(b, dict))), None)
    bblk = next((b for b in (big or {}).get("content", []) if isinstance(b, dict) and b.get("truncated")), {}) if big else {}
    full_ev = get(f"/agents/api/core/projects/{P}/sessions/{sid}/events/{big['seq']}")[1] if big else None
    full_len = len(json.dumps(full_ev.get("content", []), ensure_ascii=False)) if full_ev else 0
    ok("cache: lite replay trims a big tool result to ~6 KB with «truncated» and the full length, the single-event endpoint returns it whole", bool(big) and bblk.get("full_len", 0) > 20000 and len(bblk.get("content") if isinstance(bblk.get("content"), str) else json.dumps(bblk.get("content"))) <= 7000 and full_len > 20000, f"full_len={bblk.get('full_len')} lite={len(bblk.get('content') or '') if isinstance(bblk.get('content'), str) else 'arr'} full_ev={full_len}")
    time.sleep(1.5)
    # the large Read card is not necessarily the last one (under load the model adds more calls): take the one where "show whole" promises the most KB
    cards_t = pg.locator(".thread .chat .m.tool"); best_i, best_kb = cards_t.count() - 1, -1
    for i in range(cards_t.count()):
        for txt in cards_t.nth(i).locator("button.expand").all_inner_texts():
            m_kb = re.search(r"(\d+)\s*КБ", txt)
            if m_kb and int(m_kb.group(1)) > best_kb: best_kb, best_i = int(m_kb.group(1)), i
    card = cards_t.nth(max(best_i, 0))
    if card.count() and not card.evaluate("e => e.open"): card.locator("summary").first.click(); time.sleep(0.4)   # the tool card is collapsed
    ev_resp = []; pg.on("response", lambda r: ev_resp.append((r.url.split('/sessions/')[-1][-24:], r.status)) if "/events/" in r.url else None)
    ex = card.locator("button.expand")
    pre_len0 = card.locator("pre.res").evaluate("e => e.textContent.length") if card.locator("pre.res").count() else 0
    had_ex = ex.count()
    if had_ex: ex.first.click()
    pre_len1 = 0
    for _ in range(40):
        time.sleep(0.5); pre_len1 = card.locator("pre.res").evaluate("e => e.textContent.length") if card.locator("pre.res").count() else 0
        if pre_len1 > 20000: break
    ok("cache: the tool card offers «показать целиком (N КБ)» and the click loads the full event into the card", had_ex >= 1 and pre_len0 <= 4000 and pre_len1 > 20000, f"before={pre_len0} after={pre_len1} cards={pg.locator('.thread .chat .m.tool').count()} buttons_now={card.locator('button.expand').count()} resp={ev_resp} toasts={pg.locator('.toast').all_inner_texts()[:2]}")
    # cache: after a reload the feed is drawn from IndexedDB, the socket connects with since=last_seq and replays
    # nothing
    last_seq = (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("last_seq")
    time.sleep(2.5)   # delayed cache write
    ndb = pg.evaluate("""() => new Promise(res => { const r = indexedDB.open('agents-chat', 1); r.onsuccess = () => { try { const t = r.result.transaction('sessions').objectStore('sessions').count(); t.onsuccess = () => res(t.result); t.onerror = () => res(-1); } catch (e) { res(-2); } }; r.onerror = () => res(-3); })""")
    ws_urls = []; blocked_ev = []
    pg.on("websocket", lambda w: ws_urls.append(w.url) if f"/sessions/{sid}/ws" in w.url else None)
    pg.route(f"**/sessions/{sid}/events?*", lambda r: (blocked_ev.append(r.request.url), r.abort()))   # the HTTP page is unavailable: the feed can be drawn only from the cache
    pg.reload(wait_until="load"); pg.wait_for_selector(".app .rail", timeout=30000)
    try: pg.wait_for_selector(".thread .chat .m.agent", timeout=5000)
    except Exception: pass
    painted = pg.locator(".thread .chat .m.agent").count()
    pg.wait_for_selector(".thread .chat .composer-status .dot.ok, .thread .chat .composer-status .dot.working", timeout=20000); time.sleep(1)
    since_q = next((u.split("since=")[1].split("&")[0] for u in ws_urls if "since=" in u), None)
    ok("cache: the conversation is cached on the device and reopens from the cache with the events endpoint blocked — agent bubbles are on screen, the socket asks for since=last_seq with the lite flag and no stale mark remains", ndb >= 1 and painted >= 1 and since_q == str(last_seq) and any("lite=1" in u for u in ws_urls) and pg.locator(".thread .chat .cached-mark").count() == 0, f"db={ndb} painted={painted} since={since_q} last_seq={last_seq} blocked={len(blocked_ev)}")
    pg.unroute(f"**/sessions/{sid}/events?*")
    # "About": the number of conversations in the cache; "Reset cache and reload"
    # clears IndexedDB
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('О системе')").click(); time.sleep(1)
    line = pg.locator(".chat-cache-line").inner_text() if pg.locator(".chat-cache-line").count() else ""
    pg.locator(".settings .reset-cache").click(); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(2)
    ndb2 = pg.evaluate("""() => new Promise(res => { const r = indexedDB.open('agents-chat', 1); r.onsuccess = () => { try { const t = r.result.transaction('sessions').objectStore('sessions').count(); t.onsuccess = () => res(t.result); t.onerror = () => res(-1); } catch (e) { res(-2); } }; r.onerror = () => res(-3); })""")
    ok("cache: «О системе» counts cached conversations and «Сбросить кэш» empties the device cache", "кэш бесед" in line and ndb2 == 0, f"line={line[:60]!r} after={ndb2}")
    raw(f"/agents/api/core/projects/{P}/fs?path=/home/coder/big-{MARK}.txt", "DELETE")
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6); tree_expand(pg, P); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000); time.sleep(0.5)

    # ---- voice (slice 51, docs/agents-ux.md §4.7): the "Voice" tab, keys never returned to the browser, PTT dictation into both composers through a
    # mock provider ----
    VOICE_TEXT = f"диктовка работает {MARK}"; vseen = []; VOICE_REPLY = [VOICE_TEXT]   # what the mock "recognizes": dictation text or a voice command
    # streaming recognition (slice 72): the mock also answers the /v1/realtime WebSocket upgrade like OpenAI Realtime
    # transcription
    STREAM_FINAL = [f"поток работает {MARK}"]; STREAM_MODE = ["ok"]; vstream = []   # ok | refuse (503 on the upgrade) | break (an error after the first delta)
    STREAM_WORDS = ["текст по ходу", " речи"]
    class VoiceMock(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"   # the WebSocket handshake needs HTTP/1.1 (the handler answers 1.0 by default)
        def log_message(self, *a): pass
        def _json(self, code, obj): body = json.dumps(obj).encode(); self.close_connection = True; self.send_response(code); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(body))); self.send_header("Connection", "close"); self.end_headers(); self.wfile.write(body)
        def _ws(self):
            """Minimal WebSocket server on the stdlib: the handshake, masked client frames, text answers"""
            import hashlib
            key = self.headers.get("Sec-WebSocket-Key", ""); acc = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
            self.close_connection = True
            self.send_response(101); self.send_header("Upgrade", "websocket"); self.send_header("Connection", "Upgrade"); self.send_header("Sec-WebSocket-Accept", acc); self.end_headers(); self.wfile.flush()
            conn = self.connection; conn.settimeout(90)
            def send(obj, op=0x1):
                payload = json.dumps(obj).encode() if obj is not None else b""; n = len(payload)
                hdr = bytes([0x80 | op]) + (bytes([n]) if n < 126 else bytes([126]) + n.to_bytes(2, "big") if n < 65536 else bytes([127]) + n.to_bytes(8, "big"))
                conn.sendall(hdr + payload)
            def rd(n):
                buf = b""
                while len(buf) < n:
                    ch = self.rfile.read(n - len(buf))
                    if not ch: raise EOFError
                    buf += ch
                return buf
            rec = {"t": time.time(), "path": self.path, "auth": self.headers.get("Authorization", ""), "msgs": [], "audio": 0, "appends": 0}; vstream.append(rec); deltas = 0
            try:
                while True:
                    h = rd(2); op = h[0] & 0x0F; masked = h[1] & 0x80; ln = h[1] & 0x7F
                    if ln == 126: ln = int.from_bytes(rd(2), "big")
                    elif ln == 127: ln = int.from_bytes(rd(8), "big")
                    mask = rd(4) if masked else b""; data = rd(ln)
                    if masked: data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
                    if op == 8: send(None, 0x8); return
                    if op == 9: conn.sendall(bytes([0x8A, len(data)]) + data); continue
                    if op != 1: continue
                    try: m = json.loads(data.decode())
                    except Exception: continue
                    kind = m.get("type", "")
                    if kind == "input_audio_buffer.append":
                        rec["appends"] += 1; rec["audio"] += len(base64.b64decode(m.get("audio", "") or ""))
                        if rec["appends"] in (2, 6) and deltas < len(STREAM_WORDS):
                            send({"type": "conversation.item.input_audio_transcription.delta", "item_id": "it", "content_index": 0, "delta": STREAM_WORDS[deltas]}); deltas += 1
                            if STREAM_MODE[0] == "break": send({"type": "error", "error": {"type": "server_error", "message": "boom"}}); return
                        continue
                    rec["msgs"].append(m)
                    if kind == "session.update": send({"type": "session.updated", "session": m.get("session")})
                    elif kind == "input_audio_buffer.commit": send({"type": "input_audio_buffer.committed", "item_id": "it"}); send({"type": "conversation.item.input_audio_transcription.completed", "item_id": "it", "content_index": 0, "transcript": STREAM_FINAL[0]})
            except Exception: pass
        def do_GET(self):
            if self.path.startswith("/v1/models"): self._json(200, {"data": [{"id": "whisper-1"}, {"id": "gpt-4o-mini-transcribe"}, {"id": "gpt-4o"}]})
            elif self.path.startswith("/v1/user/subscription"): self._json(200, {"tier": "starter", "character_count": 120, "character_limit": 30000})
            elif self.path.startswith("/v1/realtime"):
                if STREAM_MODE[0] == "refuse": self._json(503, {"error": {"message": "realtime unavailable"}})
                else: self._ws()
            else: self._json(404, {"error": "no"})
        def do_POST(self):
            n = int(self.headers.get("Content-Length") or 0); body = self.rfile.read(n)
            vseen.append({"path": self.path, "ctype": self.headers.get("Content-Type", ""), "len": n, "auth": self.headers.get("Authorization", ""), "xi": self.headers.get("xi-api-key", ""), "body": body[:4000], "t": time.time()})
            if self.path.startswith("/v1/audio/transcriptions"): self._json(200, {"text": VOICE_REPLY[0]})
            elif self.path.startswith("/v1/speech-to-text"): self._json(200, {"text": VOICE_TEXT, "language_code": "ru"})
            elif self.path.startswith("/v1/audio/speech"):   # read-aloud (slice 64): three seconds of silence as WAV — the browser plays it without an mp3 decoder
                import io, wave
                buf = io.BytesIO(); w = wave.open(buf, "wb"); w.setnchannels(1); w.setsampwidth(2); w.setframerate(8000); w.writeframes(b"\x00\x00" * 8000 * 3); w.close(); wav = buf.getvalue()
                self.close_connection = True; self.send_response(200); self.send_header("Content-Type", "audio/wav"); self.send_header("Content-Length", str(len(wav))); self.send_header("Connection", "close"); self.end_headers(); self.wfile.write(wav)
            else: self._json(404, {"error": "no"})
    vsrv = http.server.ThreadingHTTPServer(("127.0.0.1", 18765), VoiceMock); threading.Thread(target=vsrv.serve_forever, daemon=True).start()
    def hold(btn, secs=1.5, shot=None):
        """Holding the button with the pointer; in the middle — a state snapshot (red button, a pill with a timer) and a frame"""
        bb = btn.bounding_box(); pg.mouse.move(bb["x"] + bb["width"] / 2, bb["y"] + bb["height"] / 2); pg.mouse.down(); time.sleep(secs * 0.6)
        mid = pg.evaluate("(() => { const w = document.querySelector('.ptt.rec'); const p = document.querySelector('.ptt-pill.rec'); const b = w && w.querySelector('.ptt-btn'); return { rec: !!w, pressed: b ? b.getAttribute('aria-pressed') : null, pill: p ? p.innerText : null, bg: b ? getComputedStyle(b).backgroundColor : null }; })()")
        if shot: pg.screenshot(path=f"shots/{shot}.png")
        time.sleep(secs * 0.4); pg.mouse.up(); return mid
    vsid = sid or next((x["id"] for x in (get(f"/agents/api/core/projects/{P}/sessions")[1] or []) if x.get("agent") == "claude"), None)
    def wait_text(sel, needle, secs=12):
        for _ in range(int(secs * 5)):
            if needle in (pg.locator(sel).first.input_value() if pg.locator(sel).first.evaluate("e => e.tagName") == "TEXTAREA" else pg.locator(sel).first.inner_text()): return True
            time.sleep(0.2)
        return False
    # the owner's ElevenLabs key is left alone: check on the OpenAI provider (mock), restore the original provider and
    # language at the end
    v0 = get("/agents/api/voice")[1] or {}
    raw("/agents/api/voice", "PUT", json.dumps({"provider": "openai", "language": "", "openai": {"clear": True, "model": "", "base": ""}}).encode())
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".subnav button:has-text('Голос')", timeout=20000); pg.locator(".subnav button:has-text('Голос')").click(); pg.wait_for_selector(".voice .voice-provider", timeout=20000); time.sleep(0.5)
    ok("voice: the Voice tab shows the provider switch, model, language, both key fields and the API address; dictation is reported off while the selected provider has no key", pg.locator(".voice-provider button").count() == 2 and pg.locator(".voice-provider button.active").inner_text().startswith("OpenAI") and pg.locator(".voice-model").count() == 1 and pg.locator(".voice-lang").count() == 1 and pg.locator(".voice-key-elevenlabs").count() == 1 and pg.locator(".voice-key-openai").count() == 1 and pg.locator(".voice-base").count() == 1 and pg.locator(".voice-ready:not(.ok)").count() == 1 and pg.locator(".voice-check").is_disabled())
    pg.screenshot(path="shots/ap15-voice-settings.png")
    # an OpenAI-compatible provider on the mock: model whisper-1, language ru, key, API address →
    # save
    pg.locator(".voice-provider button").nth(1).click(); pg.locator(".voice-model").select_option("whisper-1"); pg.locator(".voice-lang").select_option("ru")
    pg.locator(".voice-key-openai").fill("sk-e2e-fake-123456"); pg.locator(".voice-base").fill("http://127.0.0.1:18765/v1"); pg.locator(".voice-save").click()
    try: pg.wait_for_selector(".voice-ready.ok", timeout=10000)
    except Exception: pass
    vs = get("/agents/api/voice")[1] or {}
    ok("voice: saving stores provider, model, language, API address and the key; the browser gets only the key tail and «ready»", vs.get("provider") == "openai" and vs.get("ready") is True and vs.get("language") == "ru" and vs.get("openai", {}).get("model") == "whisper-1" and vs.get("openai", {}).get("base") == "http://127.0.0.1:18765/v1" and vs.get("openai", {}).get("key_tail") == "3456" and vs.get("openai", {}).get("has_key") is True and "fake-123456" not in json.dumps(vs) and pg.locator(".voice-ready.ok").count() == 1, f"tail={vs.get('openai', {}).get('key_tail')} ready={vs.get('ready')}")
    ok("voice: the saved key field is empty again and shows «сохранён ····3456» with a remove link", pg.locator(".voice-key-openai").input_value() == "" and "3456" in pg.locator(".voice-key-state[data-provider='openai']").inner_text() and pg.locator(".voice-key-state[data-provider='openai'] .voice-key-clear").count() == 1)
    pg.locator(".voice-check").click()
    try: pg.wait_for_selector(".voice-check-result", timeout=15000)
    except Exception: pass
    ok("voice: «Проверить ключ» asks the provider and reports the model count", pg.locator(".voice-check-result.ok").count() == 1 and "ключ действителен" in pg.locator(".voice-check-result").inner_text() and "3" in pg.locator(".voice-check-result").inner_text(), pg.locator(".voice-check-result").inner_text() if pg.locator(".voice-check-result").count() else "no result")
    # read-aloud (slice 64): the "Answer read-aloud" block — model and voice, "Test" saves the form and reads a phrase through the
    # provider (the mock returns WAV)
    tts_ok = pg.locator(".voice-tts").count() == 1 and pg.locator(".voice-tts-model option[value='gpt-4o-mini-tts']").count() == 1 and pg.locator(".voice-tts-voice option[value='alloy']").count() == 1
    pg.locator(".voice-tts-voice").select_option("nova"); n_speech0 = len([x for x in vseen if x["path"].startswith("/v1/audio/speech")])
    pg.locator(".voice-tts-try").click()
    try: pg.wait_for_selector(".voice-tts-try[aria-pressed='true']", timeout=10000)
    except Exception: pass
    sp = [x for x in vseen if x["path"].startswith("/v1/audio/speech")]; spb = json.loads(sp[-1]["body"] or b"{}") if len(sp) > n_speech0 else {}
    vs_t = get("/agents/api/voice")[1] or {}; pressed_n = pg.locator(".voice-tts-try[aria-pressed='true']").count()
    ok("voice: the TTS block offers model and voice; «Проба» saves the voice (nova) and the service asks the provider's /audio/speech with that voice and the sample text, the button turns into «Остановить» while playing", tts_ok and len(sp) > n_speech0 and spb.get("voice") == "nova" and spb.get("model") == "gpt-4o-mini-tts" and "Проверка озвучки" in spb.get("input", "") and pressed_n == 1 and vs_t.get("openai", {}).get("tts_voice") == "nova", f"tts_ok={tts_ok} req={spb.get('model')}/{spb.get('voice')} pressed={pressed_n}")
    pg.locator(".voice-tts-try").click(); time.sleep(0.4)
    ok("voice: a second click stops the playback", pg.locator(".voice-tts-try[aria-pressed='true']").count() == 0)
    # a test on the tab: hold for 1.5 s → audio (webm/opus) goes to the service → mock →
    # text
    hold(pg.locator(".voice-sample .ptt-btn"))
    got = wait_text(".voice-sample-text", VOICE_TEXT)
    ok("voice: the sample button records while held and shows the recognised text after release", got and pg.locator(".voice-sample-text").inner_text().strip() == VOICE_TEXT, pg.locator(".voice-sample-text").inner_text() if pg.locator(".voice-sample-text").count() else "no text")
    ok("voice: the service forwarded real audio as multipart to the provider with the bearer key, model, language and a webm file", bool(vseen) and vseen[-1]["path"].startswith("/v1/audio/transcriptions") and vseen[-1]["auth"] == "Bearer sk-e2e-fake-123456" and vseen[-1]["ctype"].startswith("multipart/form-data") and vseen[-1]["len"] > 1000 and all(k in vseen[-1]["body"] for k in (b'name="model"', b"whisper-1", b'name="language"', b'filename="audio.webm"')), f"len={vseen[-1]['len'] if vseen else 0}")
    pg.screenshot(path="shots/ap15-voice-sample.png")
    # a short tap — the "hold" hint, no request
    nseen = len(vseen); pg.locator(".voice-sample .ptt-btn").click(); time.sleep(0.8)
    ok("voice: a short tap does not transcribe, it shows the «hold and speak» hint", len(vseen) == nseen and pg.locator(".ptt-pill").count() == 1 and "Удерживайте" in pg.locator(".ptt-pill").inner_text(), pg.locator(".ptt-pill").inner_text() if pg.locator(".ptt-pill").count() else "no pill")
    # dictation into the agent conversation composer: text at the cursor
    # position, no send
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.8)
    if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.4)
    pg.locator(f".panel .sess[data-id='{vsid}']").click(); pg.wait_for_selector(".thread .chat .composer textarea", timeout=20000); time.sleep(0.6)
    ta_ = pg.locator(".thread .chat .composer textarea"); ta_.fill("Начало текста"); ta_.evaluate("e => { e.focus(); e.setSelectionRange(6, 6); }")
    n_user = pg.locator(".thread .chat .m.user").count()
    ok("voice: the agent conversation composer has the microphone button next to the attachments", pg.locator(".thread .chat .composer-actions .ptt-btn").count() == 1)
    mid = hold(pg.locator(".thread .chat .composer-actions .ptt-btn"), shot="ap15-voice-recording")
    ok("voice: while held the button is pressed (aria-pressed, red) and a pill shows the timer with «отпустите — вставлю текст»", mid["rec"] and mid["pressed"] == "true" and bool(mid["pill"]) and re.search(r"0:0\d / 2:00", mid["pill"] or "") and "отпустите" in (mid["pill"] or "") and mid["bg"] not in (None, "rgba(0, 0, 0, 0)", "transparent"), str(mid))
    got = wait_text(".thread .chat .composer textarea", VOICE_TEXT); time.sleep(0.5)
    # 🔊 on the agent answer bubble: the text without markdown goes to /audio/speech, the button is
    # "pressed" while it plays
    bub = pg.locator(".thread .chat .m.agent").first
    if bub.count():
        bub.hover(); n_sp = len([x for x in vseen if x["path"].startswith("/v1/audio/speech")]); bub.locator(".ma.speak").click()
        try: pg.wait_for_selector(".thread .chat .m.agent .ma.speak.on", timeout=10000)
        except Exception: pass
        sp2 = [x for x in vseen if x["path"].startswith("/v1/audio/speech")]; spb2 = json.loads(sp2[-1]["body"] or b"{}") if len(sp2) > n_sp else {}
        ok("voice: the 🔊 button on an agent reply sends its text (markdown stripped) to the provider and shows the playing state; a second click stops", len(sp2) > n_sp and bool(spb2.get("input", "").strip()) and "`" not in spb2.get("input", "") and pg.locator(".thread .chat .m.agent .ma.speak.on").count() == 1, f"input={spb2.get('input', '')[:60]!r} on={pg.locator('.thread .chat .m.agent .ma.speak.on').count()}")
        pre = pg.evaluate("() => { const b = document.querySelector('.thread .chat .m.agent .ma.speak'); return b ? b.getAttribute('aria-pressed') + '/' + b.className : 'none'; }")
        # Media Session (§9.9): while playing — the "Reading the answer" metadata and the playing state, to stop from the lock
        # screen
        ms_playing = pg.evaluate("() => ({ title: navigator.mediaSession?.metadata?.title ?? null, state: navigator.mediaSession?.playbackState ?? null })")
        bub.locator(".ma.speak").click(); time.sleep(0.5)
        ms_stopped = pg.evaluate("() => ({ title: navigator.mediaSession?.metadata?.title ?? null, state: navigator.mediaSession?.playbackState ?? null })")
        n_after_stop = len([x for x in vseen if x["path"].startswith("/v1/audio/speech")])
        ok("voice: the second click on the bubble stops the playback without a new request", pg.locator(".thread .chat .m.agent .ma.speak.on").count() == 0 and n_after_stop == len(sp2), f"before={pre} requests {len(sp2)} → {n_after_stop}")
        ok("voice: Media Session shows «Читаю ответ» while the reply is read aloud and is cleared on stop", ms_playing.get("title") == "Читаю ответ" and ms_playing.get("state") == "playing" and ms_stopped.get("state") == "none", f"{ms_playing} → {ms_stopped}")
    else: ok("voice: the 🔊 button on an agent reply sends its text (markdown stripped) to the provider and shows the playing state; a second click stops", False, "no agent bubble")
    ok("voice: the recognised text lands at the caret with spaces around it and nothing is sent", got and ta_.input_value() == f"Начало {VOICE_TEXT} текста" and pg.locator(".thread .chat .m.user").count() == n_user, repr(ta_.input_value()))
    # read-aloud as the answer goes (slice 68): the toggle in "Options", sentences go to /audio/speech as the answer arrives, the
    # "reading…" chip with a stop
    live_before = pg.locator(".thread .chat .m.agent.live").count()
    composer_option(pg, "Озвучивать ответы по ходу"); time.sleep(1.5)   # a pause: the tails of previous read-aloud requests do not fall into the check window
    auto_on = pg.evaluate("() => localStorage.getItem('agents.autoSpeak')")
    n_sp3 = len([x for x in vseen if x["path"].startswith("/v1/audio/speech")]); t_auto = time.time()
    raw(f"/agents/api/core/projects/{P}/sessions/{vsid}/prompt", "POST", json.dumps({"content": f"Reply with exactly these two sentences and nothing else: Первая фраза про {MARK}. Вторая фраза про озвучку."}).encode())
    seen_chip = False
    for _ in range(150):
        if pg.locator(".thread .chat .composer-reading").count(): seen_chip = True
        sp3 = [json.loads(x["body"] or b"{}").get("input", "") for x in [y for y in vseen if y["path"].startswith("/v1/audio/speech")][n_sp3:]]
        if seen_chip and len(sp3) >= 2 and any("Вторая фраза" in t for t in sp3): break
        time.sleep(0.4)
    sp3 = [json.loads(x["body"] or b"{}").get("input", "") for x in [y for y in vseen if y["path"].startswith("/v1/audio/speech")][n_sp3:]]
    sp3s = [t for t in sp3 if "Первая" in t or "Вторая" in t]
    ok("voice: with auto-read on, the reply's sentences go to the provider in order as they arrive and the composer shows «читаю…» with a stop", auto_on == "1" and seen_chip and len(sp3s) == 2 and f"Первая фраза про {MARK}" in sp3s[0] and "Вторая фраза" in sp3s[1] and all(t.count("Первая") + t.count("Вторая") == 1 for t in sp3s), f"auto={auto_on} chip={seen_chip} live_before={live_before} inputs={[(round(x['t'] - t_auto, 1), json.loads(x['body'] or b'{}').get('input', '')[:30]) for x in [y for y in vseen if y['path'].startswith('/v1/audio/speech')][n_sp3:]][:4]}")
    if pg.locator(".thread .chat .composer-reading").count(): pg.locator(".thread .chat .composer-reading").click(); time.sleep(0.3)
    composer_option(pg, "Озвучивать ответы по ходу"); time.sleep(0.3)
    ok("voice: the toggle switches auto-read off again (stored in the browser)", pg.evaluate("() => localStorage.getItem('agents.autoSpeak')") == "0")
    ta_.fill("")
    # voice commands (slice 69): a recognized short phrase is executed, not inserted; "текст: …" — forced insertion
    mic_ = pg.locator(".thread .chat .composer-actions .ptt-btn")
    def ui_sel_is(sid_u): return bool(sid_u) and (pg.locator(f".panel .sess.active[data-id='{sid_u}']").count() == 1 or pg.locator(f".thread [data-session='{sid_u}']").count() == 1)
    VOICE_REPLY[0] = "текст: стоп"; hold(mic_); got_t = wait_text(".thread .chat .composer textarea", "стоп", 8)
    ok("vcmd: «текст: стоп» inserts the word instead of running the command", got_t and ta_.input_value().strip() == "стоп", repr(ta_.input_value()))
    VOICE_REPLY[0] = "Очисти, пожалуйста."; hold(mic_); time.sleep(1)
    ok("vcmd: «очисти» clears the draft with a toast", ta_.input_value() == "" and pg.locator(".toast:has-text('Черновик очищен')").count() >= 1)
    VOICE_REPLY[0] = "стоп"; hold(mic_); time.sleep(1)
    ok("vcmd: «стоп» with nothing running explains itself and sends nothing", pg.locator(".toast:has-text('ничего не идёт')").count() >= 1 and ta_.input_value() == "")
    n_user_v = pg.locator(".thread .chat .m.user").count(); ta_.fill(f"Reply with exactly: vcmd-{MARK}")
    VOICE_REPLY[0] = "Отправь."; hold(mic_); toast_sent = False
    for _ in range(100):
        if pg.locator(".toast:has-text('Отправлено')").count(): toast_sent = True
        if pg.locator(".thread .chat .m.user").count() > n_user_v and toast_sent: break
        time.sleep(0.3)
    for _ in range(120):
        time.sleep(1); i_v = (get(f"/agents/api/core/projects/{P}/sessions/{vsid}")[1] or {}).get("info", {})
        if i_v.get("status") == "idle" and f"vcmd-{MARK}" in (i_v.get("last_text") or ""): break
    ok("vcmd: «отправь» sends the draft (user bubble, toast «Отправлено») and the agent answers", pg.locator(".thread .chat .m.user").count() == n_user_v + 1 and toast_sent and f"vcmd-{MARK}" in (i_v.get("last_text") or ""), f"users {n_user_v} → {pg.locator('.thread .chat .m.user').count()} toast={toast_sent} last={(i_v.get('last_text') or '')[:40]!r}")
    n_sp4 = len([x for x in vseen if x["path"].startswith("/v1/audio/speech")]); VOICE_REPLY[0] = "прочитай ответ"; hold(mic_); time.sleep(2.5)
    sp4 = [json.loads(x["body"] or b"{}").get("input", "") for x in [y for y in vseen if y["path"].startswith("/v1/audio/speech")][n_sp4:]]
    ok("vcmd: «прочитай ответ» reads the last reply aloud", any(f"vcmd-{MARK}" in t for t in sp4), str(sp4)[:80])
    if pg.locator(".thread .chat .m.agent .ma.speak.on").count(): pg.locator(".thread .chat .m.agent .ma.speak.on").first.click()
    VOICE_REPLY[0] = "новая беседа в энерджи хаб"; ids_v = {x["id"] for x in (get(f"/agents/api/core/projects/{P}/sessions")[1] or [])}; n_sess_v = len(ids_v); hold(mic_)
    toast_v = False; sess_v = []
    for _ in range(40):
        if pg.locator(".toast:has-text('Создаю беседу')").count(): toast_v = True
        sess_v = get(f"/agents/api/core/projects/{P}/sessions")[1] or []
        if len(sess_v) > n_sess_v and toast_v: break
        time.sleep(0.3)
    newest = next((x for x in sess_v if x["id"] not in ids_v), {})
    ok("vcmd: «новая беседа в энерджи хаб» matches the workspace by sound and opens a fresh Claude conversation there", len(sess_v) == n_sess_v + 1 and newest.get("agent") == "claude" and toast_v and (pg.locator(f".panel .sess[data-id='{newest.get('id')}']").count() == 1 or ui_sel_is(newest.get("id"))), f"sessions {n_sess_v} → {len(sess_v)} newest={newest.get('title')} toast={toast_v}")
    if len(sess_v) == n_sess_v + 1:
        for _ in range(3):
            if raw(f"/agents/api/core/projects/{P}/sessions/{newest['id']}", "DELETE")[0] in (204, 404): break
            time.sleep(2)
        pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6); tree_expand(pg, P); pg.locator(f".panel .sess[data-id='{vsid}']").click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000); time.sleep(0.5)
    VOICE_REPLY[0] = VOICE_TEXT
    pg.screenshot(path="shots/ap15-voice-composer.png")
    # text as you speak (slice 72, §4.7): the toggle on the "Voice" tab, interim text in the field, the result, a command, the fallback
    # path, a break
    v_stream0 = bool((get("/agents/api/voice")[1] or {}).get("stream"))
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.locator(".subnav button:has-text('Голос')").click(); pg.wait_for_selector(".voice-stream-on input", timeout=20000); time.sleep(0.4)
    ok("stream: the Voice tab shows the «text as you speak» switch (off by default) and the streaming model field for OpenAI", pg.locator(".voice-stream-on input").count() == 1 and not pg.locator(".voice-stream-on input").is_checked() and pg.locator(".voice-stream-model").count() == 1 and pg.locator(".voice-stream-model").get_attribute("placeholder") == "gpt-live-transcribe", pg.locator(".voice-stream-model").get_attribute("placeholder") or "")
    pg.locator(".voice-stream-on input").check(); pg.locator(".voice-save").click(); time.sleep(1.2)
    vs_on = get("/agents/api/voice")[1] or {}
    ok("stream: saving the tab turns streaming on (stream: true, stream_model gpt-live-transcribe)", vs_on.get("stream") is True and vs_on.get("stream_model") == "gpt-live-transcribe", str({k: vs_on.get(k) for k in ("stream", "stream_model")}))
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6); tree_expand(pg, P); pg.locator(f".panel .sess[data-id='{vsid}']").click(); pg.wait_for_selector(".thread .chat .composer-actions .ptt-btn", timeout=20000); time.sleep(0.5)
    pg.locator(".thread .chat textarea").fill(""); mic_s = pg.locator(".thread .chat .composer-actions .ptt-btn")
    def hold_stream(secs=3.2, needle=None):
        """Holding while waiting for interim text in the field; returns (the field text in the middle, the pill, data-live)"""
        bb = mic_s.bounding_box(); pg.mouse.move(bb["x"] + bb["width"] / 2, bb["y"] + bb["height"] / 2); pg.mouse.down(); t0 = time.time(); mid_val = ""; pill = ""; live = None
        while time.time() - t0 < secs:
            time.sleep(0.2); mid_val = pg.locator(".thread .chat textarea").first.input_value(); p_ = pg.locator(".ptt-pill.rec")
            if p_.count(): pill = p_.inner_text(); live = p_.get_attribute("data-live")
            if needle and needle in mid_val and time.time() - t0 > 1.2: break
        time.sleep(0.3); pg.mouse.up(); return mid_val, pill, live
    n_tr0 = len([x for x in vseen if x["path"].startswith("/v1/audio/transcriptions")]); n_st0 = len(vstream)
    mid_val, pill, live = hold_stream(needle="речи")
    got_final = wait_text(".thread .chat textarea", STREAM_FINAL[0], 10)
    ok("stream: while holding, the interim text appears in the field and the pill says «текст по ходу»", "текст по ходу" in mid_val and live == "on" and "текст по ходу" in pill, f"mid={mid_val!r} pill={pill!r} live={live}")
    ok("stream: on release the final text replaces the interim one, nothing is sent and no batch request was made", got_final and pg.locator(".thread .chat textarea").first.input_value().strip() == STREAM_FINAL[0] and len([x for x in vseen if x["path"].startswith("/v1/audio/transcriptions")]) == n_tr0, repr(pg.locator(".thread .chat textarea").first.input_value()))
    st_rec = vstream[-1] if len(vstream) > n_st0 else {}
    su = next((m for m in st_rec.get("msgs", []) if m.get("type") == "session.update"), {}); tr_cfg = (((su.get("session") or {}).get("audio") or {}).get("input") or {})
    ok("stream: the service opened the provider socket with the bearer key, configured a transcription session (gpt-live-transcribe, pcm 24 kHz, languages [ru]) and sent PCM16 audio then a commit", st_rec.get("path", "").endswith("/v1/realtime?intent=transcription") and st_rec.get("auth") == "Bearer sk-e2e-fake-123456" and (su.get("session") or {}).get("type") == "transcription" and (tr_cfg.get("format") or {}).get("rate") == 24000 and (tr_cfg.get("transcription") or {}).get("model") == "gpt-live-transcribe" and (tr_cfg.get("transcription") or {}).get("languages") == ["ru"] and st_rec.get("audio", 0) >= 30000 and any(m.get("type") == "input_audio_buffer.commit" for m in st_rec.get("msgs", [])), f"audio={st_rec.get('audio')} appends={st_rec.get('appends')} msgs={[m.get('type') for m in st_rec.get('msgs', [])]} cfg={str(tr_cfg)[:160]}")
    pg.screenshot(path="shots/ap15-voice-stream.png")
    # a command from the stream result: the interim text is removed, the
    # field is cleared
    STREAM_FINAL[0] = "Очисти, пожалуйста."; n_m_s = pg.locator(".thread .chat .m").count(); hold_stream(needle="речи"); time.sleep(1.5)
    ok("stream: a voice command as the final text is executed and leaves no interim text behind (field cleared, nothing sent)", pg.locator(".thread .chat textarea").first.input_value() == "" and pg.locator(".thread .chat .m").count() == n_m_s, repr(pg.locator(".thread .chat textarea").first.input_value()))
    STREAM_FINAL[0] = f"поток работает {MARK}"
    # a provider without realtime (503 on the upgrade): the "stream did not come up" pill, the recording goes
    # as a batch, text in the field
    STREAM_MODE[0] = "refuse"; n_tr1 = len([x for x in vseen if x["path"].startswith("/v1/audio/transcriptions")])
    mid_val, pill, live = hold_stream(secs=2.0); got_b = wait_text(".thread .chat textarea", VOICE_TEXT, 12)
    ok("stream: when the provider refuses the socket, the pill says the stream is off and the recording is transcribed in batch on release", live == "off" and got_b and pg.locator(".thread .chat textarea").first.input_value().strip() == VOICE_TEXT and len([x for x in vseen if x["path"].startswith("/v1/audio/transcriptions")]) == n_tr1 + 1, f"live={live} pill={pill!r} val={pg.locator('.thread .chat textarea').first.input_value()!r}")
    ok("stream: the fallback is explained in the pill", pg.locator(".ptt-pill.hint:has-text('поток не поднялся')").count() == 1 or pg.locator(".ptt-pill:has-text('поток не поднялся')").count() == 1, pg.locator(".ptt-pill").all_inner_texts().__str__()[:160])
    pg.locator(".thread .chat textarea").fill("")
    # a break in the middle: the provider sent a delta and an error — the interim text stays, the pill
    # explains
    STREAM_MODE[0] = "break"; n_tr2 = len([x for x in vseen if x["path"].startswith("/v1/audio/transcriptions")])
    mid_val, pill, live = hold_stream(secs=2.5, needle="текст по ходу"); time.sleep(1.5)
    ok("stream: when the stream breaks after a delta, the interim text stays in the field, no batch request is made and the pill quotes the provider error", pg.locator(".thread .chat textarea").first.input_value().strip() == "текст по ходу" and len([x for x in vseen if x["path"].startswith("/v1/audio/transcriptions")]) == n_tr2 and pg.locator(".ptt-pill.bad:has-text('поток оборвался: OpenAI: boom')").count() == 1, f"val={pg.locator('.thread .chat textarea').first.input_value()!r} pills={pg.locator('.ptt-pill').all_inner_texts()}")
    STREAM_MODE[0] = "ok"; pg.locator(".thread .chat textarea").fill("")
    raw("/agents/api/voice", "PUT", json.dumps({"stream": v_stream0}).encode())
    ok("stream: streaming switched back off for the rest of the block", (get("/agents/api/voice")[1] or {}).get("stream") is v_stream0)
    # simple chat: the same button in its
    # composer
    cst, cb = raw("/agents/api/chats", "POST", json.dumps({"title": f"voice {MARK}"}).encode()); vchat = (json.loads(cb or b"{}") if cst in (200, 201) else {}).get("id")
    if vchat:
        pg.locator(".rail .nav[data-tip='Чат']").click(); pg.wait_for_selector(".top .engine button[data-engine='simple']", timeout=20000); pg.locator(".top .engine button[data-engine='simple']").click(); time.sleep(0.6)
        pg.locator(f".panel .sess[data-chat='{vchat}']").click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000); time.sleep(0.5)
        hold(pg.locator(".thread .chat .composer-actions .ptt-btn"))
        got = wait_text(".thread .chat textarea", VOICE_TEXT)
        ok("voice: the simple chat composer dictates into its field too", got and pg.locator(".thread .chat textarea").first.input_value().strip() == VOICE_TEXT and pg.locator(".thread .chat .m").count() == 0, repr(pg.locator(".thread .chat textarea").first.input_value()))
        raw(f"/agents/api/chats/{vchat}", "DELETE")
    else: ok("voice: simple chat for dictation created", False, f"status {cst}")
    # remove the key on the tab → dictation is off, the composer button leads to Settings →
    # Voice
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.locator(".subnav button:has-text('Голос')").click(); pg.wait_for_selector(".voice-key-state[data-provider='openai'] .voice-key-clear", timeout=20000)
    pg.locator(".voice-key-state[data-provider='openai'] .voice-key-clear").click(); pg.locator(".voice-save").click(); time.sleep(1.2)
    vs = get("/agents/api/voice")[1] or {}
    ok("voice: «убрать» + save deletes the key on the server and dictation is off again", vs.get("openai", {}).get("has_key") is False and vs.get("ready") is False and pg.locator(".voice-ready:not(.ok)").count() == 1)
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6); pg.locator(f".panel .sess[data-id='{vsid}']").click(); pg.wait_for_selector(".thread .chat .composer-actions .ptt-btn", timeout=20000); time.sleep(0.4)
    pg.locator(".thread .chat .composer-actions .ptt-btn").click(); time.sleep(0.8)
    ok("voice: without a key the microphone button opens Настройки → Голос", pg.locator(".voice .voice-provider").count() == 1 and pg.locator(".subnav button.active:has-text('Голос')").count() == 1)
    raw("/agents/api/voice", "PUT", json.dumps({"provider": v0.get("provider") or "elevenlabs", "language": v0.get("language") or "", "openai": {"clear": True, "model": "", "base": ""}}).encode()); vsrv.shutdown()
    v1 = get("/agents/api/voice")[1] or {}
    ok("voice: the owner's provider settings are restored after the block (ElevenLabs key untouched)", v1.get("provider") == (v0.get("provider") or "elevenlabs") and v1.get("elevenlabs", {}).get("has_key") == v0.get("elevenlabs", {}).get("has_key") and v1.get("elevenlabs", {}).get("key_tail") == v0.get("elevenlabs", {}).get("key_tail"))
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6)

    # ---- event hub (ADR-007, docs/agents-ux.md §4.4): the tree and statuses arrive by push, polling /workspaces is off
    # ----
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1.5)
    hub_on = pg.locator(".panel .hub-live").count() == 1
    hh = get("/agents/api/health")[1] or {}
    ok("hub: the panel footer shows «хаб» (live hub) and the service reports workspace subscriptions", hub_on and (hh.get("hub_subscriptions") or 0) >= 1, f"hub_subscriptions={hh.get('hub_subscriptions')}")
    # a direct connection: the first frame is a full snapshot
    # with projects
    frames = pg.evaluate("""() => new Promise(res => { const out = []; const w = new WebSocket(location.origin.replace(/^http/, 'ws') + '/agents/api/hub'); const t = setTimeout(() => { res(out); w.close(); }, 8000); w.onmessage = e => { out.push(JSON.parse(e.data)); if (out.length >= 4) { clearTimeout(t); w.close(); res(out); } }; w.onerror = () => { clearTimeout(t); res(out); }; })""") or []
    first = frames[0] if frames else None
    ok("hub: a fresh client gets a full workspaces snapshot as the first frame", isinstance(first, dict) and first.get("kind") == "workspaces" and any(p.get("name") == P for p in first.get("projects", [])), str(first)[:100] if first else "none")
    # slice 60: next — the derived "Inbox" and "Chat" status (the service computes them from the same snapshot), no polling of
    # these sections needed
    kinds = [f.get("kind") for f in frames[1:4]]; fi = next((f for f in frames if f.get("kind") == "inbox"), {}); fc = next((f for f in frames if f.get("kind") == "chat"), {})
    ok("hub: the next frames are the spend limits, the derived inbox and the chat status, so those sections need no polling", kinds == ["limits", "inbox", "chat"] and all(k in fi for k in ("waiting", "failed", "done")) and "status" in fc and (hh.get("hub_derived") or {}).get("inbox") is True, f"kinds={kinds} derived={hh.get('hub_derived')}")
    # polling is blocked in the browser → a new session via the API must appear in the tree
    # through the hub
    blocked = []
    pg.route("**/agents/api/workspaces*", lambda r: (blocked.append(1), r.abort()))
    # who triggered a poll with a live hub: the fetch call stack from the browser (printed only on
    # failure)
    pg.evaluate("() => { window.__wsCalls = []; const f = window.fetch; window.fetch = function(u, ...a) { try { if (String(u).includes('/agents/api/workspaces')) window.__wsCalls.push(new Error().stack.split('\\n').slice(1, 6).join(' | ')); } catch (e) {} return f.call(this, u, ...a); }; }")
    tree_expand(pg, P)   # under a parallel run there are more than eight live sessions — rows behind "N more"
    hs = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"hub probe {MARK}"}).encode())[1] or b"{}"); hsid = hs.get("id")
    t0 = time.time(); seen_at = None
    for _ in range(60):
        if hsid and pg.locator(f".panel .sess[data-id='{hsid}']").count(): seen_at = time.time() - t0; break
        time.sleep(0.1)
    ok("hub: with polling blocked, a session created through the API shows up in the tree within 3 s", seen_at is not None and seen_at <= 3.0, f"after {seen_at if seen_at is not None else '>6'} s, polls blocked={len(blocked)}")
    # a status change (a turn) — also without polling: the row status
    # "thinking" → "ready"
    if hsid:
        raw(f"/agents/api/core/projects/{P}/sessions/{hsid}/prompt", "POST", json.dumps({"content": f"Reply with exactly: hub-{MARK}"}).encode())
        saw_working = False; done = False
        for _ in range(900):
            txt = pg.locator(f".panel .sess[data-id='{hsid}']").inner_text() if pg.locator(f".panel .sess[data-id='{hsid}']").count() else ""
            if "думает" in txt or "работает" in txt: saw_working = True
            if saw_working and ("готов" in txt or "завершён" in txt): done = True; break
            time.sleep(0.2)
        ok("hub: the row status goes «думает» → «готов» during a turn without a single poll", saw_working and done, f"polls blocked={len(blocked)}")
        # "Inbox" through the hub: polling /inbox is blocked, a stopped conversation with a result appears in "finished"
        # by push
        inbox_polls = []
        pg.route("**/agents/api/inbox*", lambda r: (inbox_polls.append(1), r.abort()))
        pg.locator(".rail .nav[data-tip='Входящие']").click(); time.sleep(1.2)
        ok("hub: the inbox section renders from the hub frame while /inbox is blocked", pg.locator(".inbox").count() == 1 and pg.locator(".load-error, .inbox [role='alert']").count() == 0, f"inbox polls={len(inbox_polls)}")
        raw(f"/agents/api/core/projects/{P}/sessions/{hsid}", "DELETE")   # stop → "finished" with a result
        seen_done = None; t1 = time.time()
        for _ in range(60):
            if pg.locator(f".inbox .item.done:has-text('hub probe {MARK}')").count(): seen_done = time.time() - t1; break
            time.sleep(0.1)
        ok("hub: the stopped session shows up among «завершились» within 3 s by push, no /inbox request", seen_done is not None and seen_done <= 3.0 and len(inbox_polls) == 0, f"after {seen_done if seen_done is not None else '>6'} s, inbox polls={len(inbox_polls)}")
        pg.unroute("**/agents/api/inbox*")
        pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6)
        for _ in range(3):
            if raw(f"/agents/api/core/projects/{P}/sessions/{hsid}", "DELETE")[0] in (204, 404): break
            time.sleep(3)
        gone = False
        for _ in range(50):
            if not pg.locator(f".panel .sess[data-id='{hsid}']").count(): gone = True; break
            time.sleep(0.1)
        ok("hub: a deleted session disappears from the tree by push", gone)
    pg.unroute("**/agents/api/workspaces*")
    ok("hub: the browser never needed the polling endpoint while the hub was live", len(blocked) == 0, f"blocked polls={len(blocked)}" + (f" callers={pg.evaluate('() => window.__wsCalls')}" if blocked else ""))
    # "Chat" through the hub: the scratch status comes as a `chat` frame, the simple chat list is re-read on the `chats` signal (no
    # polling every 5 s)
    chat_polls = []; chats_gets = []
    pg.route("**/agents/api/chat", lambda r: (chat_polls.append(1), r.abort()))
    pg.route("**/agents/api/chats", lambda r: (chats_gets.append(r.request.method), r.continue_()))
    pg.locator(".rail .nav[data-tip='Чат']").click(); time.sleep(1)
    if pg.locator(".chat-top .engine [data-engine='simple'], .engine [data-engine='simple']").count(): pg.locator(".engine [data-engine='simple']").first.click()   # the simple chat list (after the scratch block "advanced" may have stayed)
    time.sleep(5.5)
    n_before = len([m for m in chats_gets if m == "GET"])
    hc = json.loads(raw("/agents/api/chats", "POST", json.dumps({"title": f"hub chat {MARK}"}).encode())[1] or b"{}")
    seen_chat = None; t2 = time.time()
    for _ in range(60):
        if hc.get("id") and pg.locator(f".sess[data-chat='{hc['id']}']").count(): seen_chat = time.time() - t2; break
        time.sleep(0.1)
    n_after = len([m for m in chats_gets if m == "GET"])
    ok("hub: the chat section never polled the scratch status over 6 s and a chat created through the API appears within 3 s via the chats signal", len(chat_polls) == 0 and seen_chat is not None and seen_chat <= 3.0 and 1 <= n_after - n_before <= 2, f"status polls={len(chat_polls)} list gets before/after={n_before}/{n_after} seen={seen_chat}")
    if hc.get("id"): raw(f"/agents/api/chats/{hc['id']}", "DELETE")
    pg.unroute("**/agents/api/chat"); pg.unroute("**/agents/api/chats")
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6)

    # ---- pwa: "Agents" as a separate app (docs/agents-ux.md §9.8): its own origin :8445, manifest, service worker, offline from the cache
    # ----
    AP = int(os.environ.get("AGENTS_PORT", "8445")); AH = f"{HOST}:{AP}"
    st_sys, sysv = get("/agents/api/core/status")
    ok("pwa: the core publishes the standalone app address", st_sys == 200 and (sysv or {}).get("agents_url") == f"https://{AH}/agents/", str((sysv or {}).get("agents_url")))
    r_root = pg.request.get(f"https://{AH}/", max_redirects=0)
    ok("pwa: the root of the standalone origin redirects to /agents/", r_root.status == 302 and r_root.headers.get("location") == "/agents/", f"{r_root.status} {r_root.headers.get('location')}")
    r_sw = pg.request.get(f"https://{AH}/agents/sw.js")
    ok("pwa: the service worker script is served without long caching", r_sw.status == 200 and "javascript" in r_sw.headers.get("content-type", "") and "no-cache" in r_sw.headers.get("cache-control", ""), f"{r_sw.status} {r_sw.headers.get('content-type')} {r_sw.headers.get('cache-control')}")
    aj = get("/agents/api/core/apps")[1]; port_apps = [a for a in (aj if isinstance(aj, list) else (aj or {}).get("apps") or []) if a.get("port")]
    if port_apps:
        r_app = pg.request.get(f"https://{HOST}:{port_apps[0]['port']}/", max_redirects=3); csp = r_app.headers.get("content-security-policy", "")
        ok("pwa: route-port apps may be framed from the standalone origin too (frame-ancestors)", f"https://{AH}" in csp and f"https://{HOST}" in csp, csp[:160])
    pc = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1300, "height": 900}); pc.add_init_script("try { if (!localStorage.getItem('agents.lang')) localStorage.setItem('agents.lang', 'ru'); } catch (e) {}"); pp = pc.new_page(); perr = []; pp.on("pageerror", lambda e: perr.append(str(e)))
    pp.goto(f"https://{AH}/agents/", wait_until="load"); pp.wait_for_selector(".rail", timeout=30000); time.sleep(1.5)
    cdp = pc.new_cdp_session(pp); mf = cdp.send("Page.getAppManifest"); mfd = {}
    try: mfd = json.loads(mf.get("data") or "{}")
    except Exception: pass
    ok("pwa: on its own origin the app has a manifest with id/scope/start_url = /agents/ and no manifest errors", (mf.get("url") or "").endswith(f":{AP}/agents/manifest.webmanifest") and mfd.get("scope") == "/agents/" and mfd.get("id") == "/agents/" and mfd.get("start_url") == "/agents/" and mfd.get("display") == "standalone" and not mf.get("errors"), f"{mf.get('url')} {str(mfd)[:120]} errors={mf.get('errors')}")
    swr = pp.evaluate("() => Promise.race([navigator.serviceWorker.ready.then(r => ({scope: r.scope, active: !!r.active})), new Promise(res => setTimeout(() => res({timeout: true}), 20000))])")
    ok("pwa: the Agents service worker is registered and active with scope /agents/", bool(swr.get("active")) and str(swr.get("scope", "")).endswith(f":{AP}/agents/"), str(swr))
    pp.reload(wait_until="load"); pp.wait_for_selector(".rail", timeout=30000); time.sleep(1.5)
    ck = pp.evaluate("() => caches.keys()")
    ok("pwa: after a reload the page is controlled by the worker and the shell/assets caches exist (index.html not precached)", pp.evaluate("() => !!navigator.serviceWorker.controller") and "agents-shell" in ck and "agents-assets" in ck and pp.evaluate("() => caches.open('agents-shell').then(c => c.keys()).then(ks => ks.some(k => k.url.endsWith('/agents/')))") and not pp.evaluate("() => caches.keys().then(ks => Promise.all(ks.filter(k => k.startsWith('workbox-precache')).map(k => caches.open(k).then(c => c.keys())))).then(a => a.flat().some(k => k.url.includes('index.html')))"), str(ck))
    # the mini-head conversation: open it, let the feed cache and the tree snapshot be written; then
    # offline → reload
    tree_expand(pp, P); pp.locator(f".panel .sess[data-id='{sid}']").click(); time.sleep(1)
    if pp.locator(".top .seg button:has-text('Thread')").count(): pp.locator(".top .seg button:has-text('Thread')").click()   # a fresh browser opens the canvas
    pp.wait_for_selector(".thread .pane .tabs", timeout=30000); pp.wait_for_selector(".thread .chat .m", timeout=30000); time.sleep(3.5)
    n_online = pp.locator(".thread .chat .m").count(); ws_snap = pp.evaluate("() => (localStorage.getItem('agents.ws') || '').length")
    ok("pwa: the tree snapshot is kept on the device for offline opens", ws_snap > 1000, f"{ws_snap} bytes")
    pc.set_offline(True); pp.reload(wait_until="commit")
    try: pp.wait_for_selector(".thread .chat .m", timeout=20000)
    except Exception: pass
    got_mark = False
    for _ in range(40):
        if pp.locator(".cached-mark").count() and "Нет связи" in pp.locator(".cached-mark").inner_text(): got_mark = True; break
        time.sleep(0.3)
    ok("pwa: offline, the shell loads from the worker, the tree comes from the snapshot with a notice and the conversation renders from the device cache marked «нет связи»", pp.locator(".rail").count() == 1 and pp.locator(".load-warning.ws-offline").count() == 1 and "дерево на момент" in pp.locator(".load-warning.ws-offline").inner_text() and pp.locator(".thread .chat .m").count() >= 1 and pp.locator(".thread .chat .m").count() <= n_online and got_mark, f"rail={pp.locator('.rail').count()} warn={pp.locator('.load-warning').all_inner_texts()[:1]} msgs={pp.locator('.thread .chat .m').count()}/{n_online} mark={pp.locator('.cached-mark').all_inner_texts()[:1]}")
    pp.screenshot(path="shots/ap16-pwa-offline.png")
    pc.set_offline(False); back = False
    for _ in range(60):
        if pp.locator(".load-warning.ws-offline").count() == 0 and pp.locator(".cached-mark").count() == 0: back = True; break
        time.sleep(0.5)
    ok("pwa: back online, the snapshot notice and the cache mark go away by themselves", back and not perr, f"warn={pp.locator('.load-warning').count()} mark={pp.locator('.cached-mark').count()} errors={perr[:2]}")
    pp.locator(".rail .nav[data-tip='Настройки']").click(); pp.locator(".subnav button:has-text('О системе')").click(); time.sleep(1.5)
    ok("pwa: «О системе» on the standalone origin says the app is opened at its own address", pp.locator(".install-separate-here").count() == 1 and pp.locator(".install-separate").count() == 0)
    pc.close()
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.locator(".subnav button:has-text('О системе')").click(); time.sleep(1.5)
    ok("pwa: «О системе» on the Desktop origin shows the standalone install address with a copy button", pg.locator(".install-separate").count() == 1 and pg.locator(".install-separate .install-url").inner_text() == f"https://{AH}/agents/" and pg.locator(".install-separate .install-copy").count() == 1, pg.locator(".install-separate").all_inner_texts()[:1].__str__()[:160])
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6)
    # §9.9: the manifest to the max — shortcuts, launch_handler, share_target
    ok("pwa: the manifest has shortcuts, launch_handler focus-existing, share_target (and reachable screenshots, if listed)", [x.get("url") for x in mfd.get("shortcuts", [])] == ["/agents/?new=1", "/agents/?section=inbox", "/agents/?section=chat"] and (mfd.get("launch_handler") or {}).get("client_mode", [None])[0] == "focus-existing" and (mfd.get("share_target") or {}).get("action") == "/agents/share" and all(pg.request.get(f"https://{AH}{x['src']}").status == 200 for x in mfd.get("screenshots", [])), str({k: mfd.get(k) for k in ("shortcuts", "launch_handler")})[:200])
    # push: a push service mock in this process; a subscription with real P-256 keys from WebCrypto; a test → an encrypted POST
    # with VAPID
    pushed = []
    class PushMock(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        def log_message(self, *a): pass
        def do_POST(self):
            n = int(self.headers.get("Content-Length") or 0); body = self.rfile.read(n)
            pushed.append({"path": self.path, "ttl": self.headers.get("TTL"), "urgency": self.headers.get("Urgency"), "enc": self.headers.get("Content-Encoding"), "auth": self.headers.get("Authorization", ""), "len": n, "t": time.time()})
            code = 410 if self.path.endswith("/gone") else 201
            self.close_connection = True; self.send_response(code); self.send_header("Content-Length", "0"); self.send_header("Connection", "close"); self.end_headers()
    psrv = http.server.ThreadingHTTPServer(("127.0.0.1", 18766), PushMock); threading.Thread(target=psrv.serve_forever, daemon=True).start()
    pinfo0 = get("/agents/api/push")[1] or {}
    ok("pwa: the service publishes a VAPID public key and the push preferences (all on by default)", len(pinfo0.get("public_key", "")) == 87 and pinfo0.get("prefs", {}).get("permission") is True and pinfo0.get("prefs", {}).get("done") is True, str({k: pinfo0.get(k) for k in ("count", "prefs")}))
    keys = pg.evaluate("async () => { const k = await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits']); const raw = new Uint8Array(await crypto.subtle.exportKey('raw', k.publicKey)); const b64 = (a) => btoa(String.fromCharCode(...a)).replace(/\\+/g, '-').replace(/\\//g, '_').replace(/=+$/, ''); return { p256dh: b64(raw), auth: b64(crypto.getRandomValues(new Uint8Array(16))) }; }")
    EP_OK = "http://127.0.0.1:18766/dev/ok"; EP_GONE = "http://127.0.0.1:18766/dev/gone"
    st_s1, _ = raw("/agents/api/push/subscribe", "POST", json.dumps({"endpoint": EP_OK, "keys": keys, "ua": "e2e Chromium"}).encode())
    st_s2, _ = raw("/agents/api/push/subscribe", "POST", json.dumps({"endpoint": EP_GONE, "keys": keys, "ua": "e2e stale"}).encode())
    st_bad, _ = raw("/agents/api/push/subscribe", "POST", json.dumps({"endpoint": "http://evil.example/x", "keys": keys}).encode())
    pinfo1 = get("/agents/api/push")[1] or {}
    ok("pwa: subscriptions are stored without their keys; a non-https endpoint is refused", st_s1 == 200 and st_s2 == 200 and st_bad == 400 and pinfo1.get("count") == 2 and all("p256dh" not in x for x in pinfo1.get("subscriptions", [])) and any(x.get("ua") == "e2e Chromium" for x in pinfo1.get("subscriptions", [])), f"{st_s1} {st_s2} {st_bad} count={pinfo1.get('count')}")
    st_t, tres = raw("/agents/api/push/test", "POST", b"{}"); tres = json.loads(tres or b"{}"); time.sleep(0.5)
    pinfo2 = get("/agents/api/push")[1] or {}
    ok("pwa: a test push is encrypted (aes128gcm), signed (VAPID), carries a TTL, and a 410 from the push service drops that subscription", st_t == 200 and tres.get("sent") == 1 and len([x for x in pushed if x["path"] == "/dev/ok"]) == 1 and pushed[0]["enc"] == "aes128gcm" and pushed[0]["auth"].startswith("vapid t=") and pushed[0]["ttl"] == "21600" and pushed[0]["len"] > 100 and pinfo2.get("count") == 1 and all(not x["endpoint"].endswith("/gone") for x in pinfo2.get("subscriptions", [])), f"{st_t} {tres} pushed={[(x['path'], x['enc'], x['urgency']) for x in pushed]} count={pinfo2.get('count')}")
    # the "Notifications" tab: the push block, the device list, the event kind
    # toggle
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.locator(".subnav button:has-text('Уведомления')").click(); pg.wait_for_selector(".card-s.push", timeout=20000); time.sleep(1.2)
    ok("pwa: the Notifications tab lists the subscribed device and the four event kinds", pg.locator(".push-device").count() == 1 and "127.0.0.1" in pg.locator(".push-device").inner_text() and pg.locator(".push-pref").count() == 4 and pg.locator(".push-pref[data-kind='done']").is_checked(), pg.locator(".push-devices").inner_text()[:160])
    pg.locator(".push-pref[data-kind='done']").uncheck(); time.sleep(0.8)
    ok("pwa: unchecking «finished turns» is saved on the server", (get("/agents/api/push")[1] or {}).get("prefs", {}).get("done") is False)
    pg.locator(".push-pref[data-kind='done']").check(); time.sleep(0.8)
    # a real permission request → a push with urgency high and a badge; the answer → an "answer ready" push
    # (urgency normal)
    pg.add_init_script("navigator.setAppBadge = (n) => { window.__badge = n; return Promise.resolve(); }; navigator.clearAppBadge = () => { window.__badge = 0; return Promise.resolve(); };")
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6); pg.reload(wait_until="load"); pg.wait_for_selector(".panel .sess", timeout=30000); time.sleep(1.5)
    n_push0 = len(pushed)
    pw = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"push probe {MARK}", "permission_mode": "default"}).encode())[1] or b"{}")
    time.sleep(4); raw(f"/agents/api/core/projects/{P}/sessions/{pw['id']}/prompt", "POST", json.dumps({"content": f"Run this exact shell command with the Bash tool: touch push-ask-{MARK}.txt   (relative path, nothing else). Then reply with exactly: push-ask-{MARK}"}).encode())
    pwi = {}
    for _ in range(40):
        time.sleep(3); pwi = get(f"/agents/api/core/projects/{P}/sessions/{pw['id']}")[1] or {}
        if pwi.get("info", {}).get("status") in ("waiting", "idle", "error"): break
    perm_push = None
    for _ in range(20):
        perm_push = next((x for x in pushed[n_push0:] if x["urgency"] == "high"), None)
        if perm_push: break
        time.sleep(1)
    badge = pg.evaluate("() => window.__badge")
    ok("pwa: a pending permission request sends a high-urgency push to the subscribed device and sets the app badge", pwi.get("info", {}).get("status") == "waiting" and bool(perm_push) and perm_push["enc"] == "aes128gcm" and badge is not None and badge >= 1, f"status={pwi.get('info', {}).get('status')} pushes={[(x['urgency'], x['ttl']) for x in pushed[n_push0:]]} badge={badge}")
    pend_p = (pwi.get("pending") or [{}])[0] if pwi.get("pending") else {}
    # the answer to the request — through the worker, as by the "Deny" button in the notification (the notification-action
    # message = the same code); headless-shell forbids notifications, so this part runs in full Chromium (channel="chromium"),
    # where the permission is granted
    pb = p.chromium.launch(channel="chromium"); pw_ctx = pb.new_context(locale="ru-RU", ignore_https_errors=True); pw_ctx.grant_permissions(["notifications"], origin=f"https://{AH}")
    pwp = pw_ctx.new_page(); pwp.goto(f"https://{AH}/agents/", wait_until="load"); pwp.wait_for_selector(".rail", timeout=30000)
    pwp.evaluate("() => navigator.serviceWorker.ready.then(() => new Promise(r => setTimeout(r, 300)))")
    pwp.evaluate("() => { window.__swm = []; navigator.serviceWorker.addEventListener('message', (e) => window.__swm.push(e.data)); navigator.serviceWorker.startMessages(); }")
    pwp.evaluate("(d) => navigator.serviceWorker.ready.then(reg => reg.active.postMessage({ kind: 'notification-action', action: 'deny', data: d }))", {"project": P, "id": pw["id"], "request_id": pend_p.get("request_id", ""), "url": f"/agents/?open={P}/{pw['id']}"})
    answered = None
    for _ in range(30):
        answered = pwp.evaluate("() => (window.__swm || []).find(m => m.kind === 'permission-answered') || null")
        if answered: break
        time.sleep(0.5)
    for _ in range(40):
        time.sleep(3); pwi = get(f"/agents/api/core/projects/{P}/sessions/{pw['id']}")[1] or {}
        if pwi.get("info", {}).get("status") in ("idle", "error"): break
    done_push = None
    for _ in range(20):
        done_push = next((x for x in pushed[n_push0:] if x["urgency"] == "normal"), None)
        if done_push: break
        time.sleep(1)
    ok("pwa: «Отклонить» from the worker answers the core (windows get permission-answered) and the finished turn sends a normal-urgency push", bool(answered) and answered.get("ok") is True and pwi.get("info", {}).get("status") == "idle" and not pwi.get("pending") and bool(done_push), f"answered={answered} status={pwi.get('info', {}).get('status')} pending={len(pwi.get('pending') or [])} pushes={[(x['urgency']) for x in pushed[n_push0:]]}")
    badge2 = pg.evaluate("() => window.__badge")
    ok("pwa: the badge is cleared when nothing waits", badge2 == 0 or badge2 is None, f"badge={badge2}")
    # the same path as a real push (simulate-push → handlePush): a visible window — no notification, the window gets the data; force — the notification
    # is shown
    r1 = pwp.evaluate("(d) => new Promise((res) => { const h = (e) => { if (e.data && e.data.kind === 'push-handled') { navigator.serviceWorker.removeEventListener('message', h); res(e.data); } }; navigator.serviceWorker.addEventListener('message', h); navigator.serviceWorker.ready.then(reg => reg.active.postMessage({ kind: 'simulate-push', data: d })); setTimeout(() => res({ timeout: true }), 8000); })", {"kind": "permission", "title": "e2e ждёт решения", "body": "Bash: touch x", "project": P, "id": pw["id"], "request_id": "r-e2e", "tag": "perm-e2e", "badge": 1, "actions": [{"action": "allow", "title": "Разрешить"}, {"action": "deny", "title": "Отклонить"}]})
    shown_visible = pwp.evaluate("() => navigator.serviceWorker.ready.then(r => r.getNotifications()).then(l => l.map(n => n.title))")
    got_msg = pwp.evaluate("() => (window.__swm || []).filter(m => m.kind === 'push' && m.data && m.data.request_id === 'r-e2e').length")
    r2 = pwp.evaluate("(d) => new Promise((res) => { const h = (e) => { if (e.data && e.data.kind === 'push-handled') { navigator.serviceWorker.removeEventListener('message', h); res(e.data); } }; navigator.serviceWorker.addEventListener('message', h); navigator.serviceWorker.ready.then(reg => reg.active.postMessage({ kind: 'simulate-push', data: d })); setTimeout(() => res({ timeout: true }), 8000); })", {"kind": "permission", "title": "e2e force", "body": "Bash: rm -rf x", "project": P, "id": pw["id"], "request_id": "r-e2e-2", "tag": "perm-e2e-2", "force": True, "actions": [{"action": "allow", "title": "Разрешить"}, {"action": "deny", "title": "Отклонить"}]})
    shown_force = pwp.evaluate("() => navigator.serviceWorker.ready.then(r => r.getNotifications()).then(l => l.map(n => ({ title: n.title, actions: (n.actions || []).map(a => a.action), tag: n.tag })))")
    ok("pwa: a push handled by the worker is handed to the visible window instead of a notification; a forced one shows a notification with Allow/Deny actions", r1.get("shown") is False and r1.get("reason") == "visible" and not shown_visible and got_msg >= 1 and r2.get("shown") is True and [n["title"] for n in shown_force] == ["e2e force"] and shown_force[0]["actions"] == ["allow", "deny"], f"r1={r1} r2={r2} visible={shown_visible} msgs={got_msg} force={shown_force}")
    pwp.evaluate("() => navigator.serviceWorker.ready.then(r => r.getNotifications()).then(l => l.forEach(n => n.close()))")
    pw_ctx.close(); pb.close()
    for _ in range(3):
        if raw(f"/agents/api/core/projects/{P}/sessions/{pw['id']}", "DELETE")[0] in (204, 404): break
        time.sleep(3)
    raw("/agents/api/push/subscribe", "DELETE", json.dumps({"endpoint": EP_OK}).encode()); psrv.shutdown()
    ok("pwa: push cleanup — the e2e device is unsubscribed", (get("/agents/api/push")[1] or {}).get("count") == 0)
    # share_target: a multipart POST → 303 to /agents/?share=<id> → the text and image in the draft of the open conversation; the
    # record is one-time
    png1 = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==")
    r_sh = pg.request.post(f"https://{AH}/agents/share", multipart={"title": "Ошибка сборки", "text": f"share-{MARK}", "url": "https://example.com/log", "files": {"name": "shot.png", "mimeType": "image/png", "buffer": png1}}, max_redirects=0)
    loc = r_sh.headers.get("location", ""); share_id = loc.split("share=")[-1] if "share=" in loc else ""
    ok("pwa: share_target POST is answered with a redirect into the app carrying a one-time id", r_sh.status == 303 and loc.startswith("/agents/?share=") and len(share_id) >= 16, f"{r_sh.status} {loc}")
    sc = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1300, "height": 900}); sc.add_init_script("try { localStorage.setItem('agents.lang', 'ru'); localStorage.setItem('agents.ui', JSON.stringify({ view: 'thread', sel: { project: %r, kind: 'session', id: %r } })); } catch (e) {}" % (P, sid)); sp = sc.new_page()
    sp.goto(f"https://{AH}{loc}", wait_until="load"); sp.wait_for_selector(".thread .chat textarea", timeout=30000)
    for _ in range(50):
        if f"share-{MARK}" in sp.locator(".thread .chat textarea").first.input_value(): break
        time.sleep(0.2)
    sval = sp.locator(".thread .chat textarea").first.input_value()
    ok("pwa: the shared title, text and link land in the draft of the open conversation and the shared image is attached; the address is cleaned", "Ошибка сборки" in sval and f"share-{MARK}" in sval and "https://example.com/log" in sval and sp.locator(".thread .chat .thumbs img, .thread .chat .thumb img, .thread .chat .composer img").count() >= 1 and "share=" not in sp.url, f"val={sval[:80]!r} imgs={sp.locator('.thread .chat .composer img').count()} url={sp.url}")
    ok("pwa: a shared item can be taken only once", get(f"/agents/api/share/{share_id}")[0] == 404)
    sc.close()
    # address parameters: ?section=inbox and
    # ?open=<project>/<id>
    oc = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1300, "height": 900}); oc.add_init_script("try { localStorage.setItem('agents.lang', 'ru'); } catch (e) {}"); op = oc.new_page()
    op.goto(f"https://{AH}/agents/?section=inbox", wait_until="load"); op.wait_for_selector(".rail", timeout=30000); time.sleep(1)
    inbox_active = op.locator(".rail .nav.active[data-tip='Входящие']").count() == 1 or op.locator(".inbox").count() >= 1
    op.goto(f"https://{AH}/agents/?open={P}/{sid}", wait_until="load"); op.wait_for_selector(".thread .chat", timeout=30000); time.sleep(1)
    ok("pwa: ?section=inbox opens «Входящие», ?open=<project>/<id> opens the conversation in Thread, both are removed from the address", inbox_active and op.locator(".thread .chat textarea").count() == 1 and "open=" not in op.url and "section=" not in op.url, f"inbox={inbox_active} url={op.url}")
    # Wake Lock during dictation (a replaced navigator.wakeLock) and persistent storage by the button in "About"
    op.add_init_script("window.__wake = { req: 0, rel: 0 }; Object.defineProperty(navigator, 'wakeLock', { configurable: true, value: { request: async () => { window.__wake.req++; return { release: async () => { window.__wake.rel++; }, addEventListener() {} }; } } }); navigator.storage.persist = async () => { window.__persist = (window.__persist || 0) + 1; return true; }; navigator.storage.persisted = async () => false;")
    op.reload(wait_until="load"); op.wait_for_selector(".thread .chat .composer-actions .ptt-btn", timeout=30000); time.sleep(1)
    mic_w = op.locator(".thread .chat .composer-actions .ptt-btn"); bbw = mic_w.bounding_box(); op.mouse.move(bbw["x"] + 10, bbw["y"] + 10); op.mouse.down(); time.sleep(0.9); wk_mid = op.evaluate("() => ({ ...window.__wake })"); op.mouse.up(); time.sleep(1.5); wk_end = op.evaluate("() => ({ ...window.__wake })")
    ok("pwa: the screen wake lock is taken while dictating and released after", wk_mid["req"] >= 1 and wk_end["rel"] >= 1, f"mid={wk_mid} end={wk_end}")
    op.locator(".rail .nav[data-tip='Настройки']").click(); op.locator(".subnav button:has-text('О системе')").click(); op.wait_for_selector(".storage-line", timeout=20000); time.sleep(0.5)
    op.locator(".storage-persist").click(); time.sleep(0.8)
    ok("pwa: «О системе» shows storage usage and «make persistent» asks the browser for persistent storage", "хранилище:" in op.locator(".storage-line").inner_text() and op.evaluate("() => window.__persist") == 1, op.locator(".storage-line").inner_text()[:120])
    # outbox without a network: offline → "Send" puts into the outbox; online → goes by itself, the
    # message is in the feed
    op.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.6); op.wait_for_selector(".thread .chat textarea", timeout=30000); time.sleep(1)
    n_users0 = op.locator(".thread .chat .m.user").count()
    oc.set_offline(True); time.sleep(2.5)
    op.locator(".thread .chat textarea").fill(f"Reply with exactly: outbox-{MARK}"); op.locator(".thread .chat button.send").click(); time.sleep(1.5)
    queued = op.locator(".composer-notice.outbox").count() == 1 and "ждёт сети: 1" in op.locator(".composer-notice.outbox").inner_text() and op.locator(".thread .chat textarea").first.input_value() == ""
    ok("pwa: offline, «Отправить» puts the message into the outbox and the composer says it waits for the network", queued, op.locator(".composer-notice.outbox").all_inner_texts()[:1].__str__()[:160])
    oc.set_offline(False); sent_ok = False
    for _ in range(60):
        time.sleep(1)
        if op.locator(".composer-notice.outbox").count() == 0 and op.locator(f".thread .chat .m.user:has-text('outbox-{MARK}')").count() >= 1: sent_ok = True; break
    ok("pwa: back online, the queued message is sent by itself and appears in the conversation", sent_ok, f"notice={op.locator('.composer-notice.outbox').count()} users={op.locator('.thread .chat .m.user').count()}/{n_users0}")
    for _ in range(40):
        time.sleep(3); _oi = (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {})
        if _oi.get("status") == "idle": break
    # §9.10: a half-open socket during a live answer. Playwright's offline emulation neither breaks nor mutes open WebSockets, so we
    # mute them ourselves: the replaced WebSocket with window.__wsMute does not call onmessage/onopen (as after a network change on
    # the phone). The silence watchdog closes the socket (ping every 25 s, 40 s without frames → a 50 s tick), reconnects go deaf →
    # "reconnecting…"; removing the mute + the online event → the socket is alive right away, the feed catches up
    oc.close(); oc = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1300, "height": 900})
    oc.add_init_script("""(() => { try { localStorage.setItem('agents.lang', 'ru'); localStorage.setItem('agents.ui', JSON.stringify({ view: 'thread', sel: { project: %r, kind: 'session', id: %r } })); } catch (e) {}
      const Orig = window.WebSocket; window.__wsMute = false;
      const setter = (k) => Object.getOwnPropertyDescriptor(Orig.prototype, k).set;
      function Muted(url, protos) { const s = protos === undefined ? new Orig(url) : new Orig(url, protos); const wrap = (fn) => (ev) => { if (window.__wsMute) return; fn.call(s, ev); };
        for (const k of ['onmessage', 'onopen']) { let cur = null; Object.defineProperty(s, k, { configurable: true, get: () => cur, set: (fn) => { cur = fn; setter(k).call(s, fn ? wrap(fn) : null); } }); }
        return s; }
      Muted.prototype = Orig.prototype; Muted.CONNECTING = 0; Muted.OPEN = 1; Muted.CLOSING = 2; Muted.CLOSED = 3; window.WebSocket = Muted; })();""" % (P, sid))
    op = oc.new_page(); op.goto(f"https://{AH}/agents/", wait_until="load"); op.wait_for_selector(".thread .chat textarea", timeout=30000); time.sleep(1.5)
    op.locator(".thread .chat textarea").fill(f"Count from 1 to 40 in words, one number per line, slowly. Then reply with exactly: net-done-{MARK}"); op.locator(".thread .chat button.send").click()
    try: op.wait_for_selector(".thread .chat .m.agent.live", timeout=30000)
    except Exception: pass
    op.evaluate("() => { window.__wsMute = true; }"); t_off = time.time(); lost_at = None
    for _ in range(320):
        time.sleep(0.25)
        if "переподключение" in op.locator(".thread .chat .composer-status").inner_text(): lost_at = time.time() - t_off; break
    op.evaluate("() => { window.__wsMute = false; window.dispatchEvent(new Event('online')); }"); t_on = time.time(); back_at = None; done_at = None
    for _ in range(240):
        time.sleep(0.25)
        if back_at is None and "переподключение" not in op.locator(".thread .chat .composer-status").inner_text(): back_at = time.time() - t_on
        if op.locator(f".thread .chat .m.agent:has-text('net-done-{MARK}')").count(): done_at = time.time() - t_on; break
    ok("net: a silent (half-open) socket is detected by the watchdog within ~50 s and the composer says «переподключение…»; after the network is back the session socket is live within 3 s and the answer catches up", lost_at is not None and lost_at < 70 and back_at is not None and back_at < 3.0 and done_at is not None, f"lost_after={lost_at} back_after={back_at} done_after={done_at}")
    op.locator(".rail .nav[data-tip='Настройки']").click(); op.locator(".subnav button:has-text('О системе')").click(); op.wait_for_selector(".hub-state", timeout=45000); hub_ok = False
    for _ in range(40):
        if "опрос" not in op.locator(".hub-state").inner_text(): hub_ok = True; break
        time.sleep(0.5)
    ok("net: the event hub is live again after the network came back (no 30 s backoff wait)", hub_ok, op.locator(".hub-state").inner_text()[:80])
    oc.close()
    # ---- live answer text as it goes (slice 39, docs/agents-ux.md §4.3.12): `delta` chunks over WebSocket → the .m.agent.live bubble grows, the finished message
    # replaces it
    def live_check(label, sid_l):
        pg.wait_for_selector(f".panel .sess[data-id='{sid_l}']", timeout=30000); pg.locator(f".panel .sess[data-id='{sid_l}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab").first.click(); time.sleep(1)
        last = max([e["seq"] for e in e_events(sid_l)] or [0])
        e_prompt(sid_l, f"Write 30 numbered lines; line N must be exactly 'line N of thirty, token live-{MARK}'. No other text before or after.")
        seen = []; t0 = time.time()
        while time.time() - t0 < 75:
            lb = pg.locator(".thread .chat .m.agent.live")
            if lb.count(): seen.append(len(lb.first.inner_text()))
            if len(seen) >= 2 and seen[-1] > seen[0]: break
            time.sleep(0.1)
        evl = e_wait(sid_l, lambda e: e["kind"] == "result" and e["seq"] > last, 150); rl = [e for e in e_results(evl) if e["seq"] > last]
        try: pg.wait_for_selector(f".thread .chat .m.agent:has-text('live-{MARK}')", timeout=20000)
        except Exception: pass
        time.sleep(1.5); gone = pg.locator(".thread .chat .m.agent.live").count() == 0; final = pg.locator(f".thread .chat .m.agent:has-text('live-{MARK}')").count() >= 1
        ok(f"live: {label} streams the answer while it is written — the live bubble appears and grows, then the journal message replaces it and no live bubble stays",
           len(seen) >= 2 and seen[-1] > seen[0] and bool(rl) and rl[-1]["subtype"] == "success" and gone and final, f"seen={seen[:3]}…{seen[-2:]} n={len(seen)} gone={gone} final={final} result={[r['subtype'] for r in rl]}")
    if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5)
    live_check("Gemini CLI (ACP chunks)", gid)
    lc = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"live claude {MARK}"}).encode())[1] or b"{}"); lcid = lc.get("id")
    lx = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "codex", "title": f"live codex {MARK}"}).encode())[1] or b"{}"); lxid = lx.get("id")
    if lcid: e_wait(lcid, lambda e: e["kind"] == "init", 90); live_check("Claude Code (stream_event text deltas)", lcid)
    if lxid: e_wait(lxid, lambda e: e["kind"] == "init", 120); live_check("Codex (item/agentMessage/delta)", lxid)
    if lxid:
        # Codex context window (§9.6): for GPT-6 Codex itself takes 272k (×0.95 = 258 400) — agentd passes model_context_window from the
        # catalog/family, and tokenUsage after the turn shows the real window (1 050 000 × 0.95)
        lxs = get(f"/agents/api/core/projects/{P}/sessions/{lxid}")[1] or {}; lx_model = (lxs.get("info") or {}).get("model") or ""; lx_win = (lxs.get("caps") or {}).get("context_window")
        lx_tu = ((lxs.get("caps") or {}).get("token_usage") or {}).get("modelContextWindow")
        # Codex derives its working window from the passed 1 050 000 (828 400 on the stand — its internal calculation): the point is that
        # it is not 258 400
        ok("codex: the context window Codex reports for a GPT-6 model is derived from the real one, not the 258k default", (lx_model != "" and not lx_model.startswith("gpt-6")) or ((lx_win or 0) >= 600_000 and (lx_tu or 0) >= 600_000), f"model={lx_model!r} context_window={lx_win} tokenUsage.modelContextWindow={lx_tu}")
    for sdel in (lcid, lxid):
        if sdel:
            for _ in range(3):
                if raw(f"/agents/api/core/projects/{P}/sessions/{sdel}", "DELETE")[0] in (204, 404): break
                time.sleep(3)
    for sdel in (gid, pid):
        if sdel:
            for _ in range(3):
                if raw(f"/agents/api/core/projects/{P}/sessions/{sdel}", "DELETE")[0] in (204, 404): break
                time.sleep(3)
    ok("engines: probe sessions removed", (not gid or get(f"/agents/api/core/projects/{P}/sessions/{gid}")[0] == 404) and (not pid or get(f"/agents/api/core/projects/{P}/sessions/{pid}")[0] == 404))
    b.close()
if sid:
    for _ in range(2):
        try: urllib.request.urlopen(urllib.request.Request(f"https://{HOST}/agents/api/core/projects/{P}/sessions/{sid}", method="DELETE"), context=ctx_ssl, timeout=30)
        except Exception: pass
        time.sleep(3)
    ok("cleanup: session removed", get(f"/agents/api/core/projects/{P}/sessions/{sid}")[0] == 404)
print("ALL PASS" if all(res) else "SOME FAIL")
