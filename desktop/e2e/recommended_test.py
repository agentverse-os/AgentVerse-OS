# e2e списка «Рекомендуем»: полка в Store, фильтр по источнику «Рекомендуем», плашка в карточках, причина в карточке приложения.
import os, time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    b = p.chromium.launch()
    for label, vp in [("desktop", {"width": 1440, "height": 900}), ("phone", {"width": 390, "height": 844})]:
        ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport=vp, has_touch=(label == "phone")); ctx.add_init_script("localStorage.setItem('cloudos.firstrun', '1'); localStorage.setItem('cloudos.certwarn.until', '9999999999999')"); pg = ctx.new_page()
        s0 = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); s0.pop("session", None); pg.request.put(f"https://{HOST}/api/settings/desktop", data=s0)
        apps = pg.request.get(f"https://{HOST}/api/apps").json(); rec = [a for a in apps if a["manifest"].get("recommended")]
        if label == "desktop": ok("api: recommended merged into manifests", len(rec) >= 10 and rec[0]["manifest"]["recommended"]["reason"], f"{len(rec)} шт")
        pg.goto(f"https://{HOST}/#store", wait_until="domcontentloaded"); pg.wait_for_selector(".card.app", timeout=30000); time.sleep(1.5)
        picks = pg.locator(".picks .pick")
        ok(f"{label}: shelf «Рекомендуем» rendered", picks.count() >= 10, f"{picks.count()} карточек")
        first = min(rec, key=lambda a: a["manifest"]["recommended"]["rank"]); ft = first["manifest"].get("title") or first["name"]
        ok(f"{label}: {ft} is the first pick", ft in picks.first.inner_text())
        pg.screenshot(path=f"shots/rc-{label}-shelf.png")
        ok(f"{label}: ★ badge on grid cards", pg.locator(".card.app .badge.pickb").count() >= 1)
        pg.fill("input.search", first["name"]); time.sleep(0.8)
        ok(f"{label}: shelf hidden while searching", pg.locator(".picks").count() == 0)
        card = pg.locator(".card.app", has_text=ft).first
        inst = first["installed"]
        ok(f"{label}: first pick card state", card.locator("button", has_text="Открыть").count() == 1 if inst else card.locator(".needs").count() == 1, "установлен" if inst else "не установлен")
        card.locator("button", has_text="Подробнее").click(); time.sleep(2.5)
        det = pg.locator(".win", has_text="Описание").last
        rr = first["manifest"]["recommended"]; reason = ((rr.get("i18n") or {}).get("ru") or {}).get("reason") or rr["reason"]
        ok(f"{label}: detail shows reason", det.locator(".badge.pickb").count() == 1 and reason[:24] in det.inner_text())
        pg.screenshot(path=f"shots/rc-{label}-detail.png")
        det.locator(".bar button.close").first.click(); time.sleep(0.3)
        pg.fill("input.search", "vaultwarden"); time.sleep(0.8)
        vw = pg.locator(".card.app", has_text="Runtipi").filter(has_text="Vaultwarden").first
        ok(f"{label}: needs line for a wizard-type pick", "создадите при первом входе" in (vw.locator(".needs").inner_text() if vw.locator(".needs").count() else ""))
        pg.fill("input.search", ""); time.sleep(0.5)
        # фильтр «Рекомендуем» в выпадающем списке источников
        dd = pg.locator(".filters .dd").first
        dd.locator("button.dd-btn").click(); time.sleep(0.3)
        pg.locator(".dd-list li[role=option]", has_text="Рекомендуем").first.click(); time.sleep(0.8)
        n = pg.locator(".card.app").count(); ok(f"{label}: filter shows only recommended", n == len(rec) and pg.locator(".card.app .badge.pickb").count() == n, f"{n} карточек")
        ctx.close()
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
