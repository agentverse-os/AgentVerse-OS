# e2e UX Store: индикаторы установки (уведомление с этапами, прогресс-бар и спиннер на карточке), уведомление «готово» с действием,
# удаление через диалог с этапом и уведомлением, живой статус на карточке.
import os, time, json, ssl, urllib.request
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); APP = "linkding-coolify"; os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET"): r = urllib.request.urlopen(urllib.request.Request(f"https://{HOST}/api{path}", method=method), context=ctx_ssl); t = r.read(); return json.loads(t) if t else None
if api(f"/apps/{APP}")["installed"]:
    api(f"/apps/{APP}?purge=true", "DELETE")
    for _ in range(60):
        if not api(f"/apps/{APP}")["installed"]: break
        time.sleep(2)
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); ctx.add_init_script("localStorage.setItem('cloudos.firstrun', '1'); localStorage.setItem('cloudos.certwarn.until', '9999999999999')"); pg = ctx.new_page()
    pg.goto(f"https://{HOST}/#store", wait_until="domcontentloaded")
    sk = pg.locator(".card.app.sk").count(); pg.wait_for_selector(".card.app:not(.sk)", timeout=30000); time.sleep(1)
    ok("store: header shows running summary", "работают" in pg.locator(".win", has_text="Store").first.inner_text())
    ok("store: live status badges with dot", pg.locator(".card.app .badge.st.ok").count() >= 3, f"skeleton at start: {sk}")
    pg.fill("input.search", "linkding"); time.sleep(0.8)
    card = pg.locator(".card.app", has_text="Coolify").filter(has_text="Linkding").first
    card.locator("button", has_text="Установить").click(); time.sleep(1.2)
    ok("install: progress toast", pg.locator(".toast.progress", has_text="Устанавливаю Linkding").count() == 1)
    ok("install: card progress bar + spinner", card.locator(".progress").count() == 1 and card.locator("button .spin").count() == 1)
    pg.screenshot(path="shots/ux-installing.png")
    okt = pg.locator(".toast.ok", has_text="Устанавливаю Linkding")
    try: okt.wait_for(timeout=420000); ok("install: ok toast with action", okt.locator("button.act", has_text="Открыть").count() == 1)
    except Exception as e: ok("install: ok toast with action", False, str(e)[:80])
    time.sleep(1.5)
    ok("install: card shows running status", card.locator(".badge.st").count() == 1 and card.locator(".badge.st").inner_text().strip() == "работает", card.locator(".badge.st").inner_text() if card.locator(".badge.st").count() else "нет .badge.st")
    pg.screenshot(path="shots/ux-installed.png")
    okt.locator("button.act").click(); time.sleep(2)
    ok("toast action opens app window", pg.locator(".win iframe[src*='8']").count() >= 1)
    # закрыть всё, кроме Store (окно «Готово» и окно приложения перекрывают карточку)
    for _ in range(6):
        others = pg.locator(".win").filter(has_not=pg.locator(".bar .title", has_text="Store"))
        if not others.count(): break
        try: others.first.locator(".bar button.close").click(timeout=3000)
        except Exception: break
        time.sleep(0.3)
    ok("windows closed except Store", pg.locator(".win").count() == 1, str(pg.locator(".win").count()))
    # удаление с этапами
    card.locator("button.danger", has_text="Удалить").click(); time.sleep(0.5)
    dlg = pg.locator(".dlg"); dlg.locator("label", has_text="Удалить полностью").click(); dlg.locator("input.confirm").fill("delete"); dlg.locator("button.danger").click(); time.sleep(1.0)
    ok("remove: dialog spinner and progress toast", dlg.locator("button.danger .spin").count() == 1 and pg.locator(".toast.progress", has_text="Удаляю Linkding").count() == 1)
    rt = pg.locator(".toast.ok", has_text="Удаляю Linkding")
    try: rt.wait_for(timeout=240000); ok("remove: ok toast", "стёрты" in rt.inner_text())
    except Exception as e: ok("remove: ok toast", False, str(e)[:80])
    time.sleep(1.5); ok("remove: card back to «Установить»", card.locator("button", has_text="Установить").count() == 1 and not api(f"/apps/{APP}")["installed"])
    pg.screenshot(path="shots/ux-removed.png")
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
