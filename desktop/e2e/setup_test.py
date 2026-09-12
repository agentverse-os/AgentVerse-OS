# e2e мастера первого запуска: автозапуск при первом входе, пока setup.done не выставлен; шаги на настроенном стенде зелёные
# (Tailscale Running, адрес = MagicDNS-имя, сертификат Let's Encrypt); проверка «с этого устройства»; «Готово» пишет setup.done
# и окно больше не открывается само; повторное открытие из Настроек → Система и по /#setup; «Показывать снова» сбрасывает флаг.
import os, time, json, ssl, urllib.request
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET", body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"https://{HOST}/api{path}", data=data, method=method, headers={"Content-Type": "application/json"} if data else {})
    return json.load(urllib.request.urlopen(req, context=ctx_ssl))
st = api("/setup")
ok("api: setup state has steps", {"tailscale", "edge", "projects", "done"} <= set(st), str(sorted(st))[:120])
ok("api: tailscale running with name and https certs", st["tailscale"]["state"] == "Running" and st["tailscale"]["https_certs"] and st["tailscale"]["dns_name"] == HOST, str({k: st["tailscale"][k] for k in ("state", "dns_name", "https_certs")}))
ok("api: edge matches tailscale, reachable, cert ok", st["edge"]["matches_tailscale"] and st["edge"]["reachable"] and st["edge"]["cert_ok"] is True, str(st["edge"]))
try:
    urllib.request.urlopen(urllib.request.Request(f"https://{HOST}/api/setup/edge-host", data=b'{"host":"evil.example.com"}', method="POST", headers={"Content-Type": "application/json"}), context=ctx_ssl); rejected = False
except urllib.error.HTTPError as e: rejected = e.code in (400, 500) and "не совпадает" in e.read().decode()
ok("api: switch host to foreign name rejected", rejected)
api("/setup/done", "PUT", {"done": False})
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); pg = ctx.new_page()
    # первый вход из нового браузера (без cloudos.firstrun) → мастер открывается сам вместо Проектов
    ctx.add_init_script("localStorage.setItem('cloudos.certwarn.until', '9999999999999')")
    s0 = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); s0.pop("session", None); pg.request.put(f"https://{HOST}/api/settings/desktop", data=s0)
    pg.goto(f"https://{HOST}/", wait_until="domcontentloaded"); pg.wait_for_selector(".win .setup", timeout=30000); time.sleep(3)
    win = pg.locator(".win", has_text="Первый запуск").first
    ok("first visit: setup wizard auto-opened", win.count() == 1 and pg.locator(".win").count() == 1, str(pg.locator(".win").count()))
    steps = win.locator(".step")
    ok("wizard: five sections", steps.count() == 5, str(steps.count()))
    ok("wizard: tailscale step green with node name", steps.nth(0).evaluate("e => e.classList.contains('ok')") and HOST in steps.nth(0).inner_text())
    ok("wizard: address step green", steps.nth(1).evaluate("e => e.classList.contains('ok')"), steps.nth(1).inner_text()[:100].replace("\n", " "))
    ok("wizard: certificate step says Let's Encrypt", "Let's Encrypt" in steps.nth(2).inner_text() and steps.nth(2).evaluate("e => e.classList.contains('ok')"))
    ok("wizard: projects step green", steps.nth(3).evaluate("e => e.classList.contains('ok')") and "Проектов:" in steps.nth(3).inner_text())
    win.locator("button", has_text="Проверить с этого устройства").click(); time.sleep(2.5)
    ok("wizard: device check passes from browser", "✅ С этого устройства" in steps.nth(1).inner_text(), steps.nth(1).inner_text()[-160:].replace("\n", " "))
    pg.screenshot(path="shots/setup-wizard.png")
    win.locator("button.primary", has_text="Готово").click(); time.sleep(1.5)
    ok("wizard: done closes window and sets setup.done", pg.locator(".win", has_text="Первый запуск").count() == 0 and api("/setup")["done"] is True)
    # новый браузер: мастер больше не открывается сам, открываются Проекты
    ctx2 = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); ctx2.add_init_script("localStorage.setItem('cloudos.certwarn.until', '9999999999999')"); pg2 = ctx2.new_page()
    pg2.goto(f"https://{HOST}/", wait_until="domcontentloaded"); time.sleep(4)
    ok("after done: first visit opens Projects, not wizard", pg2.locator(".win", has_text="Первый запуск").count() == 0 and pg2.locator(".win", has_text="Проекты").count() >= 1, str([w.inner_text()[:20] for w in pg2.locator(".win .title").all()]))
    # из Настроек → Система и по deep link
    pg2.goto(f"https://{HOST}/#settings", wait_until="domcontentloaded"); time.sleep(2.5)
    pg2.locator(".win", has_text="Настройки").first.locator("button", has_text="Мастер первого запуска").click(); time.sleep(2.5)
    w2 = pg2.locator(".win", has_text="Первый запуск").first
    ok("settings: opens wizard, marked as done", w2.count() == 1 and "мастер пройден" in w2.inner_text())
    w2.locator("button", has_text="Показывать при первом входе снова").click(); time.sleep(1.5)
    ok("wizard: reset flag via button", api("/setup")["done"] is False)
    pg2.goto(f"https://{HOST}/#setup", wait_until="domcontentloaded"); time.sleep(2.5)
    ok("deep link #setup opens wizard", pg2.locator(".win", has_text="Первый запуск").count() >= 1)
    b.close()
api("/setup/done", "PUT", {"done": True})  # стенд настроен — оставляем пройденным
print("ALL PASS" if all(res) else "SOME FAIL")
