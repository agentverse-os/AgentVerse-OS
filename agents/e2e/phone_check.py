import os, re, time, json, ssl, urllib.request, random, string
from playwright.sync_api import sync_playwright
# A walk through all app screens on the phone (docs/qa-plan.md §2, docs/agents-ux.md §9.4): every section, tab, menu, popup
# and form — on every screen it is checked that the page is no wider than the window and no visible element goes past the
# edge without a scrollable container; long screens are scrolled in steps. Clipped labels and small buttons are notes, not
# failures. Frames — shots/phone-<screen>.png, the notes summary — shots/phone-audit.json.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note, f"[{time.strftime('%H:%M:%S')}]", flush=True)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def raw(path, method="GET", body=None, ctype="application/json"):
    try:
        r = urllib.request.urlopen(urllib.request.Request(f"https://{HOST}{path}", data=body, method=method, headers={"Content-Type": ctype} if body is not None else {}), context=ctx_ssl, timeout=120); return r.status, r.read()
    except urllib.error.HTTPError as e: return e.code, e.read()
MARK = "ph-" + "".join(random.choice(string.ascii_lowercase) for _ in range(6))
DEVICE = os.environ.get("PHONE", "iPhone 13")
audit_log = {}

from audit_js import AUDIT_JS, SCROLLER_JS
_st, _ag = raw("/agents/api/agents", "POST", json.dumps({"name": f"phone probe {MARK}", "brief": "Тестовый агент для обхода экранов телефона; ходов не делает.", "provider": "claude", "workspace": P, "mode": "ask"}).encode())
PROBE_AGENT = (json.loads(_ag or b"{}") if _st == 200 else {}).get("id")
try:
  with sync_playwright() as p:
    b = p.chromium.launch()
    ctx = b.new_context(**p.devices[DEVICE], locale="ru-RU", ignore_https_errors=True)
    ctx.add_init_script("try { if (!localStorage.getItem('agents.lang')) localStorage.setItem('agents.lang', 'ru'); localStorage.removeItem('agents.ui'); } catch (e) {}")
    pg = ctx.new_page(); errors = []; pg.on("pageerror", lambda e: errors.append(str(e)))

    def vp(): return pg.evaluate("({w: window.innerWidth, h: window.innerHeight, sw: document.documentElement.scrollWidth})")
    def inside(sel):
        b_ = pg.locator(sel).first.bounding_box() if pg.locator(sel).count() else None; v = vp()
        return bool(b_) and b_["x"] >= -1 and b_["y"] >= -1 and b_["x"] + b_["width"] <= v["w"] + 1 and b_["y"] + b_["height"] <= v["h"] + 1
    seen_err = [0]
    def screen(name, extra=True, note=""):
        """A frame + a screen check: the page is no wider than the window, visible elements are within it (or inside a scrollable container),
        no new page errors since the previous screen."""
        time.sleep(0.5); a = pg.evaluate(AUDIT_JS); audit_log[name] = a
        pg.screenshot(path=f"shots/phone-{name}.png")
        fresh_err = errors[seen_err[0]:]; seen_err[0] = len(errors); a["errors"] = fresh_err
        ok(f"{name}: fits the phone screen (no page overflow, no element outside the viewport, every control tappable, no page errors)" + (f" — {note}" if note else ""), a["sw"] <= a["w"] + 1 and not a["wide"] and not a["covered"] and extra and not fresh_err, f"sw={a['sw']} w={a['w']} wide={a['wide'][:3]} covered={a['covered'][:3]}" + (f" errors={fresh_err[:2]}" if fresh_err else ""))
        if a["clipped"]: print(f"NOTE  {name}: clipped text {a['clipped'][:4]}", flush=True)
        if a["tiny"]: print(f"NOTE  {name}: small tap targets {a['tiny'][:4]}", flush=True)
        return a
    def scrolled(name, steps=3):
        """Extra frames of a long screen: scrolling the largest container in window-height steps."""
        s = pg.evaluate(SCROLLER_JS)
        if not s: return
        n = 1
        while n <= steps and s and s["top"] + s["ch"] < s["sh"] - 8:
            pg.evaluate("(() => { const e = document.querySelector('[data-pw=scroller]'); e.scrollTop = Math.min(e.scrollTop + e.clientHeight - 40, e.scrollHeight); })()"); time.sleep(0.4)
            s = pg.evaluate("(() => { const e = document.querySelector('[data-pw=scroller]'); return e ? { sh: e.scrollHeight, ch: e.clientHeight, top: e.scrollTop } : null; })()")
            n += 1; screen(f"{name}-{n}")
        pg.evaluate("(() => { const e = document.querySelector('[data-pw=scroller]'); if (e) e.scrollTop = 0; })()")
    def goto(name):
        if name in ("Агенты", "Настройки"): pg.locator(".tabbar .tb:has-text('Ещё')").click(); pg.wait_for_selector(".sheet .mi", timeout=5000); pg.locator(f".sheet .mi:has-text('{name}')").click()
        else: pg.locator(f".tabbar .tb:has-text('{name}')").click()
        time.sleep(0.9)
    def has_panel(): return pg.locator(".panel").count() > 0 and pg.locator(".top .burger").first.is_visible()
    def panel_open():
        if has_panel() and not pg.locator(".app.popen").count(): pg.locator(".top .burger").first.click(); time.sleep(0.5)
    def panel_close():
        if pg.locator(".app.popen").count(): pg.keyboard.press("Escape"); time.sleep(0.4)
    def esc(n=1):
        for _ in range(n): pg.keyboard.press("Escape"); time.sleep(0.3)
    def step(name, fn):
        try: fn()
        except Exception as e: ok(f"{name}: step completed without exceptions", False, str(e).split("\n")[0][:200]); pg.screenshot(path=f"shots/phone-{name}-error.png"); esc(2)

    pg.goto(f"https://{HOST}/agents/", wait_until="load", timeout=60000); pg.wait_for_selector(".app", timeout=60000); time.sleep(3)
    v = vp(); ok("shell: phone layout (tab bar, no rail)", pg.locator(".tabbar .tb").count() == 5 and not pg.locator(".rail").is_visible(), f"w={v['w']}")

    # ---- workplaces: the tree, a conversation, menus, panel tabs, search, the "new"
    # menu ----
    sid = None
    def work():
        global sid
        goto("Рабочие места"); panel_open(); pg.wait_for_selector(".panel .ws", timeout=30000); time.sleep(0.6)
        screen("work-tree", inside(".panel"), "workspace tree drawer")
        row = pg.locator(f".panel .ws:has(.wname:has-text('{P}')) .sess[data-id]").first
        try: row.wait_for(state="attached", timeout=15000)
        except Exception: pass
        sid = row.get_attribute("data-id") if row.count() else None
        more = pg.locator(f".panel .ws:has(.wname:has-text('{P}')) .sess[data-id] button").first
        if more.count() or row.count():
            # the row menu on the phone is a long press (contextmenu); Playwright sends it with the
            # right button
            if more.count(): more.click()
            else: row.click(button="right")
            time.sleep(0.5); screen("work-tree-menu", pg.locator(".menu").count() == 1 and inside(".menu"), "session row menu (long press)")
            # own name (§4.8): the "Rename…" item → a dialog on the screen, Esc closes without changes
            ren = pg.locator(".menu .mi").filter(has_text="Переименовать")
            if ren.count():
                ren.first.click(); time.sleep(0.6); screen("work-rename", pg.locator(".rename-dlg").count() == 1 and inside(".rename-dlg") and inside(".rename-dlg .rename-save"), "rename dialog from the row menu"); esc(); time.sleep(0.3)
                ok("work: Esc closes the rename dialog", pg.locator(".rename-dlg").count() == 0)
            else: esc()
        plus = pg.locator(f".panel .ws:has(.wname:has-text('{P}')) .icon.xs").first
        if plus.count():
            plus.click(); time.sleep(0.5); screen("work-new-menu", pg.locator(".menu").count() == 1 and inside(".menu"), "new session menu"); esc()
        if not sid: print(f"SKIP  work: no session in {P}", flush=True); panel_close(); return
        panel_open(); row.click(); time.sleep(0.6); panel_close()
        if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5)
        pg.wait_for_selector(".thread .chat .composer", timeout=20000)
        screen("work-thread", inside(".thread .chat .composer") and inside(".thread .t-head"), "thread with composer")
        ov = pg.evaluate("Array.from(document.querySelectorAll('.thread .chat .m.user')).filter(m => m.querySelector('.txt') && m.querySelector('.macts')).map(m => { const r1 = m.querySelector('.txt').getBoundingClientRect(), r2 = m.querySelector('.macts').getBoundingClientRect(); return r1.left < r2.right && r2.left < r1.right && r1.top < r2.bottom && r2.top < r1.bottom; })")
        ok("work: user-message actions (edit, fork) sit below the text on touch and never overlap it", bool(ov) and not any(ov), f"bubbles={len(ov)}")
        mic = pg.locator(".thread .chat .composer-actions .ptt-btn").first; mb = mic.bounding_box() if mic.count() else None
        clip = pg.locator(".thread .chat .composer-actions .composer-attach:not(.ptt-btn)").first; cb = clip.bounding_box() if clip.count() else None
        ok("work: the dictation button (§4.7) sits in the composer actions, ≥ 44 px wide and as tall as the attach button, inside the screen", bool(mb) and bool(cb) and mb["width"] >= 44 and abs(mb["height"] - cb["height"]) < 1 and mb["height"] >= 32 and inside(".thread .chat .composer-actions .ptt-btn"), f"mic={ {k: round(v) for k, v in mb.items()} if mb else None} clip_h={round(cb['height']) if cb else None}")
        if pg.locator(".top .seg button:has-text('Code')").count():
            pg.locator(".top .seg button:has-text('Code')").click(); time.sleep(1.0); screen("work-code", True, "Code view")
            # collapsible cards (§4.5): collapsed by default on the phone (except the just-added one), a tap on the header expands one and collapses the
            # previous
            cards = pg.locator(".canvas .card"); n_cards = cards.count()
            if n_cards >= 1:
                hs = [cards.nth(i).bounding_box()["height"] for i in range(n_cards)]
                ok("canvas: on the phone at most one card is expanded, the rest are folded to a header (≤ 60 px)", sum(1 for h in hs if h > 60) <= 1, f"heights={[round(h) for h in hs]}")
                folded_i = next((i for i in range(n_cards) if hs[i] <= 60), None)
                if folded_i is not None:
                    cards.nth(folded_i).locator(".c-head .ttl").click(); time.sleep(0.8)
                    hs2 = [cards.nth(i).bounding_box()["height"] for i in range(n_cards)]
                    ok("canvas: tapping a folded card's title expands it (≥ 50% of the screen) and folds the previously expanded one", hs2[folded_i] >= vp()["h"] * 0.5 and sum(1 for h in hs2 if h > 60) == 1 and cards.nth(folded_i).locator(".c-body").count() == 1, f"heights={[round(h) for h in hs2]}")
                    screen("work-code-expanded", True, "one card expanded on the phone")
            pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.5)
        pg.locator(".thread .chat .composer-controls .ctl.model").click(); time.sleep(0.6)
        screen("work-model-menu", pg.locator(".menu").count() == 1 and inside(".menu"), "model menu"); esc()
        # On the phone the model and "Options" are the main buttons; mode, effort, skills, MCP and context are available in the
        # "Options" menu (mode and context above the field as text, their clickable duplicates are hidden, slice 45).
        pg.locator(".thread .chat .composer-controls .ctl.more").click(); time.sleep(0.5)
        pg.get_by_role("menuitem", name=re.compile(r"^Режим разрешений", re.I)).click(); time.sleep(0.6)
        screen("work-modes", pg.locator(".thread .chat .modes").count() == 1 and inside(".thread .chat .modes"), "permission modes via Parameters"); esc()
        pg.locator(".thread .chat .composer-controls .ctl.more").click(); time.sleep(0.5)
        pg.get_by_role("menuitem", name=re.compile(r"^навыки(?:\s*·.*)?$", re.I)).click(); time.sleep(0.8)
        screen("work-skills", pg.locator(".skills-pop").count() == 1, "skills popup via Parameters"); esc()
        if pg.locator(".thread .chat .composer-state .composer-short-context").count() and pg.locator(".thread .chat .composer-state .composer-short-context").is_visible():
            pg.locator(".thread .chat .composer-controls .ctl.more").click(); time.sleep(0.5)
            pg.get_by_role("menuitem", name=re.compile(r"^Контекст", re.I)).click(); time.sleep(0.6); screen("work-context-menu", pg.locator(".menu").count() == 1 and inside(".menu"), "context menu via Parameters"); esc()
        m = pg.locator(".thread .t-head .thread-menu")
        if m.count(): m.click(); time.sleep(0.5); screen("work-session-menu", pg.locator(".menu").count() == 1 and inside(".menu"), "session menu"); esc()
        for tab, nm in (("CLI", "cli"), ("Shell", "shell"), ("Файлы", "files"), ("Git", "git"), ("Чат", "chat")):
            t_ = pg.locator(f".thread .pane .tabs .tab:has-text('{tab}')")
            if t_.count():
                t_.first.click(); time.sleep(1.8 if nm in ("cli", "shell") else 1.2); screen(f"work-tab-{nm}", inside(".thread .pane"), f"pane tab {tab}")
                if nm == "files" and pg.locator(".thread .pane .row, .thread .pane .file, .thread .pane .entry").count():
                    pg.locator(".thread .pane .row, .thread .pane .file, .thread .pane .entry").first.click(); time.sleep(1.0); screen("work-tab-files-open", inside(".thread .pane"), "file opened")
        panel_open(); pg.locator(".panel .search input").fill("Reply")
        try: pg.wait_for_selector(".panel .hits .hit", timeout=20000)
        except Exception: pass
        screen("work-search", inside(".panel .hits .hit") if pg.locator(".panel .hits .hit").count() else True, "message search")
        pg.locator(".panel .search input").fill(""); time.sleep(0.3); panel_close()
    step("work", work)

    # ---- inbox ----
    def inbox():
        goto("Входящие"); time.sleep(0.8); screen("inbox", True, "inbox")
        it = pg.locator("main.main .item, main.main .card-s, main.main .inb").first
        if it.count(): it.click(); time.sleep(0.8); screen("inbox-open", True, "inbox item")
    step("inbox", inbox)

    # ---- chat: the list, a conversation, the model, the system prompt and presets, tools, the second engine,
    # a new chat ----
    def chat():
        goto("Чат"); pg.wait_for_selector(".top .engine button[data-engine='simple']", timeout=20000)
        ok("chat: the engine switch lives in the top bar and both buttons are tappable on the phone", inside(".top .engine button[data-engine='simple']") and inside(".top .engine button[data-engine='advanced']") and pg.evaluate("(() => { const e = document.querySelector(\".top .engine button[data-engine='advanced']\"); const b = e.getBoundingClientRect(); const h = document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2); return !!h && e.contains(h); })()"))
        pg.locator(".top .engine button[data-engine='simple']").click(); time.sleep(0.8); panel_open(); screen("chat-list", inside(".panel"), "simple chats list")
        c = pg.locator(".panel .sess[data-chat]").first
        if c.count():
            c.click(); time.sleep(0.5); panel_close()
            try: pg.wait_for_selector(".thread .chat .m", timeout=20000)
            except Exception: pass
            screen("chat-open", inside(".thread .chat .composer"), "simple chat")
            # the typed text is not wiped by polling/route loading (the owner's complaint: "wipes the entered text after a
            # pause")
            ta_ = pg.locator(".thread .chat textarea").first; ta_.click(); ta_.type("ка"); time.sleep(7)
            ok("chat: typed text survives the periodic refresh and the routes load (7 s)", ta_.input_value() == "ка", repr(ta_.input_value())); ta_.fill("")
            pg.locator(".thread .chat .composer-controls .composer-model").click(); time.sleep(0.6); screen("chat-model-menu", pg.locator(".menu").count() == 1 and inside(".menu"), "model menu with search"); esc()
            pg.locator(".thread .chat .composer-controls .composer-more").click(); time.sleep(0.5)
            # item names are looked up by text: the accessible name includes "✓" on an enabled item and the "/img" hint, and \b in a
            # JS regex does not work after Cyrillic — get_by_role with exact/a regex did not find them
            screen("chat-parameters", inside(".menu") and pg.locator(".menu .mi[role='menuitemcheckbox']").filter(has_text="Системный промпт").count() == 1 and pg.locator(".menu .mi[role='menuitem']").filter(has_text="картинка").count() == 1, "Parameters contains system prompt and image generation")
            pg.locator(".menu .mi").filter(has_text="Системный промпт").first.click(); time.sleep(0.8)
            screen("chat-system", pg.locator(".syspop").count() == 1 and inside(".syspop"), "system prompt + presets via Parameters"); scrolled("chat-system", 1); pg.locator(".syspop .mh button").click(); time.sleep(0.3)
            if pg.locator(".thread .chat .m .diagram").count(): screen("chat-diagram", pg.evaluate("Array.from(document.querySelectorAll('.thread .chat .diagram')).every(d => d.getBoundingClientRect().right <= window.innerWidth + 1)"), "diagram cards")
        else: print("SKIP  chat: no simple chats", flush=True); panel_close()
        panel_close(); pg.locator(".top .engine button[data-engine='advanced']").click(); time.sleep(0.8); panel_open(); screen("chat-advanced", inside(".panel"), "advanced engine list")
        s = pg.locator(".panel .sess[data-id]").first
        if s.count():
            s.click(); time.sleep(0.6); panel_close()
            screen("chat-advanced-open", not pg.locator(".top .title").first.is_visible() and inside(".top button.icon.sm.ghost"), "scratch session: title gives way to the action buttons")
        else: panel_close()
    step("chat", chat)

    # ---- agents: four tabs, cards, the new-agent form ----
    def routines():
        goto("Агенты"); time.sleep(0.8)
        ok("routines: no burger and no squeezed title in the phone top bar (tabs and «+» own the row)", not pg.locator("main.main .top .burger").first.is_visible() and not pg.locator("main.main .top .title").first.is_visible() and inside("main.main .top .seg") and inside("main.main .top button.sm.primary"))
        for idx, (nm, item) in enumerate((("agents", ".rt-item[data-agent]"), ("routines", ".rt-item[data-routine]"), ("skills", ".rt-item[data-skill]"), ("runs", ".runs .run, .runs tr, .runs .row"))):
            seg = pg.locator("main.main .top .seg button"); seg.nth(idx).click(); time.sleep(0.9)
            screen(f"routines-{nm}-list", True, f"{nm} list")
            it = pg.locator(f"main.main {item}").first
            if it.count() and nm != "runs":
                it.click(); time.sleep(0.9); screen(f"routines-{nm}", True, f"{nm} card"); scrolled(f"routines-{nm}", 2)
            if nm != "runs":
                nb = pg.locator("main.main .top button.sm.primary")
                if nb.count(): nb.click(); time.sleep(0.9); screen(f"routines-{nm}-new", True, f"new {nm} form"); scrolled(f"routines-{nm}-new", 2)
    step("routines", routines)

    # ---- models: cards, tiles, routes, the table — with scrolling ----
    def models():
        goto("Модели"); time.sleep(1.5)
        k0 = pg.locator(".card-s.kpi").first.bounding_box() or {"width": 0}; kw = pg.locator(".kpis").first.bounding_box() or {"width": 1}
        screen("models", pg.locator(".card-s.kpi").count() >= 3 and k0["width"] >= kw["width"] - 2 and inside(".kpi .csv") and inside(".kpi .spark") if pg.locator(".kpi .spark").count() else pg.locator(".card-s.kpi").count() >= 3 and k0["width"] >= kw["width"] - 2 and inside(".kpi .csv"), "KPI cards: the spend card spans the row with CSV and sparkline inside"); scrolled("models", 4)
        per = pg.locator("main.main .top .seg button, main.main .period button")
        if per.count() >= 2: per.nth(1).click(); time.sleep(1.2); screen("models-period", True, "another period")
    step("models", models)

    # ---- settings: all nine tabs, account forms ----
    def settings():
        goto("Настройки"); time.sleep(0.8)
        tabs = pg.locator(".subnav button"); names = [re.sub(r"\s+", " ", tabs.nth(i).inner_text()).strip() for i in range(tabs.count())]
        ok("settings: ten tabs", tabs.count() == 10, str(names))
        for i, nm in enumerate(names):
            tabs.nth(i).click(); time.sleep(1.4 if i == 0 else 0.8); key = ["accounts", "routes", "chat", "notify", "perm", "mcp", "look", "voice", "browser", "about"][i] if i < 10 else f"tab{i}"
            if key == "accounts":
                try: pg.wait_for_selector(".acct-card .lrow, .acct-card .lerr", timeout=45000)
                except Exception: pass
            extra = True
            if key == "routes" and pg.locator(".tbl.prices").count():
                pg.locator(".tbl.prices").scroll_into_view_if_needed(); time.sleep(0.3)
                pr = pg.evaluate("(() => { const tw = document.querySelector('.tbl.prices .tw'); const rows = [...document.querySelectorAll('.tbl.prices tr')]; return { noScroll: tw.scrollWidth <= tw.clientWidth + 1, rows: rows.length, labelled: rows.filter(r => r.querySelector('td.num')).every(r => r.querySelector('td.num').getBoundingClientRect().top > r.querySelector('td.mono').getBoundingClientRect().top + 8 && getComputedStyle(r.querySelector('td.num'), '::before').content.length > 2) }; })()")
                extra = pr["noScroll"] and pr["rows"] >= 1 and pr["labelled"]
                print(f"INFO  prices on phone: {pr}", flush=True)
            screen(f"settings-{key}", extra, f"tab {nm}" + (" — prices as labelled rows without horizontal scroll" if key == "routes" else "")); scrolled(f"settings-{key}", 3)
            if key == "accounts":
                for j, fname in enumerate(("import", "login", "add")):
                    btn = pg.locator(".settings .page-h button.sm").nth(j)
                    if btn.count(): btn.click(); time.sleep(0.7); screen(f"settings-accounts-{fname}", True, f"{fname} form"); btn.click(); time.sleep(0.3)
                if pg.locator(".acct-card .acts button").first.count():
                    pg.locator(".acct-card .acts button").first.click(); time.sleep(2.0); screen("settings-accounts-refreshed", True, "after per-account refresh")
                sel_ = pg.locator(".settings select").first
                if sel_.count(): sel_.scroll_into_view_if_needed(); time.sleep(0.3); screen("settings-accounts-defaults", inside(".settings select"), "project default selects")
                # subscription windows are labelled the same for all providers: 10080 min — "7 days", not
                # "a week"
                lbls = [pg.locator(".acct-card .lrow .lbl").nth(i).inner_text().strip() for i in range(pg.locator(".acct-card .lrow .lbl").count())]
                ok("settings: Accounts — window labels are uniform («5 ч» / «7 дней», no «неделя»)", bool(lbls) and "неделя" not in lbls and any(l.startswith("7") for l in lbls), str(lbls[:6]))
                # a gate account on the phone: avatar, name and actions in one row, the pill under
                # the name
                g = pg.locator(".glogin-row").first
                if g.count():
                    g.scroll_into_view_if_needed(); time.sleep(0.3)
                    bb = lambda sel: g.locator(sel).first.bounding_box() or {"x": -1, "y": -1, "width": 0, "height": 0}
                    av, ell, acts, pill = bb(".av"), bb(".ell"), bb(".gacts"), bb(".pill")
                    okrow = av["x"] < ell["x"] < acts["x"] and abs(av["y"] - ell["y"]) < 14 and pill["y"] > ell["y"] + 10 and acts["x"] + acts["width"] <= vp()["w"] + 1
                    screen("settings-accounts-gate-row", okrow, f"gate login row: avatar/name/actions on one line, pill below (av={av['x']:.0f} name={ell['x']:.0f} acts={acts['x']:.0f})")
                # a gate target without a management key: the hint and ⟳ in one row, the heading does not break
                # by letters
                hint = pg.locator(".gtarget:has(.ghint)").first
                if hint.count():
                    hint.scroll_into_view_if_needed(); time.sleep(0.3)
                    gh = pg.evaluate("(() => { const g = document.querySelector('.gtarget:has(.ghint)'); const h = g.querySelector('.ghint').getBoundingClientRect(), r = g.querySelector('.row button:last-child').getBoundingClientRect(), b = g.querySelector('.row > b').getBoundingClientRect(); return { sameLine: Math.abs(h.top - r.top) < 24 || (r.top >= h.top && r.bottom <= h.bottom + 2), titleWidth: b.width, w: window.innerWidth }; })()")
                    ok("settings: gate target without a management key — hint and ⟳ share a line, the title keeps its width", gh["sameLine"] and gh["titleWidth"] > gh["w"] * 0.6, str(gh))
    step("settings", settings)

    # ---- menu sections: the bottom bar (five items) and the "More" sheet
    # (four) ----
    def menus():
        tabs = pg.locator(".tabbar .tb"); names = [re.sub(r"\s+", " ", tabs.nth(i).inner_text()).strip() for i in range(tabs.count())]
        okall = True
        for i, nm in enumerate(names[:-1]):
            tabs.nth(i).click(); time.sleep(0.7); act = pg.locator(".tabbar .tb.active").first
            good = act.count() == 1 and re.sub(r"\s*\d+$", "", act.inner_text().strip()) == re.sub(r"\s*\d+$", "", nm) and pg.locator("main.main").count() == 1 and not pg.locator(".app.popen").count()
            okall = okall and good
            if not good: print(f"INFO  tabbar «{nm}»: active={act.inner_text().strip() if act.count() else None}", flush=True)
        ok("menus: every tab-bar item opens its section and highlights itself (a count badge on «Входящие» is allowed)", okall and len(names) == 5, str(names))
        for nm, check in (("Агенты", lambda: pg.locator("main.main .top .seg button:has-text('Рутины')").count() == 1), ("Настройки", lambda: pg.locator(".subnav button").count() == 10)):
            pg.locator(".tabbar .tb:has-text('Ещё')").click(); pg.wait_for_selector(".sheet .mi", timeout=5000); pg.locator(f".sheet .mi:has-text('{nm}')").click(); time.sleep(0.9)
            ok(f"menus: sheet item «{nm}» opens the section and closes the sheet", check() and not pg.locator(".sheet").count() and pg.locator(".tabbar .tb.active:has-text('Ещё')").count() == 1)
        pg.locator(".tabbar .tb:has-text('Ещё')").click(); pg.wait_for_selector(".sheet .mi", timeout=5000); items = [re.sub(r"\s+", " ", pg.locator(".sheet .mi").nth(i).inner_text()).strip() for i in range(pg.locator(".sheet .mi").count())]
        ok("menus: the sheet lists Agents, Settings, theme and search", len(items) == 4 and any("Агенты" in x for x in items) and any("Настройки" in x for x in items) and any("Поиск" in x for x in items), str(items))
        pg.locator(".sheet").click(position={"x": 10, "y": 10}); time.sleep(0.4)
        ok("menus: a tap outside the sheet closes it", not pg.locator(".sheet").count())
    step("menus", menus)

    # ---- buttons and toggles: the conversation header, the composer, settings (appearance, language, permissions,
    # notifications, accounts), forms ----
    def controls():
        global sid
        if sid:
            goto("Рабочие места"); panel_open(); pg.locator(f".panel .sess[data-id='{sid}']").first.click(); time.sleep(0.5); panel_close()
            pg.wait_for_selector(".thread .chat .composer", timeout=20000)
            hb = pg.locator(".thread .t-head button"); vis_ = [i for i in range(hb.count()) if hb.nth(i).is_visible()]
            ok("controls: the thread menu is visible and all visible header buttons are inside the screen", pg.locator(".thread .t-head .thread-menu").is_visible() and len(vis_) >= 1 and all(inside(f".thread .t-head button >> nth={i}") for i in vis_), f"visible={len(vis_)} of {hb.count()}")
            m = pg.locator(".thread .t-head .thread-menu"); m.click(); time.sleep(0.5); mi = pg.locator(".menu .mi"); k = mi.count()
            # a long menu (11 items) scrolls inside itself: what matters is that the menu itself is on screen and the items are either visible
            # or reachable by scrolling
            menu_fit = inside(".menu") and pg.evaluate("(() => { const m = document.querySelector('.menu'); if (!m) return false; const r = m.getBoundingClientRect(); const items = [...m.querySelectorAll('.mi')]; return items.every(i => { const b = i.getBoundingClientRect(); return b.left >= r.left - 1 && b.right <= r.right + 1; }) && (m.scrollHeight <= m.clientHeight + 1 || /(auto|scroll)/.test(getComputedStyle(m).overflowY)); })()")
            ok("controls: the session menu lists its items inside the screen (long menus scroll within themselves)", k >= 3 and menu_fit, f"items={k}"); esc()
            model_sel = ".thread .chat .composer-controls .ctl.model"; more_sel = ".thread .chat .composer-controls .ctl.more"
            control_boxes = [pg.locator(sel).bounding_box() or {"height": 0} for sel in (model_sel, more_sel)]
            labels = [pg.locator(sel).get_attribute("title") or "" for sel in (model_sel, more_sel)]
            # slice 45: the settings row on the phone is compacted to 32 px, targets ≥ 44 px wide; mode and context — as text in the top status
            # line
            ok("controls: only model and Parameters are visible in the phone settings row (32 px, targets ≥ 44 px wide)", pg.locator(".thread .chat .composer-controls .ctl:visible").count() == 2 and all(pg.locator(sel).is_visible() and inside(sel) for sel in (model_sel, more_sel)) and all(box["height"] >= 32 and box.get("width", 0) >= 44 for box in control_boxes), str(labels)[:120] + " " + str([(round(b.get("width", 0)), round(b["height"])) for b in control_boxes]))
            tools_h = pg.locator(".thread .chat .composer-controls").bounding_box() or {"height": 999}
            ok("controls: the phone composer settings bar is one compact 32 px row", 32 <= tools_h["height"] <= 36, f"height={tools_h['height']:.0f}")
            ok("controls: the permission mode is shown as text in the upper state row; its tappable summary and the settings chip are hidden", inside(".thread .chat .composer-state .composer-compact-mode") and pg.locator(".thread .chat .composer-state .composer-compact-mode").is_visible() and pg.locator(".thread .chat .composer-state .composer-compact-mode").inner_text().strip() != "" and not pg.locator(".thread .chat .composer-state .composer-mode-summary").is_visible() and not pg.locator(".thread .chat .composer-controls .ctl.mode").is_visible())
            if pg.locator(".thread .chat .composer-state .composer-short-context").count():
                ok("controls: context is shown as short text in the upper state row, its tappable chip is hidden and the settings row has none", inside(".thread .chat .composer-state .composer-short-context") and pg.locator(".thread .chat .composer-state .composer-short-context").is_visible() and not pg.locator(".thread .chat .composer-state .composer-context").is_visible() and not pg.locator(".thread .chat .composer-controls .composer-context").count())
            pg.locator(more_sel).click(); time.sleep(0.5); screen("controls-more-sheet", pg.locator(".menu .mi").count() >= 2 and inside(".menu"), "Parameters menu with mode, skills and available effort/MCP/cache settings"); esc()
            pg.locator(more_sel).click(); time.sleep(0.5); effort = pg.get_by_role("menuitem", name=re.compile(r"^reasoning effort\b", re.I))
            if effort.count():
                effort.click(); time.sleep(0.6); screen("controls-effort", pg.locator(".modes, .effpop").count() == 1, "effort panel via Parameters"); esc()
            else: esc()
            pg.locator(more_sel).click(); time.sleep(0.5); mcp = pg.get_by_role("menuitem", name=re.compile(r"^MCP\b"))
            if mcp.count():
                mcp.click(); time.sleep(0.9)
                ok("controls: Parameters → MCP opens Settings → MCP", pg.locator(".subnav button.active:has-text('MCP')").count() == 1); screen("controls-mcp-from-chip", True, "Settings → MCP from Parameters")
            else: esc()
        # Settings → Appearance: theme, language, density — the radios switch and switch
        # back
        goto("Настройки"); pg.locator(".subnav button:has-text('Внешний вид')").click(); time.sleep(0.6)
        # the "Appearance" radio buttons: after another session's rework the groups are named appearance-mode/-density/-language inside
        # .radio-options
        NAMES = {"th": "appearance-mode", "dn": "appearance-density", "lg": "appearance-language"}
        def pick(name, text):
            r = pg.locator(f"label:has(input[name='{NAMES.get(name, name)}']):has-text('{text}') input, .radios label:has(input[name='{name}']):has-text('{text}') input").first; r.check(force=True); time.sleep(0.6); return r.is_checked()
        light = pick("th", "Светлая") and pg.evaluate("document.documentElement.dataset.theme") == "light"; screen("controls-theme-light", light, "light theme via radio")
        system = pick("th", "систем"); dark = pick("th", "Тёмная") and pg.evaluate("document.documentElement.dataset.theme") == "dark"
        ok("controls: theme radios (light / system / dark) switch the theme and the dark one restores it", light and system and dark)
        dense = pick("dn", "омпакт"); screen("controls-density-compact", True, "compact density")
        ok("controls: density radios switch (compact → normal)", dense and pick("dn", "бычн"))
        en = pick("lg", "English") and pg.locator(".subnav button:has-text('Accounts')").count() == 1; screen("controls-lang-en", en, "English via radio")
        ru = pick("lg", "Русский") and pg.locator(".subnav button:has-text('Аккаунты')").count() == 1
        ok("controls: language radios switch the dictionary without a reload (en → ru)", en and ru)
        # Permissions: the scope segment and rule chips (locally, without
        # saving)
        pg.locator(".subnav button:has-text('Разрешения')").click(); time.sleep(0.8)
        seg = pg.locator("main.main .seg button:has-text('проект')").first
        if seg.count():
            seg.click(); time.sleep(0.5); ok("controls: permissions scope segment switches to «проект»", "active" in (seg.get_attribute("class") or "")); pg.locator("main.main .seg button:has-text('пользователь')").first.click(); time.sleep(0.4)
        chips = pg.locator("main.main button.xs:has-text('+ ')"); ta = pg.locator("main.main textarea").first; before_v = ta.input_value()
        pick_chip = next((chips.nth(i) for i in range(chips.count()) if chips.nth(i).inner_text().strip().lstrip('+ ').strip() not in before_v), None)
        if pick_chip:
            rule = pick_chip.inner_text().strip().lstrip('+ ').strip(); pick_chip.click(); time.sleep(0.4)
            ok("controls: a rule chip appends its rule to the allow list (unsaved)", rule in ta.input_value() and ta.input_value() != before_v, rule)
        sw = pg.locator("main.main button.switch").first
        if sw.count(): sw.scroll_into_view_if_needed(); time.sleep(0.3)
        ok("controls: the bypass-policy switch is present and tappable (not toggled by the test)", sw.count() == 1 and inside("main.main button.switch") and sw.get_attribute("aria-pressed") in ("true", "false") and pg.evaluate("(() => { const e = document.querySelector('main.main button.switch'); const b = e.getBoundingClientRect(); const h = document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2); return !!h && (e.contains(h) || h.contains(e)); })()"))
        # Notifications: the button shows the
        # subscription data
        pg.locator(".subnav button:has-text('Уведомления')").click(); time.sleep(0.6)
        btn = pg.locator("main.main button:has-text('показать данные подписки')")
        if btn.count():
            btn.click(); time.sleep(1.5); screen("controls-notify-shown", pg.locator("main.main .card-s").count() >= 1 and not btn.count(), "subscription data shown")
        # Accounts: 🗑 asks for confirmation — decline
        pg.locator(".subnav button:has-text('Аккаунты')").click(); time.sleep(1.0)
        seen = {}
        def on_dialog(d): seen["msg"] = d.message; d.dismiss()
        pg.once("dialog", on_dialog); pg.locator(".acct-card .acts button").last.click(); time.sleep(0.8)
        ok("controls: the account delete button asks for confirmation (dismissed, nothing deleted)", "Удалить аккаунт" in seen.get("msg", "") and pg.locator(".acct-card").count() >= 1, seen.get("msg", "")[:60])
        # Agents → the routine form: the "enabled" toggle
        goto("Агенты"); pg.locator("main.main .top .seg button:has-text('Рутины')").click(); time.sleep(0.7); pg.locator("main.main .top button.sm.primary").click(); time.sleep(0.8)
        sw = pg.locator("main.main button.switch").first
        if sw.count():
            sw.scroll_into_view_if_needed(); a0 = sw.get_attribute("aria-pressed"); sw.click(); time.sleep(0.3); a1 = sw.get_attribute("aria-pressed"); sw.click(); time.sleep(0.3); a2 = sw.get_attribute("aria-pressed")
            ok("controls: the routine form switch toggles and toggles back", a0 != a1 and a0 == a2, f"{a0}→{a1}→{a2}")
        # Inbox: "open" leads to the session
        goto("Входящие"); time.sleep(0.8); op = pg.locator("main.main button.xs.ghost:has-text('открыть')").first
        if op.count():
            op.click(); time.sleep(1.2); ok("controls: the inbox «открыть» button leads to the conversation", pg.locator(".thread .chat").count() == 1 and pg.locator(".tabbar .tb.active:has-text('Рабочие места')").count() == 1)
        # Models: the period segment — three
        # positions
        goto("Модели"); time.sleep(1.0); seg = pg.locator("main.main .top .seg button"); okp = True
        for i in range(seg.count()):
            seg.nth(i).click(); time.sleep(1.0); okp = okp and "active" in (seg.nth(i).get_attribute("class") or "") and pg.locator(".card-s.kpi").count() >= 3
        ok("controls: the Models period segment (today / 7d / 30d) switches and keeps the KPI cards", okp and seg.count() == 3); seg.nth(0).click(); time.sleep(0.5)
    step("controls", controls)

    # ---- the permission modal: a live session in the "ask" mode, a command outside the rules → a card with buttons on the
    # phone ----
    def perm_modal():
        s = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"phone perm {MARK}", "permission_mode": "default"}).encode())[1] or b"{}"); psid = s.get("id")
        if not psid: ok("perm: probe session created", False); return
        try:
            time.sleep(4); raw(f"/agents/api/core/projects/{P}/sessions/{psid}/prompt", "POST", json.dumps({"content": f"Run this exact shell command with the Bash tool: touch phone-perm-{MARK}.txt   (relative path, nothing else). Then reply with exactly: phone-perm-{MARK}"}).encode())
            goto("Рабочие места"); panel_open(); pg.wait_for_selector(f".panel .sess[data-id='{psid}']", timeout=30000); pg.locator(f".panel .sess[data-id='{psid}']").click(); time.sleep(0.5); panel_close()
            try: pg.wait_for_selector(".thread .chat .m.perm:not(.done)", timeout=120000)
            except Exception: pass
            card = pg.locator(".thread .chat .m.perm:not(.done)").last
            btns = card.locator("button"); n = btns.count()
            screen("perm-card", card.count() == 1 and n >= 2 and all(inside(f".thread .chat .m.perm:not(.done) button >> nth={i}") for i in range(n)), f"permission card with {n} buttons on the phone")
            if card.count() and card.locator("button:has-text('Разрешить и запомнить')").count():
                card.locator("button:has-text('Разрешить и запомнить')").click(); time.sleep(0.5)
                screen("perm-remember-menu", pg.locator(".menu .mi").count() >= 3 and inside(".menu"), "remember-scope menu"); esc()
            if card.count() and card.locator("button:has-text('Отклонить')").count():
                card.locator("button:has-text('Отклонить')").click(); time.sleep(1.0)
                ok("perm: «Отклонить» resolves the card on the phone", pg.locator(".thread .chat .m.perm.done").count() >= 1)
            for _ in range(40):
                time.sleep(3); st_ = (json.loads(raw(f"/agents/api/core/projects/{P}/sessions/{psid}")[1] or b"{}").get("info") or {}).get("status")
                if st_ in ("idle", "error", "exited"): break
        finally:
            for _ in range(3):
                if raw(f"/agents/api/core/projects/{P}/sessions/{psid}", "DELETE")[0] in (204, 404): break
                time.sleep(3)
    step("perm_modal", perm_modal)

    # ---- the "More" sheet, the palette, the
    # light theme ----
    def misc():
        pg.locator(".tabbar .tb:has-text('Ещё')").click(); pg.wait_for_selector(".sheet .mi", timeout=5000); time.sleep(0.4); screen("sheet", inside(".sheet .mi"), "More sheet")
        pg.locator(".sheet .mi:has-text('Поиск')").click(); time.sleep(0.6)
        if pg.locator(".palette").count():
            pg.locator(".palette input").fill("Reply"); time.sleep(1.5); screen("palette", inside(".palette input"), "command palette with results"); esc()
        else: screen("palette", False, "palette did not open")
        pg.locator(".tabbar .tb:has-text('Ещё')").click(); pg.wait_for_selector(".sheet .mi", timeout=5000); pg.locator(".sheet .mi:has-text('Светлая')").click(); time.sleep(0.6)
        light = pg.evaluate("document.documentElement.dataset.theme") == "light"
        goto("Рабочие места"); panel_open(); time.sleep(0.5); screen("light-work-tree", light, "light theme, tree"); panel_close()
        if sid:
            screen("light-work-thread", True, "light theme, thread")
        goto("Настройки"); pg.locator(".subnav button").first.click(); time.sleep(1.2); screen("light-settings-accounts", True, "light theme, accounts")
        pg.locator(".tabbar .tb:has-text('Ещё')").click(); pg.wait_for_selector(".sheet .mi", timeout=5000); pg.locator(".sheet .mi:has-text('Тёмная')").click(); time.sleep(0.4)
    step("misc", misc)

    # ---- a model turn and live text in the phone layout, in its own session
    # ----
    def turn():
        s = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"phone probe {MARK}"}).encode())[1] or b"{}"); psid = s.get("id")
        if not psid: ok("turn: probe session created", False); return
        try:
            goto("Рабочие места"); panel_open(); pg.wait_for_selector(f".panel .sess[data-id='{psid}']", timeout=20000); pg.locator(f".panel .sess[data-id='{psid}']").click(); time.sleep(0.5); panel_close()
            pg.wait_for_selector(".thread .chat textarea", timeout=20000); ta = pg.locator(".thread .chat textarea"); ta.click(); time.sleep(0.3)
            ta.fill(f"Reply with exactly: {MARK}"); screen("turn-typing", inside(".thread .chat .composer") and pg.evaluate("document.activeElement?.tagName") == "TEXTAREA", "composer focused with text")
            pg.locator(".thread .chat button.send").click()
            try: pg.wait_for_selector(f".thread .chat .m.agent:has-text('{MARK}')", timeout=180000); got = True
            except Exception: got = False
            screen("turn-answer", got and inside(".thread .chat .composer"), "answer in the phone layout")
        finally:
            for _ in range(3):
                if raw(f"/agents/api/core/projects/{P}/sessions/{psid}", "DELETE")[0] in (204, 404): break
                time.sleep(3)
    step("turn", turn)

    ok("phone: no page errors", not errors, str(errors[:2])[:200])
    json.dump(audit_log, open("shots/phone-audit.json", "w"), ensure_ascii=False, indent=1)
    ctx.close(); b.close()
finally:
    if PROBE_AGENT: raw(f"/agents/api/agents/{PROBE_AGENT}", "DELETE")
print("ALL PASS" if all(res) else "SOME FAIL")
