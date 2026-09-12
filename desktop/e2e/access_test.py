# e2e доступа (docs/userflow-apps.md, P0). Браузер работает на самом стенде: при создании/удалении docker-сети приложения
# Chromium обрывает текущие запросы (ERR_NETWORK_CHANGED) — поэтому Desktop повторяет install/remove и ждёт статус опросом.
import os, time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); APP = "linkding-coolify"; SECOND = os.environ.get("SECOND_APP_NAME", "Hermes Agent")  # второе установленное приложение с генерируемым входом для проверки поповера
os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}, permissions=["clipboard-read", "clipboard-write"])
    ctx.add_init_script("localStorage.setItem('cloudos.firstrun', '1'); localStorage.setItem('cloudos.certwarn.until', '9999999999999')"); pg = ctx.new_page()
    s0 = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); s0.pop("session", None); pg.request.put(f"https://{HOST}/api/settings/desktop", data=s0)
    a = pg.request.get(f"https://{HOST}/api/apps/{APP}").json()
    if a["installed"]: pg.request.delete(f"https://{HOST}/api/apps/{APP}?purge=true"); time.sleep(3)
    ok("api: single app view has login", a["manifest"]["login"]["mode"] == "generated" and a["manifest"]["login"]["user"] == "SERVICE_USER_LINKDING", str(a["manifest"]["login"]))
    # Store: «Что понадобится»
    pg.goto(f"https://{HOST}/#store", wait_until="domcontentloaded"); pg.wait_for_selector(".card.app", timeout=30000); time.sleep(1.5)
    pg.fill("input.search", "linkding"); time.sleep(0.8)
    card = pg.locator(".card.app", has_text="Coolify").filter(has_text="Linkding").first
    ok("store: needs line", "логин и пароль создаст AgentVerse OS" in (card.locator(".needs").inner_text() if card.locator(".needs").count() else ""), card.locator(".needs").inner_text() if card.locator(".needs").count() else "нет .needs")
    pg.fill("input.search", "jellyfin"); time.sleep(0.8)
    g = pg.locator(".card.app", has_text="Runtipi").filter(has_text="Jellyfin").first
    ok("store: needs line for wizard app", "создадите при первом входе" in (g.locator(".needs").inner_text() if g.locator(".needs").count() else ""))
    pg.fill("input.search", "linkding"); time.sleep(0.8)
    pg.screenshot(path="shots/ac-store-needs.png")
    # установка → окно «Готово»
    t0 = time.time(); card.locator("button", has_text="Установить").click()
    ready = pg.locator(".win", has_text="Данные для входа").filter(has_text="установлено")
    try: ready.first.wait_for(timeout=420000); ok("install: «Готово» window", True, f"{time.time() - t0:.0f}s")
    except Exception as e: ok("install: «Готово» window", False, str(e)[:100])
    pg.screenshot(path="shots/ac-ready.png")
    pw = pg.request.get(f"https://{HOST}/api/apps/{APP}/settings/SERVICE_PASSWORD_LINKDING/reveal").json()["value"]
    fld = ready.first.locator(".field[data-field=пароль]")
    ok("ready: login + password rows", ready.first.locator(".field[data-field=логин]").count() == 1 and fld.count() == 1)
    fld.locator("button[title=показать]").click(); time.sleep(1.2)
    ok("ready: reveal shows the real password", fld.locator("code").inner_text() == pw and len(pw) == 24)
    ready.first.locator("button", has_text="Скопировать всё текстом").click(); time.sleep(0.8)
    try: clip = pg.evaluate("navigator.clipboard.readText()")
    except Exception: clip = ""
    ok("ready: copy all puts login+password to clipboard", ("пароль: " + pw) in clip and "логин: " in clip, clip.replace(pw, "***")[:80].replace("\n", " | "))
    pg.screenshot(path="shots/ac-ready-revealed.png")
    # Открыть → окно приложения с 🔑, поповер раскрыт сам при первом открытии
    ready.first.locator("button.primary", has_text="Открыть").click(); time.sleep(2.5)
    appwin = pg.locator(".win", has=pg.locator("iframe[src*='aios.tail0fe52f.ts.net']")).filter(has_text="Linkding").first
    ok("app window: 🔑 button in title bar", appwin.locator(".bar button.key").count() == 1)
    ok("app window: access popover auto-opened on first open", appwin.locator(".keypop .field[data-field=пароль]").count() == 1)
    pg.screenshot(path="shots/ac-app-key.png")
    appwin.locator(".keypop button[title=закрыть]").click(); time.sleep(0.3); ok("app window: popover closes", appwin.locator(".keypop").count() == 0)
    appwin.locator(".bar button.key").click(); time.sleep(0.5); ok("app window: 🔑 reopens popover", appwin.locator(".keypop").count() == 1)
    appwin.locator(".keypop .field[data-field=пароль] button[title=показать]").click(); time.sleep(1.0)
    ok("app window: popover reveals password", appwin.locator(".keypop .field[data-field=пароль] code").inner_text() == pw)
    # второе приложение с генерируемым входом из иконки на столе: поповер сам открывается один раз, второй раз — нет
    for _ in range(12):
        if not pg.locator(".win").count(): break
        try: pg.locator(".win .bar button.close").last.click(timeout=2000)
        except Exception: pass
        time.sleep(0.3)
    ok("windows closed before desktop test", pg.locator(".win").count() == 0, str(pg.locator(".win").count()))
    pg.locator(".icons .icon", has_text=SECOND).first.dblclick(); time.sleep(2.5)
    hw = pg.locator(".win", has_text=SECOND).first
    ok("second app: popover auto-opened once", hw.locator(".keypop").count() == 1)
    hw.locator(".bar button.close").click(); time.sleep(0.5)
    pg.locator(".icons .icon", has_text=SECOND).first.dblclick(); time.sleep(2.0)
    hw = pg.locator(".win", has_text=SECOND).first
    ok("second app: no auto popover on second open", hw.locator(".keypop").count() == 0 and hw.locator(".bar button.key").count() == 1)
    hw.locator(".bar button.close").click(); time.sleep(0.3)
    # карточка: блок доступа, удаление полностью с подтверждением словом delete
    pg.goto(f"https://{HOST}/#app={APP}", wait_until="domcontentloaded"); time.sleep(3)
    det = pg.locator(".win", has_text="Описание").last
    ok("card: access box", det.locator(".accessbox .field[data-field=пароль]").count() == 1)
    det.locator("button.danger", has_text="Удалить").click(); time.sleep(0.5)
    dlg = pg.locator(".dlg")
    ok("remove dialog: two options", dlg.locator("input[type=radio]").count() == 2 and "Оставить данные" in dlg.inner_text())
    dlg.locator("label", has_text="Удалить полностью").click(); time.sleep(0.3)
    ok("remove dialog: purge needs the word delete", dlg.locator("button.danger").is_disabled())
    dlg.locator("input.confirm").fill(APP); time.sleep(0.2); ok("remove dialog: app name is not enough", dlg.locator("button.danger").is_disabled())
    dlg.locator("input.confirm").fill("Delete"); time.sleep(0.2); ok("remove dialog: enabled after delete", not dlg.locator("button.danger").is_disabled())
    pg.screenshot(path="shots/ac-remove.png")
    dlg.locator("button.danger").click()
    for _ in range(60):
        time.sleep(2)
        if not pg.request.get(f"https://{HOST}/api/apps/{APP}").json()["installed"]: break
    a2 = pg.request.get(f"https://{HOST}/api/apps/{APP}").json(); ok("purge: app not installed", not a2["installed"])
    pw2 = pg.request.get(f"https://{HOST}/api/apps/{APP}/settings/SERVICE_PASSWORD_LINKDING/reveal").json()["value"]
    ok("purge: secret deleted", pw2 == "", repr(pw2)[:10])
    ev = pg.request.get(f"https://{HOST}/api/events").json(); ok("purge: event recorded", any("удалено полностью" in e.get("message", "") for e in ev), str(len(ev)))
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
