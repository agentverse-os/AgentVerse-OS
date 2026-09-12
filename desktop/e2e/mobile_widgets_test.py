import os, time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); res = []
def ok(n, c, note=""): res.append(c); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 390, "height": 844}, has_touch=True, is_mobile=True); pg = ctx.new_page()
    pg.add_init_script("localStorage.setItem('cloudos.certwarn.until', '9999999999999')")  # браузер тестов не доверяет сертификату — баннер не должен мешать кликам
    # воспроизводим состояние телефона пользователя: локально сохранён пустой список без маркера cleared
    pg.goto(f"https://{HOST}/", wait_until="domcontentloaded"); pg.wait_for_selector(".mobile .grid .tile", timeout=30000)
    pg.evaluate("localStorage.setItem('cloudos.widgets.phone', '[]'); localStorage.removeItem('cloudos.widgets.phone.cleared')"); pg.reload(wait_until="domcontentloaded"); pg.wait_for_selector(".mobile .grid .tile", timeout=30000); time.sleep(3)
    ok("phone: empty local list without cleared → defaults shown", pg.locator(".mobile .widget").count() >= 1, f"{pg.locator('.mobile .widget').count()}")
    # явное удаление всех → пусто с подсказкой
    while pg.locator(".mobile .widget").count(): pg.locator(".mobile .widget .whead button[title='убрать виджет']").first.click(); time.sleep(0.4)
    pg.reload(wait_until="domcontentloaded"); pg.wait_for_selector(".mobile .grid .tile", timeout=30000); time.sleep(3)
    ok("phone: explicitly cleared stays empty with hint", pg.locator(".mobile .widget").count() == 0 and pg.locator(".wlayer .empty").count() == 1)
    pg.locator(".wlayer .empty").click(); time.sleep(0.5); pg.locator(".picker .pitem", has_text="Часы").click(); time.sleep(0.8)
    ok("phone: add from empty state", pg.locator(".mobile .widget", has_text="Часы").count() == 1)
    ok("phone: no fullwidth plus glyph", "＋" not in pg.inner_text("body"))
    pg.screenshot(path="shots/m5-widgets.png")
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
