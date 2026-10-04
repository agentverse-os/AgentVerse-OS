import os, time, json, ssl, urllib.request
from playwright.sync_api import sync_playwright
# Карточка Chat в окне «Агенты» (ADR-006, этап 1, срез 3): сессия Claude создаётся из окна, ответ приходит в чат,
# запрос разрешения показывается карточкой и подтверждается кнопкой, история переживает закрытие окна, остановка и
# возобновление. Скриншоты в shots/. Тратит немного токенов через gate проекта.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET", body=None):
    req = urllib.request.Request(f"https://{HOST}/api{path}", method=method, data=json.dumps(body).encode() if body is not None else None, headers={"content-type": "application/json"})
    try:
        r = urllib.request.urlopen(req, context=ctx_ssl, timeout=90); t = r.read(); return r.status, (json.loads(t) if t else None)
    except urllib.error.HTTPError as e: return e.code, None
MARK = f"chat-e2e-{int(time.time())}"
def wait(pg, sel, timeout=120):
    pg.wait_for_selector(sel, timeout=timeout * 1000)
def open_agents(pg):
    pg.goto(f"https://{HOST}/#agents", wait_until="networkidle"); pg.wait_for_selector(".agents", timeout=30000); time.sleep(1.5)

before = {s["id"] for s in (api(f"/projects/{P}/sessions")[1] or [])}
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); pg = ctx.new_page()
    open_agents(pg)
    pg.locator(".agents .modes button:text-is('Thread')").click(); pg.locator(f".agents .proj:has-text('{P}') button.add").click(); pg.locator(f".agents .proj:has-text('{P}') .menu button:text-is('💬 Claude')").click(); wait(pg, ".agents .chat", 30); time.sleep(2)
    new = list({s["id"] for s in (api(f"/projects/{P}/sessions")[1] or [])} - before); ok("chat: session created from the window", len(new) == 1, str(new)); sid = new[0] if new else ""
    pg.locator(".chat textarea").fill(f"Reply with exactly this and nothing else: {MARK}"); pg.locator(".chat button:has-text('Отправить')").click()
    wait(pg, f".chat .m.user:has-text('{MARK}')", 10); ok("chat: user message shown", True)
    wait(pg, f".chat .m.agent:has-text('{MARK}')", 150); ok("chat: assistant reply streamed", True); pg.screenshot(path="shots/ch1-reply.png")
    wait(pg, ".chat .m.sys:has-text('$')", 30); ok("chat: result line with cost", pg.locator(".chat .m.sys:has-text('$')").count() >= 1)
    # разрешение инструмента карточкой
    path = f"/home/coder/{MARK}.txt"
    pg.locator(".chat textarea").fill(f"Create the file {path} containing the single word hello. Use the Write tool. Do not ask questions."); pg.locator(".chat button:has-text('Отправить')").click()
    wait(pg, ".chat .m.perm:not(.done)", 120); ok("chat: permission card shown", "Write" in pg.locator(".chat .m.perm:not(.done)").last.inner_text()); pg.screenshot(path="shots/ch2-permission.png")
    st_now = ""
    for _ in range(10):
        st_now = ((api(f"/projects/{P}/sessions/{sid}")[1] or {}).get("info") or {}).get("status", "")
        if st_now == "waiting": break
        time.sleep(0.5)
    ok("chat: status waiting", st_now == "waiting", st_now)
    pg.locator(".chat .m.perm:not(.done) button:has-text('Разрешить')").first.click(); time.sleep(1)
    wait(pg, ".chat .m.tool", 120); time.sleep(3)
    ok("chat: tool call rendered after approval", pg.locator(".chat .m.tool:has-text('Write')").count() >= 1)
    ok("chat: permission card settled", pg.locator(".chat .m.perm.done").count() >= 1)
    # закрыть окно и открыть снова — история из журнала
    pg.evaluate("document.querySelector('.win.active .ctl .close')?.click()"); time.sleep(1)
    open_agents(pg); pg.locator(".agents .modes button:text-is('Thread')").click(); pg.locator(f".agents .sess[data-id='{sid}']").click(); wait(pg, ".agents .chat", 30); time.sleep(2)
    ok("reopen: history replayed", pg.locator(f".chat .m.user:has-text('{MARK}')").count() >= 1 and pg.locator(".chat .m.tool:has-text('Write')").count() >= 1); pg.screenshot(path="shots/ch3-reopened.png")
    # остановить, возобновить, спросить про историю
    pg.locator(".agents .head button:has-text('Остановить')").click(); wait(pg, ".chat button:has-text('Возобновить')", 30); ok("stop: session exited", True)
    pg.locator(".chat button:has-text('Возобновить')").click(); time.sleep(3)
    pg.locator(".chat textarea").fill("What was the exact marker string I asked you to reply with earlier? Reply with just the string."); pg.locator(".chat button:has-text('Отправить')").click()
    wait(pg, f".chat .m.agent:has-text('{MARK}') >> nth=1", 150); ok("resume: history survives", True); pg.screenshot(path="shots/ch4-resumed.png")
    b.close()
# уборка
api(f"/projects/{P}/sessions/{sid}", "DELETE"); time.sleep(3); api(f"/projects/{P}/sessions/{sid}", "DELETE")
ok("removed", api(f"/projects/{P}/sessions/{sid}")[0] == 404)
print("ALL PASS" if all(res) else "SOME FAIL")
