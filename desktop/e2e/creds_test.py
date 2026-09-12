# e2e: где взять логин/пароль Coder — Настройки → Устройства и окно «Пароли и доступы»; статусы Store по-русски; страница вместо 502.
import os, time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net")
os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900})
    ctx.add_init_script("localStorage.setItem('cloudos.firstrun', '1'); localStorage.setItem('cloudos.certwarn.until', '9999999999999')"); pg = ctx.new_page()
    st = pg.request.get(f"https://{HOST}/api/status").json()
    ok("api: status carries coder_user + coder_password_set", st.get("coder_user") == "admin@cloudos.local" and st.get("coder_password_set") is True, str({k: st.get(k) for k in ("coder_user", "coder_password_set")}))
    cr = pg.request.get(f"https://{HOST}/api/system/credentials/coder/reveal").json()
    ok("api: reveal returns url/user/password", cr.get("user") == st.get("coder_user") and len(cr.get("password", "")) >= 8 and cr.get("url", "").startswith("https://"), cr.get("url"))
    # Настройки → Устройства
    pg.goto(f"https://{HOST}/#settings", wait_until="domcontentloaded"); win = pg.locator(".win", has_text="Настройки").first; win.wait_for(timeout=30000); time.sleep(1)
    ok("settings: «Устройства» section present", win.locator("h3", has_text="Устройства").count() == 1)
    txt = win.inner_text()
    ok("settings: Coder login shown", "admin@cloudos.local" in txt and "логин" in txt)
    ok("settings: password hidden by default", cr["password"] not in txt)
    win.locator("button[title=показать]").first.click(); time.sleep(1.2)
    ok("settings: reveal shows real Coder password", cr["password"] in win.inner_text())
    pg.screenshot(path="shots/creds-settings.png")
    # «Пароли и доступы»
    pg.goto(f"https://{HOST}/#vault", wait_until="domcontentloaded"); v = pg.locator(".win", has=pg.locator("h2", has_text="Пароли и доступы")).first; v.wait_for(timeout=30000); time.sleep(1.5)
    sec = v.locator("section.sys").first
    ok("vault: Coder section on top", sec.count() == 1 and "Coder" in sec.inner_text() and "admin@cloudos.local" in sec.inner_text())
    sec.locator("button[title=показать]").click(); time.sleep(1.2)
    ok("vault: reveal shows Coder password", cr["password"] in sec.inner_text())
    ok("vault: open link goes to Coder", (sec.locator("a", has_text="").first.get_attribute("href") or "").startswith(st["coder_url"]))
    pg.screenshot(path="shots/creds-vault.png")
    # Store: статусы по-русски
    pg.goto(f"https://{HOST}/#store", wait_until="domcontentloaded"); pg.wait_for_selector(".card.app", timeout=30000); time.sleep(1.5)
    pg.fill("input.search", "whoami"); time.sleep(0.8)
    c = pg.locator(".card.app", has_text="whoami").first
    ok("store: running state in Russian", c.locator(".badge.st").inner_text().strip() == "работает", c.locator(".badge.st").inner_text() if c.locator(".badge.st").count() else "нет .badge.st")
    ok("store: no «Логи» button for a healthy app", c.locator("button", has_text="Логи").count() == 0)
    # событие о показе пароля
    ev = pg.request.get(f"https://{HOST}/api/events?limit=50").text()
    ok("events: reveal is logged", "показан пароль администратора Coder" in ev)
    b.close()
print(f"\n{sum(res)}/{len(res)} passed"); raise SystemExit(0 if all(res) else 1)
