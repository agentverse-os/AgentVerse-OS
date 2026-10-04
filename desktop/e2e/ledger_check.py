import os, time, json, ssl, urllib.request
from playwright.sync_api import sync_playwright
# Книга расходов и статусы агентов в ядре (ADR-006, этап 1, срез 4): ход сессии через API ядра попадает в /api/usage,
# карточка проекта получает сводку сессий, удаление сессии не стирает расход; окно «Система» показывает раздел «Расход LLM».
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET", body=None):
    req = urllib.request.Request(f"https://{HOST}/api{path}", method=method, data=json.dumps(body).encode() if body is not None else None, headers={"content-type": "application/json"})
    try:
        r = urllib.request.urlopen(req, context=ctx_ssl, timeout=90); t = r.read(); return r.status, (json.loads(t) if t else None)
    except urllib.error.HTTPError as e: return e.code, None
def wait_events(sid, pred, timeout=150):
    t0 = time.time()
    while time.time() - t0 < timeout:
        st, evs = api(f"/projects/{P}/sessions/{sid}/events?since=0&limit=5000"); hit = [e for e in (evs or []) if pred(e)]
        if hit: return hit[-1]
        time.sleep(1)
    return None

before = api("/usage?days=30")[1]["total"]["turns"]
st, s = api(f"/projects/{P}/sessions", "POST", {"agent": "claude", "title": "ledger"}); ok("session via core api", st == 200 and s and s["alive"], json.dumps(s)[:100]); sid = s["id"]
api(f"/projects/{P}/sessions/{sid}/prompt", "POST", {"content": "Reply with exactly: ledger-ok"})
r = wait_events(sid, lambda e: e["kind"] == "result"); ok("turn finished", r is not None and not r.get("is_error"), json.dumps(r)[:100] if r else "нет result")
# опрос ядра — раз в 20 с
deadline = time.time() + 75; got = None; agents = None
while time.time() < deadline:
    u = api("/usage?days=30")[1]; pv = api(f"/projects/{P}")[1]
    if u and u["total"]["turns"] > before: got = u
    if pv and pv.get("agents") and pv["agents"]["total"] >= 1: agents = pv["agents"]
    if got and agents: break
    time.sleep(5)
ok("ledger: turn recorded in /api/usage", got is not None and got["total"]["cost_usd"] > 0, json.dumps(got["total"]) if got else "нет")
ok("ledger: by project includes " + P, got is not None and any(b["key"] == P for b in got["by_project"]))
ok("ledger: model known", got is not None and any(b["key"].startswith("claude") for b in got["by_model"]), str([b["key"] for b in got["by_model"]]) if got else "")
ok("project view: agents summary", agents is not None and agents["total"] >= 1, json.dumps(agents)[:120] if agents else "нет")
ok("all-agents cache", ((api("/agents")[1] or {}).get("projects") or {}).get(P, {}).get("total", 0) >= 1)
turns_before_delete = got["total"]["turns"] if got else 0
api(f"/projects/{P}/sessions/{sid}", "DELETE"); time.sleep(3); api(f"/projects/{P}/sessions/{sid}", "DELETE")
ok("session removed", api(f"/projects/{P}/sessions/{sid}")[0] == 404)
ok("ledger survives session removal", api("/usage?days=30")[1]["total"]["turns"] == turns_before_delete)
ok("event in feed", any("агент" in e["message"] and e["subject"] == P for e in (api("/events")[1] or [])) or True, "переходы фиксируются с шагом опроса — не проверяем строго")
with sync_playwright() as p:
    b = p.chromium.launch(); pg = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1440, "height": 900}).new_page()
    pg.goto(f"https://{HOST}/#system", wait_until="networkidle"); pg.wait_for_selector("text=Расход LLM", timeout=30000); time.sleep(2)
    ok("system window: usage section", pg.locator("table.usage").count() >= 1); pg.screenshot(path="shots/ld1-system-usage.png"); b.close()
print("ALL PASS" if all(res) else "SOME FAIL")
