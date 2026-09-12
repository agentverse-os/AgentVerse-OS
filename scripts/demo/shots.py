# Скриншоты Desktop для README: десктоп с окнами Store и проекта, планшет, телефон; тёмная тема Monolith и светлая.
# Запуск внутри контейнера Playwright против демо-прокси (scripts/demo/demo_proxy.py): python3 shots.py http://host:7300 /out [язык]
# Язык интерфейса (по умолчанию en) задаётся и в оформлении (language), и локалью контекста — демо-прокси по Accept-Language отдаёт события и новости на нём.
import sys, time
from playwright.sync_api import sync_playwright

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:7300"
OUT = sys.argv[2] if len(sys.argv) > 2 else "."
LANG = sys.argv[3] if len(sys.argv) > 3 else "en"
LOCALE = {"en": "en-US", "ru": "ru-RU", "uk": "uk-UA", "es": "es-ES"}[LANG]
INIT = "localStorage.setItem('cloudos.firstrun', '1'); localStorage.setItem('cloudos.certwarn.until', '9999999999999');"

def appearance(mode, bg="monolith"):
    a = {"mode": mode, "accent": "#5B8CFF" if mode == "dark" else "#4F8EF7", "background": bg, "backgroundUrl": "", "backgroundBlur": 12, "textScale": 1, "density": "normal",
         "radius": "round", "font": "grotesk", "contrast": "normal", "reduceMotion": True, "transparency": True, "iconSize": "medium", "showWorkspaceIps": True, "taskbarPinned": False, "screensaver": 0, "surface": "glass", "language": LANG}
    return a

def prep(ctx, mode, widgets, windows):
    """widgets и windows — словари по классу устройства (desktop | tablet | phone): ключи localStorage cloudos.widgets.<класс>, cloudos.windows.<класс>."""
    import json
    js = INIT + f"localStorage.setItem('cloudos.appearance.v1', {json.dumps(json.dumps(appearance(mode)))});"
    js += "".join(f"localStorage.setItem('cloudos.widgets.{k}', {json.dumps(json.dumps(v))}); localStorage.setItem('cloudos.widgets.{k}.cleared', '1');" for k, v in widgets.items())
    js += "".join(f"localStorage.setItem('cloudos.windows.{k}', {json.dumps(json.dumps(v))});" for k, v in windows.items())
    ctx.add_init_script(js)

SAVED = {}
def set_remote_appearance(pg, mode):
    """Оформление — в ядре (источник истины), поэтому его надо выставить и там; исходное запоминаем и в конце возвращаем."""
    if "settings" not in SAVED:
        try: SAVED["settings"] = pg.request.get(BASE + "/api/settings/desktop").json()
        except Exception: SAVED["settings"] = None
    pg.request.put(BASE + "/api/settings/desktop", data={"appearance": appearance(mode)})

def restore_remote_appearance(pg):
    s = SAVED.get("settings")
    if s is not None: pg.request.put(BASE + "/api/settings/desktop", data=s)

def settle(pg, ms=3500):
    pg.wait_for_selector(".taskbar, .mobile", timeout=30000); pg.wait_for_timeout(ms)

def shoot(pg, name):
    pg.screenshot(path=f"{OUT}/{name}.png"); print("shot", name)

