import os, re, time, json, ssl, urllib.request, random, string
from playwright.sync_api import sync_playwright
# Device matrix (docs/qa-plan.md §2): one scenario on the phone, tablet (two orientations), laptop and desktop — the frame
# without overflow, all sections, opening a conversation, the composer and menus on screen, tabs, message search, export in the
# menu, the simple chat with diagrams, agents, models (CSV), settings (theme). One model turn — only on the phone, in its own
# session.
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
    # the panel is a slide-out not only on the phone (up to 1099px): detect by the layout, not by
    # the width
    def drawer(): return bool(pg.evaluate("(() => { const p = document.querySelector('.panel'); return p && getComputedStyle(p).position === 'absolute'; })()"))
    def panel_open():
        if drawer() and not pg.locator(".app.popen").count(): pg.locator(".top .burger").first.click(); time.sleep(0.5)
    def panel_close():
        if pg.locator(".app.popen").count(): pg.keyboard.press("Escape"); time.sleep(0.4)
    pg.goto(f"https://{HOST}/agents/", wait_until="load"); pg.wait_for_selector(".app", timeout=30000); time.sleep(2)
    v = vp()
    okd("shell: no horizontal overflow", v["sw"] <= v["w"] + 1, f"scrollWidth={v['sw']} innerWidth={v['w']}")
    if phone: okd("shell: bottom tab bar instead of the side rail", pg.locator(".tabbar .tb").count() == 5 and pg.locator(".tabbar").is_visible() and not pg.locator(".rail").is_visible())
    else: okd("shell: side rail with six sections", pg.locator(".rail .nav").count() >= 6 and pg.locator(".rail").is_visible() and not pg.locator(".tabbar").is_visible())
    # --- workplaces ---
    goto("Рабочие места"); panel_open()
    pg.wait_for_selector(".panel .ws", timeout=30000)
    pb = pg.locator(".panel").bounding_box() or {}; v = vp()
    okd("work: workspace tree in a visible panel that fits the screen", pg.locator(".panel .ws").count() >= 1 and pb and pb.get("x", -1) >= -1 and pb["x"] + pb["width"] <= v["w"] + 1, f"panel={pb} drawer={drawer()}")
    row = pg.locator(f".panel .ws:has(.wname:has-text('{P}')) .sess[data-id]").first
    try: row.wait_for(state="attached", timeout=15000)
    except Exception: pass
    sid = row.get_attribute("data-id") if row.count() else None
    if sid:
        row.click(); time.sleep(0.6); panel_close()
        if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5)
        try: pg.wait_for_selector(".thread .chat .composer", timeout=20000)
        except Exception: pass
        okd("work: a session opens in Thread with the composer fully on screen", pg.locator(".thread .chat .composer").count() == 1 and inside(".thread .chat .composer"), str(pg.locator(".thread .chat .composer").first.bounding_box()))
        okd("work: thread header fits (title, chips, buttons)", inside(".thread .t-head"))
        if v["w"] >= 1800:
            tb = pg.locator(".thread").first.bounding_box() or {"x": 0, "width": 0}; mb = pg.locator("main.main").first.bounding_box() or {"x": 0, "width": 0}
            okd("work: on an ultra-wide screen the conversation is capped at 1600px and centered", tb["width"] <= 1601 and abs((tb["x"] - mb["x"]) - ((mb["x"] + mb["width"]) - (tb["x"] + tb["width"]))) < 40, f"thread={tb['width']:.0f} main={mb['width']:.0f}")
        if pg.locator(".thread .chat .composer .ctl.model").count():
            pg.locator(".thread .chat .composer .ctl.model").first.click(); time.sleep(0.6)
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
    # message search: a word that is certainly in the stand's
    # conversations
    panel_open(); pg.locator(".panel .search input").fill("Reply")
    try: pg.wait_for_selector(".panel .hits .hit", timeout=20000)
    except Exception: pass
    shot("search")
    okd("work: message search shows hits that fit the panel", pg.locator(".panel .hits .hit").count() >= 1 and inside(".panel .hits .hit"), f"hits={pg.locator('.panel .hits .hit').count()} first={pg.locator('.panel .hits .hit').first.bounding_box() if pg.locator('.panel .hits .hit').count() else None} vp={vp()}")
    if pg.locator(".panel .hits .hit").count():
        (pg.locator(".panel .hits .hit[data-kind='session']").first if pg.locator(".panel .hits .hit[data-kind='session']").count() else pg.locator(".panel .hits .hit").first).click(); time.sleep(1.5)
        okd("work: a hit opens the conversation in Thread", pg.locator(".thread .chat").count() == 1 and (not phone or not pg.locator(".app.popen").count()))
    panel_open(); pg.locator(".panel .search input").fill(""); time.sleep(0.4); panel_close()
    # --- chat ---
    goto("Чат")
    try: pg.wait_for_selector(".top .engine button[data-engine='simple']", timeout=20000)
    except Exception: pass
    if pg.locator(".top .engine button[data-engine='simple']").count():
        # the engine switch is in the header (slice 42): click before opening the panel, otherwise on narrow screens the
        # backdrop covers it
        pg.locator(".top .engine button[data-engine='simple']").click(); time.sleep(0.8); panel_open()
        c = pg.locator(".panel .sess[data-chat]").first
        if c.count():
            c.click(); time.sleep(0.5); panel_close()
            try: pg.wait_for_selector(".thread .chat .m, .thread .chat .empty", timeout=20000)
            except Exception: pass
            # the owner's first chat may be empty ("Ask anything") — what matters is that the conversation opened and the
            # composer is on screen
            okd("chat: a simple chat opens with its messages (or the empty state) and the composer on screen", (pg.locator(".thread .chat .m").count() >= 1 or pg.locator(".thread .chat .empty").count() == 1) and inside(".thread .chat .composer"))
            n_dg = pg.locator(".thread .chat .diagram svg").count()
            if n_dg:
                fits = pg.evaluate("Array.from(document.querySelectorAll('.thread .chat .diagram')).every(d => d.getBoundingClientRect().right <= window.innerWidth + 1)")
                okd("chat: diagram cards fit the width", fits, f"diagrams={n_dg}")
            if pg.locator(".thread .chat .composer-model").count():
                pg.locator(".thread .chat .composer-model").click(); time.sleep(0.6)
                okd("chat: the model menu with search fits the viewport", pg.locator(".menu").count() == 1 and inside(".menu") and pg.locator(".menu input.ms").count() == 1); pg.keyboard.press("Escape"); time.sleep(0.3)
        else: print(f"SKIP  {dev}: no simple chats to open", flush=True)
        shot("chat")
        panel_close(); pg.locator(".top .engine button[data-engine='advanced']").click(); time.sleep(0.8); panel_open()
        okd("chat: the advanced engine lists scratch sessions (or an empty state) inside the panel", (pg.locator(".panel .sess[data-id]").count() >= 1 or pg.locator(".panel .empty, .panel .muted").count() >= 1) and inside(".panel")); panel_close()
    else: okd("chat: engine switch present", False, "no engine switch")
    # --- inbox ---
    goto("Входящие"); time.sleep(0.8)
    okd("inbox: section renders without overflow", pg.locator("main.main").count() == 1 and vp()["sw"] <= vp()["w"] + 1); shot("inbox")
    # --- agents ---
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
    # --- models ---
    goto("Модели"); time.sleep(1.2)
    okd("models: KPI cards and the CSV export button are present and fit", pg.locator(".card-s.kpi").count() >= 3 and pg.locator(".kpi .csv").count() == 1 and vp()["sw"] <= vp()["w"] + 1)
    st_csv, _ = raw("/agents/api/core/usage/export.csv?days=7")
    okd("models: the CSV export answers 200", st_csv == 200, str(st_csv)); shot("models")
    # --- settings ---
    goto("Настройки"); time.sleep(0.8)
    okd("settings: tabs render inside the screen", pg.locator(".subnav button").count() >= 7 and vp()["sw"] <= vp()["w"] + 1)
    # the "Accounts" tab (§9.3): a card per account, each within the screen width, limit rows or a sign-in error, buttons on
    # screen
    pg.locator(".subnav button:has-text('Аккаунты')").first.click(); time.sleep(1.2)
    try: pg.wait_for_selector(".acct-card .lrow, .acct-card .lerr", timeout=45000)
    except Exception: pass
    n_acc = len((get("/agents/api/core/accounts")[1] or {}).get("accounts", [])); cards = pg.locator(".acct-card"); n_cards = cards.count(); v = vp()
    boxes = [cards.nth(i).bounding_box() for i in range(n_cards)]
    fits = all(bx and bx["x"] >= -1 and bx["x"] + bx["width"] <= v["w"] + 1 for bx in boxes)
    cols = len({round(bx["x"]) for bx in boxes if bx}) if boxes else 0
    okd("settings: Accounts — a card per account, each within the screen width, no horizontal overflow, limit rows or a sign-in error, buttons reachable", n_cards == n_acc and n_cards >= 1 and fits and v["sw"] <= v["w"] + 1 and (pg.locator(".acct-card .lrow").count() >= 1 or pg.locator(".acct-card .lerr").count() >= 1) and inside(".acct-card .acts button"), f"cards={n_cards} accounts={n_acc} columns={cols} fits={fits} sw={v['sw']} w={v['w']}")
    okd("settings: Accounts — card columns match the screen class (one on phone and tablets, two from laptop up)", (cols == 1) if v["w"] < 1100 else (cols == 2), f"columns={cols} w={v['w']}")
    shot("settings-accounts")
    look = pg.locator(".subnav button:has-text('Вид')")
    if look.count():
        look.click(); time.sleep(0.5)
        lt = pg.locator("label:has(input[name='appearance-mode']):has-text('Светлая') input, .radios label:has-text('Светлая') input").first
        if lt.count(): lt.check(force=True); time.sleep(0.6)
        light = pg.evaluate("document.documentElement.dataset.theme") == "light"
        okd("settings: the light theme applies from the Look tab", light)
        shot("settings-light")
        dk = pg.locator("label:has(input[name='appearance-mode']):has-text('Тёмная') input, .radios label:has-text('Тёмная') input").first
        if dk.count(): dk.check(force=True); time.sleep(0.4)
    # --- a model turn: only on the phone, in its own
    # session ---
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

# a temporary agent without model turns — so the agent card opens on every device
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
        ("wide", {"viewport": {"width": 2560, "height": 1200}}),
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
