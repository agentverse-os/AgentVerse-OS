import os, time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True)
with sync_playwright() as p:
    b = p.chromium.launch(); pg = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1280, "height": 900}).new_page()
    pg.goto(f"https://{HOST}/#store", wait_until="networkidle"); pg.wait_for_selector("text=Store", timeout=15000); time.sleep(2)
    pg.screenshot(path="shots/d4-store-catalog.png")
    pg.fill("input[placeholder='поиск…']", "git"); time.sleep(1); pg.screenshot(path="shots/d5-store-search.png")
    body = pg.inner_text("body"); print("PASS store catalog" if "приложений" in body and "Gitea" in body else "FAIL store", body.split("\n")[1][:80])
    b.close()
