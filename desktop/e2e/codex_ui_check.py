import os, time, json, ssl, urllib.request, random, string
from playwright.sync_api import sync_playwright
# Карточка Chat с провайдером Codex из окна «Агенты»: кнопка «💬 Codex» создаёт сессию, в шапке видна модель (🧠),
# ответ приходит в чат, размышления — раскрывающимся блоком, результат хода — строкой. Скриншоты в shots/. Один ход через gate.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note, flush=True)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET", body=None):
    req = urllib.request.Request(f"https://{HOST}/api{path}", method=method, data=json.dumps(body).encode() if body is not None else None, headers={"content-type": "application/json"})
    try:
        r = urllib.request.urlopen(req, context=ctx_ssl, timeout=90); t = r.read(); return r.status, (json.loads(t) if t else None)
    except urllib.error.HTTPError as e: return e.code, None
MARK = "ui-" + "".join(random.choice(string.ascii_lowercase) for _ in range(6))
def wait(pg, sel, timeout=120): pg.wait_for_selector(sel, timeout=timeout * 1000)
before = {s["id"] for s in (api(f"/projects/{P}/sessions")[1] or [])}
sid = None
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); pg = ctx.new_page()
    pg.goto(f"https://{HOST}/#agents", wait_until="networkidle"); pg.wait_for_selector(".agents", timeout=30000); time.sleep(1.5)
    proj = pg.locator(f".agents .proj:has-text('{P}')")
    # режимы сверху; меню «＋» у workspace: чаты Claude/Codex и TUI терминал/Claude/Codex (text-is — в списке сессий есть «💬 <название>»)
    ok("modes: Agent · Code · Thread · Chat", [b.strip() for b in pg.locator(".agents .modes button").all_inner_texts()] == ["Agent", "Code", "Thread", "Chat"])
    pg.locator(".agents .modes button:text-is('Thread')").click(); proj.locator("button.add").click(); time.sleep(0.5)
    ok("menu: chat Claude/Codex and TUI shell/Claude/Codex", all(proj.locator(f".menu button:text-is('{t}')").count() == 1 for t in ("💬 Claude", "💬 Codex", "⌨ Терминал", "⌨ Claude", "⌨ Codex")))
    proj.locator(".menu button:text-is('💬 Codex')").click(); wait(pg, ".agents .chat", 30); time.sleep(3)
    new = [s for s in (api(f"/projects/{P}/sessions")[1] or []) if s["id"] not in before and s["agent"] == "codex"]; ok("codex session created from the window", len(new) == 1, [s["id"] for s in new]); sid = new[0]["id"] if new else None
    wait(pg, ".agents .head button.model:has-text('gpt')", 60); ok("header: model chip shows the gate model", True, pg.locator(".agents .head button.model").inner_text())
    pg.screenshot(path="shots/cx1-new.png")
    pg.locator(".chat textarea").fill(f"Reply with exactly this and nothing else: {MARK}. Do not run any commands."); pg.locator(".chat button:has-text('Отправить')").click()
    wait(pg, f".chat .m.user:has-text('{MARK}')", 10); ok("chat: user message shown", True)
    # gate стенда режет ответы gpt-oss недетерминированно — проверяем сам факт ответа, совпадение маркера только отмечаем
    wait(pg, ".chat .m.agent", 200); ok("chat: codex reply shown", True, "маркер " + ("совпал" if pg.locator(f".chat .m.agent:has-text('{MARK}')").count() else "обрезан gate'ом: " + pg.locator(".chat .m.agent").last.inner_text()[:40]))
    ok("chat: reasoning block rendered", pg.locator(".chat details.think").count() >= 1)
    wait(pg, ".chat .m.sys:has-text('$')", 30); ok("chat: result line", True, pg.locator(".chat .m.sys:has-text('$')").last.inner_text()[:100])
    pg.screenshot(path="shots/cx2-reply.png")
    b.close()
if sid:
    api(f"/projects/{P}/sessions/{sid}", "DELETE"); time.sleep(3); api(f"/projects/{P}/sessions/{sid}", "DELETE")
    ok("removed", api(f"/projects/{P}/sessions/{sid}")[0] == 404)
print("ALL PASS" if all(res) else "SOME FAIL")
