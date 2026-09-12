# e2e P1 userflow: смена сгенерированного секрета из карточки (приложение перевыкатывается с новым значением), заметка,
# окно «Пароли и доступы» с экспортом, мастер установки для приложения с обязательными настройками.
import os, re, time, json, ssl, urllib.request
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); P = os.environ.get("ROTATE_APP", "hermes-agent"); W = "adguardhome-sync";  # P — установленное приложение с сгенерированным паролем (login.mode generated) os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET", body=None):
    req = urllib.request.Request(f"https://{HOST}/api{path}", method=method, data=json.dumps(body).encode() if body is not None else None, headers={"Content-Type": "application/json"} if body is not None else {})
    r = urllib.request.urlopen(req, context=ctx_ssl); t = r.read(); return json.loads(t) if t else None
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}, permissions=["clipboard-read", "clipboard-write"])
    ctx.add_init_script("localStorage.setItem('cloudos.firstrun', '1'); localStorage.setItem('cloudos.certwarn.until', '9999999999999')"); pg = ctx.new_page(); pg.on("dialog", lambda d: d.accept())
    KEY = api(f"/apps/{P}")["manifest"]["login"]["password"]  # имя переменной с сгенерированным паролем
    old = api(f"/apps/{P}/settings/{KEY}/reveal")["value"]
    pg.goto(f"https://{HOST}/#app={P}", wait_until="domcontentloaded"); time.sleep(3)
    det = pg.locator(".win", has_text="Описание").last
    fld = det.locator(".accessbox .field[data-field=пароль]")
    ok("card: rotate button on generated password", fld.locator("button.rotate").count() == 1)
    fld.locator("button.rotate").click()
    new = old
    for _ in range(60):
        time.sleep(2); new = api(f"/apps/{P}/settings/{KEY}/reveal")["value"]
        if new != old and api(f"/apps/{P}")["state"] == "running": break
    ok("rotate: new secret stored and app redeployed", new != old and len(new) == len(old))
    # заметка
    det.locator("button.notebtn").click(); time.sleep(0.5)
    det.locator(".note textarea").fill("тестовая заметка: администратор admin"); det.locator(".note button.primary").click(); time.sleep(1.5)
    ok("note: saved via API", api(f"/apps/{P}/note")["text"].startswith("тестовая заметка") and api(f"/apps/{P}")["has_note"])
    pg.goto(f"https://{HOST}/#app={P}", wait_until="domcontentloaded"); time.sleep(3)
    det = pg.locator(".win", has_text="Описание").last
    ok("note: indicator after reload", det.locator("button.notebtn", has_text="📝 заметка").count() == 1)
    pg.screenshot(path="shots/p1-card.png")
    # «Пароли и доступы»
    pg.goto(f"https://{HOST}/#vault", wait_until="domcontentloaded"); time.sleep(3)
    v = pg.locator(".win", has_text="Пароли и доступы").last
    ok("vault: rows for installed apps", v.locator(".vrow").count() >= 5, str(v.locator(".vrow").count()))
    ok("vault: password fields with reveal", v.locator(".vrow .field[data-field=пароль] button[title=показать]").count() >= 1, str(v.locator(".vrow .field[data-field=пароль] button[title=показать]").count()))
    v.locator("input.search").fill(P); time.sleep(0.6); ok("vault: search filters", 1 <= v.locator(".vrow").count() <= 2)
    v.locator("button", has_text="Экспорт текстом").click(); time.sleep(3)
    clip = pg.evaluate("navigator.clipboard.readText()")
    ok("vault: export text contains password and note", ("пароль: " + new) in clip and "заметка: тестовая заметка" in clip, clip.replace(new, "***")[:120].replace("\n", " | "))
    pg.screenshot(path="shots/p1-vault.png")
    # мастер установки для приложения с обязательными настройками (не устанавливаем)
    if not api(f"/apps/{W}")["installed"]:
        pg.goto(f"https://{HOST}/#store", wait_until="domcontentloaded"); pg.wait_for_selector(".card.app", timeout=30000); time.sleep(1.5)
        pg.fill("input.search", W); time.sleep(0.8)
        wt = api(f"/apps/{W}")["manifest"].get("title") or W
        pg.locator(".card.app", has_text=wt).first.locator("button", has_text="Установить").click(); time.sleep(0.6)
        dlg = pg.locator(".dlg", has_text="Установить")
        ok("install wizard: dialog with required fields", dlg.count() == 1 and dlg.locator("label.fld").count() >= 3, str(dlg.locator("label.fld").count()))
        dlg.locator("button.primary", has_text="Установить").click(); time.sleep(0.5)
        ok("install wizard: validation blocks empty required", "заполните" in dlg.inner_text() and not api(f"/apps/{W}")["installed"])
        pg.screenshot(path="shots/p1-wizard.png"); dlg.locator("button", has_text="Отмена").click()
    else: ok("install wizard: skipped (app installed)", True)
    api(f"/apps/{P}/note", "PUT", {"text": ""}); ok("note: cleanup", not api(f"/apps/{P}")["has_note"])
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
