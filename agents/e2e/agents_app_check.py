import os, re, time, json, ssl, urllib.request, random, string, uuid, base64, struct, zlib
from playwright.sync_api import sync_playwright
# Приложение «Агенты» как отдельный сервис (cloudos-agents на :7300, маршрут /agents/ на edge): API сервиса (здоровье, рабочие
# места, входящие, модели, настройки, прокси к ядру), затем в браузере — каркас, дерево workspace'ов, создание сессии Claude
# из меню «+», Thread с чатом и ответом модели, входящие, модели, настройки, светлая тема. Один ход модели через gate.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
GATE_APP = os.environ.get("GATE_APP", "llm-external")
# что лежит в home workspace'а PROJECT: папка с текстовым файлом для проверки «Файлы», слово из имён его файлов и бесед, фрагмент названия существующей беседы
HOME_DIR = os.environ.get("HOME_DIR", "docs"); HOME_FILE = os.environ.get("HOME_FILE", "llms.txt"); KNOWN_WORD = os.environ.get("KNOWN_WORD", "docs"); KNOWN_SESSION = os.environ.get("KNOWN_SESSION", "docs")  # приложение-провайдер llm с Management API, чью карточку проверяет блок gate
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note, f"[{time.strftime('%H:%M:%S')}]", flush=True)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def raw(path, method="GET", body=None, ctype="application/json"):
    try:
        r = urllib.request.urlopen(urllib.request.Request(f"https://{HOST}{path}", data=body, method=method, headers={"Content-Type": ctype} if body is not None else {}), context=ctx_ssl, timeout=120); return r.status, r.read()
    except urllib.error.HTTPError as e: return e.code, e.read()
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
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1500, "height": 900}); ctx.add_init_script("try { if (!localStorage.getItem('agents.lang')) localStorage.setItem('agents.lang', 'ru'); } catch (e) {}"); pg = ctx.new_page()
    errors = []; pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"https://{HOST}/agents/", wait_until="networkidle"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(2)
    ok("ui: shell with six sections", pg.locator(".rail .nav").count() >= 6)
    pg.wait_for_selector(f".panel .ws .wname:has-text('{P}')", timeout=30000); ok("ui: workspaces tree from the core", True)
    pg.screenshot(path="shots/ap1-work.png")
    # новая сессия Claude из меню «+» у workspace
    ws = pg.locator(f".panel .ws:has(.wname:has-text('{P}'))"); ws.locator(".wrow").hover(); ws.locator(".wrow button").click(); pg.wait_for_selector(".menu", timeout=5000)
    pg.locator(".menu .mi:has-text('Чат с Claude')").click(); pg.wait_for_selector(".card .chat, .thread .chat", timeout=30000); time.sleep(2)
    new = [x for x in (get(f"/agents/api/core/projects/{P}/sessions")[1] or []) if x["id"] not in before]; sid = new[0]["id"] if new else None
    ok("ui: session created via the service", bool(sid) and pg.locator(f".panel .sess[data-id='{sid}']").count() == 1)
    pg.locator(".top .seg button:has-text('Thread')").click(); pg.wait_for_selector(".thread .chat", timeout=20000); time.sleep(1)
    ok("ui: thread with session tabs", [x.strip() for x in pg.locator(".thread .pane .tabs .tab").all_inner_texts()] == ["Чат", "CLI", "Терминал", "Файлы", "Git"])
    # окно контекста: голый id нативной 1M-модели (fable) через gate Claude Code режет до 200k — agentd запускает CLI с меткой [1m] (окно 1 000 000)
    cinfo = (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}); cmods = [m.get("value") for m in ((get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("caps", {}) or {}).get("models", [])]
    ok("ctx: the catalog has a single Fable entry with the [1m] mark (1M window) and the session model, once known, carries it too", cmods.count("claude-fable-5-1[1m]") == 1 and "claude-fable-5-1" not in cmods and (cinfo.get("model") is None or str(cinfo.get("model")).endswith("[1m]")), f"model={cinfo.get('model')} fable entries={[m for m in cmods if m and 'fable' in m]}")
    # фокус с курсора/клавиатуры на столе: клик в поле ставит курсор, набор попадает в поле
    pg.locator(".thread .chat textarea").click(); pg.keyboard.type("x"); ok("ui: textarea takes focus and keystrokes on desktop", pg.evaluate("document.activeElement?.tagName") == "TEXTAREA" and pg.locator(".thread .chat textarea").input_value() == "x")
    pg.locator(".thread .chat textarea").fill(f"Reply with exactly this and nothing else: {MARK}"); pg.locator(".thread .chat button.send").click()
    pg.wait_for_selector(f".thread .chat .m.agent:has-text('{MARK}')", timeout=180000); ok("ui: model reply streamed through service → core → agentd", True)
    pg.screenshot(path="shots/ap2-thread.png")
    ok("ui: inspector shows the session", pg.locator(".inspector").is_visible() and pg.locator(".inspector select").count() >= 3)
    # композер v2 (адаптер Claude): меню модели из initialize CLI, усилие, режим; итог хода с кэшем; кольцо контекста; кэш-чип
    tools = pg.locator(".thread .chat .tools")
    # итог хода (result) приходит чуть позже текста ответа — дождаться строки итога и кэш-чипа, а не проверять «сразу после ответа»
    try: pg.wait_for_selector(".thread .chat .m.sys.turn", timeout=30000); pg.wait_for_selector(".thread .chat .tools .chip:has-text('кэш')", timeout=20000)
    except Exception: pass
    # метку `[1m]` agentd ставит после init асинхронно (set_model) — первый ход мог уйти раньше неё; опрашиваем до 10 с
    for _ in range(5):
        ctl = json.loads(raw(f"/agents/api/core/projects/{P}/sessions/{sid}/control", "POST", json.dumps({"subtype": "get_context_usage", "detail": "summary"}).encode())[1] or b"{}"); cu = ctl.get("response") or ctl
        if cu.get("maxTokens") == 1000000: break
        time.sleep(2)
    ok("ctx: Claude Code reports a 1M window for the gate session (get_context_usage.maxTokens), not 200k", cu.get("maxTokens") == 1000000 and (cu.get("autoCompactThreshold") or 0) > 900000, f"maxTokens={cu.get('maxTokens')} threshold={cu.get('autoCompactThreshold')} model={cu.get('model')}")
    ok("cmp: turn footer with duration, tokens and cache", pg.locator(".thread .chat .m.sys.turn").count() >= 1 and "кэш" in pg.locator(".thread .chat .m.sys.turn").last.inner_text())
    ok("cmp: context ring after the first turn", tools.locator(".ctl.ctx .ring").count() == 1 and "%" in tools.locator(".ctl.ctx").inner_text())
    ok("cmp: cache chip with TTL countdown", tools.locator(".chip:has-text('кэш')").count() == 1 and ("5 мин" in tools.locator(".chip:has-text('кэш')").inner_text() or "1 ч" in tools.locator(".chip:has-text('кэш')").inner_text()))
    # часовой кэш промпта: выбрать 1h в попапе (apply_flag_settings promptCacheTtl), сделать ход — usage должен показать ephemeral_1h
    tools.locator(".ctl").nth(1).click(); pg.wait_for_selector(".thread .chat .modes .seg", timeout=5000); pg.locator(".thread .chat .modes .seg button:has-text('1h')").click(); time.sleep(1.5)
    ok("cache: 1h requested through the popover (meta + journal line)", (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("cache_ttl") == "1h" and pg.locator(".thread .chat .m.sys:has-text('кэш промпта')").count() >= 1 and pg.locator(".thread .chat .modes .seg button.on:has-text('1h')").count() == 1)
    pg.keyboard.press("Escape"); pg.locator(".thread .chat textarea").fill(f"Reply with exactly this and nothing else: cache-{MARK}"); pg.locator(".thread .chat button.send").click()
    pg.wait_for_selector(f".thread .chat .m.agent:has-text('cache-{MARK}')", timeout=180000); time.sleep(1.5)
    evs_c = get(f"/agents/api/core/projects/{P}/sessions/{sid}/events?since=0&limit=500")[1] or []; last_res = [e for e in evs_c if e["kind"] == "result" and e.get("usage") and e.get("num_turns")][-1]
    cc = (last_res["usage"].get("cache_creation") or {})
    ok("cache: the API applied the 1-hour cache (ephemeral_1h in usage), chip shows 1 ч", cc.get("ephemeral_1h_input_tokens", 0) > 0 and "1 ч" in tools.locator(".chip:has-text('кэш')").inner_text(), json.dumps(cc))
    tools.locator(".ctl").nth(1).click(); pg.wait_for_selector(".thread .chat .modes .seg", timeout=5000); pg.locator(".thread .chat .modes .seg button:has-text('auto')").click(); time.sleep(1); pg.keyboard.press("Escape")
    ok("cmp: no dollar spend in the composer (header has it)", tools.locator(".chip:has-text('$')").count() == 0 and pg.locator(".thread .t-head .chip:has-text('$')").count() == 1)
    tools.locator(".ctl").nth(0).click(); pg.wait_for_selector(".menu", timeout=5000); mitems = [x.strip() for x in pg.locator(".menu .mi").all_inner_texts()]
    ok("cmp: model menu lists CLI models with context windows; native-1M families (Fable) are one entry with a 1M window, no «Fable · 200k» / «(1M context)» pair", any("Fable" in x and "1M" in x for x in mitems) and not any("Fable (1M context)" in x for x in mitems) and sum(1 for x in mitems if x.startswith("Fable")) == 1 and any("Sonnet" in x for x in mitems) and len(mitems) >= 6, str(mitems)[:200]); pg.keyboard.press("Escape")
    # «Режим и усилие» как в Claude Code: режимы с описанием (Вручную · Правки автоматически · План · Авто · Обход · Не спрашивать) и ползунок усилия
    tools.locator(".ctl").nth(1).click(); pg.wait_for_selector(".thread .chat .modes", timeout=5000); modes = [x.split("\n")[0].strip() for x in pg.locator(".thread .chat .modes .mode").all_inner_texts()]
    ok("cmp: modes popover lists Claude Code modes with descriptions", modes[:4] == ["Вручную", "Правки автоматически", "План", "Авто"] and "Обход" in modes and "Не спрашивать" not in modes and pg.locator(".thread .chat .modes .mode .muted").count() >= 5, str(modes))
    dots = [pg.locator(".thread .chat .modes .slider .dot").nth(i).get_attribute("title") for i in range(pg.locator(".thread .chat .modes .slider .dot").count())]
    ok("cmp: Claude effort slider as in its own picker (low…max, ultracode on top), original names", dots == ["low", "medium", "high", "xhigh", "max", "ultracode"], str(dots))
    ok("cmp: no Codex-only rows on a Claude session (no tier switch, no separate effort popover)", pg.locator(".thread .chat .modes .switch").count() == 0 and pg.locator(".thread .chat .effpop").count() == 0)
    pg.locator(".thread .chat .modes .slider .dot[title='high']").click(); time.sleep(2)
    einfo = get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}
    ok("cmp: effort applied from the slider (meta + journal event)", einfo.get("info", {}).get("effort") == "high" and pg.locator(".thread .chat .m.sys:has-text('усилие')").count() >= 1 and "high" in tools.locator(".ctl").nth(2).inner_text() and pg.locator(".thread .chat .modes .slider .dot.on").count() == 1)
    ok("cmp: effort row shows the level with its description like Claude Code's picker", "high" in pg.locator(".thread .chat .modes .eff:not(.sub) .mi-t").inner_text())
    pg.locator(".thread .chat .modes .slider .dot[title='ultracode']").click(); time.sleep(2)
    ok("cmp: ultracode is the top step and applies", (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("effort") == "ultracode" and "workflow" in pg.locator(".thread .chat .modes .eff:not(.sub) .mi-t").inner_text())
    pg.locator(".thread .chat .modes .eff button[title='сбросить на auto']").click(); time.sleep(1.5)
    ok("cmp: reset returns effort to auto", (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("effort") is None and "auto" in pg.locator(".thread .chat .modes .eff:not(.sub) .mi-t").inner_text())
    pg.locator(".thread .chat .modes .mode:has-text('План')").click(); time.sleep(2)
    ok("cmp: mode chosen in the popover applies to the live session", (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("permission_mode") == "plan" and "План" in tools.locator(".ctl").nth(1).inner_text())
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
    # вкладки карточки сессии в приложении: Терминал (shell в cwd), Файлы (каталог, папка, предпросмотр), Git
    pane = pg.locator(".thread .pane")
    pane.locator(".tabs .tab:has-text('Терминал')").click(); pg.wait_for_selector(".thread .pane .xterm", timeout=30000); time.sleep(2.5)
    pg.keyboard.type(f"echo tab-{MARK}"); pg.keyboard.press("Enter"); seen = ""
    for _ in range(24):
        seen = pane.locator(".xterm-rows").inner_text() if pane.locator(".xterm-rows").count() else ""
        if seen.count(f"tab-{MARK}") >= 2: break
        time.sleep(0.5)
    ok("tabs: terminal runs a shell and echoes back", seen.count(f"tab-{MARK}") >= 2, " ".join(seen.split())[:120])
    pane.locator(".tabs .tab:has-text('Файлы')").click(); pg.wait_for_selector(".thread .pane .files .list", timeout=20000); time.sleep(1)
    ok("tabs: files list the workspace home", pane.locator(f".files .list .open:has-text('{HOME_DIR}')").count() == 1)
    pane.locator(f".files .list .open:has-text('{HOME_DIR}')").click(); time.sleep(1.5); pane.locator(f".files .list .open:has-text('{HOME_FILE}')").first.click(); pg.wait_for_selector(".thread .pane .files .preview", timeout=20000); time.sleep(1)
    ok("tabs: files open a folder and preview a text file", pane.locator(f".files .crumbs .crumb:has-text('{HOME_DIR}')").count() == 1 and len(pane.locator(".files .preview").inner_text()) > 20)
    pane.locator(".tabs .tab:has-text('Git')").click(); pg.wait_for_selector(".thread .pane .git", timeout=20000); time.sleep(1.5); gtxt = pane.locator(".git").inner_text()
    ok("tabs: git pane renders for the session directory", len(gtxt.strip()) > 5 and ("/home/coder" in gtxt or pane.locator(".git .row").count() > 0 or "ветк" in gtxt.lower()), " ".join(gtxt.split())[:80])
    pane.locator(".tabs .tab:has-text('Чат')").click(); pg.wait_for_selector(".thread .chat textarea", timeout=10000)
    pg.locator(".thread .t-head button:has-text('Остановить')").click(); pg.wait_for_selector(".thread .chat .tools .st:has-text('завершён')", timeout=30000); time.sleep(1)
    pg.locator(".thread .chat textarea").click(); pg.keyboard.type("Reply with exactly this and nothing else: again-" + MARK)
    ok("ui: finished session still accepts text", pg.locator(".thread .chat textarea").input_value().endswith(MARK) and not pg.locator(".thread .chat textarea").is_disabled())
    # режим и модель на остановленной сессии: сохраняются в мету (уйдут флагами при возобновлении), без ошибки
    for _ in range(20):
        if (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("alive") is False: break
        time.sleep(0.5)
    tools.locator(".ctl").nth(1).click(); pg.wait_for_selector(".thread .chat .modes", timeout=5000); pg.locator(".thread .chat .modes .mode:has-text('План')").click()
    mmode = None
    for _ in range(12):
        time.sleep(0.5); mmode = (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("permission_mode")
        if mmode == "plan": break
    mlabel = ""
    for _ in range(12):
        mlabel = tools.locator(".ctl").nth(1).inner_text()
        if "План" in mlabel: break
        time.sleep(0.5)
    merr = pg.locator(".thread .chat .muted:has-text('не запущен')").count()
    ok("mode: switch on a stopped session persists without error", mmode == "plan" and "План" in mlabel and merr == 0, f"api={mmode} label={mlabel!r} err={merr}")
    ok("mode: bypass item blocked by the workspace policy", pg.locator(".thread .chat .modes .mode.dis:has-text('Обход')").count() == 1 and "политикой" in pg.locator(".thread .chat .modes .mode.dis").first.inner_text())
    pg.locator(".thread .chat .modes .mode:has-text('Вручную')").click(); time.sleep(1); pg.keyboard.press("Escape")
    pg.locator(".thread .chat button.send").click(); pg.wait_for_selector(f".thread .chat .m.agent:has-text('again-{MARK}')", timeout=180000); ok("ui: send resumes the session and delivers the message", True)
    # терминальный режим (CLI): та же беседа в родном TUI; headless-процесс останавливается, TUI живёт в PTY сессии с tui=true
    def tui_pty():
        return [x for x in (get(f"/agents/api/core/projects/{P}/ptys")[1] or []) if x.get("session") == sid and x.get("tui") and x.get("alive")]
    tabs = [x.strip() for x in pg.locator(".thread .pane .tabs .tab").all_inner_texts()]; ok("cli: tab in the session pane", tabs == ["Чат", "CLI", "Терминал", "Файлы", "Git"], str(tabs))
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
    # подписи «Перезапустить»/«Отключиться» в узком Thread (открыт инспектор) прячутся, остаются иконки с title
    btns = pg.locator(".thread .pane .tabs button.xs"); head = [btns.nth(0).inner_text().strip()] + [btns.nth(i).get_attribute("title") for i in (1, 2)]
    ok("cli: controls bypass / restart / disconnect", head == ["Обход", "Перезапустить", "Отключиться"], str(head))
    keys = [x.strip() for x in pg.locator(".thread .pane .tabs .keys .key").all_inner_texts()]; ok("cli: phone keys ↑ ↓ ⏎ Esc in the tab strip", keys == ["↑", "↓", "⏎", "Esc"], str(keys))
    if "trust" in seen.lower():   # диалог доверия каталогу: ↓ + ⏎ выбирают «Yes, I trust this folder» — TUI идёт дальше
        pg.locator(".thread .pane .tabs .keys .key[title='Down']").dispatch_event("pointerdown"); time.sleep(0.4); pg.locator(".thread .pane .tabs .keys .key[title='Enter']").dispatch_event("pointerdown")
        gone = False
        for _ in range(30):
            time.sleep(0.5); txt = pane.locator(".xterm-rows").inner_text() if pane.locator(".xterm-rows").count() else ""
            if "Quick safety check" not in txt: gone = True; break
        ok("cli: keys drive the TUI (trust dialog answered with ↓ ⏎)", gone, " ".join(txt.split())[:120])
    # картинка в TUI: событие paste с файлом → файл в ~/.cloudos/paste, в терминал вставляется путь, Claude Code делает вложение [Image #1]
    def png8():
        def chunk(tp, d): return struct.pack(">I", len(d)) + tp + d + struct.pack(">I", zlib.crc32(tp + d) & 0xffffffff)
        raw = b"".join(b"\x00" + b"\xff\x40\x10" * 8 for _ in range(8))
        return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 8, 8, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
    ok("cli: 📎 attach button in the tab strip", pg.locator(".thread .pane .tabs .clipbtn input[type=file]").count() == 1)
    # настоящий Ctrl+V: xterm обязан отдать его браузеру (иначе в TUI уходит ^V и Claude Code отвечает «No image found in clipboard»);
    # картинка в буфере обмена браузера — через Async Clipboard API, если он работает в headless
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
    # отправка из чата при подключённом TUI: agentd закрывает TUI и возобновляет headless-процесс
    pg.locator(".thread .pane .tabs .tab:has-text('Чат')").click(); pg.wait_for_selector(".thread .chat textarea", timeout=10000)
    ph = ""
    for _ in range(20):
        ph = pg.locator(".thread .chat textarea").get_attribute("placeholder") or ""
        if "CLI" in ph: break
        time.sleep(0.5)
    ok("cli: chat hints that sending returns the session from the CLI", "CLI" in ph, ph)
    pg.locator(".thread .chat textarea").fill(f"Reply with exactly this and nothing else: cli-{MARK}"); pg.locator(".thread .chat button.send").click()
    pg.wait_for_selector(f".thread .chat .m.agent:has-text('cli-{MARK}')", timeout=180000)
    s_back = get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}
    ok("cli: sending from chat closes the TUI and resumes the headless session", not tui_pty() and s_back.get("info", {}).get("alive") is True and not s_back["info"].get("tui"))
    pg.locator(".thread .pane .tabs .tab:has-text('CLI')").click(); pg.wait_for_selector(".thread .pane .start button:has-text('Подключиться')", timeout=10000); ok("cli: pane shows the start panel again", True)
    # обход в CLI: заблокирован политикой workspace'а; после включения политики владельцем — доступен и попадает в команду TUI
    ok("cli: bypass switch blocked by the workspace policy", pg.locator(".thread .pane .start .switch").is_disabled() and "политикой" in pg.locator(".thread .pane .start .opt").inner_text())
    raw("/agents/api/core/agents/policy", "PUT", json.dumps({"bypass": True}).encode())
    pg.locator(".rail .nav[data-tip='Входящие']").click(); time.sleep(1); pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs .tab:has-text('CLI')", timeout=20000); pg.locator(".thread .pane .tabs .tab:has-text('CLI')").click(); pg.wait_for_selector(".thread .pane .start button:has-text('Подключиться')", timeout=10000); time.sleep(1.5)
    ok("cli: bypass switch enabled once the policy allows it", not pg.locator(".thread .pane .start .switch").is_disabled())
    pg.locator(".thread .pane .start .switch").click(); pg.locator(".thread .pane .start button:has-text('Подключиться')").click(); pg.wait_for_selector(".thread .pane .xterm", timeout=30000); time.sleep(2)
    p3 = tui_pty(); ok("cli: bypass switch passes bypassPermissions to the TUI", bool(p3) and "bypassPermissions" in p3[0]["cmd"][2] and pg.locator(".thread .pane .tabs button.xs.on:has-text('Обход')").count() == 1, p3[0]["cmd"][2][-80:] if p3 else "")
    pg.locator(".thread .pane .tabs button.xs:has-text('Отключиться')").click(); pg.wait_for_selector(".thread .pane .start button:has-text('Подключиться')", timeout=20000); time.sleep(1)
    ok("cli: disconnect kills the TUI pty; session resumable", not tui_pty() and (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {}).get("alive") is False)
    pg.locator(".thread .pane .start .switch").click(); raw("/agents/api/core/agents/policy", "PUT", json.dumps({"bypass": False}).encode())
    # политика владельца: разрешить обход → файл managed-settings в workspace пустой, новая сессия стартует в bypassPermissions, live-переключение работает
    st_p, body_p = raw("/agents/api/core/agents/policy"); ok("policy: default forbids bypass", st_p == 200 and json.loads(body_p).get("bypass") is False, body_p[:80])
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Разрешения')").click(); pg.wait_for_selector(".settings .frm .switch", timeout=10000); time.sleep(1)
    sw = pg.locator(".settings .frm .switch").first
    ok("policy: switch in Settings shows the current state", not sw.evaluate("e => e.classList.contains('on')") and "запрещён" in pg.locator(".settings .frm").first.inner_text())
    sw.click(); pg.wait_for_selector("text=Политика применена", timeout=20000); time.sleep(1)
    st_p, body_p = raw("/agents/api/core/agents/policy"); pol = json.loads(body_p) if st_p == 200 else {}
    ok("policy: enabling from Settings applies to running workspaces", sw.evaluate("e => e.classList.contains('on')") and pol.get("bypass") is True and "разрешён" in pg.locator(".settings .frm").first.inner_text(), body_p[:120])
    st_f, body_f = raw(f"/agents/api/core/projects/{P}/fs/file?path=/etc/claude-code/managed-settings.json"); ok("policy: managed-settings rewritten inside the workspace", st_f == 200 and body_f.strip() == b"{}", body_f[:60])
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
    # холст: карточка с длинной историей не растёт — высота задаётся строкой сетки
    # холст на длинной беседе с диска (у тестовой сессии история короткая и может уместиться в карточку)
    pg.locator(".top .seg button:has-text('Code')").click(); time.sleep(1); pg.locator(f".panel .ws:has(.wname:has-text('{P}')) .sess:has-text('{KNOWN_SESSION}')").first.click(); time.sleep(2.5)
    card = pg.locator(".canvas .card").filter(has_text=KNOWN_SESSION)
    # в карточке может быть открыта другая вкладка — журнал есть только в «Чат»; на холсте бывают чужие живые карточки владельца
    # (раскладка общая, из ядра), из-за них клик по вкладке может «плыть» — жмём с force и не роняем прогон
    if card.count():
        try: card.first.scroll_into_view_if_needed(); card.first.locator(".tabs .tab").first.click(force=True, timeout=10000)
        except Exception as e: print("note: canvas tab click:", str(e)[:100], flush=True)
        time.sleep(2.5)
    hgt = card.first.evaluate("e => e.getBoundingClientRect().height") if card.count() else 0; scr = card.first.evaluate("e => { const l = e.querySelector('.log'); return l ? l.scrollHeight > l.clientHeight + 4 : false; }") if card.count() else False
    ok("canvas: card height is fixed by the grid row and history scrolls inside", card.count() == 1 and 380 <= hgt <= 470 and scr, f"height {hgt:.0f}, scrolls {scr}")
    if card.count():
        try: card.first.locator(".acts button[data-tip='Снять с холста']").click(force=True, timeout=10000)   # чужие живые карточки владельца могут перекрывать
        except Exception as e: print("note: remove card click:", str(e)[:100], flush=True)
        time.sleep(0.5)
    pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); time.sleep(1)
    # беседы с диска workspace'а: `claude -p` в терминале workspace'а (вне agentd) → agentd подхватывает журнал Claude Code
    # с историей; Codex — синтетический rollout. Удаление сессии убирает и файл провайдера.
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
    pg.locator(".rail .nav[data-tip='Рабочие места']").click()
    try: pg.wait_for_selector(f".panel .sess[data-id='{disk['id']}']", timeout=20000); pg.wait_for_selector(f".panel .sess[data-id='{codex['id']}']", timeout=20000)
    except Exception: pass
    ok("disk: adopted sessions appear in the tree", pg.locator(f".panel .sess[data-id='{disk['id']}']").count() == 1 and pg.locator(f".panel .sess[data-id='{codex['id']}']").count() == 1)
    # промотка на длинной подхваченной беседе: открытие сразу в конце, история окном, чтение вверх не сбивается новыми событиями
    t_open = time.time(); pg.locator(f".panel .sess:has-text('{KNOWN_SESSION}')").first.click(); pg.wait_for_selector(".thread .chat .log .m", timeout=20000)
    for _ in range(80):
        if pg.locator(".thread .chat .log .m").count() >= 40 and pg.locator(".thread .chat .log").evaluate("l => l.scrollHeight - l.scrollTop - l.clientHeight < 60"): break
        time.sleep(0.25)
    t_open = time.time() - t_open; time.sleep(1)
    ok("scroll: long conversation is fully rendered and at the end within seconds, not tens", t_open < 6, f"{t_open:.1f} s")
    at_end = pg.locator(".thread .chat .log").evaluate("l => l.scrollHeight - l.scrollTop - l.clientHeight < 60"); has_more = pg.locator(".thread .chat .log .earlier").count() == 1
    pg.locator(".thread .chat .log").evaluate("l => { l.scrollTop = 0; }"); time.sleep(1.2); stays_top = pg.locator(".thread .chat .log").evaluate("l => l.scrollTop < 40")
    ok("scroll: long conversation opens at the end, older events behind 'show more', reading up is not yanked down", at_end and has_more and stays_top, f"end {at_end} more {has_more} top {stays_top}")
    n_before = pg.locator(".thread .chat .log .m").count(); pg.locator(".thread .chat .log .earlier").click(); time.sleep(1.5); ok("scroll: 'show more' reveals earlier history", pg.locator(".thread .chat .log .earlier").count() == 0 and pg.locator(".thread .chat .log .m").count() > n_before)
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
    # адаптер Codex: свои модели, усилия (без max), режимы одобрений и только локальные команды — без ходов модели
    cx = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "codex", "title": "codex adapter"}).encode())[1])
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(3); pg.locator(f".panel .sess[data-id='{cx['id']}']").click(); pg.wait_for_selector(".thread .chat .tools", timeout=20000); time.sleep(2)
    ctools = pg.locator(".thread .chat .tools"); ctools.locator(".ctl").nth(0).click(); pg.wait_for_selector(".menu", timeout=5000); cm = [x.split("\n")[0].strip() for x in pg.locator(".menu .mi").all_inner_texts()]; pg.keyboard.press("Escape")
    # через gate каталог маршрута — всё, что прокси отдаёт в OpenAI-формате (с 2026-10 там и Claude-модели) — Sonnet в списке допустим
    ok("codex: model picker like Codex's own — Default (recommended set) then models from model/list by display name", cm[0] == "Default" and any(x.startswith("GPT-6") for x in cm) and len(cm) >= 5, str(cm)[:200])
    ctools.locator(".ctl").nth(1).click(); pg.wait_for_selector(".thread .chat .modes", timeout=5000)
    cmo = [x.split("\n")[0].strip() for x in pg.locator(".thread .chat .modes .mode").all_inner_texts()]
    ok("codex: approval modes on-request / untrusted / never, no effort rows inside", len(cmo) == 3 and not any("План" in x for x in cmo) and pg.locator(".thread .chat .modes .slider").count() == 0, str(cmo)[:160]); pg.keyboard.press("Escape")
    ctools.locator(".ctl").nth(2).click(); pg.wait_for_selector(".thread .chat .effpop", timeout=5000)
    ce = [x.strip() for x in pg.locator(".thread .chat .effpop .ep-step").all_inner_texts()]
    ok("codex: effort popover like Codex's own — gradient slider, levels incl. ultra, model name", ce == ["low", "medium", "high", "xhigh", "max", "ultra"] and pg.locator(".thread .chat .effpop input.range").count() == 1 and len(pg.locator(".thread .chat .effpop .ep-cur .muted").inner_text().strip()) > 3, str(ce) + " " + pg.locator(".thread .chat .effpop .ep-cur").inner_text().replace("\n", " "))
    pg.locator(".thread .chat .effpop .ep-step:has-text('ultra')").click(); time.sleep(1.5)
    ok("codex: choosing ultra applies and shows in the model chip", (get(f"/agents/api/core/projects/{P}/sessions/{cx['id']}")[1] or {}).get("info", {}).get("effort") == "ultra" and "ultra" in ctools.locator(".ctl").nth(0).inner_text())
    ok("codex: Fast tier from the model catalog", pg.locator(".thread .chat .effpop .ep-tier:has-text('Fast')").count() == 1)
    pg.locator(".thread .chat .effpop .ep-tier .switch").click(); time.sleep(1.5)
    ok("codex: fast tier toggles service tier (priority) for the next turn", (get(f"/agents/api/core/projects/{P}/sessions/{cx['id']}")[1] or {}).get("info", {}).get("service_tier") == "priority" and pg.locator(".thread .chat .effpop .switch.on").count() == 1)
    pg.locator(".thread .chat .effpop .ep-tier .switch").click(); time.sleep(1); pg.keyboard.press("Escape")
    cta = pg.locator(".thread .chat textarea"); cta.click(); cta.fill("/"); pg.wait_for_selector(".thread .chat .popup", timeout=5000); cc = [x.strip() for x in pg.locator(".thread .chat .popup .pi .pn").all_inner_texts()]
    ok("codex: slash popup has only local commands", set(cc) == {"/model", "/effort", "/mode"}, str(cc)); cta.fill("")
    cta.fill("см @net"); time.sleep(2.5); cf = [x.strip() for x in pg.locator(".thread .chat .popup .pi .pn").all_inner_texts()]
    ok("codex: @ popup via agentd file search", any(KNOWN_WORD in x for x in cf), str(cf)[:120]); pg.keyboard.press("Escape"); cta.fill("")
    ctools.locator(".ctl").nth(2).click(); pg.wait_for_selector(".thread .chat .effpop", timeout=5000); pg.locator(".thread .chat .effpop .ep-step:has-text('low')").click(); time.sleep(1.5); pg.keyboard.press("Escape")
    ok("codex: effort stored for the next turn", (get(f"/agents/api/core/projects/{P}/sessions/{cx['id']}")[1] or {}).get("info", {}).get("effort") == "low")
    # живой ход Codex через gate (маршрут OpenAI-семейства там нестабилен): ждём ответ или ошибку до 150 с
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
    # ---- агенты, рутины, запуски (routines.rs, раздел «Агенты») ----
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
    # первые события: status/spawn/queued/init/unqueued — сообщение рутины уходит из очереди после init (гонка resume+prompt закрыта очередью)
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
    # планировщик: cron «каждую минуту» должен выстрелить на ближайшей минуте (тик 15 с); запуск тут же выключаем и останавливаем
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
    pg.locator(".top .seg button:has-text('Запуски')").click(); time.sleep(0.6)
    ok("agents: runs tab lists both runs with summaries", pg.locator(f".runs .tr[data-run='{rr.get('id')}']").count() == 1 and pg.locator(f".runs .tr[data-run='{hk.get('run')}']").count() == 1 and pg.locator(f".runs .tr[data-run='{hk.get('run')}'] .sum").inner_text().find("hook-") >= 0)
    pg.locator(f".runs .tr[data-run='{hk.get('run')}'] button:has-text('открыть сессию')").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); time.sleep(1)
    ok("agents: 'open session' jumps to the run's session in Thread", pg.locator(".thread .t-head h2").inner_text().startswith(f"e2e routine {MARK}") and pg.locator(".thread .chat .m.agent:has-text('hook-')").count() >= 1)
    # ---- агенты v2 (срез 29): каталог навыков, место работы, полномочия, режим рутины, почта, рефлексия, бриф через ИИ ----
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
    # почта: пауза → 409; письмо владельца адресату; цепочка «ПОЧТА:» от агента к агенту и ответ в память отправителя
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
    # рефлексия: после «Поручить» память переписывается дешёвой моделью (reflected_at), остаётся в бюджете и датированной
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
    # ---- разрешения по правилам: settings workspace'а через agentd perms.rs, подсказки правил CLI, «запомнить» с областью ----
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
    # ---- фон после хода: task_started … task_notification, полоса задач, стоп, сабагенты под карточкой Agent ----
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
    # в headless-режиме CLI шлёт task_notification событием, а <task-notification>-сообщение пользователя кладёт только в транскрипт
    # (оно видно в беседах из TUI); проверяем рендер, если такое сообщение есть в журнале
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
    # карточка Agent свёрнута, вложенный <details> невидим — читаем text_content, затем раскрываем для скриншота
    sub_txt = (pg.locator(".thread .chat .m.tool .sub summary").first.text_content() or "") if pg.locator(".thread .chat .m.tool .sub").count() else "no sub group"
    ok("bg: subagent messages are grouped under the Agent tool card", pg.locator(".thread .chat .m.tool .sub").count() >= 1 and "сабагент" in sub_txt, sub_txt[:80])
    if pg.locator(".thread .chat .m.tool:has(.sub)").count(): pg.locator(".thread .chat .m.tool:has(.sub) > summary").first.click(); time.sleep(0.4); pg.locator(".thread .chat .m.tool .sub > summary").first.click(); time.sleep(0.4)
    pg.screenshot(path="shots/ap14-subagent.png")
    for sid_x in (ps.get("id"), bs.get("id")):
        for _ in range(3):
            if raw(f"/agents/api/core/projects/{P}/sessions/{sid_x}", "DELETE")[0] in (204, 404): break
            time.sleep(3)
    # ---- правка и форк сообщений, очередь, отложенная отправка, история ввода, черновик, Esc/Tab (agentd fork.rs + queue, сервис scheduled.rs) ----
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
    # форк «отсюда» со второго сообщения: новая беседа с историей до него, свой provider_session_id, открывается сама
    u2 = pg.locator(".thread .chat .m.user").nth(1); u2.hover(); u2.locator(".ma[title='Форк отсюда']").click(force=True)
    try: pg.wait_for_selector(".thread .chat .m.sys:has-text('форк беседы')", timeout=30000)
    except Exception: pass
    time.sleep(2)
    fsess = [x for x in (get(f"/agents/api/core/projects/{P}/sessions")[1] or []) if str(x.get("title", "")).startswith("⑂") and MARK in str(x.get("title", ""))]; fid = fsess[0]["id"] if fsess else None
    finfo = sinfo(fid) if fid else {}; fev = sevents(fid) if fid else []; fu = utexts(fev)
    ok("fork: ⑂ on a user bubble creates a session with the history before it, its own provider session and a «forked» row", bool(fid) and finfo.get("provider_session_id") not in (None, psid0) and len(fu) == 1 and any(e["kind"] == "forked" for e in fev) and pg.locator(f".panel .sess[data-id='{fid}']").count() == 1 and pg.locator(".thread .chat .m.sys:has-text('форк беседы')").count() == 1, f"fid={fid} users={len(fu)} psid={finfo.get('provider_session_id')}")
    pg.screenshot(path="shots/ap15-fork.png")
    turn("Which words did I ask you to remember? List them all.", timeout=120000)
    ftxt = " ".join(pg.locator(".thread .chat .m.agent").all_inner_texts())
    ok("fork: the forked CLI session knows only the history before the fork point", f"apple-{MARK}" in ftxt and f"pear-{MARK}" not in ftxt, ftxt[-160:].replace("\n", " "))
    # правка второго сообщения: журнал режется, CLI возобновляется с якоря (--resume-session-at) на той же provider-сессии
    open_sess(es["id"])
    u2 = pg.locator(".thread .chat .m.user").nth(1); u2.hover(); u2.locator(".ma[title='Править и переотправить']").click(force=True); time.sleep(0.5)
    ok("edit: ✎ on a user bubble switches the composer into edit mode with the original text", pg.locator(".thread .chat .editbar").count() == 1 and f"pear-{MARK}" in pg.locator(".thread .chat textarea").input_value(), pg.locator(".thread .chat textarea").input_value()[:60])
    pg.locator(".thread .chat textarea").fill("Which words did I ask you to remember? List them all."); pg.locator(".thread .chat button.send").click()
    # ждём по бэкенду: отметка обрезки, новый ответ после неё, сессия свободна (правка = стоп процесса + перезапуск с якоря + ход)
    for _ in range(75):
        ev1 = sevents(es["id"]); cut = [e for e in ev1 if e["kind"] == "history_truncated"]
        if cut and any(e["kind"] == "result" and e["seq"] > cut[0]["seq"] for e in ev1) and sinfo(es["id"]).get("status") == "idle": break
        time.sleep(2)
    time.sleep(2); ev1 = sevents(es["id"]); u1 = utexts(ev1); cut = [e for e in ev1 if e["kind"] == "history_truncated"]; psid1 = sinfo(es["id"]).get("provider_session_id")
    atxt = " ".join(pg.locator(".thread .chat .m.agent").all_inner_texts())
    ok("edit: the journal is cut at the edited message, the CLI resumes at the anchor on the same provider session and answers the new text", len(cut) == 1 and bool(cut[0].get("anchor")) and psid1 == psid0 and len(u1) == 2 and not any(f"pear-{MARK}" in x for x in u1) and pg.locator(".thread .chat .m.sys.cut").count() == 1 and f"apple-{MARK}" in atxt and f"pear-{MARK}" not in atxt, f"cut={len(cut)} anchor={bool(cut and cut[0].get('anchor'))} same_psid={psid1 == psid0} users={len(u1)} {atxt[-140:]}".replace("\n", " "))
    pg.screenshot(path="shots/ap16-edit.png")
    # очередь: сообщения во время хода встают в очередь agentd, × убирает, после хода первое уходит само
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
    # Esc прерывает ход
    pg.locator(".thread .chat textarea").fill(f"Use the Bash tool to run: sleep 90; echo esc-{MARK}. Then reply with exactly: slept2"); pg.locator(".thread .chat button.send").click(); time.sleep(6)
    pg.locator(".thread .chat textarea").press("Escape"); est = ""
    for _ in range(15):
        est = sinfo(es["id"]).get("status"); 
        if est == "idle": break
        time.sleep(2)
    ok("keys: Esc in the composer interrupts the running turn", est == "idle" and "slept2" not in " ".join(pg.locator(".thread .chat .m.agent").all_inner_texts()), f"status={est}")
    # ↑/↓ листают свои сообщения, Tab переключает режим, черновик переживает перезагрузку
    ta_e = pg.locator(".thread .chat textarea"); ta_e.fill(""); ta_e.press("ArrowUp"); time.sleep(0.3); v_up = ta_e.input_value(); ta_e.press("ArrowUp"); time.sleep(0.3); v_up2 = ta_e.input_value(); ta_e.press("ArrowDown"); ta_e.press("ArrowDown"); time.sleep(0.3); v_dn = ta_e.input_value()
    ok("keys: ↑ recalls previous messages, ↓ returns to the empty field", f"esc-{MARK}" in v_up and f"queued-one-{MARK}" in v_up2 and v_dn == "", f"up={v_up[:40]!r} up2={v_up2[:40]!r} down={v_dn!r}")
    m0 = sinfo(es["id"]).get("permission_mode"); ta_e.press("Tab"); time.sleep(2.5); m1 = sinfo(es["id"]).get("permission_mode")
    ok("keys: Tab cycles the permission mode of the live session", m0 == "auto" and m1 not in (None, m0), f"{m0} → {m1}")
    ta_e.fill(f"draft-{MARK}"); time.sleep(0.6); pg.reload(wait_until="networkidle"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(2); open_sess(es["id"])
    ok("keys: an unsent draft survives a reload (localStorage per session)", pg.locator(".thread .chat textarea").input_value() == f"draft-{MARK}", pg.locator(".thread .chat textarea").input_value()[:40])
    # отложенная отправка: через API на +1 ч (карточка + отмена), через меню 🕒 «своё время…» на ближайшую минуту — уходит сама планировщиком сервиса
    st_s, sm1 = raw("/agents/api/scheduled", "POST", json.dumps({"project": P, "session": es["id"], "content": f"Reply with exactly: never-sched-{MARK}", "at": int(time.time()) + 3600}).encode()); sm1 = json.loads(sm1 or b"{}")
    ok("sched: POST /agents/api/scheduled stores a pending message", st_s == 200 and sm1.get("status") == "pending" and str(sm1.get("id", "")).startswith("sm_"), f"{st_s} {json.dumps(sm1)[:100]}")
    # ближайшая минутная граница не раньше чем через 60 с (минутная точность поля: +61 с у конца минуты давало «через 1 с», и планировщик успевал раньше проверки отмены)
    when_s = time.strftime("%Y-%m-%d %H:%M", time.localtime((int(time.time() + 60) // 60 + 1) * 60)); pg.once("dialog", lambda d: d.accept(when_s))
    pg.locator(".thread .chat textarea").fill(f"Reply with exactly: sched-{MARK}"); pg.locator(".thread .chat button.sched").click(); pg.wait_for_selector(".menu .mi", timeout=5000)
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
    # ---- git-панель и worktrees (agentd fs.rs git_*, WsGit.svelte, docs/agents-ux.md §4.3.6): репозиторий и bare-remote во временных каталогах ----
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
    # каталоги транскриптов Claude Code для временных cwd (пустые после удаления сессий)
    for d in (f"-home-coder-e2e-git-{MARK}", f"-home-coder-e2e-git-{MARK}--worktrees-feat-{MARK}"): raw(f"/agents/api/core/projects/{P}/fs?path=~/.claude/projects/{d}&recursive=true", "DELETE")
    # ---- чат scratch (chat.rs, раздел «Чат»): служебный workspace, чат в своём каталоге, перенос в проект ----
    st_c, ch = raw("/agents/api/chat/ensure", "POST", b"{}"); ch = json.loads(ch or b"{}")
    for _ in range(80):
        if ch.get("status") == "running" and ch.get("reachable"): break
        time.sleep(3); ch = get("/agents/api/chat")[1] or {}
    if not (ch.get("status") == "running" and ch.get("reachable")):
        print("SKIP  chat: scratch is not running yet (", ch.get("status"), ch.get("error", ""), ") — the container takes minutes on first creation; rerun later", flush=True)
    else:
        ok("chat: scratch is a hidden core project (absent from /projects, present with ?all=1)", all(p["name"] != "scratch" for p in (get("/agents/api/core/projects")[1] or [])) and any(p["name"] == "scratch" and p["spec"].get("hidden") for p in (get("/agents/api/core/projects?all=1")[1] or [])))
        # режим auto: команда в scratch выполняется без вопроса (иначе ход встанет на карточке разрешения)
        st_n, cs = raw("/agents/api/chat", "POST", json.dumps({"agent": "claude", "permission_mode": "auto"}).encode()); cs = json.loads(cs or b"{}")
        ok("chat: new chat is a session in its own directory of scratch", st_n == 200 and cs.get("cwd", "").startswith("/home/coder/chats/") and cs.get("agent") == "claude" and cs.get("permission_mode") == "auto", f"{st_n} {cs.get('cwd')}")
        pg.locator(".rail .nav[data-tip='Чат']").click(); pg.wait_for_selector(".panel .engine button[data-engine='advanced']", timeout=20000); pg.locator(".panel .engine button[data-engine='advanced']").click(); time.sleep(0.5); pg.wait_for_selector(f".panel .sess[data-id='{cs.get('id')}']", timeout=20000); pg.locator(f".panel .sess[data-id='{cs.get('id')}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab").first.click(); time.sleep(1)
        ok("chat: section lists the chat and opens it without a Git tab", [x.strip() for x in pg.locator(".thread .pane .tabs .tab").all_inner_texts()] == ["Чат", "CLI", "Терминал", "Файлы"], str(pg.locator(".thread .pane .tabs .tab").all_inner_texts()))
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
    # вход OAuth из Настроек: терминал входа открывается и печатает ссылку; импорт до завершения входа — понятная ошибка
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Аккаунты')").click(); time.sleep(0.5)
    # Claude «по ссылке» = OAuth-ссылка (возврат на localhost:54545, адрес вставляется обратно); «по коду» = код со страницы
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
    # ChatGPT (Codex) «по ссылке» = OAuth-ссылка auth.openai.com с возвратом на localhost:1455; «по коду» = device-code
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
    # провайдеры через gate: карточка с входами локального прокси (management API), меню «войти ▾» и «плагины ▾», device-вход Kimi даёт ссылку и код
    pg.wait_for_selector(".settings .gate .gtarget", timeout=30000); gtg = pg.locator(f".settings .gate .gtarget[data-app='{GATE_APP}']"); time.sleep(0.5)
    gprov = [x.strip() for x in gtg.locator(".glogin-row .gp").all_inner_texts()]
    ok("gate: the providers card lists the local LLM proxy with its sign-ins (auth files) read through the Management API", gtg.count() == 1 and gtg.locator(".dot.ok").count() == 1 and "antigravity" in gprov, str(gprov)[:120])
    gtg.locator("button.glogin").click(); pg.wait_for_selector(".menu .mi", timeout=5000); gm = [x.split("\n")[0].strip() for x in pg.locator(".menu .mi").all_inner_texts()]
    ok("gate: the sign-in menu offers Antigravity, Kimi, xAI Grok, ChatGPT and Claude", all(any(n in x for x in gm) for n in ("Antigravity", "Kimi", "xAI", "ChatGPT", "Claude")), str(gm)[:160])
    # ChatGPT и Claude в gate — OAuth-ссылки самого прокси (возврат на localhost:1455 / :54545, адрес вставляется обратно), как в панели прокси
    pg.locator(".menu .mi:has-text('ChatGPT (Codex)')").first.click(); pg.wait_for_selector(".settings .gflow input.mono, .settings .gflow .err", timeout=45000); time.sleep(0.5)
    curl = pg.locator(".settings .gflow a:has-text('Открыть ссылку')").get_attribute("href") if pg.locator(".settings .gflow a:has-text('Открыть ссылку')").count() else ""
    ok("gate: ChatGPT (Codex) sign-in into the proxy is the proxy's OAuth link (auth.openai.com, return to localhost:1455) with a field for the address", (curl or "").startswith("https://auth.openai.com/oauth/authorize?") and "localhost%3A1455" in (curl or "") and pg.locator(".settings .gflow input.mono").count() == 1 and "1455" in pg.locator(".settings .gflow").inner_text(), (curl or "")[:90])
    pg.locator(".settings .gflow button:has-text('Отмена')").click(); time.sleep(0.3)
    gtg.locator("button.glogin").click(); pg.wait_for_selector(".menu .mi", timeout=5000); pg.locator(".menu .mi:has-text('Claude')").first.click(); pg.wait_for_selector(".settings .gflow input.mono, .settings .gflow .err", timeout=30000); time.sleep(0.5)
    clurl = pg.locator(".settings .gflow a:has-text('Открыть ссылку')").get_attribute("href") if pg.locator(".settings .gflow a:has-text('Открыть ссылку')").count() else ""
    ok("gate: Claude sign-in into the proxy is the proxy's OAuth link (return to localhost:54545) with a field for the address", "oauth/authorize" in (clurl or "") and "localhost%3A54545" in (clurl or "") and pg.locator(".settings .gflow input.mono").count() == 1, (clurl or "")[:90])
    pg.locator(".settings .gflow button:has-text('Отмена')").click(); time.sleep(0.3)
    # запасные способы потоками ядра: код устройства (ChatGPT) и код со страницы (Claude), файл входа ушёл бы в прокси
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
    # подписка ChatGPT, если владелец уже вошёл: сессия Codex с аккаунтом (CODEX_HOME) отвечает через ChatGPT, не через gate
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
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); ok("ui: settings with nine tabs (accounts … MCP … about)", pg.locator(".settings .subnav button").count() == 9, str(pg.locator(".settings .subnav button").count()))
    pg.locator(".settings .subnav button:has-text('Внешний вид')").click(); pg.locator("input[name='th']").nth(1).check(); time.sleep(0.5)
    ok("ui: light theme applied", pg.evaluate("document.documentElement.dataset.theme") == "light"); pg.screenshot(path="shots/ap5-settings-light.png")
    # мультиязычность (i18n.ts + i18n/messages/app.ts): переключение в «Внешнем виде» — реактивно, без перезагрузки; порядок радио: auto, en, ru, uk, es
    pg.locator("input[name='lg']").nth(1).check(); time.sleep(0.8)
    ok("i18n: English — rail tooltips, title, settings tabs and <html lang> switch without a reload", pg.locator(".rail .nav[data-tip='Settings']").count() == 1 and pg.locator(".settings .subnav button:has-text('Appearance')").count() == 1 and pg.locator(".top h1").inner_text().strip() == "Settings" and pg.evaluate("document.documentElement.lang") == "en", pg.locator(".top h1").inner_text())
    pg.locator(".rail .nav[data-tip='Agents']").click(); time.sleep(0.8); pg.locator(".top .seg button:has-text('Agents')").click(); time.sleep(0.5)  # раздел помнит вкладку — вернуться на «Агенты»
    ok("i18n: Agents section in English (tabs, button, empty state)", pg.locator(".top .seg button:has-text('Routines')").count() == 1 and pg.locator(".top h1").inner_text().strip() == "Agents" and pg.locator(".top button.primary:has-text('New agent')").count() == 1 and (pg.locator(".content:has-text('No agents yet')").count() == 1 or pg.locator(".rt-item").count() >= 1), pg.locator(".content").inner_text()[:80]); pg.screenshot(path="shots/ap11-en-agents.png")
    pg.locator(".rail .nav[data-tip='Workspaces']").click(); time.sleep(0.8)
    ok("i18n: workspaces panel and thread header in English", pg.locator(".panel .p-head h2").inner_text().strip() == "Workspaces" and pg.locator(".top .seg button:has-text('Thread')").count() == 1)
    pg.locator(".rail .nav[data-tip='Settings']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Appearance')").click(); pg.locator("input[name='lg']").nth(3).check(); time.sleep(0.8)
    ok("i18n: Ukrainian dictionary", pg.locator(".rail .nav[data-tip='Налаштування']").count() == 1 and pg.locator(".top h1").inner_text().strip() == "Налаштування" and pg.evaluate("document.documentElement.lang") == "uk")
    pg.locator("input[name='lg']").nth(4).check(); time.sleep(0.8)
    ok("i18n: Spanish dictionary", pg.locator(".rail .nav[data-tip='Ajustes']").count() == 1 and pg.locator(".top h1").inner_text().strip() == "Ajustes" and pg.locator(".settings .subnav button:has-text('Apariencia')").count() == 1)
    pg.reload(wait_until="networkidle"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(1.2)
    ok("i18n: the chosen language survives a reload", pg.locator(".rail .nav[data-tip='Ajustes']").count() == 1 and pg.evaluate("document.documentElement.lang") == "es")
    pg.locator(".rail .nav[data-tip='Ajustes']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Apariencia')").click(); pg.locator("input[name='lg']").nth(0).check(); time.sleep(0.8)
    ok("i18n: auto follows the browser locale (ru-RU) again", pg.locator(".rail .nav[data-tip='Настройки']").count() == 1 and pg.evaluate("document.documentElement.lang") == "ru" and pg.locator("input[name='lg']").nth(0).is_checked())
    pg.locator("input[name='lg']").nth(2).check(); time.sleep(0.4)
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab:has-text('Чат')").click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000); time.sleep(1)
    lum = "e => { const c = (getComputedStyle(e).backgroundColor.match(/[\\d.]+/g) || [0,0,0]).map(Number); return (0.2126*c[0] + 0.7152*c[1] + 0.0722*c[2]) / 255; }"
    pg.locator(".thread .chat textarea").fill("/"); time.sleep(0.6); pl = pg.locator(".thread .chat .popup").evaluate(lum); pg.keyboard.press("Escape"); pg.locator(".thread .chat textarea").fill("")
    pg.locator(".thread .chat .tools .ctl").nth(0).click(); time.sleep(0.6); ml = pg.locator(".menu").evaluate(lum); pg.keyboard.press("Escape"); cl = pg.locator(".thread .chat .composer").evaluate(lum)
    pg.locator(".thread .chat .tools .ctl").nth(1).click(); time.sleep(0.5); pl2 = pg.locator(".thread .chat .modes").evaluate(lum); pg.keyboard.press("Escape")
    ok("ui: light theme styles the popup, menu, modes popover and composer", pl > 0.7 and ml > 0.7 and cl > 0.7 and pl2 > 0.7, f"popup {pl:.2f} menu {ml:.2f} modes {pl2:.2f} composer {cl:.2f}"); pg.screenshot(path="shots/ap5b-light-chat.png")
    pg.locator(".rail .nav[data-tip='Настройки']").click(); pg.wait_for_selector(".settings", timeout=20000); pg.locator(".settings .subnav button:has-text('Внешний вид')").click(); pg.locator("input[name='th']").nth(0).check(); time.sleep(0.5)
    # переключатель аккаунтов в меню «⋯» сессии (карточка, дерево, Thread, чат): gate проекта и аккаунты провайдера, текущий отмечен;
    # выбор перезапускает сессию с той же историей (даже завершённую)
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.8); pg.locator(f".panel .sess[data-id='{sid}']").click(); pg.wait_for_selector(".thread .t-head", timeout=20000)
    pg.locator(".thread .t-head button.icon.sm.ghost").last.click(); pg.wait_for_selector(".menu", timeout=5000)
    acc_items = [x.strip() for x in pg.locator(".menu .mi").all_inner_texts()]
    ok("account: session menu has an account section with the project gate checked and provider accounts listed", pg.locator(".menu .mh:has-text('Аккаунт')").count() == 1 and pg.locator(".menu .mi.on:has-text('gate проекта')").count() == 1 and all("codex" not in x.lower() for x in acc_items), str(acc_items)[:160])
    pg.locator(".menu .mi:has-text('gate проекта')").click(); time.sleep(6)
    acc_after = get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}
    ok("account: choosing an account restarts the session with the same history", acc_after.get("info", {}).get("alive") is True and acc_after["info"].get("account") is None and acc_after["info"].get("provider_session_id") == sid and pg.locator(".toast:has-text('перезапущена')").count() >= 0, str({k: acc_after.get("info", {}).get(k) for k in ("alive", "account")}))
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); pg.locator(".top .seg button:has-text('Code')").click(); time.sleep(0.8)
    card_more = pg.locator(f".card[data-card='{P}:{sid}'] .c-head .acts button").nth(3)
    # холст общий с владельцем: чужие карточки перекрывают нашу, обычный клик перехватывается — событие отправляем кнопке напрямую
    if card_more.count(): card_more.dispatch_event("click"); pg.wait_for_selector(".menu", timeout=5000); ok("account: the canvas card menu carries the same account section", pg.locator(".menu .mh:has-text('Аккаунт')").count() == 1); pg.keyboard.press("Escape")
    else: ok("account: the canvas card menu carries the same account section", False, "card not on canvas")
    pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5)
    ok("ui: no page errors", not errors, "; ".join(errors)[:200])
    dp = b.new_page(); dp.goto(f"https://{HOST}/?app=agents", wait_until="domcontentloaded"); dp.wait_for_selector(".agents .sess", timeout=30000); time.sleep(3)
    ok("desktop: Agents window lists workspace conversations too", dp.locator(".agents .sess").count() >= 3 and dp.locator(f".agents .sess:has-text('{KNOWN_WORD}')").count() >= 1, str(dp.locator(".agents .sess").count())); dp.screenshot(path="shots/ap7-desktop-agents.png"); dp.close()
    ph = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 390, "height": 844}, device_scale_factor=2, is_mobile=True, has_touch=True); ph.add_init_script("try { if (!localStorage.getItem('agents.lang')) localStorage.setItem('agents.lang', 'ru'); } catch (e) {}"); pp = ph.new_page()
    pp.goto(f"https://{HOST}/agents/", wait_until="networkidle"); pp.wait_for_selector(".tabbar", timeout=30000); time.sleep(2)
    ok("phone: bottom tab bar and thread view", pp.locator(".tabbar .tb").count() == 5 and pp.locator(".thread, .empty").count() >= 1); pp.screenshot(path="shots/ap6-phone.png")
    pp.locator(".tabbar .tb").nth(0).click(); time.sleep(0.8)
    if pp.locator(".burger").count(): pp.locator(".burger").first.click(); time.sleep(0.8)
    pp.locator(f".panel .sess[data-id='{sid}']").click(force=True); pp.wait_for_selector(".thread .pane .tabs", timeout=20000); pp.locator(".thread .pane .tabs .tab").first.click(); pp.wait_for_selector(".thread .chat .tools", timeout=20000); time.sleep(2.5)
    ptools = pp.locator(".thread .chat .tools")
    try: pp.wait_for_selector(".thread .chat .tools .ctl.ctx", timeout=20000); pp.wait_for_selector(".thread .chat .tools .chip:has-text('кэш')", timeout=20000)
    except Exception: pass
    th = ptools.evaluate("e => e.getBoundingClientRect().height")
    ok("phone: composer tools wrap into two rows with context ring and cache chip", ptools.locator(".ctl.ctx").count() == 1 and ptools.locator(".chip:has-text('кэш')").count() == 1 and th >= 44, f"tools height {th:.0f}")
    pp.locator(".thread .chat textarea").click(); pp.locator(".thread .chat textarea").fill("/"); time.sleep(0.8)
    ok("phone: slash popup opens above the input", pp.locator(".thread .chat .popup .pi").count() >= 3 and pp.locator(".thread .chat .popup").evaluate("e => e.getBoundingClientRect().bottom <= innerHeight")); pp.keyboard.press("Escape"); pp.locator(".thread .chat textarea").fill("")
    ptools.locator(".ctl").nth(0).click(); time.sleep(0.8)
    ok("phone: model menu opens within the viewport", pp.locator(".menu").is_visible() and pp.locator(".menu").evaluate("e => { const r = e.getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth + 1 && r.bottom <= innerHeight + 1; }")); pp.keyboard.press("Escape"); pp.screenshot(path="shots/ap6b-phone-composer.png")
    # телефон, терминал (docs/agents-ux.md §4.3.7): поля 16px против автозума iOS, шрифт 11px, ввод, подгонка при «клавиатуре» (уменьшенный viewport)
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
    # ---- простой чат (simplechat.rs, раздел «Чат», движок «Простой»): маршруты ядра, поток SSE, картинка, инструмент страниц, инкогнито, расход, в продвинутый ----
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
    # второй заход (срез 37, §6.5): пресеты системного промпта, PDF с текстовым слоем, картинка по /img
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
    # инкогнито: в памяти, с expires_at, не в списке SQLite после «оставить»→ станет обычным
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
    # интерфейс: переключатель движка, список простых чатов, ответ в чате, метка инкогнито у сохранённого — уже нет, у нового — есть
    pg.keyboard.press("Escape"); time.sleep(0.4)
    for _ in range(3):
        pg.locator(".rail .nav[data-tip='Чат']").click(); time.sleep(1)
        if pg.locator(".panel .engine button[data-engine='simple']").count(): break
    pg.wait_for_selector(".panel .engine button[data-engine='simple']", timeout=20000); pg.locator(".panel .engine button[data-engine='simple']").click(); time.sleep(0.8)
    pg.wait_for_selector(f".panel .sess[data-chat='{sc['id']}']", timeout=20000); pg.locator(f".panel .sess[data-chat='{sc['id']}']").click(); pg.wait_for_selector(".thread .chat .m.agent", timeout=20000); time.sleep(1)
    ok("schat: the panel switch shows simple chats and the chat renders its messages, tools and model chip", pg.locator(".thread .chat .m.agent").count() >= 2 and pg.locator(f".thread .chat .m.agent:has-text('simple-{MARK}')").count() == 1 and pg.locator(".thread .chat .m.tool").count() >= 1 and pg.locator(".thread .chat .atts img").count() >= 1 and pg.locator(".thread .chat .tools .ctl.mono").inner_text().find(MD.split('/')[-1][:12]) >= 0 and pg.locator(".thread .chat .ctl.tfetch.on").count() == 1, f"agent={pg.locator('.thread .chat .m.agent').count()} tool={pg.locator('.thread .chat .m.tool').count()}"); pg.screenshot(path="shots/ap12-simple-chat.png")
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
    # рендер чата (срез 33, docs/agents-ux.md §4.3.9): диаграмма mermaid карточкой с SVG, формулы KaTeX, кнопка копирования у кода, переключатель «код»
    pg.locator(".thread .chat .tools .ctl.tsys").click(); pg.wait_for_selector(".thread .chat .syspop", timeout=10000); time.sleep(0.6)
    n_pre = pg.locator(".thread .chat .syspop .preset").count()
    pg.locator(".thread .chat .syspop .preset:has-text('Код-ревьюер') button.apply").click(); time.sleep(0.3)
    applied_txt = pg.locator(".thread .chat .syspop textarea.sys").input_value()
    pg.locator(".thread .chat .syspop button.save").click(); time.sleep(1.5)
    sc_after = (get(f"/agents/api/chats/{sc['id']}")[1] or {}).get("chat", {})
    ok("schat: the system-prompt popover lists presets, «apply» fills the text and «save» writes it to the chat", n_pre >= 3 and "код-ревьюер" in applied_txt.lower() and sc_after.get("system", "").startswith("Ты строгий код-ревьюер") and pg.locator(".thread .chat .syspop").count() == 0 and pg.locator(".thread .chat .tools .ctl.tsys.on").count() == 1, f"presets={n_pre} system={sc_after.get('system', '')[:40]!r}")
    ok("schat: the image button prefixes /img into the composer and the PDF chip shows the extracted-text mark", (pg.locator(".thread .chat .tools .ctl.timg").click() or True) and pg.locator(".thread .chat textarea").input_value().startswith("/img ") and pg.locator(".thread .chat .m.user .atts .chip.file:has-text('codeword.pdf')").count() == 1 and "текст извлечён" in pg.locator(".thread .chat .m.user .atts .chip.file:has-text('codeword.pdf')").inner_text()); pg.locator(".thread .chat textarea").fill("")
    pg.screenshot(path="shots/ap19-schat2.png")
    n_agent = pg.locator(".thread .chat .m.agent").count()
    # после ответа чат перечитывается (chatGet) и поле ввода может пересоздаться — набираем, пока кнопка «Отправить» не станет активной
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
    pg.locator(".panel .engine button[data-engine='advanced']").click(); time.sleep(0.6)
    ok("schat: the advanced engine lists scratch sessions again", pg.locator(".panel .sess[data-id]").count() >= 1 and pg.locator(".panel .sess[data-chat]").count() == 0)
    for cid in (sc.get("id"), ic.get("id"), inc_id):
        if cid: raw(f"/agents/api/chats/{cid}", "DELETE")
    if hsid:
        for _ in range(3):
            if raw(f"/agents/api/core/projects/scratch/sessions/{hsid}", "DELETE")[0] in (204, 404): break
            time.sleep(3)
    ok("schat: chats removed", all(c["id"] not in (sc.get("id"), ic.get("id")) for c in (get("/agents/api/chats")[1] or {}).get("chats", [])))
    # ---- поиск по беседам и экспорт (срез 34, docs/agents-ux.md §4.6): сервис /search по agentd всех workspace'ов и простым чатам, переход к сообщению, экспорт Markdown/JSON, CSV книги расходов ----
    # свои данные: сессия Claude с кодовым словом и простой чат с другим — предыдущие блоки свои беседы уже убрали
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
    # интерфейс: поле поиска панели показывает совпадения в сообщениях, клик открывает беседу и подсвечивает сообщение
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
        # дерево панели при поиске отфильтровано по названиям — строки сессии там может не быть; беседа открыта в Thread
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
    # ---- MCP-серверы и навыки в беседе (срез 36, docs/agents-ux.md §4.3.10): список agentd и раскладка тремя CLI, вкладка «Настройки → MCP», попап навыков в композере, блок навыков в сообщении ----
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
    pg.screenshot(path="shots/ap17-mcp.png")
    st_rm, mr = raw(MCP, "POST", json.dumps({"servers": base_srv}).encode())
    cj2 = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.claude.json")[1] or b""; ct2 = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.codex/config.toml")[1] or b""; gj2 = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.gemini/settings.json")[1] or b""
    ok("mcp: removed servers disappear from all three CLI configs", st_rm == 200 and all(n.encode() not in cj2 and n.encode() not in ct2 and n.encode() not in gj2 for n in (MCPN, MCPH)), f"{st_rm}")
    # навыки в беседе: чип → попап с каталогом, отметка «report», первое сообщение уходит с блоком навыков, второе — без; у Claude появляется SKILL.md
    ks = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"skills probe {MARK}"}).encode())[1] or b"{}"); ksid = ks.get("id")
    pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1)
    try: pg.wait_for_selector(f".panel .sess[data-id='{ksid}']", timeout=20000)
    except Exception: pass
    if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.4)
    pg.locator(f".panel .sess[data-id='{ksid}']").click(); pg.wait_for_selector(".thread .chat .tools .ctl.skills", timeout=20000); time.sleep(0.8)
    pg.locator(".thread .chat .tools .ctl.skills").click(); pg.wait_for_selector(".thread .chat .skills-pop label.sk", timeout=10000); time.sleep(0.4)
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
    # ---- рестарт agentd без потери бесед (срез 38, docs/agents-ux.md §4.3.11): живые Claude- и Gemini-беседы возвращаются сами после systemctl restart ----
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
    # Gemini CLI 0.62 не находит свои ACP-сессии в session/load («Invalid session identifier») — поднимается новой сессией с пересказом истории; loaded у неё не требуем
    ok("restart: both sessions come back alive by themselves with a restart note in the journal and the same provider session for Claude", bool(back) and back["a"].get("alive") and back["g"].get("alive") and any(e["kind"] == "system" and e.get("subtype") == "agentd_restart" for e in ev_a2) and any(e["kind"] == "system" and e.get("subtype") == "agentd_restart" for e in ev_g2) and back["a"].get("provider_session_id") == psid_before and len([e for e in ev_g2 if e["kind"] == "init"]) >= 2, f"a={(back.get('a') or {}).get('status')} g={(back.get('g') or {}).get('status')} psid_same={(back.get('a') or {}).get('provider_session_id') == psid_before} gemini_loaded={any(e['kind'] == 'init' and e.get('loaded') is True for e in ev_g2)}")
    raw(f"/agents/api/core/projects/{P}/sessions/{rsid}/prompt", "POST", json.dumps({"content": "What word did I ask you to remember? Reply with just that word."}).encode())
    raw(f"/agents/api/core/projects/{P}/sessions/{rgid}/prompt", "POST", json.dumps({"content": "What word did I ask you to remember? Reply with just that word."}).encode())
    for _ in range(60):
        time.sleep(3)
        ev_a3 = get(f"/agents/api/core/projects/{P}/sessions/{rsid}/events?since=0&limit=200")[1] or []; ev_g3 = get(f"/agents/api/core/projects/{P}/sessions/{rgid}/events?since=0&limit=200")[1] or []
        if len([e for e in ev_a3 if e["kind"] == "result"]) >= 2 and len([e for e in ev_g3 if e["kind"] == "result"]) >= 2: break
    ta = " ".join(str(b.get("text", "")) for e in ev_a3 if e["kind"] == "assistant" for b in (e.get("content") or []) if isinstance(b, dict)); tg = " ".join(str(b.get("text", "")) for e in ev_g3 if e["kind"] == "assistant" for b in (e.get("content") or []) if isinstance(b, dict))
    recap = any(e["kind"] == "system" and e.get("subtype") == "recap_sent" for e in ev_g3)
    ok("restart: the resumed sessions keep their context — Claude natively, Gemini through the journal recap sent with the first prompt after the restart (system line) — both recall the word", f"restart-{MARK}" in ta and f"gemini-{MARK}" in tg and recap, f"claude={ta[-60:]!r} gemini={tg[-60:]!r} recap={recap}")
    for sdel in (rsid, rgid):
        if sdel:
            for _ in range(3):
                if raw(f"/agents/api/core/projects/{P}/sessions/{sdel}", "DELETE")[0] in (204, 404): break
                time.sleep(3)
    # ---- движки по ACP (срез 32: agentd acp.rs + engines.rs, docs/agents-ux.md §4.3.8): каталог движков, Gemini CLI и pi через gate, разрешения, модель и режим на лету, меню «+» ----
    st_e, eng = get(f"/agents/api/core/projects/{P}/engines"); engs = {e["id"]: e for e in (eng or {}).get("engines", [])}
    ok("engines: catalog — Claude Code, Codex, Gemini CLI and pi installed with versions; the last two speak ACP", st_e == 200 and set(engs) == {"claude", "codex", "gemini", "pi"} and all(engs[k]["installed"] and engs[k].get("version") for k in engs) and engs["gemini"]["protocol"] == "acp" and engs["pi"]["protocol"] == "acp", str({k: (v.get("version"), v.get("protocol")) for k, v in engs.items()})[:160])
    def e_sess(sid_e): return get(f"/agents/api/core/projects/{P}/sessions/{sid_e}")[1] or {}
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
    # инструмент в режиме default: `echo` Gemini пропускает как безопасную команду, `touch` — спрашивает → session/request_permission → карточка; «разрешить» → allow_once → tool_result
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
    st_md, mdres = raw(f"/agents/api/core/projects/{P}/sessions/{gid}/model", "POST", json.dumps({"model": alt}).encode()); mdres = json.loads(mdres or b"{}"); gi3 = e_sess(gid)
    ok("gemini: the model switches live (session/set_model with a gate id the CLI does not know) — caps and session info follow", bool(alt) and st_md == 200 and mdres.get("applied") == "live" and (gi3.get("caps") or {}).get("model") == alt and (gi3.get("info") or {}).get("model") == alt, f"{st_md} {alt} {str(mdres)[:60]}")
    # pi через pi-acp: провайдер gate в ~/.pi/agent/models.json, модель через configOptions, инструменты без разрешений, вывод bash из файла сессии pi, ошибки модели — из него же
    ps = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "pi", "title": f"pi probe {MARK}", "model": "claude-sonnet-4-6"}).encode())[1] or b"{}"); pid = ps.get("id")
    pev = e_wait(pid, lambda e: e["kind"] == "init", 90) if pid else []; pinit = next((e for e in pev if e["kind"] == "init"), {})
    pc = e_sess(pid).get("caps") or {}; pmods = [(m.get("value"), m.get("label")) for m in pc.get("models", [])]
    ok("pi: the session starts over pi-acp through the gate — init with route gate and the requested model under the gate provider, no provider errors", bool(pid) and pinit.get("protocol") == "acp" and pinit.get("route") == "gate" and pinit.get("model") == "agentverse-gate/claude-sonnet-4-6" and not any(e["kind"] == "system" and e.get("subtype") == "provider_error" for e in pev), f"{pinit.get('model')} {[e.get('data') for e in pev if e['kind'] == 'system'][:1]}")
    ok("pi: caps — only gate models (labels without the provider prefix), pi's thinking levels as modes, model switching via configOptions", bool(pmods) and all(str(v).startswith("agentverse-gate/") and not str(l).startswith("agentverse-gate/") for v, l in pmods) and ("agentverse-gate/claude-sonnet-4-6", "claude-sonnet-4-6") in pmods and pc.get("protocol") == "acp" and pc.get("model") == "agentverse-gate/claude-sonnet-4-6" and any(o.get("id") == "model" for o in (pc.get("config_options") or [])), f"{len(pmods)} {pmods[:2]} model={pc.get('model')}")
    e_prompt(pid, f"Run the shell command `echo pi-{MARK}` and tell me its output.")
    pev = e_wait(pid, lambda e: e["kind"] == "result", 150); pr = e_results(pev)
    time.sleep(3); pev = e_events(pid); ptres = [e for e in pev if e["kind"] == "tool_result"]
    ok("pi: a shell tool runs without a permission request — tool_use Bash, tool_result with the output taken from pi's session file, result success with the gate model, context usage and tokens", bool(pr) and pr[-1]["subtype"] == "success" and not any(e["kind"] == "permission_request" for e in pev) and any(f"pi-{MARK}" in json.dumps(e.get("content")) for e in ptres) and f"pi-{MARK}" in e_atext(pev) and pr[-1].get("model") == "agentverse-gate/claude-sonnet-4-6" and (pr[-1].get("context") or {}).get("size") and ((pr[-1].get("usage") or {}).get("output_tokens") or 0) > 0, f"{(pr or [{}])[-1].get('subtype')} tool={str([e.get('content') for e in ptres])[:60]} {e_atext(pev)[:40]} usage={(pr or [{}])[-1].get('usage')}")
    # ошибка модели: провайдер pi без ключа (anthropic/…) — pi отвечает ошибкой, pi-acp молчит, agentd берёт errorMessage из файла сессии
    st_bad, _ = raw(f"/agents/api/core/projects/{P}/sessions/{pid}/model", "POST", json.dumps({"model": "anthropic/claude-haiku-4-5"}).encode())
    pseq = pr[-1]["seq"] if pr else 0; e_prompt(pid, "Reply with exactly: never")
    pev = e_wait(pid, lambda e: e["kind"] == "result" and e["seq"] > pseq, 120); pr2 = [e for e in e_results(pev) if e["seq"] > pseq]
    ok("pi: a model pi cannot reach (its anthropic provider without a key) ends the turn as an error with pi's message (pi-acp alone reports end_turn with no text)", st_bad == 200 and bool(pr2) and pr2[-1]["is_error"] and pr2[-1]["subtype"] == "error_during_execution" and len(str(pr2[-1].get("text") or "")) > 5, f"{st_bad} {str((pr2 or [{}])[-1].get('text'))[:120]}")
    # TUI движка из меню «+» (PTY без сессии): agentd готовит конфигурацию движка — pi получает провайдер gate и модель по умолчанию, иначе стартует со своей моделью без ключа (401)
    raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.pi/agent/settings.json", "PUT", b'{"quietStartup": true}', "application/octet-stream")
    st_pt, ptb = raw(f"/agents/api/core/projects/{P}/ptys", "POST", json.dumps({"cmd": ["bash", "-lic", "exec pi"], "title": "pi", "engine": "pi"}).encode()); ptb = json.loads(ptb or b"{}")
    time.sleep(3); st_ps, pset = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.pi/agent/settings.json"); st_pm, pmod = raw(f"/agents/api/core/projects/{P}/fs/file?path=/home/coder/.pi/agent/models.json")
    try: pset = json.loads(pset or b"{}"); pmod = json.loads(pmod or b"{}")
    except Exception: pset, pmod = {}, {}
    gate_prov = (pmod.get("providers") or {}).get("agentverse-gate") or {}
    ok("engines: a pi TUI from the «+» menu gets the gate provider and a default model written before pi starts (models.json + settings.json)", st_pt == 200 and ptb.get("id") and st_ps == 200 and pset.get("defaultProvider") == "agentverse-gate" and str(pset.get("defaultModel", "")).startswith("claude-sonnet") and pset.get("quietStartup") is True and st_pm == 200 and gate_prov.get("api") == "openai-completions" and gate_prov.get("apiKey") == "$OPENAI_API_KEY" and len(gate_prov.get("models") or []) > 20, f"{st_pt} settings={str(pset)[:90]} models={len(gate_prov.get('models') or [])}")
    if ptb.get("id"): raw(f"/agents/api/core/projects/{P}/ptys/{ptb['id']}", "DELETE")
    # интерфейс: меню «+» с движками, карточка Gemini-сессии — аватар движка, ответ, карточка инструмента, итоги ходов
    pg.keyboard.press("Escape"); time.sleep(0.3); pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(1)
    wse = pg.locator(f".panel .ws:has(.wname:has-text('{P}'))"); wse.locator(".wrow").hover(); wse.locator(".wrow button").click(); pg.wait_for_selector(".menu", timeout=5000); time.sleep(0.5)
    mi = [x.strip() for x in pg.locator(".menu .mi").all_inner_texts()]
    ok("engines: the «+» menu offers a chat and a TUI for every installed engine", any(x.startswith("Чат с Gemini CLI") for x in mi) and any(x.startswith("Чат с pi") for x in mi) and any(x.startswith("Gemini CLI TUI") for x in mi) and any(x.startswith("pi TUI") for x in mi) and any(x.startswith("Чат с Claude") for x in mi) and any(x.startswith("Чат с Codex") for x in mi), str(mi)[:220]); pg.keyboard.press("Escape"); time.sleep(0.3)
    if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5)
    pg.locator(f".panel .sess[data-id='{gid}']").click(); pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab").first.click(); time.sleep(1)
    for sel in (f".thread .chat .m.agent:has-text('gem-{MARK}')", f".thread .chat .m.tool:has-text('tool-{MARK}')"):
        try: pg.wait_for_selector(sel, timeout=30000)
        except Exception: pass
    n_ag = pg.locator(f".thread .chat .m.agent:has-text('gem-{MARK}')").count(); n_tl = pg.locator(f".thread .chat .m.tool:has-text('tool-{MARK}')").count(); n_tn = pg.locator(".thread .chat .m.sys.turn").count(); ttxt = pg.locator(".thread .chat .tools").inner_text()
    ok("engines: the Gemini session renders like any other — engine avatar in the header, assistant text, the Bash tool card with its output, turn footers, the yolo mode in the composer and no «thinking» indicator while idle", pg.locator(".thread .t-head .av.gemini").count() >= 1 and n_ag == 1 and n_tl >= 1 and n_tn >= 2 and "YOLO" in ttxt and "думает" not in ttxt, f"av={pg.locator('.thread .t-head .av.gemini').count()} agent={n_ag} tool={n_tl} turns={n_tn} tools-text={ttxt[:80]!r}")
    pg.screenshot(path="shots/ap14-gemini.png")
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
