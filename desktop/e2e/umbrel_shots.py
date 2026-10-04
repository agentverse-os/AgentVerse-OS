import os, time, json
from playwright.sync_api import sync_playwright
# Тема «Umbrel»: переключение в Настройках, док и сетка на рабочем столе, окна-листы, обратное переключение. Скриншоты в shots/.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append((n, c)); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); pg = ctx.new_page()
    pg.goto(f"https://{HOST}/#settings", wait_until="networkidle"); pg.wait_for_selector("text=Акцентный цвет", timeout=20000); time.sleep(1)
    before = pg.evaluate("JSON.parse(localStorage.getItem('cloudos.appearance.v1')||'{}')")
    wsnap = pg.request.get(f"https://{HOST}/api/settings/desktop").json().get('widgets') or {}
    # виджеты пользователя сохранены под его экран (могут быть за краем 1440 px) — на время прогона подсаживаем набор под вьюпорт
    seed = [{"id": "w-clock", "type": "clock", "x": 1000, "y": 24, "w": 260, "h": 140, "config": {"tz": "", "seconds": False}},
            {"id": "w-monitor", "type": "monitor", "x": 1000, "y": 184, "w": 420, "h": 400, "config": {"show": "all"}},
            {"id": "w-news", "type": "news", "x": 1000, "y": 604, "w": 420, "h": 200, "config": {"interval": 15, "images": True, "compact": False, "preset": "ai", "count": 10, "feeds": [], "lang": "both"}}]
    pg.request.put(f"https://{HOST}/api/settings/desktop", data=json.dumps({"widgets": {**wsnap, "desktop": seed}}), headers={"content-type": "application/json"})
    pg.reload(wait_until="networkidle"); pg.wait_for_selector("text=Акцентный цвет", timeout=20000); time.sleep(1)
    pg.click("button.thm-card:has-text('Umbrel')"); time.sleep(1.5)
    ok("umbrel: look applied", pg.evaluate("document.documentElement.dataset.look") == "umbrel")
    ok("umbrel: accent from wallpaper", pg.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--accent').trim().toUpperCase()") == "#F2690D")
    pg.screenshot(path="shots/u1-settings.png")
    # обои темы меняют акцент
    pg.click("button.bg:has-text('Umbrel · сияние')"); time.sleep(1)
    ok("umbrel: wallpaper switch sets accent", pg.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--accent').trim().toUpperCase()") == "#702EFF")
    pg.click("button.bg:has-text('Umbrel · закат')"); time.sleep(1)
    # рабочий стол: док и сетка
    pg.evaluate("document.querySelector('.win .ctl .close')?.click()"); time.sleep(1)
    ok("umbrel: dock present", pg.locator("nav.dock").count() == 1 and pg.locator("nav.dock .item").count() >= 6)
    ok("umbrel: greeting header", pg.locator(".icons.umbrel .hdr h1").count() == 1)
    ok("umbrel: no classic taskbar", pg.locator("nav.taskbar").count() == 0)
    pg.screenshot(path="shots/u2-desktop.png")
    pg.hover("nav.dock .slot:nth-child(4) .item"); time.sleep(.6); pg.screenshot(path="shots/u3-dock-hover.png")
    pg.click("nav.dock .slot:nth-child(4) .item"); time.sleep(1.5)  # Store
    ok("umbrel: window opened from dock", pg.locator(".win").count() >= 1)
    pg.screenshot(path="shots/u4-store-window.png")
    pg.dblclick(".win .bar .title"); time.sleep(.8); pg.screenshot(path="shots/u5-store-max.png")
    pg.dblclick(".win .bar .title"); time.sleep(.5)
    pg.click("nav.dock .slot .item:has(svg)", timeout=3000) if False else None
    # виджеты — свободный слой и в этом облике: перетаскивание за заголовок сдвигает виджет (окно Store закрыть, чтобы не мешало жесту)
    pg.evaluate("document.querySelector('.win.active .ctl .close')?.click()"); time.sleep(.6)
    ws = pg.locator('.wlayer .widget')
    if ws.count() >= 1:
        t0 = ws.first.get_attribute('style'); hb = ws.first.locator('.whead').bounding_box()
        pg.mouse.move(hb['x'] + 30, hb['y'] + 10); pg.mouse.down(); pg.mouse.move(hb['x'] - 150, hb['y'] + 60, steps=10); pg.mouse.up(); time.sleep(.6)
        ok("umbrel: widget drag moves it", ws.first.get_attribute('style') != t0); pg.screenshot(path="shots/u12-widget-moved.png")
    # окно «Проекты» из дока: чипы и кнопки карточек не должны съезжать
    pg.click("nav.dock .slot:nth-child(3) .item"); time.sleep(1.5); pg.screenshot(path="shots/u9-projects.png")
    pg.evaluate("document.querySelector('.win.active .ctl .close')?.click()"); time.sleep(.6)
    # планшетный вьюпорт: рабочий стол с виджетами и окно
    pt = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1000, "height": 1300}).new_page()
    pt.goto(f"https://{HOST}/", wait_until="networkidle"); time.sleep(2); pt.screenshot(path="shots/u10-tablet-desktop.png")
    pt.click("nav.dock .slot:nth-child(3) .item"); time.sleep(1.5); pt.screenshot(path="shots/u11-tablet-projects.png"); pt.context.close()
    # меню запуска из дока
    pg.click("nav.dock .item.start"); time.sleep(.8); pg.screenshot(path="shots/u6-start-menu.png"); pg.keyboard.press("Escape")
    # заставка (тестовый хук)
    pg.evaluate("localStorage.setItem('cloudos.screensaver.testMs','1500')"); pg.reload(wait_until="networkidle"); time.sleep(3.5)
    ok("umbrel: screensaver uses wallpaper", pg.locator(".saver.umbrel").count() == 1); pg.screenshot(path="shots/u7-saver.png")
    pg.evaluate("localStorage.removeItem('cloudos.screensaver.testMs')"); pg.mouse.move(10, 10); time.sleep(.5)
    # сохранено в ядре и видно другому устройству
    r = pg.request.get(f"https://{HOST}/api/settings/desktop"); j = r.json()
    ok("umbrel: look saved in cloudd", j.get("appearance", {}).get("look") == "umbrel", json.dumps(j)[:100])
    ctx2 = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1280, "height": 800}); p2 = ctx2.new_page()
    p2.goto(f"https://{HOST}/", wait_until="networkidle"); time.sleep(1.5)
    ok("umbrel: roams to another device", p2.evaluate("document.documentElement.dataset.look") == "umbrel")
    # обратно: тема Monolith возвращает панель задач
    pg.goto(f"https://{HOST}/#settings", wait_until="networkidle"); pg.wait_for_selector("text=Акцентный цвет", timeout=20000); time.sleep(.8)
    pg.click("button.thm-card:has-text('Monolith')"); time.sleep(1.2)
    ok("umbrel: back to default look", pg.evaluate("document.documentElement.dataset.look") == "default" and pg.locator("nav.taskbar").count() == 1)
    pg.screenshot(path="shots/u8-back-monolith.png")
    # вернуть прежнее оформление пользователя
    pg.evaluate("(d) => fetch('/api/settings/desktop', {method: 'PUT', headers: {'content-type': 'application/json'}, body: JSON.stringify(d)})", {"appearance": before, "widgets": wsnap}); time.sleep(.5)
    b.close()
print("ALL PASS" if all(c for _, c in res) else "SOME FAIL")
