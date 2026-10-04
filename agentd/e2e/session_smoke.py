import json, os, sys, time, urllib.request
# Сессии агентов в cloudos-agentd на живом workspace (провайдер Claude, родной stream-json через gate проекта):
# создать сессию, спросить, дождаться result с usage; вызвать инструмент, который требует разрешения, ответить
# «разрешить» с любого клиента, увидеть результат; остановить и возобновить с той же историей.
# Запуск: AGENTD=http://10.77.7.146:7200 AGENTD_TOKEN=… python3 session_smoke.py
BASE = os.environ.get("AGENTD", "http://127.0.0.1:7200"); TOKEN = os.environ.get("AGENTD_TOKEN", ""); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
def api(path, method="GET", body=None):
    req = urllib.request.Request(BASE + path, method=method, data=json.dumps(body).encode() if body is not None else None,
                                 headers={"content-type": "application/json", "authorization": f"Bearer {TOKEN}"})
    try:
        r = urllib.request.urlopen(req, timeout=60); t = r.read(); return r.status, (json.loads(t) if t else None)
    except urllib.error.HTTPError as e: return e.code, json.loads(e.read() or b"null")
def wait_for(sid, pred, timeout=120):
    t0 = time.time(); seen = []
    while time.time() - t0 < timeout:
        st, evs = api(f"/v1/sessions/{sid}/events?since=0&limit=5000"); seen = evs or []
        hit = [e for e in seen if pred(e)]
        if hit: return hit[-1], seen
        time.sleep(1)
    return None, seen
MARK = f"agentd-{int(time.time())}"

st, s = api("/v1/sessions", "POST", {"title": "smoke", "permission_mode": "default"}); ok("session created", st == 200 and s and s["status"] in ("starting", "idle"), json.dumps(s)[:140]); sid = s["id"]
st, _ = api(f"/v1/sessions/{sid}/prompt", "POST", {"content": f"Reply with exactly this and nothing else: {MARK}"}); ok("prompt accepted", st == 200)
r, evs = wait_for(sid, lambda e: e["kind"] == "result", 150)
ok("turn finished with result", r is not None and not r.get("is_error"), json.dumps(r)[:160] if r else "нет result; события: " + ", ".join(e["kind"] for e in evs))
init = next((e for e in evs if e["kind"] == "init"), None); ok("claude started (system init)", init is not None and init.get("model"), json.dumps(init)[:120] if init else "нет init")
ok("answer streamed as assistant event", any(e["kind"] == "assistant" and MARK in json.dumps(e) for e in evs))
ok("usage recorded", r is not None and (r.get("usage") or {}).get("output_tokens", 0) > 0 and r.get("total_cost_usd") is not None, json.dumps(r.get("usage") if r else None))
st, info = api(f"/v1/sessions/{sid}"); ok("status idle, usage totals", info["info"]["status"] == "idle" and info["info"]["usage"]["turns"] == 1, json.dumps(info["info"]["usage"]))
# разрешение инструмента
path = f"/home/coder/{MARK}.txt"
api(f"/v1/sessions/{sid}/prompt", "POST", {"content": f"Create the file {path} containing the single word hello. Use the Write tool. Do not ask questions."})
p, _ = wait_for(sid, lambda e: e["kind"] == "permission_request", 90)
ok("permission requested for Write", p is not None and p.get("tool_name") in ("Write", "Bash", "Edit"), json.dumps(p)[:140] if p else "нет запроса")
st, info = api(f"/v1/sessions/{sid}"); ok("status waiting while pending", info["info"]["status"] == "waiting" and info["pending"], json.dumps(info["pending"])[:100])
st, _ = api(f"/v1/sessions/{sid}/permission", "POST", {"request_id": p["request_id"], "behavior": "allow"}); ok("permission answered", st == 200)
r2, evs = wait_for(sid, lambda e: e["kind"] == "result" and e["seq"] > (p or {}).get("seq", 0), 120)
ok("turn finished after approval", r2 is not None and not r2.get("is_error"), json.dumps(r2)[:120] if r2 else "нет result")
ok("tool_result observed", any(e["kind"] == "tool_result" for e in evs))
# остановить и возобновить с историей
st, _ = api(f"/v1/sessions/{sid}", "DELETE"); ok("stop accepted", st == 202)
ex, _ = wait_for(sid, lambda e: e["kind"] == "exit", 30); ok("process exited", ex is not None)
st, s2 = api("/v1/sessions", "POST", {"resume": sid}); ok("resumed same session id", st == 200 and s2 and s2["id"] == sid, json.dumps(s2)[:120] if s2 else str(st))
api(f"/v1/sessions/{sid}/prompt", "POST", {"content": "What was the exact marker string I asked you to reply with earlier? Reply with just the string."})
r3, _ = wait_for(sid, lambda e: e["kind"] == "result" and e["seq"] > (ex or {}).get("seq", 0), 120)
ok("history survives resume", r3 is not None and MARK in (r3.get("text") or ""), (r3 or {}).get("text", "")[:80])
st, usage = api("/v1/usage?since=0"); ok("usage feed for ledger", st == 200 and len([u for u in usage if u["session"] == sid]) >= 3, f"записей: {len(usage or [])}")
api(f"/v1/sessions/{sid}", "DELETE"); time.sleep(2); api(f"/v1/sessions/{sid}", "DELETE")
ok("removed", api(f"/v1/sessions/{sid}")[0] == 404)
print("ALL PASS" if all(res) else "SOME FAIL"); sys.exit(0 if all(res) else 1)
