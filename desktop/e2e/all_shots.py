import os, time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True)
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1280, "height": 800})
    pg = ctx.new_page()
    for tab, sel, name in [("projects", "text=capabilities", "s1-projects"), ("store", "text=Store", "s2-store"), ("system", "text=komodo", "s3-system")]:
        pg.goto(f"https://{HOST}/#{tab}", wait_until="networkidle"); pg.wait_for_selector(sel, timeout=15000); time.sleep(1.5)
        pg.screenshot(path=f"shots/{name}.png"); print("ok", name)
    m = ctx.new_page(); m.set_viewport_size({"width": 390, "height": 844}); m.goto(f"https://{HOST}/#projects", wait_until="networkidle"); time.sleep(1.5); m.screenshot(path="shots/s4-mobile.png", full_page=True); print("ok s4-mobile")
    b.close()
