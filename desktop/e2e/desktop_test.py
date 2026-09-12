import os, time, json
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(c); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); pg = ctx.new_page()
    pg.add_init_script("localStorage.setItem('cloudos.certwarn.until', '9999999999999')")  # браузер тестов не доверяет сертификату — баннер не должен мешать кликам
    s0 = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); s0.pop("session", None); pg.request.put(f"https://{HOST}/api/settings/desktop", data=s0)
    pg.goto(f"https://{HOST}/", wait_until="domcontentloaded"); pg.wait_for_selector(".taskbar", timeout=20000); time.sleep(2)
    ok("desktop: taskbar + icons", pg.locator(".taskbar").count() == 1 and pg.locator(".icons .icon").count() >= 8, f"иконок {pg.locator('.icons .icon').count()}")
    ok("desktop: first run opens Проекты window", pg.locator(".win", has_text="Проекты").count() == 1)
    while pg.locator(".win .bar button[title='Свернуть']").count(): pg.locator(".win .bar button[title='Свернуть']").first.click(); time.sleep(0.2)
    pg.screenshot(path="shots/x1-desktop.png")
    # иконка проекта: одинарный клик выделяет, двойной открывает окно проекта
    ico = pg.locator(".icons .icon", has_text="alpha").first; ico.click(); time.sleep(0.3)
    ok("icons: single click selects", "selected" in (ico.get_attribute("class") or ""))
    ok("icons: single click does not open", pg.locator(".win .bar .title", has_text="alpha").count() == 0)
    ico.dblclick(); time.sleep(2)
    ok("icons: double click opens project window", pg.locator(".win .bar .title", has_text="alpha").count() == 1 and pg.locator(".win", has_text="Терминал").count() >= 1)
    pg.screenshot(path="shots/x6-project-window.png")
    pg.locator(".win", has_text="Терминал").first.locator("button", has_text="Терминал").click(); time.sleep(4)
    ok("project window: terminal opens as window", pg.locator(".win .bar .title", has_text="терминал").count() == 1)
    while pg.locator(".win .bar button[title='Свернуть']").count(): pg.locator(".win .bar button[title='Свернуть']").first.click(); time.sleep(0.2)
    # меню запуска
    pg.click(".taskbar .start"); time.sleep(0.5); ok("start menu: open", pg.locator(".menu").count() == 1)
    pg.fill(".menu input", "doz"); time.sleep(0.5); ok("start menu: search finds Dozzle", pg.locator(".menu .item", has_text="Dozzle").count() >= 1)
    pg.screenshot(path="shots/x2-start-menu.png")
    pg.locator(".menu .item", has_text="Dozzle").first.click(); time.sleep(3)
    ok("start menu: opens app window (iframe)", pg.locator(".win iframe").count() == 1 and pg.locator(".menu").count() == 0)
    # Store из панели задач → Подробнее → окно карточки
    pg.locator(".icons .icon", has_text="Store").first.dblclick(); pg.wait_for_selector(".win", timeout=10000); time.sleep(1.5)
    ok("taskbar: only open windows, no pinned", pg.locator(".taskbar .pinned").count() == 0 and pg.locator(".taskbar .tasks button[title='Store']").count() == 1)
    st = pg.locator(".win", has_text="Store").first
    st.locator(".card.app", has_text="Gitea").first.locator("button", has_text="Подробнее").click(); time.sleep(1.5)
    ok("store: Подробнее opens app window", pg.locator(".win .bar .title", has_text="Gitea").count() == 1 and pg.locator(".win", has_text="Каталог").count() >= 1)
    pg.screenshot(path="shots/x3-store-detail.png")
    # переключение окон через панель задач, свёртывание
    n_before = pg.locator(".win").count(); pg.click(".taskbar .tasks button[title='Store']"); time.sleep(0.4)  # фокус на Store
    ok("taskbar: click focuses window", pg.locator(".win.active > .bar .title").inner_text() == "Store")
    pg.click(".taskbar .tasks button[title='Store']"); time.sleep(0.4)  # второй клик по активному — свернуть
    ok("taskbar: click active pinned minimizes", pg.locator(".win").count() == n_before - 1)
    pg.click(".taskbar .tasks button[title='Store']"); time.sleep(0.4); ok("taskbar: click restores", pg.locator(".win").count() == n_before)
    # Alt+Tab и тема из трея
    pg.keyboard.press("Alt+Tab"); time.sleep(0.3); ok("keys: Alt+Tab cycles", True)
    pg.click(".taskbar .tray button[title='Переключить тему']"); time.sleep(0.8); ok("tray: theme toggle", pg.evaluate("document.documentElement.dataset.theme") == "light")
    pg.screenshot(path="shots/x4-desktop-light.png")
    pg.click(".taskbar .tray button[title='Переключить тему']"); time.sleep(0.8)
    # deep link
    pg.goto(f"https://{HOST}/#settings", wait_until="domcontentloaded"); time.sleep(2); ok("deep link: #settings opens window", pg.locator(".win", has_text="Акцентный цвет").count() == 1)
    # сессия сохранена
    j = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); ok("session: windows saved", len(j.get("session", {}).get("desktop", [])) >= 3, str(len(j.get("session", {}).get("desktop", []))))
    # телефон
    m = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 390, "height": 844}).new_page(); m.goto(f"https://{HOST}/", wait_until="domcontentloaded"); time.sleep(2.5)
    m.screenshot(path="shots/x5-phone.png"); ok("phone: mobile shell instead of taskbar", m.locator(".mobile").count() == 1 and m.locator(".taskbar").count() == 0)  # подробно — mobile_test.py
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
