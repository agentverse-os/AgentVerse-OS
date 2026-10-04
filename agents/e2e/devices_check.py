import os, re, time, json, ssl, urllib.request, random, string
from playwright.sync_api import sync_playwright
# Матрица устройств (docs/qa-plan.md §2): один сценарий на телефоне, планшете (две ориентации), ноутбуке и столе —
# каркас без переполнения, все разделы, открытие беседы, композер и меню в экране, вкладки, поиск по сообщениям, экспорт в меню,
# простой чат с диаграммами, агенты, модели (CSV), настройки (тема). Один ход модели — только на телефоне, в своей сессии.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
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
MARK = "dev-" + "".join(random.choice(string.ascii_lowercase) for _ in range(6))
ONLY = [d for d in os.environ.get("DEVICES", "").split(",") if d]

def scenario(dev, pg, phone, touch):
    def okd(n, c, note=""): ok(f"{dev}: {n}", c, note)
    def shot(name): pg.screenshot(path=f"shots/dev-{dev}-{name}.png")
    def vp(): return pg.evaluate("({w: window.innerWidth, h: window.innerHeight, sw: document.documentElement.scrollWidth})")
    def inside(sel):
        b = pg.locator(sel).first.bounding_box() if pg.locator(sel).count() else None; v = vp()
        return bool(b) and b["x"] >= -1 and b["y"] >= -1 and b["x"] + b["width"] <= v["w"] + 1 and b["y"] + b["height"] <= v["h"] + 1
    def goto(name):
        if phone:
            if name in ("Агенты", "Настройки"): pg.locator(".tabbar .tb:has-text('Ещё')").click(); pg.wait_for_selector(".sheet .mi", timeout=5000); pg.locator(f".sheet .mi:has-text('{name}')").click()
            else: pg.locator(f".tabbar .tb:has-text('{name}')").click()
        else: pg.locator(f".rail .nav[data-tip='{name}']").click()
        time.sleep(0.9)
    # панель — выдвижная не только на телефоне (до 1099px): определяем по вёрстке, а не по ширине
    def drawer(): return bool(pg.evaluate("(() => { const p = document.querySelector('.panel'); return p && getComputedStyle(p).position === 'absolute'; })()"))
    def panel_open():
        if drawer() and not pg.locator(".app.popen").count(): pg.locator(".top .burger").first.click(); time.sleep(0.5)
    def panel_close():
        if pg.locator(".app.popen").count(): pg.keyboard.press("Escape"); time.sleep(0.4)
    pg.goto(f"https://{HOST}/agents/", wait_until="networkidle"); pg.wait_for_selector(".app", timeout=30000); time.sleep(2)
    v = vp()
    okd("shell: no horizontal overflow", v["sw"] <= v["w"] + 1, f"scrollWidth={v['sw']} innerWidth={v['w']}")
    if phone: okd("shell: bottom tab bar instead of the side rail", pg.locator(".tabbar .tb").count() == 5 and pg.locator(".tabbar").is_visible() and not pg.locator(".rail").is_visible())
    else: okd("shell: side rail with six sections", pg.locator(".rail .nav").count() >= 6 and pg.locator(".rail").is_visible() and not pg.locator(".tabbar").is_visible())
    # --- рабочие места ---
    goto("Рабочие места"); panel_open()
    pg.wait_for_selector(".panel .ws", timeout=30000)
    pb = pg.locator(".panel").bounding_box() or {}; v = vp()
    okd("work: workspace tree in a visible panel that fits the screen", pg.locator(".panel .ws").count() >= 1 and pb and pb.get("x", -1) >= -1 and pb["x"] + pb["width"] <= v["w"] + 1, f"panel={pb} drawer={drawer()}")
    row = pg.locator(f".panel .ws:has(.wname:has-text('{P}')) .sess[data-id]").first
    sid = row.get_attribute("data-id") if row.count() else None
    if sid:
        row.click(); time.sleep(0.6); panel_close()
        if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5)
        try: pg.wait_for_selector(".thread .chat .composer", timeout=20000)
        except Exception: pass
        okd("work: a session opens in Thread with the composer fully on screen", pg.locator(".thread .chat .composer").count() == 1 and inside(".thread .chat .composer"), str(pg.locator(".thread .chat .composer").first.bounding_box()))
        okd("work: thread header fits (title, chips, buttons)", inside(".thread .t-head"))
        if pg.locator(".thread .chat .tools .ctl").count():
            pg.locator(".thread .chat .tools .ctl").first.click(); time.sleep(0.6)
            okd("work: the model menu opens inside the viewport", pg.locator(".menu").count() == 1 and inside(".menu"), str(pg.locator(".menu").first.bounding_box()) if pg.locator(".menu").count() else "no menu"); pg.keyboard.press("Escape"); time.sleep(0.3)
        for tab in ("Файлы", "Git", "Чат"):
            t_ = pg.locator(f".thread .pane .tabs .tab:has-text('{tab}')")
            if t_.count(): t_.first.click(); time.sleep(1.2)
        okd("work: Files and Git tabs render and the pane stays within the screen", pg.locator(".thread .pane").count() == 1 and inside(".thread .pane"))
        shot("work")
        more = pg.locator(".thread .t-head button.icon.sm.ghost").last
        if more.count():
            more.click(); time.sleep(0.5)
            okd("work: the session menu offers Markdown and JSON export", pg.locator(".menu .mi:has-text('Экспорт → Markdown')").count() == 1 and pg.locator(".menu .mi:has-text('Экспорт → JSON')").count() == 1 and inside(".menu")); pg.keyboard.press("Escape"); time.sleep(0.3)
            st_dl, body_dl = raw(f"/agents/api/core/projects/{P}/sessions/{sid}/download?format=md")
            okd("work: the Markdown export of the open session downloads", st_dl == 200 and (body_dl or b"").startswith(b"# "), f"{st_dl} {len(body_dl or b'')}B")
    else: print(f"SKIP  {dev}: no session in {P} to open", flush=True)
    # поиск по сообщениям: слово, которое точно есть в беседах стенда
    panel_open(); pg.locator(".panel .search input").fill("Reply")
    try: pg.wait_for_selector(".panel .hits .hit", timeout=20000)
    except Exception: pass
    shot("search")
    okd("work: message search shows hits that fit the panel", pg.locator(".panel .hits .hit").count() >= 1 and inside(".panel .hits .hit"), f"hits={pg.locator('.panel .hits .hit').count()} first={pg.locator('.panel .hits .hit').first.bounding_box() if pg.locator('.panel .hits .hit').count() else None} vp={vp()}")
    if pg.locator(".panel .hits .hit").count():
        pg.locator(".panel .hits .hit").first.click(); time.sleep(1.5)
        okd("work: a hit opens the conversation in Thread", pg.locator(".thread .chat").count() == 1 and (not phone or not pg.locator(".app.popen").count()))
    panel_open(); pg.locator(".panel .search input").fill(""); time.sleep(0.4); panel_close()
    # --- чат ---
    goto("Чат"); panel_open()
    try: pg.wait_for_selector(".panel .engine button[data-engine='simple']", timeout=20000)
    except Exception: pass
    if pg.locator(".panel .engine button[data-engine='simple']").count():
        pg.locator(".panel .engine button[data-engine='simple']").click(); time.sleep(0.8)
        c = pg.locator(".panel .sess[data-chat]").first
        if c.count():
            c.click(); time.sleep(0.5); panel_close()
            try: pg.wait_for_selector(".thread .chat .m", timeout=20000)
            except Exception: pass
            okd("chat: a simple chat opens with its messages and composer on screen", pg.locator(".thread .chat .m").count() >= 1 and inside(".thread .chat .composer"))
            n_dg = pg.locator(".thread .chat .diagram svg").count()
            if n_dg:
                fits = pg.evaluate("Array.from(document.querySelectorAll('.thread .chat .diagram')).every(d => d.getBoundingClientRect().right <= window.innerWidth + 1)")
                okd("chat: diagram cards fit the width", fits, f"diagrams={n_dg}")
            if pg.locator(".thread .chat .tools .ctl.mono").count():
                pg.locator(".thread .chat .tools .ctl.mono").click(); time.sleep(0.6)
                okd("chat: the model menu with search fits the viewport", pg.locator(".menu").count() == 1 and inside(".menu") and pg.locator(".menu input.ms").count() == 1); pg.keyboard.press("Escape"); time.sleep(0.3)
        else: print(f"SKIP  {dev}: no simple chats to open", flush=True)
        shot("chat")
        panel_open(); pg.locator(".panel .engine button[data-engine='advanced']").click(); time.sleep(0.8)
        okd("chat: the advanced engine lists scratch sessions (or an empty state) inside the panel", (pg.locator(".panel .sess[data-id]").count() >= 1 or pg.locator(".panel .empty, .panel .muted").count() >= 1) and inside(".panel")); panel_close()
    else: okd("chat: engine switch present", False, "no engine switch")
    # --- входящие ---
    goto("Входящие"); time.sleep(0.8)
    okd("inbox: section renders without overflow", pg.locator("main.main").count() == 1 and vp()["sw"] <= vp()["w"] + 1); shot("inbox")
    # --- агенты ---
    goto("Агенты"); panel_open(); time.sleep(0.8)
    a = pg.locator(".rt-item[data-agent]").first
    if a.count():
        a.click(); time.sleep(0.8); panel_close()
        okd("agents: an agent card opens and fits the screen", pg.locator("main.main").count() == 1 and vp()["sw"] <= vp()["w"] + 1 and pg.locator("main.main textarea, main.main input").count() >= 1)
    else: print(f"SKIP  {dev}: no agents to open", flush=True); panel_close()
    seg = pg.locator("main.main .top .seg button")
    if seg.count() >= 4:
        seg.nth(3).click(); time.sleep(0.8)
        okd("agents: the runs tab renders", pg.locator("main.main").count() == 1 and vp()["sw"] <= vp()["w"] + 1)
    shot("agents")
    # --- модели ---
    goto("Модели"); time.sleep(1.2)
    okd("models: KPI cards and the CSV export button are present and fit", pg.locator(".card-s.kpi").count() >= 3 and pg.locator(".kpi .csv").count() == 1 and vp()["sw"] <= vp()["w"] + 1)
    st_csv, _ = raw("/agents/api/core/usage/export.csv?days=7")
    okd("models: the CSV export answers 200", st_csv == 200, str(st_csv)); shot("models")
    # --- настройки ---
    goto("Настройки"); time.sleep(0.8)
    okd("settings: tabs render inside the screen", pg.locator(".subnav button").count() >= 7 and vp()["sw"] <= vp()["w"] + 1)
    look = pg.locator(".subnav button:has-text('Вид')")
    if look.count():
        look.click(); time.sleep(0.5); pg.locator("input[name='th'][value], .radios label:has-text('Светлая') input, .radios label:has-text('светлая') input").first.check(force=True) if pg.locator(".radios label:has-text('Светлая') input, .radios label:has-text('светлая') input").count() else None; time.sleep(0.6)
        light = pg.evaluate("document.documentElement.dataset.theme") == "light"
        okd("settings: the light theme applies from the Look tab", light)
        shot("settings-light")
        if pg.locator(".radios label:has-text('Тёмная') input, .radios label:has-text('тёмная') input").count(): pg.locator(".radios label:has-text('Тёмная') input, .radios label:has-text('тёмная') input").first.check(force=True); time.sleep(0.4)
    # --- ход модели: только на телефоне, в своей сессии ---
    if phone:
        s = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"device probe {MARK}"}).encode())[1] or b"{}"); psid = s.get("id")
        if psid:
            goto("Рабочие места"); panel_open()
            try: pg.wait_for_selector(f".panel .sess[data-id='{psid}']", timeout=20000)
            except Exception: pass
            if pg.locator(f".panel .sess[data-id='{psid}']").count():
                pg.locator(f".panel .sess[data-id='{psid}']").click(); time.sleep(0.5); panel_close()
                try: pg.wait_for_selector(".thread .chat textarea", timeout=20000)
                except Exception: pass
                ta = pg.locator(".thread .chat textarea"); ta.click(); time.sleep(0.3)
                okd("phone: the composer stays on screen with the field focused", inside(".thread .chat .composer") and pg.evaluate("document.activeElement?.tagName") == "TEXTAREA")
                ta.fill(f"Reply with exactly: {MARK}"); pg.locator(".thread .chat button.send").click()
                try: pg.wait_for_selector(f".thread .chat .m.agent:has-text('{MARK}')", timeout=180000); got = True
                except Exception: got = False
                okd("phone: a model turn round-trips through the phone layout", got and inside(".thread .chat .composer")); shot("phone-turn")
            for _ in range(3):
                if raw(f"/agents/api/core/projects/{P}/sessions/{psid}", "DELETE")[0] in (204, 404): break
                time.sleep(3)

