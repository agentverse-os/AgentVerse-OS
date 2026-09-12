import time
from playwright.sync_api import sync_playwright
with sync_playwright() as p:
    b = p.chromium.launch(); pg = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1366, "height": 860}).new_page()
    pg.on("console", lambda m: print("console:", m.type, m.text[:300]) if m.type in ("error", "warning") else None)
    pg.on("pageerror", lambda e: print("pageerror:", str(e)[:300]))
    pg.goto("https://aios.tail0fe52f.ts.net/#store", wait_until="networkidle"); pg.wait_for_selector("text=Dozzle", timeout=20000)
    card = pg.locator(".card.app", has_text="Dozzle").first
    print("buttons:", card.locator("button").all_inner_texts())
    card.locator("button.primary").first.click(); time.sleep(3)
    print("wins:", pg.locator(".win").count(), "dock:", pg.locator(".dock").count())
    print("html has Windows?:", pg.evaluate("document.querySelector('main')?.previousElementSibling?.tagName"))
    b.close()
