import os, time, json, ssl, urllib.request, random, string
# Маршрут «подписка» для Codex (ADR-006, решения 5–6): аккаунт `codex` импортируется из workspace, где выполнен вход
# (~/.codex/auth.json), сессия с этим аккаунтом получает CODEX_HOME и ходит своим входом (route=account), без gate-переопределения;
# usage приходит из thread/tokenUsage/updated. Один короткий ход на подписке владельца. Уборка: сессия и аккаунт.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); FROM = os.environ.get("ACCOUNT_FROM", "beta"); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note, flush=True)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET", body=None):
    req = urllib.request.Request(f"https://{HOST}/api{path}", method=method, data=json.dumps(body).encode() if body is not None else None, headers={"content-type": "application/json"})
    try:
        r = urllib.request.urlopen(req, context=ctx_ssl, timeout=120); t = r.read(); return r.status, (json.loads(t) if t else None)
    except urllib.error.HTTPError as e: return e.code, None
def events(sid, since=0): return api(f"/projects/{P}/sessions/{sid}/events?since={since}&limit=5000")[1] or []
def wait_event(sid, pred, since=0, timeout=150):
    t0 = time.time()
    while time.time() - t0 < timeout:
        m = [e for e in events(sid, since) if pred(e)]
        if m: return m[-1]
        time.sleep(2)
    return None
MARK = "sub-" + "".join(random.choice(string.ascii_lowercase) for _ in range(6))
st, a = api("/accounts/import", "POST", {"provider": "codex", "project": FROM, "label": f"codex {FROM} ({MARK})"})
ok("import codex subscription account", st == 200 and a and a["kind"] == "subscription" and "auth.json" in (a.get("files") or []), json.dumps(a)[:140] if a else str(st))
aid = a["id"] if a else None
sid = None
if aid:
    st, s = api(f"/projects/{P}/sessions", "POST", {"agent": "codex", "title": MARK, "account": aid})
    ok("codex session pinned to the account", st == 200 and s and s.get("account") == aid and "CODEX_HOME" in (s.get("env") or {}), json.dumps({k: s.get(k) for k in ("account", "env", "unset")}) if s else str(st)); sid = s["id"] if s else None
if sid:
    init = wait_event(sid, lambda e: e["kind"] == "init", 0, 60)
    ok("init: route=account (own login, no gate override)", init is not None and init.get("route") == "account" and init.get("model"), json.dumps({k: init.get(k) for k in ("model", "route")}) if init else "нет init: " + json.dumps([e for e in events(sid) if e["kind"] == "error"])[:200])
    api(f"/projects/{P}/sessions/{sid}/prompt", "POST", {"content": f"Reply with exactly this and nothing else: {MARK}. Do not run any commands."})
    r = wait_event(sid, lambda e: e["kind"] == "result", 0, 240)
    ok("turn on the subscription", r is not None and not r.get("is_error") and MARK in (r.get("text") or ""), (r or {}).get("text", "")[:80] if r else "нет result")
    ok("usage from thread/tokenUsage/updated", r is not None and (r.get("usage") or {}).get("output_tokens", 0) > 0, json.dumps((r or {}).get("usage"))[:140])
    # стоп и возобновление: thread/resume того же треда, история доступна модели
    seq = r["seq"] if r else 0
    api(f"/projects/{P}/sessions/{sid}", "DELETE"); time.sleep(3)
    st, s2 = api(f"/projects/{P}/sessions", "POST", {"resume": sid}); ok("resumed with the same id", st == 200 and s2 and s2["id"] == sid and s2["alive"] and s2.get("account") == aid, json.dumps({k: (s2 or {}).get(k) for k in ("account", "status")}))
    init2 = wait_event(sid, lambda e: e["kind"] == "init" and e["seq"] > seq, seq, 60)
    ok("resume: same provider thread", init2 is not None and init2.get("session_id") == (init or {}).get("session_id"), f"{(init2 or {}).get('session_id')} vs {(init or {}).get('session_id')}")
    api(f"/projects/{P}/sessions/{sid}/prompt", "POST", {"content": "What was the exact marker string I asked you to reply with? Reply with just the string."})
    r2 = wait_event(sid, lambda e: e["kind"] == "result" and e["seq"] > seq, seq, 240)
    ok("history survives resume", r2 is not None and not r2.get("is_error") and MARK in (r2.get("text") or ""), (r2 or {}).get("text", "")[:80] if r2 else "нет result")
    api(f"/projects/{P}/sessions/{sid}", "DELETE"); time.sleep(3); api(f"/projects/{P}/sessions/{sid}", "DELETE")
    ok("session removed", api(f"/projects/{P}/sessions/{sid}")[0] == 404)
if aid: ok("account deleted", api(f"/accounts/{aid}", "DELETE")[0] == 204 and not any(x["id"] == aid for x in api("/accounts")[1]["accounts"]))
print("ALL PASS" if all(res) else "SOME FAIL")
