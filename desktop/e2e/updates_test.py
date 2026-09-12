# e2e обновлений: /api/updates — система (версия, канал, пакеты), приложения (версии и флаги), компоненты (edge, Coder, Komodo);
# окно «Обновления» показывает всё это; канал: неверный адрес отклоняется, недоступный — «канал недоступен», сброс; загрузка пакета
# файлом (dist/*.tar.gz, если передан UPDATE_PKG) → появляется в списке как «та же версия», удаление; обновление whoami
# (плавающий тег) проходит и пишет updated_at; сводка в /api/status.
import os, sys, time, json, ssl, urllib.request
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); PKG = os.environ.get("UPDATE_PKG", ""); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET", body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"https://{HOST}/api{path}", data=data, method=method, headers={"Content-Type": "application/json"} if data else {})
    return json.load(urllib.request.urlopen(req, context=ctx_ssl, timeout=600))
u = api("/updates")
ok("api: system version and arch", u["system"]["version"] and u["system"]["arch"] == "x86_64" and u["system"]["binary"].endswith("cloudd"), f'{u["system"]["version"]} {u["system"]["build"]}')
ok("api: apps listed with versions", len(u["apps"]) >= 5 and all("has_update" in a and "floating_tags" in a for a in u["apps"]), str([(a["name"], a["installed_version"], a["catalog_version"]) for a in u["apps"]][:4]))
titles = {c["title"] for c in u["components"]}
ok("api: components edge, coder, komodo discovered", {"Edge (Caddy)", "Coder + PostgreSQL", "Komodo (App Runtime)"} <= titles, str(sorted(titles)))
ok("api: components updatable with compose files", all(c["updatable"] for c in u["components"] if c["title"] in ("Edge (Caddy)", "Komodo (App Runtime)")), str([(c["project"], c["updatable"], c["note"]) for c in u["components"]]))
ok("api: catalog size", u["catalog"]["manifests"] > 900, str(u["catalog"]["manifests"]))
flo = next((a for a in u["apps"] if a["name"] == "it-tools"), None)
ok("api: it-tools has floating tag (latest)", flo is not None and any("it-tools" in t for t in flo["floating_tags"]), str(flo and flo["floating_tags"]))
ok("api: whoami in catalog, pinned tag", any(a["name"] == "whoami" and a["in_catalog"] and not a["floating_tags"] for a in u["apps"]))
try: api("/updates/channel", "PUT", {"url": "ftp://bad"}); bad = False
except urllib.error.HTTPError as e: bad = "https://" in e.read().decode()
ok("api: channel rejects non-http url", bad)
s = api("/updates/channel", "PUT", {"url": f"https://{HOST}/no-such-channel.json"})
ok("api: unreachable channel reports error, no update", s["check_error"] and not s["available"] and s["channel_url"].endswith("no-such-channel.json"), str(s["check_error"])[:80])
s = api("/updates/channel", "PUT", {"url": ""})
ok("api: channel cleared", s["channel_url"] is None and not s["check_error"])
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); ctx.add_init_script("localStorage.setItem('cloudos.firstrun', '1'); localStorage.setItem('cloudos.certwarn.until', '9999999999999')"); pg = ctx.new_page()
    s0 = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); s0.pop("session", None); pg.request.put(f"https://{HOST}/api/settings/desktop", data=s0)
    pg.goto(f"https://{HOST}/#updates", wait_until="domcontentloaded"); pg.wait_for_selector(".win .upd table", timeout=30000); time.sleep(1.5)
    win = pg.locator(".win", has_text="Обновления").first
    ok("window: version in header", u["system"]["version"] in win.inner_text())
    ok("window: apps table rows", win.locator(".tbl").nth(0).locator("tbody tr").count() >= 5 or win.locator(".tbl").nth(1).locator("tbody tr").count() >= 5)
    ok("window: components rows with Edge and Komodo", "Edge (Caddy)" in win.inner_text() and "Komodo (App Runtime)" in win.inner_text())
    pg.screenshot(path="shots/updates-window.png")
    if PKG and os.path.exists(PKG):
        win.locator("input[type=file]").set_input_files(PKG); win.locator("button", has_text="Загрузить пакет").click()
        for _ in range(40):
            time.sleep(1)
            if win.locator("table").nth(0).locator("tbody tr").count() and "агентверс" or "agentverse-os" in win.inner_text(): pass
            if win.locator(".note").count() and "загружен" in win.locator(".note").inner_text(): break
        note = win.locator(".note").inner_text() if win.locator(".note").count() else ""
        ok("window: package uploaded and verified", "загружен и проверен" in note, note[:120])
        u2 = api("/updates"); pk = [x for x in u2["system"]["packages"] if x["file"].startswith("upload-")]
        ok("api: package listed, same version, store manifests inside", pk and pk[0]["relation"] == "same" and "hermes-agent" in pk[0]["store_apps"], str(pk and (pk[0]["version"], pk[0]["relation"], pk[0]["store_apps"])))
        ok("window: reinstall button for same version", win.locator("button", has_text="Переустановить").count() >= 1)
        pg.screenshot(path="shots/updates-package.png")
        if pk:
            api(f"/updates/packages/{pk[0]['file']}", "DELETE")
            ok("api: package deleted", not any(x["file"] == pk[0]["file"] for x in api("/updates")["system"]["packages"]))
    # обновить whoami из окна (перевыкатка стека с актуальными compose и env, pull образов)
    row = win.locator("section", has_text="Приложения из каталога").locator("tr", has_text="whoami").first  # не строка пакета, где whoami — среди манифестов Store
    t0 = time.time(); row.locator("button", has_text="Обновить").click()
    for _ in range(120):
        time.sleep(2)
        if win.locator(".note").count() and "обновлено" in win.locator(".note").inner_text(): break
    ok("window: whoami updated", win.locator(".note").count() and "обновлено" in win.locator(".note").inner_text(), f"{time.time() - t0:.0f}s")
    a = next(x for x in api("/updates")["apps"] if x["name"] == "whoami")
    ok("api: whoami updated_at set and running", a["updated_at"] and a["state"] == "running", str((a["updated_at"], a["state"])))
    ev = api("/events")
    ok("events: update recorded", any(e["kind"] == "update" and e["subject"] == "whoami" for e in ev))
    b.close()
st = api("/status")
ok("status: update fields present", "update_available" in st and "app_updates" in st and st["setup_done"] in (True, False))
print("ALL PASS" if all(res) else "SOME FAIL")
