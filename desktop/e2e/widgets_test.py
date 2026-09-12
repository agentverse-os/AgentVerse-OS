import os, time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(c); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    b = p.chromium.launch()
    for label, vp in [("desktop", {"width": 1600, "height": 900}), ("phone", {"width": 390, "height": 844})]:
        ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport=vp, has_touch=(label == "phone")); pg = ctx.new_page()
        pg.add_init_script("localStorage.setItem('cloudos.certwarn.until', '9999999999999')")  # браузер тестов не доверяет сертификату — баннер не должен мешать кликам
        s0 = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); s0.pop("session", None); s0.pop("widgets", None); pg.request.put(f"https://{HOST}/api/settings/desktop", data=s0)
        pg.goto(f"https://{HOST}/", wait_until="domcontentloaded"); pg.wait_for_selector(".widget", timeout=30000); time.sleep(6)
        n = pg.locator(".widget").count(); ok(f"{label}: default widgets", n >= 2, f"{n}")
        mon = pg.locator(".widget", has_text="Монитор системы").first
        ok(f"{label}: monitor shows CPU and RAM", "CPU" in mon.inner_text() and "RAM" in mon.inner_text() and mon.locator("svg polyline").count() == 2)
        if label == "desktop":
            news = pg.locator(".widget", has_text="Новости").first
            ok("desktop: news items loaded", news.locator(".news .item .title").count() >= 5, f"{news.locator('.news .item .title').count()} новостей")
            ok("desktop: news source chips", news.locator(".chips .chip").count() >= 2)
            news.locator(".news .item .title").first.click(); time.sleep(2.5)
            rd = pg.locator(".win", has_text="Читалка").first if pg.locator(".win .bar .title", has_text="Читалка").count() else pg.locator(".win .reader").first.locator("xpath=ancestor::*[contains(@class,'win')]").first
            ok("desktop: reader window opens article", pg.locator(".win .reader h2").count() == 1 and pg.locator(".win .reader footer a[target=_blank]").count() == 1)
            pg.screenshot(path="shots/wg-reader.png"); pg.locator(".win", has=pg.locator(".reader")).first.locator(".bar button.close").click(); time.sleep(0.4)
            # добавить виджет через правый клик по столу
            while pg.locator(".win .bar button[title='Свернуть']").count(): pg.locator(".win .bar button[title='Свернуть']").first.click(); time.sleep(0.2)
            pg.mouse.click(60, 780, button="right"); time.sleep(0.4); ok("desktop: context menu", pg.locator(".ctx").count() == 1)
            pg.locator(".ctx button", has_text="Добавить виджет").click(); time.sleep(0.4); ok("desktop: picker", pg.locator(".picker .pitem").count() == 9)
            pg.locator(".picker .pitem", has_text="Погода").click(); time.sleep(0.5)
            w = pg.locator(".widget", has_text="Погода").first; w.locator("input").fill("Kyiv"); w.locator("button", has_text="Найти").click(); time.sleep(3)
            w.locator("button.pick").first.click(); time.sleep(4)
            ok("desktop: weather shows temperature", "°" in w.inner_text() and "Киев" in w.inner_text(), w.inner_text()[:60].replace("\n", " "))
            # перетаскивание виджета
            head = w.locator(".whead"); box = head.bounding_box()
            pg.mouse.move(box["x"] + 40, box["y"] + 8); pg.mouse.down(); pg.mouse.move(box["x"] - 260, box["y"] + 108, steps=10); pg.mouse.up(); time.sleep(0.6)
            box2 = head.bounding_box(); ok("desktop: widget dragged", abs((box2["x"] - box["x"]) + 300) < 8 and abs((box2["y"] - box["y"]) - 100) < 8, f"{box['x']:.0f},{box['y']:.0f} → {box2['x']:.0f},{box2['y']:.0f}")
            pg.screenshot(path="shots/wg-desktop.png")
            # удалить часы; проверить сохранение в ядре
            clock = pg.locator(".widget", has_text="Часы").first; clock.hover(); clock.locator(".whead button[title='убрать виджет']").click(); time.sleep(1.5)
            j = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); types = [x["type"] for x in j.get("widgets", {}).get("desktop", [])]
            ok("desktop: layout saved in cloudd", "clock" not in types and "weather" in types, str(types))
        else:
            pg.screenshot(path="shots/wg-phone.png", full_page=False)
            ok("phone: widgets stacked below icons", pg.locator(".wlayer.phone").count() == 1)
        ctx.close()
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
