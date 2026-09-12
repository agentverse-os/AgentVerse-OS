import os
import time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); res = []
def ok(n, c, note=""): res.append(c); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    for engine in ("firefox", "chromium"):
        b = getattr(p, engine).launch(); pg = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}).new_page()
        pg.add_init_script("localStorage.setItem('cloudos.certwarn.until', '9999999999999')")  # браузер тестов не доверяет сертификату — баннер не должен мешать кликам
        pg.goto(f"https://{HOST}/#files", wait_until="domcontentloaded"); pg.wait_for_selector(".files .dd-btn", timeout=30000); time.sleep(2)
        # на широком экране корни — в боковой панели, выпадающий список — сортировка (3 пункта); проверяем реальные клики мышью
        pg.locator(".files .dd-btn").click(); time.sleep(0.4)
        ok(f"{engine}: dropdown opens", pg.locator(".dd-list li").count() == 3, str(pg.locator(".dd-list li").count()))
        pg.locator(".dd-list li", has_text="по дате").click(); time.sleep(0.5)
        ok(f"{engine}: option picked by mouse", "по дате" in pg.locator(".files .dd-cur").inner_text())
        pg.locator(".files .sroot", has_text="alpha").click(); time.sleep(3)
        ok(f"{engine}: root switched via sidebar", pg.locator(".files .row-entry", has_text=".npm-global").count() == 1)
        pg.locator(".files .sroot", has_text="Store").click(); time.sleep(2)
        ok(f"{engine}: switch to store root", pg.locator(".files .row-entry", has_text="whoami").count() >= 1)
        # Store: фильтр источника
        pg.goto(f"https://{HOST}/#store", wait_until="domcontentloaded"); pg.wait_for_selector(".card.app", timeout=30000); time.sleep(1.5)
        pg.locator(".filters .dd-btn").first.click(); pg.locator(".dd-list li", has_text="Umbrel").click(); time.sleep(1.2)
        badges = pg.locator(".badge.src").all_inner_texts()
        ok(f"{engine}: store source filter", badges and all(bd == "Umbrel" for bd in badges), f"{len(badges)} карточек")
        b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
