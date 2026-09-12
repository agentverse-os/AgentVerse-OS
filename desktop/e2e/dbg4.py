import time
from playwright.sync_api import sync_playwright
with sync_playwright() as p:
    for engine in ("chromium", "firefox"):
        b = getattr(p, engine).launch(); kw = {"ignore_https_errors": True, "viewport": {"width": 390, "height": 844}}
        if engine == "chromium": kw.update(has_touch=True, is_mobile=True)
        pg = b.new_context(locale="ru-RU", **kw).new_page()
        pg.on("pageerror", lambda e: print(engine, "pageerror:", str(e)[:200]))
        pg.goto("https://aios.tail0fe52f.ts.net/", wait_until="domcontentloaded"); pg.wait_for_selector(".mobile .grid .tile", timeout=30000); time.sleep(1)
        pg.locator(".mobile .search").click(); pg.wait_for_selector(".menu", timeout=10000); time.sleep(0.5)
        btn = pg.locator(".menu button", has_text="Перезагрузить оболочку"); bb = btn.bounding_box(); print(engine, "button box:", bb)
        top = pg.evaluate(f"(() => {{ const e = document.elementFromPoint({bb['x']+20}, {bb['y']+10}); return e ? e.tagName + '.' + e.className + ' | ' + (e.textContent||'').trim().slice(0,30) : null; }})()")
        print(engine, "elementFromPoint:", top)
        pg.evaluate("window.__marker = 1")
        try:
            with pg.expect_navigation(timeout=8000): btn.click()
            print(engine, "reload: navigation happened; marker after:", pg.evaluate("window.__marker"))
        except Exception as e:
            print(engine, "reload: NO navigation", str(e)[:80], "| marker:", pg.evaluate("window.__marker"), "| menu still open:", pg.locator(".menu").count())
        b.close()
