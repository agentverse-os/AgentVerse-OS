import os, time, json
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append((n, c)); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1280, "height": 860}); pg = ctx.new_page()
    pg.goto(f"https://{HOST}/#settings", wait_until="networkidle"); pg.wait_for_selector("text=Акцентный цвет", timeout=20000); time.sleep(1)
    ok("theme: dark by default", pg.evaluate("document.documentElement.dataset.theme") == "dark")
    pg.screenshot(path="shots/t1-settings-dark.png", full_page=True)
    pg.click("text=Светлая"); time.sleep(1.2)
    ok("theme: switched to light", pg.evaluate("document.documentElement.dataset.theme") == "light")
    pg.click("button[aria-label='#D65DB1']"); pg.click("text=Круглые"); pg.click("text=Просторно"); time.sleep(1.2)
    ok("theme: accent applied", pg.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--accent').trim().toLowerCase()") == "#d65db1")
    pg.screenshot(path="shots/t2-settings-light.png", full_page=True)
    pg.goto(f"https://{HOST}/#projects", wait_until="networkidle"); pg.wait_for_selector("text=capabilities", timeout=15000); time.sleep(1)
    ok("theme: persists across navigation", pg.evaluate("document.documentElement.dataset.theme") == "light")
    pg.screenshot(path="shots/t3-projects-light.png")
    # сохранено в ядре?
    r = pg.request.get(f"https://{HOST}/api/settings/desktop"); j = r.json()
    ok("theme: saved in cloudd", j.get("appearance", {}).get("mode") == "light" and j["appearance"]["accent"].lower() == "#d65db1", json.dumps(j)[:120])
    # новый браузерный контекст (другое устройство) получает те же настройки
    ctx2 = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1280, "height": 860}); p2 = ctx2.new_page()
    p2.goto(f"https://{HOST}/#store", wait_until="networkidle"); time.sleep(1.5)
    ok("theme: roams to another device", p2.evaluate("document.documentElement.dataset.theme") == "light")
    p2.screenshot(path="shots/t4-store-light.png")
    # вернуть тёмную по умолчанию
    pg.goto(f"https://{HOST}/#settings", wait_until="networkidle"); pg.wait_for_selector("text=Сброс", timeout=15000); pg.click("text=Вернуть оформление по умолчанию"); time.sleep(1.2)
    ok("theme: reset to dark", pg.evaluate("document.documentElement.dataset.theme") == "dark")
    b.close()
print("ALL PASS" if all(c for _, c in res) else "SOME FAIL")
