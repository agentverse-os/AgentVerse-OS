import time, json
from playwright.sync_api import sync_playwright
with sync_playwright() as p:
    b = p.chromium.launch(); pg = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 390, "height": 844}, has_touch=True, is_mobile=True).new_page()
    pg.goto("https://aios.tail0fe52f.ts.net/", wait_until="domcontentloaded"); pg.wait_for_selector(".mobile .grid .tile", timeout=30000); time.sleep(1)
    pg.locator(".mobile .search").click(); pg.wait_for_selector(".menu", timeout=10000); time.sleep(0.5)
    print(pg.evaluate("""(() => {
      const cs = (el) => { const s = getComputedStyle(el); return { tag: el.tagName + '.' + [...el.classList].slice(0,2).join('.'), pos: s.position, z: s.zIndex, bf: s.backdropFilter, tr: s.transform, pe: s.pointerEvents }; };
      const menu = document.querySelector('.menu'); const w = document.querySelector('.mobile .widget');
      const chain = (el) => { const out = []; while (el && el !== document.body) { out.push(cs(el)); el = el.parentElement; } return out; };
      return JSON.stringify({ menu: chain(menu), widget: chain(w) }, null, 0);
    })()"""))
    b.close()
