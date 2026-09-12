import time
from playwright.sync_api import sync_playwright
with sync_playwright() as p:
    b = p.chromium.launch(); pg = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 390, "height": 844}, has_touch=True, is_mobile=True).new_page()
    pg.on("pageerror", lambda e: print("pageerror:", str(e)[:200]))
    pg.goto("https://aios.tail0fe52f.ts.net/", wait_until="domcontentloaded"); pg.wait_for_selector(".mobile .grid .tile", timeout=30000)
    pg.evaluate("localStorage.setItem('cloudos.widgets.phone', '[]'); localStorage.setItem('cloudos.widgets.phone.cleared', '1')"); pg.reload(wait_until="domcontentloaded"); pg.wait_for_selector(".wlayer .empty", timeout=30000); time.sleep(1)
    pg.locator(".wlayer .empty").click(); time.sleep(0.8)
    print("picker count:", pg.locator(".picker").count(), "items:", pg.locator(".picker .pitem").count())
    if pg.locator(".picker").count():
        bb = pg.locator(".picker").bounding_box(); print("picker box:", bb)
        it = pg.locator(".picker .pitem", has_text="Часы"); print("item box:", it.bounding_box())
        print("elementFromPoint:", pg.evaluate(f"(() => {{ const b = {it.bounding_box()}; const e = document.elementFromPoint(b['x']+10, b['y']+10); return e ? e.tagName + '.' + e.className : null; }})()"))
    pg.screenshot(path="shots/dbg-picker.png")
    b.close()
