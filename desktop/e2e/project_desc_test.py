import os, time, json, ssl, urllib.request
from playwright.sync_api import sync_playwright
# Описание проекта: задаётся через API и из карточки, видно в списке проектов, окне проекта и меню запуска (с поиском по нему);
# смена одного описания не перевыкатывает проект (событие «описание обновлено», без «сеть и gate готовы»). Скриншоты в shots/.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET", body=None):
    req = urllib.request.Request(f"https://{HOST}/api{path}", method=method, data=json.dumps(body).encode() if body is not None else None, headers={"content-type": "application/json"})
    r = urllib.request.urlopen(req, context=ctx_ssl, timeout=120); t = r.read(); return json.loads(t) if t else None
def project(): return next(p for p in api("/projects") if p["name"] == P)
def events(): return [e for e in api("/events") if e.get("subject") == P]

before = project()["spec"]; n0 = len(events())
DESC = "бот поддержки для Telegram\nв работе: оплата и напоминания"
# 1. API: только описание — короткий путь, без перевыкатки сети/gate
t0 = time.time(); after = api("/projects", "POST", {**before, "description": DESC}); dt = time.time() - t0
ok("api: description saved", after["spec"].get("description") == DESC, json.dumps(after["spec"].get("description"), ensure_ascii=False))
ok("api: quick (no redeploy)", dt < 5, f"{dt:.1f}s")
new_ev = events()[: len(events()) - n0]
ok("api: event 'описание обновлено', no gate redeploy", any("описание обновлено" in e["message"] for e in new_ev) and not any("сеть и gate готовы" in e["message"] for e in new_ev), [e["message"] for e in new_ev])
ok("api: whitespace-only clears", api("/projects", "POST", {**before, "description": "   "})["spec"].get("description") is None)
api("/projects", "POST", {**before, "description": DESC})
try:
    api("/projects", "POST", {**before, "description": "x" * 2001}); ok("api: >2000 rejected", False)
except urllib.error.HTTPError as e: ok("api: >2000 rejected with 400", e.code == 400, str(e.code))

with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); pg = ctx.new_page()
    # 2. окно «Проекты»: карточка показывает описание
    pg.goto(f"https://{HOST}/#projects", wait_until="networkidle"); pg.wait_for_selector(f".card:has(h3:text-is('{P}'))", timeout=30000); time.sleep(1)
    card = pg.locator(f".card:has(h3:text-is('{P}'))")
    ok("projects: card shows description", "бот поддержки" in card.locator(".desc").inner_text()); pg.screenshot(path="shots/pd1-projects.png")
    # 3. правка из карточки: «Изменить» → textarea → «Применить»
    card.locator("button:text-is('Изменить')").click(); time.sleep(.4)
    ta = card.locator("textarea.desc-edit"); ok("card: editor prefilled", ta.input_value() == DESC)
    ta.fill("магазин на Expo — витрина и корзина"); pg.screenshot(path="shots/pd2-edit.png")
    card.locator("button:text-is('Применить')").click(); time.sleep(3)
    ok("card: edited description applied", "магазин на Expo" in pg.locator(f".card:has(h3:text-is('{P}')) .desc").inner_text() and project()["spec"].get("description") == "магазин на Expo — витрина и корзина")
    # 4. окно проекта: описание целиком
    pg.locator(f".stack:has(h3:text-is('{P}')) .open").click(); time.sleep(1.5)
    ok("project window: description", pg.locator(".win.active .card.compact .desc").count() == 1 and "магазин" in pg.locator(".win.active .card.compact .desc").inner_text()); pg.screenshot(path="shots/pd3-window.png")
    for _ in range(2): pg.evaluate("document.querySelector('.win.active .ctl .close')?.click()"); time.sleep(.5)
    # 5. меню запуска: описание в строке проекта и поиск по нему
    pg.click("nav.dock .item.start, nav.taskbar .start"); time.sleep(.8)
    ok("start menu: note shown", "магазин" in pg.locator(f".item:has-text('{P}') .note").inner_text())
    pg.keyboard.type("корзина"); time.sleep(.5)
    ok("start menu: search by description", pg.locator(f".item:has-text('{P}')").count() == 1); pg.screenshot(path="shots/pd4-start-search.png")
    pg.keyboard.press("Escape")
    # 6. подсказка иконки на рабочем столе
    # иконки перечитывают проекты раз в 15 с — ждём, пока подсказка подхватит новое описание
    for _ in range(25):
        title = pg.locator(f".icon:has(.name:text-is('{P}'))").get_attribute("title") or ""
        if "магазин" in title: break
        time.sleep(1)
    ok("desktop icon: tooltip has description", "магазин" in title, title.replace("\n", " | "))
    # 7. новый проект: форма с полем описания (только вид, не создаём)
    pg.goto(f"https://{HOST}/#projects", wait_until="networkidle"); time.sleep(1); pg.click("button:text-is('+ Новый проект')"); time.sleep(.4)
    ok("new project form: description field", pg.locator("textarea.desc").count() == 1); pg.screenshot(path="shots/pd5-new-form.png")
    b.close()
# вернуть как было
api("/projects", "POST", {**before, "description": before.get("description")})
print("ALL PASS" if all(res) else "SOME FAIL")
