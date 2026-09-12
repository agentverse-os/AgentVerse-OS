import os, time
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(c); print(("PASS  " if c else "FAIL  ") + n, note)
with sync_playwright() as p:
    b = p.chromium.launch()
    for label, vp in [("desktop", {"width": 1440, "height": 900}), ("phone", {"width": 390, "height": 844})]:
        ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport=vp, has_touch=(label == "phone")); pg = ctx.new_page()
        pg.add_init_script("localStorage.setItem('cloudos.certwarn.until', '9999999999999')")  # браузер тестов не доверяет сертификату — баннер не должен мешать кликам
        s0 = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); s0.pop("session", None); pg.request.put(f"https://{HOST}/api/settings/desktop", data=s0)
        pg.goto(f"https://{HOST}/#files", wait_until="domcontentloaded"); pg.wait_for_selector(".files .row-entry", timeout=30000); time.sleep(1.5)
        ok(f"{label}: files window", pg.locator(".files .row-entry").count() >= 5, f"{pg.locator('.files .row-entry').count()} записей")
        pg.locator(".files .view button[title='плитки']").click(); time.sleep(0.6)
        ok(f"{label}: grid view", pg.locator(".files .tile").count() >= 5 and pg.locator(".files .tile .fold").count() >= 5)
        pg.screenshot(path=f"shots/f-{label}-grid.png")
        pg.locator(".files .view button[title='список']").click(); time.sleep(0.4)
        ok(f"{label}: back to list", pg.locator(".files .tile").count() == 0 and pg.locator(".files .row-entry").count() >= 5)
        pg.locator(".files .row-entry button.link", has_text="whoami").first.click(); time.sleep(1)
        if label == "phone":
            pg.locator(".files .more").click(); time.sleep(0.4); ok(f"{label}: folder action sheet", pg.locator(".files .sheet .sitem").count() >= 4)
            pg.once("dialog", lambda d: d.accept("notes.md")); pg.locator(".files .sheet .sitem", has_text="+ файл").click(); time.sleep(1.5)
        else:
            pg.once("dialog", lambda d: d.accept("notes.md")); pg.locator(".files .bar button", has_text="+ файл").click(); time.sleep(1.5)
        for _ in range(20):
            if pg.locator(".files .row-entry", has_text="notes.md").count() == 1: break
            time.sleep(0.5)
        ok(f"{label}: new file created", pg.locator(".files .row-entry", has_text="notes.md").count() == 1)
        pg.locator(".files .row-entry button.link", has_text="notes.md").first.click(); time.sleep(1)
        ok(f"{label}: editor opened", pg.locator(".files .editor textarea").count() == 1)
        pg.fill(".files .editor textarea", "# заметка\nиз Desktop\n"); pg.keyboard.press("Control+s"); time.sleep(1.5)
        ok(f"{label}: saved via Ctrl+S", "сохранено" == pg.locator(".files .editor .muted").first.inner_text())
        r = pg.request.get(f"https://{HOST}/api/files/apps/read?path=whoami/notes.md"); ok(f"{label}: content on server", r.ok and "из Desktop" in r.text())
        pg.screenshot(path=f"shots/f-{label}.png")
        if label == "phone":
            pg.locator(".files .editor button", has_text="Закрыть").click(); time.sleep(0.5)  # редактор на телефоне закрывает список
            pg.locator(".files .row-entry", has_text="notes.md").locator("button.rowmenu").click(); time.sleep(0.4); ok(f"{label}: entry action sheet", pg.locator(".files .sheet .sitem", has_text="Удалить").count() == 1)
            pg.once("dialog", lambda d: d.accept()); pg.locator(".files .sheet .sitem", has_text="Удалить").click(); time.sleep(1.2)
        else:
            pg.locator(".files .editor button", has_text="Закрыть").click(); time.sleep(0.4)
            pg.once("dialog", lambda d: d.accept()); pg.locator(".files .row-entry", has_text="notes.md").locator("button.danger").click(); time.sleep(1.2)
        for _ in range(20):
            if pg.locator(".files .row-entry", has_text="notes.md").count() == 0: break
            time.sleep(0.5)
        ok(f"{label}: deleted", pg.locator(".files .row-entry", has_text="notes.md").count() == 0)
        # переключение корня на workspace
        if label == "phone": pg.locator(".files .dd .dd-btn").first.click(); time.sleep(0.3); pg.locator(".dd-list li[role=option]", has_text="alpha").first.click(); time.sleep(3)
        else: pg.locator(".files .side .sroot", has_text="alpha").first.click(); time.sleep(3)  # на десктопе корни — в боковой панели
        ok(f"{label}: workspace root lists home", pg.locator(".files .row-entry", has_text=".npm-global").count() == 1)
        ctx.close()
    # Конфигурация в карточке приложения
    ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); pg = ctx.new_page()
    pg.goto(f"https://{HOST}/#app=whoami", wait_until="domcontentloaded"); pg.wait_for_selector(".tabs", timeout=30000); pg.click(".tabs button:has-text('Конфигурация')"); time.sleep(1.5)
    ok("app config tab", pg.locator(".cfg textarea").count() == 2 and pg.locator("text=итоговый compose").count() == 1)
    pg.screenshot(path="shots/f-config.png")
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
