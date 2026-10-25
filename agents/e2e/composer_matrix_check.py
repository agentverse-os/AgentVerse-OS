import os, re, time, json, ssl, urllib.request, random, string
from playwright.sync_api import sync_playwright
from audit_js import AUDIT_JS
# Screen matrix for the chat forms (docs/qa-plan.md §2, docs/agents-ux.md §4.3.13–4.3.14): the composer of an agent
# conversation and of the simple chat on unusual screens — Galaxy Z Fold 3 (the cover screen, the inner screen in two
# orientations, half-folded, "with the keyboard"), short and narrow phones, a phone in landscape, Surface Duo, iPad Pro, half
# of the desktop, a small window, ultra-wide. Checks on each: the form is entirely on screen, no horizontal overflow, the
# send/microphone buttons are on screen and not covered, the field is no smaller than a line, the form is no taller than a
# fraction of the screen, long text grows to the limit and does not push the buttons out, the model menu and "Options" are on
# screen, the touch audit; frames shots/cmx-<screen>-*.png.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note, f"[{time.strftime('%H:%M:%S')}]", flush=True)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def raw(path, method="GET", body=None, ctype="application/json"):
    try:
        r = urllib.request.urlopen(urllib.request.Request(f"https://{HOST}{path}", data=body, method=method, headers={"Content-Type": ctype} if body is not None else {}), context=ctx_ssl, timeout=120); return r.status, r.read()
    except urllib.error.HTTPError as e: return e.code, e.read()
def get(path):
    try:
        r = urllib.request.urlopen(urllib.request.Request(f"https://{HOST}{path}"), context=ctx_ssl, timeout=60); t = r.read(); return r.status, (json.loads(t) if t and r.headers.get_content_type() == "application/json" else t)
    except urllib.error.HTTPError as e: return e.code, None
MARK = "cmx-" + "".join(random.choice(string.ascii_lowercase) for _ in range(6))
ONLY = [d for d in os.environ.get("SCREENS", "").split(",") if d]
LONG = "Проверка длинного текста в поле ввода: " + " ".join(f"строка {i} — слова для переноса и роста поля," for i in range(1, 9))

# (name, width, height, scale, touch, what it
# is)
SCREENS = [
    ("fold3-cover", 320, 780, 2.6, True, "Galaxy Z Fold 3, внешний экран 25:9 (CSS 320×872 минус панели браузера)"),
    ("fold3-cover-land", 780, 320, 2.6, True, "внешний экран Fold 3 лёжа — экстремально низкий экран"),
    ("fold3-inner", 673, 760, 2.6, True, "Fold 3 раскрыт, портрет (CSS 673×841 минус панели)"),
    ("fold3-inner-land", 841, 600, 2.6, True, "Fold 3 раскрыт, ландшафт"),
    ("fold3-flex", 673, 380, 2.6, True, "Fold 3 полусложен: браузер в верхней половине"),
    ("fold3-inner-kbd", 673, 420, 2.6, True, "Fold 3 раскрыт с экранной клавиатурой (viewport уменьшен)"),
    ("phone-kbd", 390, 400, 3, True, "iPhone 13 с экранной клавиатурой"),
    ("iphone-se", 375, 553, 2, True, "iPhone SE — короткий узкий телефон"),
    ("phone-max", 430, 810, 3, True, "iPhone 15 Pro Max"),
    ("pixel7-land", 915, 340, 2.625, True, "Pixel 7 лёжа — низкий широкий экран"),
    ("surface-duo", 540, 660, 2.5, True, "Surface Duo, один экран"),
    ("ipad-pro-portrait", 1024, 1290, 2, True, "iPad Pro 12.9 портрет"),
    ("half-desktop", 960, 1040, 1, False, "половина стола 1920×1080 (окно рядом с другим)"),
    ("laptop-1366", 1366, 700, 1, False, "ноутбук 1366×768"),
    ("zoom-125", 1536, 800, 1.25, False, "1920×1080 при масштабе 125%"),
    ("small-window", 800, 600, 1, False, "маленькое окно 800×600"),
    ("ultrawide", 3440, 1400, 1, False, "ультраширокий 3440×1440"),
]

