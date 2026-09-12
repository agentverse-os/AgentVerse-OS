# Запуск как остальные e2e (см. desktop_test.py): на стенде в контейнере Playwright, EDGE_HOST задаёт хост Desktop.
# Визуальный и функциональный проход по устройствам и движкам: экран загрузки, обои двумя слоями, темы (стекло/OLED/контраст),
# раздел «Темы» в Настройках, заставка, оверлей связи, горизонтальный скролл. Настройки пользователя на сервере не меняются:
# темы применяются только в этой вкладке через атрибуты документа.
import os
from playwright.sync_api import sync_playwright
H = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(c); print(("PASS  " if c else "FAIL  ") + n, note)
DEV = {"phone": dict(viewport={"width": 390, "height": 844}, is_mobile=True, has_touch=True, device_scale_factor=2),
       "tablet": dict(viewport={"width": 820, "height": 1180}, has_touch=True),
       "desktop": dict(viewport={"width": 1440, "height": 900})}
def theme(pg, surface, mode, contrast="normal", radius="round", accent="#5B8CFF"):
    pg.evaluate(f"""() => {{ const d = document.documentElement; d.dataset.surface = '{surface}'; d.dataset.theme = '{mode}'; d.dataset.contrast = '{contrast}'; d.dataset.radius = '{radius}'; d.dataset.glass = '{"off" if contrast == "high" or surface == "oled" else "on"}'; d.style.setProperty('--accent', '{accent}');
      d.style.setProperty('--wallpaper', "url('/wallpaper-monolith-bg.webp') center / cover no-repeat #0B0D14"); d.style.setProperty('--wallpaper-top', "url('/wallpaper-monolith-mark.webp') center / cover no-repeat"); d.style.setProperty('--wallpaper-blur', '10px'); }}""")
with sync_playwright() as p:
    for engine in ("chromium", "firefox", "webkit"):
        try: b = getattr(p, engine).launch()
        except Exception as e: ok(f"{engine}: launch", False, str(e)[:80]); continue
        for dev, opts in DEV.items():
            if engine == "firefox" and opts.get("is_mobile"): opts = {k: v for k, v in opts.items() if k != "is_mobile"}  # Firefox без эмуляции is_mobile
            tag = f"{engine}/{dev}"
            try:
                ctx = b.new_context(locale="ru-RU", **opts); pg = ctx.new_page()
                pg.add_init_script("localStorage.setItem('cloudos.firstrun', '1'); localStorage.setItem('cloudos.certwarn.until', '9999999999999')")
                errors = []; pg.on("pageerror", lambda e: errors.append(str(e)[:120]))
                pg.goto(f"https://{H}/", wait_until="domcontentloaded"); pg.wait_for_timeout(4000)
                ok(f"{tag}: boot screen gone", pg.locator("#boot").count() == 0)
                ok(f"{tag}: shell rendered", pg.locator(".desktop").count() == 1 and pg.locator(".taskbar .start, .navbar, .status").count() >= 1)
                ok(f"{tag}: no horizontal scroll", pg.evaluate("document.documentElement.scrollWidth <= window.innerWidth + 1"))
                ok(f"{tag}: wallpaper layers css", "wallpaper-monolith" in pg.evaluate("getComputedStyle(document.body, '::after').backgroundImage") or pg.evaluate("getComputedStyle(document.body, '::after').backgroundImage") == "none")
                # темы: стекло тёмное/светлое, OLED, контраст — без ошибок JS и без скролла
                for name, args in {"glass-dark": ("glass", "dark"), "glass-light": ("glass", "light"), "oled": ("oled", "dark"), "contrast": ("flat", "dark", "high", "sharp", "#F2C94C")}.items():
                    theme(pg, *args); pg.wait_for_timeout(500)
                    ok(f"{tag}: theme {name} applied", pg.evaluate("document.documentElement.dataset.surface") == args[0] and pg.evaluate("document.documentElement.scrollWidth <= window.innerWidth + 1"))
                    if name == "glass-dark": pg.screenshot(path=f"shots/dev-{engine}-{dev}-glass.png")
                    if name == "glass-dark" and dev == "phone":
                        bf = pg.evaluate("getComputedStyle(document.querySelector('.status') || document.body).backdropFilter || ''")
                        ok(f"{tag}: glass backdrop on phone status bar", "blur" in bf or engine == "firefox", bf[:40])
                # Настройки → раздел «Темы»
                pg.goto(f"https://{H}/#settings", wait_until="domcontentloaded"); pg.wait_for_timeout(3000); theme(pg, "glass", "dark"); pg.wait_for_timeout(400)
                cards = pg.locator(".thm-card"); ok(f"{tag}: theme cards", cards.count() == 7, str(cards.count()))
                if cards.count():
                    bb = cards.first.bounding_box(); ok(f"{tag}: theme card visible & within viewport", bool(bb) and bb["x"] >= 0 and bb["x"] + bb["width"] <= opts["viewport"]["width"] + 1)
                pg.screenshot(path=f"shots/dev-{engine}-{dev}-settings.png")
                # заставка (тестовый таймер) и оверлей связи — только chromium/webkit на телефоне и десктопе, чтобы не тянуть время
                if engine != "firefox" and dev != "tablet":
                    pg.evaluate("localStorage.setItem('cloudos.screensaver.testMs', '2500')"); pg.goto(f"https://{H}/", wait_until="domcontentloaded"); pg.wait_for_timeout(2500); pg.mouse.move(100, 100); pg.wait_for_timeout(4000)
                    ok(f"{tag}: screensaver shows", pg.locator(".saver").count() == 1)
                    if pg.locator(".saver").count(): pg.screenshot(path=f"shots/dev-{engine}-{dev}-saver.png"); pg.mouse.move(120, 140); pg.touchscreen.tap(50, 50) if opts.get("has_touch") else None; pg.wait_for_timeout(700)
                    ok(f"{tag}: screensaver dismissed", pg.locator(".saver").count() == 0)
                    pg.evaluate("localStorage.removeItem('cloudos.screensaver.testMs')"); pg.reload(wait_until="domcontentloaded"); pg.wait_for_timeout(3000)  # таймер заставки читается при загрузке — иначе она накроет оверлей связи
                    # связь рвём на уровне контекста: page.route в WebKit не перехватывает запросы страницы под Service Worker; оверлей — через 25 с без ответа
                    ctx.set_offline(True); pg.wait_for_timeout(30000)
                    ok(f"{tag}: offline overlay", pg.locator(".offline").count() == 1); pg.screenshot(path=f"shots/dev-{engine}-{dev}-offline.png")
                    ctx.set_offline(False); pg.wait_for_timeout(14000); ok(f"{tag}: offline cleared", pg.locator(".offline").count() == 0)
                ok(f"{tag}: no JS errors", not errors, "; ".join(errors[:2]))
                ctx.close()
            except Exception as e:
                ok(f"{tag}: run", False, str(e)[:160])
        b.close()
print(f"\n{sum(res)}/{len(res)} passed"); raise SystemExit(0 if all(res) else 1)