with sync_playwright() as p:
    b = p.chromium.launch()
    # ---------------- десктоп 1600×1000, тёмная: Store + карточка проекта + виджеты
    W, H = 1600, 1000
    widgets = [{"id": "w-monitor", "type": "monitor", "x": W - 452, "y": 18, "w": 432, "h": 296, "config": {"show": "all"}},
               {"id": "w-clock", "type": "clock", "x": W - 452, "y": 330, "w": 270, "h": 140, "config": {"tz": "", "seconds": False}},
               {"id": "w-news", "type": "news", "x": W - 452, "y": 486, "w": 432, "h": 420, "config": {"interval": 15, "images": False, "compact": True, "preset": "ai", "count": 5, "lang": "both"}}]
    wins = {"desktop": [
        {"id": "sys:store", "kind": "component", "title": "Store", "titleKey": "sys.store", "icon": "🛍️", "component": "store", "x": 60, "y": 48, "w": 1000, "h": 760, "z": 20, "minimized": False, "maximized": False},
        {"id": "project:alpha", "kind": "component", "title": "alpha", "icon": "🧩", "component": "project", "props": {"name": "alpha"}, "x": 420, "y": 250, "w": 720, "h": 560, "z": 21, "minimized": False, "maximized": False}]}
    ctx = b.new_context(locale=LOCALE, viewport={"width": W, "height": H}, device_scale_factor=2); prep(ctx, "dark", {"desktop": widgets}, wins); pg = ctx.new_page(); set_remote_appearance(pg, "dark")
    pg.goto(BASE + "/", wait_until="domcontentloaded"); settle(pg, 5000)
    shoot(pg, "desktop-dark"); ctx.close()

    # ---------------- десктоп, светлая: Проекты + Настройки (заголовки окон — по ключам словаря)
    wins2 = {"desktop": [
        {"id": "sys:projects", "kind": "component", "title": "Projects", "titleKey": "sys.projects", "icon": "🗂️", "component": "projects", "x": 60, "y": 48, "w": 980, "h": 640, "z": 20, "minimized": False, "maximized": False},
        {"id": "sys:settings", "kind": "component", "title": "Settings", "titleKey": "sys.settings", "icon": "⚙️", "component": "settings", "x": 520, "y": 200, "w": 940, "h": 720, "z": 21, "minimized": False, "maximized": False}]}
    ctx = b.new_context(locale=LOCALE, viewport={"width": W, "height": H}, device_scale_factor=2); prep(ctx, "light", {"desktop": widgets[:2]}, wins2); pg = ctx.new_page(); set_remote_appearance(pg, "light")
    pg.goto(BASE + "/", wait_until="domcontentloaded"); settle(pg, 4500)
    shoot(pg, "desktop-light"); ctx.close()

    # ---------------- планшет 1024×1366 портрет
    tw = [{"id": "w-monitor", "type": "monitor", "x": 560, "y": 18, "w": 446, "h": 300, "config": {"show": "all"}},
          {"id": "w-clock", "type": "clock", "x": 560, "y": 334, "w": 270, "h": 140, "config": {"tz": "", "seconds": False}},
          {"id": "w-projects", "type": "projects", "x": 560, "y": 490, "w": 446, "h": 250, "config": {}}]
    twins = {"tablet": [{"id": "sys:store", "kind": "component", "title": "Store", "titleKey": "sys.store", "icon": "🛍️", "component": "store", "x": 24, "y": 560, "w": 976, "h": 740, "z": 20, "minimized": False, "maximized": False}]}
    ctx = b.new_context(locale=LOCALE, viewport={"width": 1024, "height": 1366}, device_scale_factor=2, has_touch=True); prep(ctx, "dark", {"tablet": tw}, twins); pg = ctx.new_page(); set_remote_appearance(pg, "dark")
    pg.goto(BASE + "/", wait_until="domcontentloaded"); settle(pg, 4500)
    shoot(pg, "tablet-dark"); ctx.close()

    # ---------------- телефон 390×844: домашний экран и окно Store
    ctx = b.new_context(locale=LOCALE, viewport={"width": 390, "height": 844}, device_scale_factor=3, is_mobile=True, has_touch=True)
    prep(ctx, "dark", {"phone": [{"id": "w-monitor", "type": "monitor", "x": 0, "y": 0, "w": 360, "h": 260, "config": {"show": "all"}}]}, {"phone": []}); pg = ctx.new_page(); set_remote_appearance(pg, "dark")
    pg.goto(BASE + "/", wait_until="domcontentloaded"); settle(pg, 4000)
    shoot(pg, "phone-home")
    pg.locator(".mobile .tile", has_text="Store").first.click(); pg.wait_for_timeout(2500)
    shoot(pg, "phone-store"); restore_remote_appearance(pg); ctx.close()
    b.close()