def scenario(name, pg, touch, w, h):
    def okd(n, c, note=""): ok(f"{name}: {n}", c, note)
    def shot(s): pg.screenshot(path=f"shots/cmx-{name}-{s}.png")
    def vp(): return pg.evaluate("({w: window.innerWidth, h: window.innerHeight, sw: document.documentElement.scrollWidth})")
    def box(sel): return pg.locator(sel).first.bounding_box() if pg.locator(sel).count() else None
    def inside(sel):
        b = box(sel); v = vp()
        return bool(b) and b["x"] >= -1 and b["y"] >= -1 and b["x"] + b["width"] <= v["w"] + 1 and b["y"] + b["height"] <= v["h"] + 1
    def tappable(sel):
        return bool(pg.evaluate("(sel) => { const e = document.querySelector(sel); if (!e) return false; const b = e.getBoundingClientRect(); const x = Math.min(innerWidth - 1, Math.max(0, b.left + b.width / 2)), y = Math.min(innerHeight - 1, Math.max(0, b.top + b.height / 2)); const hit = document.elementFromPoint(x, y); return !!hit && (e.contains(hit) || hit.contains(e)); }", sel))
    phone = w < 600
    def drawer(): return bool(pg.evaluate("(() => { const p = document.querySelector('.panel'); return p && getComputedStyle(p).position === 'absolute'; })()"))
    def panel_open():
        if drawer() and not pg.locator(".app.popen").count(): pg.locator(".top .burger").first.click(); time.sleep(0.5)
    def panel_close():
        if pg.locator(".app.popen").count(): pg.keyboard.press("Escape"); time.sleep(0.4)
    def goto(sec):
        if phone:
            if pg.locator(f".tabbar .tb:has-text('{sec}')").count(): pg.locator(f".tabbar .tb:has-text('{sec}')").click()
            else: pg.locator(".tabbar .tb:has-text('Ещё')").click(); pg.wait_for_selector(".sheet .mi", timeout=5000); pg.locator(f".sheet .mi:has-text('{sec}')").click()
        else: pg.locator(f".rail .nav[data-tip='{sec}']").click()
        time.sleep(0.8)
    def audit(tag, note=""):
        a = pg.evaluate(AUDIT_JS)
        okd(f"{tag}: no overflow, nothing outside the viewport, every control tappable" + (f" — {note}" if note else ""), a["sw"] <= a["w"] + 1 and not a["wide"] and not a["covered"], f"sw={a['sw']} w={a['w']} wide={a['wide'][:3]} covered={a['covered'][:3]}")
        if a["tiny"]: print(f"NOTE  {name}/{tag}: small targets {a['tiny'][:4]}", flush=True)
        if a["clipped"]: print(f"NOTE  {name}/{tag}: clipped text {a['clipped'][:3]}", flush=True)
    def composer_checks(tag, root, ta_sel, max_share):
        """Common form checks: on screen, no overflow, buttons on screen and not covered, the field ≥ a line, the height ≤ a fraction of the screen"""
        cb = box(root); v = vp(); tb = box(ta_sel)
        okd(f"{tag}: the composer is fully on screen", bool(cb) and inside(root), f"composer={ {k: round(x) for k, x in cb.items()} if cb else None} vp={v['w']}×{v['h']}")
        ovf = pg.evaluate("(sel) => { const e = document.querySelector(sel); return e ? { sw: e.scrollWidth, cw: e.clientWidth } : null; }", root)
        okd(f"{tag}: no horizontal overflow inside the composer", bool(ovf) and ovf["sw"] <= ovf["cw"] + 1, str(ovf))
        okd(f"{tag}: send button on screen and tappable", inside(f"{root} .composer-send") and tappable(f"{root} .composer-send"), str(box(f"{root} .composer-send")))
        okd(f"{tag}: microphone button on screen and tappable", inside(f"{root} .ptt-btn") and tappable(f"{root} .ptt-btn"), str(box(f"{root} .ptt-btn")))
        okd(f"{tag}: the text field shows at least one line", bool(tb) and tb["height"] >= 24 and tb["width"] >= 120, f"field={ {k: round(x) for k, x in tb.items()} if tb else None}")
        okd(f"{tag}: the composer takes at most {int(max_share * 100)}% of the viewport height", bool(cb) and cb["height"] <= v["h"] * max_share + 1, f"h={cb['height'] if cb else None} of {v['h']}")
        # the control row: all children inside the form (nothing stuck out past the right edge)
        ctl = pg.evaluate("(sel) => { const c = document.querySelector(sel); if (!c) return null; const r = c.getBoundingClientRect(); const bad = [...c.querySelectorAll('.composer-controls > *, .composer-actions > *')].filter(e => { const cs = getComputedStyle(e); if (cs.display === 'none') return false; const b = e.getBoundingClientRect(); return b.width > 0 && (b.right > r.right + 1 || b.left < r.left - 1 || b.bottom > r.bottom + 1); }).map(e => e.className.toString().slice(0, 40)); return { n: c.querySelectorAll('.composer-controls > *, .composer-actions > *').length, bad }; }", root)
        okd(f"{tag}: every control sits inside the composer box", bool(ctl) and not ctl["bad"], str(ctl))
    pg.goto(f"https://{HOST}/agents/", wait_until="load"); pg.wait_for_selector(".app", timeout=30000); time.sleep(2)
    v = vp(); okd("shell fits without horizontal scroll", v["sw"] <= v["w"] + 1, f"sw={v['sw']} w={v['w']} h={v['h']} touch={touch}")
    # --- agent conversation: a Claude session with
    # history ---
    goto("Рабочие места"); panel_open(); pg.wait_for_selector(".panel .ws", timeout=30000)
    row = pg.locator(f".panel .ws:has(.wname:has-text('{P}')) .sess[data-id]").first
    try: row.wait_for(state="attached", timeout=15000)
    except Exception: pass
    if not row.count(): okd("work: a session exists to open", False); return
    row.click(); time.sleep(0.6); panel_close()
    if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5)
    pg.wait_for_selector(".thread .chat .chat-composer", timeout=20000); time.sleep(0.8)
    C = ".thread .chat .chat-composer"; TA = f"{C} textarea"
    composer_checks("thread", C, TA, 0.6 if v["h"] < 500 else 0.45)
    audit("thread", "agent conversation with the composer")
    shot("thread")
    # long text: the field grows to the limit, the form stays on screen, the buttons
    # did not drift away
    pg.locator(TA).fill(LONG); pg.locator(TA).evaluate("e => e.dispatchEvent(new Event('input', { bubbles: true }))"); time.sleep(0.5)
    tb = box(TA); cb = box(C); v = vp()
    okd("thread: a long draft grows the field up to the cap and the composer still fits", bool(tb) and tb["height"] <= 190 and inside(C) and inside(f"{C} .composer-send") and tappable(f"{C} .composer-send"), f"field_h={tb['height'] if tb else None} composer_h={cb['height'] if cb else None} vp_h={v['h']}")
    shot("thread-long")
    pg.locator(TA).fill(""); time.sleep(0.3)
    # the model menu and "Options" on screen
    if pg.locator(f"{C} .ctl.model").count():
        pg.locator(f"{C} .ctl.model").first.click(); time.sleep(0.6)
        okd("thread: the model menu opens inside the viewport", pg.locator(".menu").count() == 1 and inside(".menu"), str(box(".menu")))
        shot("thread-model-menu"); pg.keyboard.press("Escape"); time.sleep(0.3)
    if pg.locator(f"{C} .ctl.more:visible").count():
        pg.locator(f"{C} .ctl.more:visible").first.click(); time.sleep(0.6)
        okd("thread: the Parameters menu opens inside the viewport", pg.locator(".menu").count() == 1 and inside(".menu"), str(box(".menu")))
        pg.keyboard.press("Escape"); time.sleep(0.3)
    # --- simple chat ---
    goto("Чат")
    try: pg.wait_for_selector(".top .engine button[data-engine='simple']", timeout=20000); pg.locator(".top .engine button[data-engine='simple']").click(); time.sleep(0.6)
    except Exception: pass
    panel_open(); c = pg.locator(".panel .sess[data-chat]").first
    if c.count(): c.click(); time.sleep(0.5)
    panel_close()
    try: pg.wait_for_selector(".thread .chat .chat-composer", timeout=20000); time.sleep(0.6)
    except Exception: pass
    if pg.locator(".thread .chat .chat-composer").count():
        composer_checks("simple", C, TA, 0.6 if v["h"] < 500 else 0.45)
        audit("simple", "simple chat with the composer")
        pg.locator(TA).fill(LONG); pg.locator(TA).evaluate("e => e.dispatchEvent(new Event('input', { bubbles: true }))"); time.sleep(0.5)
        tb = box(TA)
        okd("simple: a long draft grows the field up to the cap and the composer still fits", bool(tb) and tb["height"] <= 190 and inside(C) and inside(f"{C} .composer-send") and tappable(f"{C} .composer-send"), f"field_h={tb['height'] if tb else None}")
        shot("simple-long"); pg.locator(TA).fill("")
    else: okd("simple: the simple chat composer renders", False)

