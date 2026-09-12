import time
from playwright.sync_api import sync_playwright
with sync_playwright() as p:
    b = p.chromium.launch(); pg = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1366, "height": 860}).new_page()
    s0 = pg.request.get("https://aios.tail0fe52f.ts.net/api/settings/desktop").json(); s0.pop("session", None); pg.request.put("https://aios.tail0fe52f.ts.net/api/settings/desktop", data=s0)
    pg.goto("https://aios.tail0fe52f.ts.net/#store", wait_until="domcontentloaded"); pg.wait_for_selector("text=Dozzle", timeout=20000); time.sleep(1)
    pg.locator(".card.app", has_text="Dozzle").first.locator("button.primary", has_text="Открыть").click(); time.sleep(3)
    print("wins:", pg.locator(".win").count())
    nav = pg.locator("header nav button", has_text="Store").first; box = nav.bounding_box(); print("nav Store box:", box)
    print("elementFromPoint:", pg.evaluate(f"(() => {{ const e = document.elementFromPoint({box['x']+5}, {box['y']+5}); return e ? e.tagName + '.' + e.className + ' ' + (e.textContent||'').slice(0,20) : null; }})()"))
    print("header z:", pg.evaluate("getComputedStyle(document.querySelector('header.top')).zIndex"), "win z:", pg.evaluate("getComputedStyle(document.querySelector('.win')).zIndex"))
    print("win rect:", pg.evaluate("JSON.stringify(document.querySelector('.win').getBoundingClientRect())"))
    pg.screenshot(path="shots/dbg-window.png")
    try:
        nav.click(timeout=5000); print("click OK")
    except Exception as e: print("click failed:", str(e)[:200])
    b.close()
