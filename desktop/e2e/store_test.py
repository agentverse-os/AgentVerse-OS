import os, time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(c); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    b = p.chromium.launch()
    for label, vp in [("desktop", {"width": 1440, "height": 900}), ("tablet", {"width": 900, "height": 1200}), ("phone", {"width": 390, "height": 844})]:
        ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport=vp, has_touch=(label != "desktop")); pg = ctx.new_page()
        pg.add_init_script("localStorage.setItem('cloudos.certwarn.until', '9999999999999')")  # браузер тестов не доверяет сертификату — баннер не должен мешать кликам
        s0 = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); s0.pop("session", None); pg.request.put(f"https://{HOST}/api/settings/desktop", data=s0)
        pg.goto(f"https://{HOST}/#store", wait_until="domcontentloaded"); pg.wait_for_selector(".card.app", timeout=30000); time.sleep(2)
        n = pg.locator(".card.app").count(); ok(f"{label}: store cards", n >= 30, f"{n} на первой странице")
        ok(f"{label}: source badges", pg.locator(".badge.src").count() >= 30)
        pg.fill("input.search", "hermes"); time.sleep(0.8)
        srcs = set(pg.locator(".badge.src").all_inner_texts())
        ok(f"{label}: search hermes finds both sources", pg.locator(".card.app").count() >= 2 and {"Umbrel", "Coolify"} <= srcs, str(sorted(srcs)))  # плюс наш ручной hermes-agent
        pg.screenshot(path=f"shots/st-{label}-search.png")
        pg.locator(".card.app", has_text="Umbrel").first.locator("button", has_text="Подробнее").click(); time.sleep(3)
        det = pg.locator(".win", has_text="Описание").last
        ok(f"{label}: detail window with gallery", det.locator(".gallery .shot").count() >= 1, f"{det.locator('.gallery .shot').count()} скриншотов")
        ok(f"{label}: readme rendered", det.locator(".readme").count() == 1)
        pg.screenshot(path=f"shots/st-{label}-detail.png")
        det.locator(".gallery .shot").first.click(); time.sleep(1.5); ok(f"{label}: lightbox", pg.locator(".lightbox img").count() == 1); pg.keyboard.press("Escape"); time.sleep(0.3)
        det.locator(".tabs button", has_text="Техническое").click(); time.sleep(0.3); ok(f"{label}: tech tab", det.locator("text=Endpoint").count() == 1)
        ctx.close()
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
