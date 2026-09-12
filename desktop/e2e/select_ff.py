import os
import time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net")
with sync_playwright() as p:
    for engine in ("firefox", "chromium"):
        b = getattr(p, engine).launch(); pg = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}).new_page()
        pg.on("pageerror", lambda e: print(engine, "pageerror:", str(e)[:200]))
        pg.goto(f"https://{HOST}/#files", wait_until="domcontentloaded"); pg.wait_for_selector(".files .bar select", timeout=30000); time.sleep(2)
        sel = pg.locator(".files .bar select")
        print(engine, "options:", sel.locator("option").count(), "value:", sel.input_value())
        # реальный путь пользователя: клик по select, стрелка вниз, Enter
        sel.click(); time.sleep(0.5); pg.keyboard.press("ArrowDown"); pg.keyboard.press("Enter"); time.sleep(1.5)
        print(engine, "после клавиатуры value:", sel.input_value(), "| crumbs root:", pg.locator(".files .crumbs button").first.inner_text())
        # программный выбор
        sel.select_option("ws:alpha"); time.sleep(2)
        print(engine, "после select_option value:", sel.input_value(), "entries:", pg.locator(".files .row-entry").count())
        b.close()
