# e2e «Связи»: вкладка в карточке, подключение потребителя к провайдеру (по умолчанию Garage) из выпадающего списка, переменные от связи, отключение.
import os, time, json, ssl, urllib.request
from playwright.sync_api import sync_playwright
HOST = os.environ.get("EDGE_HOST", "aios.tail0fe52f.ts.net"); P = os.environ.get("LINK_PROVIDER", "garage"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET"): return json.load(urllib.request.urlopen(urllib.request.Request(f"https://{HOST}/api{path}", method=method), context=ctx_ssl))
# потребитель связи — первый установленный из списка (Hermes на стенде может быть удалён); связь пересобирает стек, поэтому в запасе лёгкие приложения
apps = {a["name"]: a for a in api("/apps")}
PT = apps[P]["manifest"].get("title") or P; EP = apps[P]["manifest"]["endpoint"]; VAR = P.upper().replace("-", "_") + "_URL"  # заголовок провайдера, его endpoint и переменная от связи
C = os.environ.get("LINK_APP") or next(n for n in ("hermes-agent-with-webui-coolify", "hermes-agent", "whoami", "it-tools") if apps.get(n, {}).get("installed"))
print("consumer:", C)
# предусловие: связи нет (иначе провайдера не будет в списке кандидатов)
if P in api(f"/apps/{C}")["links_to"]:
    api(f"/apps/{C}/links/{P}", "DELETE")
    for _ in range(60):
        if P not in api(f"/apps/{C}")["links_to"]: break
        time.sleep(2)
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}); ctx.add_init_script("localStorage.setItem('cloudos.firstrun', '1'); localStorage.setItem('cloudos.certwarn.until', '9999999999999')"); pg = ctx.new_page()
    s0 = pg.request.get(f"https://{HOST}/api/settings/desktop").json(); s0.pop("session", None); pg.request.put(f"https://{HOST}/api/settings/desktop", data=s0)  # без восстановленных окон сессии: карточка должна быть единственным окном
    pg.goto(f"https://{HOST}/#app={C}", wait_until="domcontentloaded"); time.sleep(3)
    det = pg.locator(".win", has_text=C).last
    det.locator(".tabs button", has_text="Связи").click(); time.sleep(0.5)
    ok("card: links tab with internal address", (api(f"/apps/{C}")["internal_url"] or "—") in det.inner_text(), api(f"/apps/{C}")["internal_url"])
    dd = det.locator(".dd").first; dd.locator("button.dd-btn").click(); time.sleep(0.3)
    pg.locator(".dd-list li[role=option]", has_text=PT).first.click(); time.sleep(0.3)
    det.locator("button.primary", has_text="Подключить").click()
    for _ in range(90):
        time.sleep(2)
        if f"{VAR}=" in det.inner_text(): break
    ok("link: provider listed after connect", P in api(f"/apps/{C}")["links_to"] and det.locator("dd", has_text=P).count() >= 1)
    txt = det.inner_text()
    ok("link: env variables shown", f"{VAR}=http://{EP['service']}:{EP['port']}" in txt, VAR)
    pg.screenshot(path="shots/lk-linked.png")
    det.locator("button.danger", has_text="отключить").first.click()
    for _ in range(90):
        time.sleep(2)
        if P not in api(f"/apps/{C}")["links_to"]: break
    ok("unlink: provider removed", P not in api(f"/apps/{C}")["links_to"])
    b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
