import os, time, json, ssl, urllib.request
from playwright.sync_api import sync_playwright
# The "Agents" window (ADR-006, stage 1, slice 2): agentd is deployed by the core into a running workspace, a terminal
# is created from the window, works through the cloudd WebSocket proxy, survives closing the window and a new tab (the
# output tail is replayed), is stopped and removed. Screenshots in shots/.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET", body=None):
    req = urllib.request.Request(f"https://{HOST}/api{path}", method=method, data=json.dumps(body).encode() if body is not None else None, headers={"content-type": "application/json"})
    try:
        r = urllib.request.urlopen(req, context=ctx_ssl, timeout=60); t = r.read(); return r.status, (json.loads(t) if t else None)
    except urllib.error.HTTPError as e: return e.code, None

# 1. the core deployed agentd and sees it
st, h = api(f"/projects/{P}/agentd"); ok("api: agentd health via cloudd", st == 200 and h and h.get("protocol") == 1, json.dumps(h)[:120])
ok("api: 404 for unknown pty maps through", api(f"/projects/{P}/ptys/nope")[0] == 404)
before = {p["id"] for p in (api(f"/projects/{P}/ptys")[1] or [])}
MARK = f"agents-e2e-{int(time.time())}"

def open_agents(pg):
    pg.goto(f"https://{HOST}/#agents", wait_until="networkidle"); pg.wait_for_selector(".agents", timeout=30000); time.sleep(1.5)

def term_text(pg):
    return pg.evaluate("Array.from(document.querySelectorAll('.xterm-rows > div')).map(r => r.textContent).join('\\n')")

with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); pg = ctx.new_page()
    open_agents(pg)
    ok("window: project listed", pg.locator(f".agents .proj:has-text('{P}')").count() == 1)
    pg.locator(".agents .modes button:text-is('Thread')").click(); pg.locator(f".agents .proj:has-text('{P}') button.add").click(); pg.locator(f".agents .proj:has-text('{P}') .menu button:has-text('Терминал')").click(); pg.wait_for_selector(".agents .xterm", timeout=20000); time.sleep(2)
    after = {p["id"] for p in (api(f"/projects/{P}/ptys")[1] or [])}; new = list(after - before)
    ok("window: terminal created in workspace", len(new) == 1, str(new)); pid = new[0] if new else ""
    pg.keyboard.type(f"echo {MARK}\n"); time.sleep(1.5)
    ok("terminal: command output streamed", MARK in term_text(pg)); pg.screenshot(path="shots/ag1-terminal.png")
    # 2. close the window — the process is alive; open again — the
    # tail is in place
    pg.evaluate("document.querySelector('.win.active .ctl .close')?.click()"); time.sleep(1)
    st, info = api(f"/projects/{P}/ptys/{pid}"); ok("survives window close", info and info["alive"], json.dumps(info)[:90])
    open_agents(pg); pg.locator(".agents .modes button:text-is('Thread')").click(); pg.locator(f".agents .proj:has-text('{P}') .sess:has-text('Терминал')").last.click(); pg.wait_for_selector(".agents .xterm", timeout=20000); time.sleep(2)
    ok("reopen: scrollback replayed", MARK in term_text(pg)); pg.screenshot(path="shots/ag2-reopened.png")
    # 3. a new tab (another device) — the same terminal with the same history
    pg2 = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1280, "height": 800}).new_page()
    open_agents(pg2); pg2.locator(".agents .modes button:text-is('Thread')").click(); pg2.locator(f".agents .proj:has-text('{P}') .sess:has-text('Терминал')").last.click(); pg2.wait_for_selector(".agents .xterm", timeout=20000); time.sleep(2)
    ok("another tab: same terminal, same history", MARK in term_text(pg2))
    pg2.keyboard.type("echo from-second-tab\n"); time.sleep(1.5)
    ok("first tab sees input from second", "from-second-tab" in term_text(pg)); pg2.context.close()
    # 4. stop and remove
    pg.locator(".agents .head button:has-text('Остановить')").click(); time.sleep(2)
    st, info = api(f"/projects/{P}/ptys/{pid}"); ok("stop: process exited", info is not None and not info["alive"], json.dumps(info)[:90])
    pg.locator(".agents .head button:has-text('Убрать')").click(); time.sleep(1.5)
    ok("remove: gone from list", api(f"/projects/{P}/ptys/{pid}")[0] == 404 and pg.locator(f".agents .proj:has-text('{P}') .sess:has-text('Терминал')").count() == 0)
    pg.screenshot(path="shots/ag3-empty.png")
    # 5. a separate web app: /?app=agents — only the "Agents" window, without the dock and desktop icons
    pg.goto(f"https://{HOST}/?app=agents", wait_until="networkidle"); pg.wait_for_selector(".standalone .agents", timeout=30000); time.sleep(1)
    ok("standalone: agents app without the shell", pg.locator("nav.dock, nav.taskbar, .icons").count() == 0 and pg.locator(f".standalone .proj:has-text('{P}')").count() == 1)
    pg.screenshot(path="shots/ag4-standalone.png"); b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
