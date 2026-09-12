import os, time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True)
with sync_playwright() as p:
    b = p.chromium.launch(); pg = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1280, "height": 760}).new_page()
    pg.goto(f"https://{HOST}/#projects", wait_until="networkidle"); pg.wait_for_selector("text=capabilities", timeout=15000); time.sleep(1.5)
    pg.screenshot(path="shots/d6-projects-caps.png")
    body = pg.inner_text("body"); print("PASS caps" if "storage.s3" in body and "notify" in body and "demo.http" in body else "FAIL caps")
    # переключаем notify у beta через галочку
    card = pg.locator(".card", has_text="beta").first
    cb = card.locator("label", has_text="notify").locator("input")
    before = cb.is_checked(); cb.click(); time.sleep(6); pg.reload(wait_until="networkidle"); time.sleep(1.5)
    card = pg.locator(".card", has_text="beta").first; after = card.locator("label", has_text="notify").locator("input").is_checked()
    print("PASS toggle notify beta" if before != after else "FAIL toggle", before, "→", after)
    pg.screenshot(path="shots/d7-projects-caps-toggled.png")
    b.close()
