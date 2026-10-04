import os, time, json, ssl, urllib.request
# Аккаунты агентов v0 (ADR-006, решение 6): импорт gate-аккаунта из гранта проекта, умолчание проекта, сессия прибивается к
# аккаунту (в agentd приходят переменные и unset, значения наружу не отдаются), ход проходит, переназначение сессии на
# «без аккаунта» перезапускает процесс с той же историей, удаление аккаунта снимает умолчание. Тратит немного токенов.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET", body=None):
    req = urllib.request.Request(f"https://{HOST}/api{path}", method=method, data=json.dumps(body).encode() if body is not None else None, headers={"content-type": "application/json"})
    try:
        r = urllib.request.urlopen(req, context=ctx_ssl, timeout=120); t = r.read(); return r.status, (json.loads(t) if t else None)
    except urllib.error.HTTPError as e: return e.code, None
def wait_result(sid, after_seq=0, timeout=150):
    t0 = time.time()
    while time.time() - t0 < timeout:
        st, evs = api(f"/projects/{P}/sessions/{sid}/events?since={after_seq}&limit=5000")
        r = [e for e in (evs or []) if e["kind"] == "result"]
        if r: return r[-1]
        time.sleep(1)
    return None
MARK = f"acc-{int(time.time())}"
# 1. импорт gate-аккаунта из гранта llm проекта
st, a = api("/accounts/import", "POST", {"provider": "gate", "project": P, "label": f"gate {P} ({MARK})"}); ok("import gate account", st == 200 and a and a["kind"] == "proxy" and "ANTHROPIC_BASE_URL" in a["env_keys"], json.dumps(a)[:120] if a else str(st)); aid = a["id"]
ok("list: account present, values hidden", any(x["id"] == aid for x in api("/accounts")[1]["accounts"]) and "ANTHROPIC_AUTH_TOKEN" not in json.dumps(api("/accounts")[1]).replace("ANTHROPIC_AUTH_TOKEN\"", ""))
ok("bad import rejected", api("/accounts/import", "POST", {"provider": "claude", "project": P})[0] in (404, 500), f"в {P} нет входа claude.ai — так и должно быть")
# 2. умолчание проекта и сессия с аккаунтом
ok("project default set", api(f"/projects/{P}/account", "PUT", {"account": aid})[0] == 204 and api("/accounts")[1]["defaults"].get(P) == aid)
st, s = api(f"/projects/{P}/sessions", "POST", {"agent": "claude", "title": MARK}); ok("session pinned to default account", st == 200 and s and s.get("account") == aid, json.dumps({k: s.get(k) for k in ("account", "env", "unset")}) if s else str(st)); sid = s["id"]
ok("env values hidden in session info", all(v == "•••" for v in (s.get("env") or {}).values()) and "ANTHROPIC_BASE_URL" in (s.get("env") or {}))
api(f"/projects/{P}/sessions/{sid}/prompt", "POST", {"content": f"Reply with exactly: {MARK}"})
r = wait_result(sid); ok("turn works through the account env", r is not None and not r.get("is_error") and MARK in (r.get("text") or ""), (r or {}).get("text", "")[:60] if r else "нет result")
# 3. переназначение на «без аккаунта» — перезапуск с той же историей
seq = r["seq"] if r else 0
st, s2 = api(f"/projects/{P}/sessions/{sid}/account", "POST", {"account": None}); ok("reassign restarts session", st == 200 and s2 and s2["id"] == sid and s2.get("account") in (None, ""), json.dumps({k: s2.get(k) for k in ("account", "status")}) if s2 else str(st))
api(f"/projects/{P}/sessions/{sid}/prompt", "POST", {"content": "What was the exact marker string I asked you to reply with? Reply with just the string."})
r2 = wait_result(sid, seq); ok("history survives reassign", r2 is not None and MARK in (r2.get("text") or ""), (r2 or {}).get("text", "")[:60] if r2 else "нет result")
# 4. уборка: сессия, аккаунт, умолчание
api(f"/projects/{P}/sessions/{sid}", "DELETE"); time.sleep(3); api(f"/projects/{P}/sessions/{sid}", "DELETE")
ok("account deleted", api(f"/accounts/{aid}", "DELETE")[0] == 204 and not any(x["id"] == aid for x in api("/accounts")[1]["accounts"]))
ok("project default cleared with the account", api("/accounts")[1]["defaults"].get(P) is None)
print("ALL PASS" if all(res) else "SOME FAIL")
