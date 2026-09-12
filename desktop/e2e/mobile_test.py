import os, time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(c); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 390, "height": 844}, has_touch=True, is_mobile=True, device_scale_factor=2); pg = ctx.new_page()
    pg.add_init_script("localStorage.setItem('cloudos.certwarn.until', '9999999999999')")  # браузер тестов не доверяет сертификату — баннер не должен мешать кликам
    s0 = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); s0.pop("session", None); pg.request.put(f"https://{HOST}/api/settings/desktop", data=s0)
    pg.goto(f"https://{HOST}/", wait_until="domcontentloaded"); pg.wait_for_selector(".mobile .grid .tile", timeout=30000); time.sleep(4)
    ok("phone: home screen with tiles", pg.locator(".mobile .grid .tile").count() >= 8, f"{pg.locator('.mobile .grid .tile').count()} плиток")
    ok("phone: no desktop taskbar", pg.locator(".taskbar").count() == 0 and pg.locator(".navbar").count() == 1)
    ok("phone: widgets on home", pg.locator(".mobile .widget").count() >= 1)
    pg.screenshot(path="shots/m1-home.png")
    # плитка → приложение на весь экран
    pg.locator(".mobile .grid .tile", has_text="Store").click(); time.sleep(2)
    win = pg.locator(".win.mobile"); bb = win.bounding_box(); ok("phone: app opens full screen under status bar", win.count() == 1 and abs(bb["width"] - 390) < 2 and abs(bb["y"] - 44) < 2 and abs(bb["y"] + bb["height"] - (844 - 56)) < 2, str(bb))
    ok("phone: home hidden while app open", pg.locator(".mobile .home").count() == 0)
    pg.screenshot(path="shots/m2-store.png")
    # второе приложение, недавние
    pg.locator(".navbar button[title='домой']").click(); time.sleep(0.6); ok("phone: home button returns", pg.locator(".mobile .home").count() == 1)
    pg.locator(".mobile .grid .tile", has_text="Dozzle").click(); time.sleep(3); ok("phone: iframe app full screen", pg.locator(".win.mobile iframe").count() == 1)
    pg.locator(".navbar button[title='недавние']").click(); time.sleep(0.6); ok("phone: recents lists 2", pg.locator(".recents .rc").count() == 2)
    pg.screenshot(path="shots/m3-recents.png")
    pg.locator(".recents .rc-open", has_text="Store").click(); time.sleep(1); ok("phone: switch via recents", pg.locator(".win.mobile", has_text="Store").count() == 1)
    pg.locator(".navbar button[title='назад']").click(); time.sleep(0.6); ok("phone: back goes home", pg.locator(".mobile .home").count() == 1)
    # ящик приложений
    pg.locator(".mobile .search").click(); time.sleep(0.6); ok("phone: drawer", pg.locator(".menu input").count() == 1); pg.fill(".menu input", "gitea"); time.sleep(0.6); ok("phone: drawer search", pg.locator(".menu .item", has_text="Gitea").count() >= 1)
    pg.screenshot(path="shots/m4-drawer.png")
    # кнопки ящика кликабельны: «Закрыть все окна» и «Перезагрузить оболочку»
    pg.locator(".menu button", has_text="Закрыть все окна").click(); time.sleep(1.8); ok("phone: drawer close-all works", pg.locator(".menu").count() == 0 and len(pg.request.get(f"https://{HOST}/api/settings/desktop").json().get("session", {}).get("phone", [])) == 0)
    pg.locator(".mobile .search").click(); pg.wait_for_selector(".menu", timeout=10000)
    with pg.expect_navigation(timeout=15000): pg.locator(".menu button", has_text="Перезагрузить оболочку").click()
    ok("phone: drawer reload works", True)
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