with sync_playwright() as p:
    b = p.chromium.launch()
    # at least one simple chat is needed — create our own and remove
    # it at the end
    cst, cb_ = raw("/agents/api/chats", "POST", json.dumps({"title": f"matrix {MARK}"}).encode()); CHAT = (json.loads(cb_ or b"{}") if cst in (200, 201) else {}).get("id")
    for name, w, h, dpr, touch, what in SCREENS:
        if ONLY and name not in ONLY: continue
        opts = {"viewport": {"width": w, "height": h}, "device_scale_factor": dpr, "is_mobile": touch and w < 1000, "has_touch": touch}
        if touch: opts["user_agent"] = "Mozilla/5.0 (Linux; Android 14; SM-F926B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Mobile Safari/537.36"
        ctx = b.new_context(**opts, locale="ru-RU", ignore_https_errors=True)
        ctx.add_init_script("try { if (!localStorage.getItem('agents.lang')) localStorage.setItem('agents.lang', 'ru'); localStorage.removeItem('agents.ui'); } catch (e) {}")
        pg = ctx.new_page(); errors = []; pg.on("pageerror", lambda e: errors.append(str(e)))
        print(f"INFO  {name}: {w}×{h} @{dpr} touch={touch} — {what}", flush=True)
        try: scenario(name, pg, touch, w, h)
        except Exception as e: ok(f"{name}: scenario completed without exceptions", False, str(e).split("\n")[0][:200]); pg.screenshot(path=f"shots/cmx-{name}-error.png")
        ok(f"{name}: no page errors", not errors, str(errors[:2])[:200])
        ctx.close()
    if CHAT: raw(f"/agents/api/chats/{CHAT}", "DELETE")
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
