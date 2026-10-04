import os, time, json, ssl, subprocess, urllib.request
# Уведомления Agent Runtime (ADR-006): сессия встаёт на разрешение → ядро замечает переход при опросе → событие «агент ждёт
# решения» в ленте и, если Desktop «ушёл» (нет видимой вкладки 90 с), push в тему ntfy `cloudos` от системного пользователя.
# Запускать на стенде (нужен docker для чтения токена подписки). Тратит немного токенов.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def api(path, method="GET", body=None):
    req = urllib.request.Request(f"https://{HOST}/api{path}", method=method, data=json.dumps(body).encode() if body is not None else None, headers={"content-type": "application/json"})
    try:
        r = urllib.request.urlopen(req, context=ctx_ssl, timeout=90); t = r.read(); return r.status, (json.loads(t) if t else None)
    except urllib.error.HTTPError as e: return e.code, None
def wait_status(sid, want, timeout=150):
    t0 = time.time()
    while time.time() - t0 < timeout:
        st, info = api(f"/projects/{P}/sessions/{sid}"); s = (info or {}).get("info", {}).get("status")
        if s == want: return True
        time.sleep(2)
    return False

MARK = f"notify-{int(time.time())}"
away = (api("/agents")[1] or {}).get("away"); print("Desktop away:", away, "(push уходит только когда away)")
st, s = api(f"/projects/{P}/sessions", "POST", {"agent": "claude", "title": MARK}); ok("session created", st == 200, json.dumps(s)[:80]); sid = s["id"]
api(f"/projects/{P}/sessions/{sid}/prompt", "POST", {"content": f"Create the file /home/coder/{MARK}.txt containing hello using the Write tool. Do not ask questions."})
ok("session is waiting for permission", wait_status(sid, "waiting"))
# ядро опрашивает раз в 20 с: ждём переход в ленте
deadline = time.time() + 70; ev = None
while time.time() < deadline and not ev:
    ev = next((e for e in (api("/events")[1] or []) if e["kind"] == "agent" and e["subject"] == P and "ждёт" in e["message"] and MARK in e["message"]), None)
    time.sleep(5)
ok("feed: 'агент ждёт решения' with session title", ev is not None, json.dumps(ev, ensure_ascii=False)[:120] if ev else "нет события")
away = (api("/agents")[1] or {}).get("away")
if away:
    ip = subprocess.run(["docker", "inspect", "-f", "{{range .NetworkSettings.Networks}}{{.IPAddress}} {{end}}", "ntfy-ntfy-1"], capture_output=True, text=True).stdout.split()[0]
    tokens = subprocess.run(["docker", "exec", "ntfy-ntfy-1", "ntfy", "token", "list", "cloudos"], capture_output=True, text=True).stdout
    tok = next((w for w in tokens.replace(",", " ").replace(")", " ").split() if w.startswith("tk_")), "")
    ok("ntfy: system user cloudos has a token", bool(tok))
    req = urllib.request.Request(f"http://{ip}:80/cloudos/json?poll=1&since=10m", headers={"authorization": f"Bearer {tok}"})
    msgs = urllib.request.urlopen(req, timeout=15).read().decode().strip().splitlines() if tok else []
    ok("ntfy: push delivered to topic cloudos", any(MARK in m for m in msgs), f"сообщений: {len(msgs)}")
else:
    ok("push skipped: Desktop is present (visible tab polling) — by design", True)
# ответить «отклонить», остановить и убрать
st, info = api(f"/projects/{P}/sessions/{sid}"); pend = (info or {}).get("pending") or []
if pend: api(f"/projects/{P}/sessions/{sid}/permission", "POST", {"request_id": pend[0]["request_id"], "behavior": "deny", "message": "test over"})
time.sleep(5); api(f"/projects/{P}/sessions/{sid}", "DELETE"); time.sleep(3); api(f"/projects/{P}/sessions/{sid}", "DELETE")
ok("cleaned up", api(f"/projects/{P}/sessions/{sid}")[0] == 404)
print("ALL PASS" if all(res) else "SOME FAIL")
