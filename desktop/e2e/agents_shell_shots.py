import os, time, json, ssl, urllib.request
from playwright.sync_api import sync_playwright
# Скриншоты app shell «Агентов» (ADR-006, этап 2) для владельца: холст (Code) отдельной вкладкой с карточками разных
# workspace'ов, режим Thread, окно внутри Desktop, телефон с выдвижной панелью. Создаёт терминал и чат Claude в PROJECT
# (без ходов — токены не тратятся), в конце убирает их. Уже существующие сессии владельца не трогает.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note, flush=True)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET", body=None):
    req = urllib.request.Request(f"https://{HOST}/api{path}", method=method, data=json.dumps(body).encode() if body is not None else None, headers={"content-type": "application/json"})
    try:
        r = urllib.request.urlopen(req, context=ctx_ssl, timeout=90); t = r.read(); return r.status, (json.loads(t) if t else None)
    except urllib.error.HTTPError as e: return e.code, None
before_s = {s["id"] for s in (api(f"/projects/{P}/sessions")[1] or [])}; before_p = {s["id"] for s in (api(f"/projects/{P}/ptys")[1] or [])}
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1600, "height": 960}); pg = ctx.new_page()
    pg.goto(f"https://{HOST}/?app=agents", wait_until="networkidle"); pg.wait_for_selector(".agents", timeout=30000); time.sleep(2)
    ok("standalone: own shell, no desktop chrome", pg.locator("nav.dock, nav.taskbar, .icons").count() == 0 and pg.locator(".agents .modes").count() == 1)
    pg.locator(".agents .modes button:text-is('Code')").click()
    # холст: терминал + чат Claude в PROJECT, плюс все живые сессии других workspace'ов (клик в дереве добавляет карточку)
    proj = pg.locator(f".agents .proj:has-text('{P}')")
    proj.locator("button.add").click(); proj.locator(".menu button:has-text('Терминал')").click(); pg.wait_for_selector(".agents .card .xterm", timeout=30000); time.sleep(1.5)
    pg.locator(".agents .card .xterm").first.click(); pg.keyboard.type("echo AgentVerse OS && uname -a && ls\n"); time.sleep(1)
    proj.locator("button.add").click(); proj.locator(".menu button:text-is('💬 Claude')").click(); pg.wait_for_selector(".agents .card .chat", timeout=30000); time.sleep(1)
    for s in pg.locator(".agents .sess:not(.dead)").all():
        if "▦" not in s.inner_text(): s.click(); time.sleep(0.6)
    time.sleep(3)
    n = pg.locator(".agents .card").count(); ok("canvas: several cards", n >= 2, f"cards={n}")
    pg.screenshot(path="shots/sh1-code.png")
    # перетаскивание: первую карточку в конец; ширина на две колонки
    cards = pg.locator(".agents .card")
    if n >= 2:
        first = cards.first.get_attribute("data-card"); cards.first.locator(".chead").drag_to(cards.last, target_position={"x": 40, "y": 10}); time.sleep(0.8)
        ok("canvas: drag reorders cards", pg.locator(".agents .card").last.get_attribute("data-card") == first)
        pg.locator(".agents .card").first.locator("button[title*='Ширина']").click(); time.sleep(0.6); ok("canvas: card widened", "w2" in (pg.locator(".agents .card").first.get_attribute("class") or ""))
        pg.screenshot(path="shots/sh2-code-wide.png")
    time.sleep(1.5)
    st, lay = api("/agents/layout"); ok("layout saved in the core", st == 200 and isinstance(lay, dict) and len(lay.get("cards") or []) == pg.locator(".agents .card").count(), json.dumps(lay)[:120])
    # Thread: одна сессия на весь экран
    pg.locator(".agents .card").first.locator("button[title*='Thread']").click(); time.sleep(1)
    ok("thread: one session full-screen", pg.locator(".agents .thread").count() == 1 and pg.locator(".agents .modes button.active").inner_text().strip() == "Thread")
    pg.screenshot(path="shots/sh3-thread.png")
    # внутри Desktop — то же приложение в окне со своей темой
    pg2 = ctx.new_page(); pg2.goto(f"https://{HOST}/#agents", wait_until="networkidle"); pg2.wait_for_selector(".agents", timeout=30000); time.sleep(2.5)
    ok("desktop window: same app, same layout", pg2.locator(".agents .card").count() == pg.locator(".agents .card").count() or pg2.locator(".agents .thread").count() == 1)
    pg2.screenshot(path="shots/sh4-window.png")
    # телефон: панель workspace'ов выдвигается
    ph = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 390, "height": 844}, device_scale_factor=2, is_mobile=True, has_touch=True); pp = ph.new_page()
    pp.goto(f"https://{HOST}/?app=agents", wait_until="networkidle"); pp.wait_for_selector(".agents", timeout=30000); time.sleep(2)
    pp.locator(".agents .burger").click(); time.sleep(0.6); ok("phone: sidebar drawer opens", pp.locator(".agents.side-open").count() == 1)
    pp.screenshot(path="shots/sh5-phone-drawer.png")
    pp.locator(f".agents .proj:has-text('{P}') .sess").first.click(); time.sleep(2); pp.screenshot(path="shots/sh6-phone-thread.png")
    ok("phone: session opens full-screen, drawer closes", pp.locator(".agents.side-open").count() == 0 and pp.locator(".agents .thread").count() == 1)
    b.close()
# уборка: только созданное этим скриптом
for s in (api(f"/projects/{P}/sessions")[1] or []):
    if s["id"] not in before_s: api(f"/projects/{P}/sessions/{s['id']}", "DELETE"); time.sleep(2); api(f"/projects/{P}/sessions/{s['id']}", "DELETE")
for s in (api(f"/projects/{P}/ptys")[1] or []):
    if s["id"] not in before_p: api(f"/projects/{P}/ptys/{s['id']}", "DELETE"); time.sleep(1); api(f"/projects/{P}/ptys/{s['id']}", "DELETE")
print("ALL PASS" if all(res) else "SOME FAIL")