# временный агент без ходов модели — чтобы на каждом устройстве открылась карточка агента
_st, _ag = raw("/agents/api/agents", "POST", json.dumps({"name": f"device probe {MARK}", "brief": "Тестовый агент для проверки экранов; ходов не делает.", "provider": "claude", "workspace": P, "mode": "ask"}).encode())
PROBE_AGENT = (json.loads(_ag or b"{}") if _st == 200 else {}).get("id")
with sync_playwright() as p:
    b = p.chromium.launch()
    DEVICES = [
        ("phone", {**p.devices["iPhone 13"]}),
        ("tablet-portrait", {**p.devices["iPad Mini"]}),
        ("tablet-landscape", {**p.devices["iPad Mini landscape"]}),
        ("laptop", {"viewport": {"width": 1280, "height": 720}}),
        ("desktop", {"viewport": {"width": 1500, "height": 900}}),
    ]
    for dev, opts in DEVICES:
        if ONLY and dev not in ONLY: continue
        ctx = b.new_context(**opts, locale="ru-RU", ignore_https_errors=True)
        ctx.add_init_script("try { if (!localStorage.getItem('agents.lang')) localStorage.setItem('agents.lang', 'ru'); localStorage.removeItem('agents.ui'); } catch (e) {}")
        pg = ctx.new_page(); errors = []; pg.on("pageerror", lambda e: errors.append(str(e)))
        w = opts["viewport"]["width"]
        try: scenario(dev, pg, phone=w < 600, touch=bool(opts.get("has_touch")))
        except Exception as e: ok(f"{dev}: scenario completed without exceptions", False, str(e).split("\n")[0][:200]); pg.screenshot(path=f"shots/dev-{dev}-error.png")
        ok(f"{dev}: no page errors", not errors, str(errors[:2])[:200])
        ctx.close()
    b.close()
if PROBE_AGENT: raw(f"/agents/api/agents/{PROBE_AGENT}", "DELETE")
print("ALL PASS" if all(res) else "SOME FAIL")
