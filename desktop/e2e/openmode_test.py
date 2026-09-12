import os, time, threading, urllib.request
from playwright.sync_api import sync_playwright
# Приложение для проверки — установленное из источника Coolify с генерируемым паролем (раньше Hermes; на стенде теперь Linkding).
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); APP = os.environ.get("OPEN_APP", "linkding-coolify"); NAME = os.environ.get("OPEN_APP_NAME", "Linkding"); PWKEY = os.environ.get("OPEN_APP_PWKEY", "SERVICE_PASSWORD_LINKDING"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(c); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); ctx.add_init_script("localStorage.setItem('cloudos.firstrun', '1'); localStorage.setItem('cloudos.certwarn.until', '9999999999999')"); pg = ctx.new_page()
    s0 = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); s0.pop("session", None); pg.request.put(f"https://{HOST}/api/settings/desktop", data=s0)
    a0 = pg.request.get(f"https://{HOST}/api/apps/{APP}").json()
    if not a0["installed"]:  # access_test удаляет Linkding полностью — ставим заново через API (запрос долгий, в фоне) и ждём «работает»
        def _inst():
            try: urllib.request.urlopen(urllib.request.Request(f"https://{HOST}/api/apps/{APP}/install", method="POST"), timeout=600)
            except Exception as e: print("install request:", str(e)[:80])
        threading.Thread(target=_inst, daemon=True).start()
        for _ in range(150):
            time.sleep(3); a0 = pg.request.get(f"https://{HOST}/api/apps/{APP}").json()
            if a0["installed"] and a0.get("state") == "running": break
        ok("precondition: app installed via api", a0["installed"] and a0.get("state") == "running", str(a0.get("state"))); time.sleep(2)
    pw = pg.request.get(f"https://{HOST}/api/apps/{APP}/settings/{PWKEY}/reveal").json()["value"]
    ok("api: generated password exists", len(pw) >= 16, pw[:4] + "…")
    port = str(next(x for x in pg.request.get(f"https://{HOST}/api/apps").json() if x["name"] == APP)["port"])
    pg.goto(f"https://{HOST}/", wait_until="domcontentloaded"); pg.wait_for_selector(".icons .grp", timeout=30000); time.sleep(1.5)
    labels = pg.locator(".icons .gl").all_inner_texts(); ok("desktop: icon groups", [l.lower() for l in labels] == ["agentverse os", "проекты", "приложения из store"], str(labels))
    pg.screenshot(path="shots/om-desktop-groups.png")
    icon = pg.locator(".icons .icon", has_text=NAME); ok("desktop: app icon has source in tooltip", "Coolify" in (icon.first.get_attribute("title") or ""))
    icon.first.dblclick(); time.sleep(2)
    ok("dblclick icon: opens iframe window, no new tab", pg.locator(f'.win iframe[src*="{port}"]').count() == 1 and len(ctx.pages) == 1)
    fr = pg.frame_locator(f'.win iframe[src*="{port}"]')
    try:
        fr.locator("body").wait_for(timeout=20000); txt = fr.locator("body").inner_text(timeout=5000); ok("iframe: app ui rendered inside window", len(txt.strip()) > 0, txt.strip()[:60].replace("\n", " "))
    except Exception as e: ok("iframe: app ui rendered inside window", False, str(e)[:80])
    pg.screenshot(path="shots/om-app-window.png")
    pg.goto(f"https://{HOST}/#store", wait_until="domcontentloaded"); pg.wait_for_selector(".card.app", timeout=30000); time.sleep(1.5)
    pg.fill("input.search", NAME.lower()); time.sleep(0.8)
    pg.locator(".card.app", has_text="Coolify").first.locator("button", has_text="Подробнее").click(); time.sleep(2.5)
    det = pg.locator(".win", has_text="Описание").last
    sec = det.locator(".accessbox .field[data-field=пароль]"); ok("card: password shown in hero", sec.count() == 1, "нет блока" if not sec.count() else "")
    sec.first.locator("button[title=показать]").click(); time.sleep(1.2); ok("card: reveal shows real password", sec.first.locator("code").inner_text() == pw)
    pg.screenshot(path="shots/om-card-secret.png")
    ok("card: open button = window mode", det.locator("button.primary", has_text="Открыть").first.inner_text().strip() == "Открыть")
    det.locator(".tabs button", has_text="Техническое").click(); time.sleep(0.4)
    det.locator("button", has_text="в новой вкладке").click(); t0 = time.time(); det.locator("button.primary", has_text="Открыть ↗").wait_for(timeout=15000)
    ok("card: toggle to newtab", True, f"{time.time() - t0:.1f}s")
    with ctx.expect_page(timeout=10000) as np: det.locator("button.primary", has_text="Открыть ↗").click()
    ok("newtab mode: opens new tab", port in np.value.url); np.value.close()
    det.locator("button", has_text="в окне Desktop").click(); t0 = time.time(); det.locator("button.primary", has_text="Открыть ↗").wait_for(state="detached", timeout=15000)
    ok("card: toggle back to window", det.locator("button.primary", has_text="Открыть").first.inner_text().strip() == "Открыть", f"{time.time() - t0:.1f}s")
    a = pg.request.get(f"https://{HOST}/api/apps").json(); ok("api: app open=iframe persisted", next(x for x in a if x["name"] == APP)["manifest"]["route"]["open"] == "iframe")
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
