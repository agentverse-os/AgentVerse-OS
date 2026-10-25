import json, os, sys, time, urllib.request
import websocket  # pip install websocket-client
# Smoke check of cloudos-agentd on a live workspace: create a terminal, attach over WebSocket, run a command, detach
# and attach again (the output tail must survive — the process is not tied to the client), resize, kill. Run:
# AGENTD=http://10.77.7.146:7200 AGENTD_TOKEN=… python3 smoke.py
BASE = os.environ.get("AGENTD", "http://127.0.0.1:7200"); TOKEN = os.environ.get("AGENTD_TOKEN", ""); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note)
def api(path, method="GET", body=None):
    req = urllib.request.Request(BASE + path, method=method, data=json.dumps(body).encode() if body is not None else None,
                                 headers={"content-type": "application/json", "authorization": f"Bearer {TOKEN}"})
    try:
        r = urllib.request.urlopen(req, timeout=15); t = r.read(); return r.status, (json.loads(t) if t else None)
    except urllib.error.HTTPError as e: return e.code, None
def attach(pid, wait_for=None, send=None, timeout=8):
    ws = websocket.create_connection(BASE.replace("http", "ws", 1) + f"/v1/ptys/{pid}/ws?token={TOKEN}", timeout=timeout)
    hello = json.loads(ws.recv()); out = b""; t0 = time.time()
    if send: time.sleep(0.4); ws.send_binary(send.encode())
    while time.time() - t0 < timeout:
        try: frame = ws.recv_frame()
        except websocket.WebSocketTimeoutException: break
        if frame.opcode == websocket.ABNF.OPCODE_BINARY: out += frame.data
        elif frame.opcode == websocket.ABNF.OPCODE_TEXT and json.loads(frame.data).get("t") == "exit": break
        elif frame.opcode == websocket.ABNF.OPCODE_CLOSE: break
        if wait_for and wait_for.encode() in out: break
    ws.close(); return hello, out

st, h = api("/health"); ok("health", st == 200 and h and h.get("protocol") == 1, json.dumps(h, ensure_ascii=False)[:160])
ok("list with token", api("/v1/ptys")[0] == 200)
try:
    urllib.request.urlopen(urllib.request.Request(BASE + "/v1/ptys", headers={"authorization": "Bearer wrong"}), timeout=10); ok("wrong token rejected", False)
except urllib.error.HTTPError as e: ok("wrong token rejected", e.code == 401, str(e.code))
st, p = api("/v1/ptys", "POST", {"title": "smoke", "cols": 100, "rows": 30}); ok("pty created", st == 200 and p and p["alive"], json.dumps(p)[:120]); pid = p["id"]
hello, out = attach(pid, wait_for="agentd-ok 42", send="echo agentd-ok $((6*7))\n")  # wait for the result, not the echo of the typed line
ok("command ran, output streamed", b"agentd-ok 42" in out, out[-80:].decode(errors="replace").strip())
ok("hello frame", hello.get("t") == "hello" and hello.get("cols") == 100)
# detached — the process is alive, the
# tail is intact
time.sleep(0.5); st, info = api(f"/v1/ptys/{pid}"); ok("survives client disconnect", info and info["alive"] and info["clients"] == 0, json.dumps(info)[:100])
hello2, replay = attach(pid, timeout=2)
ok("reattach replays scrollback", hello2.get("replay", 0) > 0 and b"agentd-ok 42" in replay, f"replay={hello2.get('replay')}")
st, info = api(f"/v1/ptys/{pid}/resize", "POST", {"cols": 80, "rows": 24}); ok("resize", st == 200 and info["cols"] == 80)
_, out2 = attach(pid, wait_for="cols=80", send="echo cols=$(tput cols)\n"); ok("pty size applied", b"cols=80" in out2)
ok("shell has gate env", b"ANTHROPIC_BASE_URL=" in attach(pid, wait_for="ANTHROPIC_BASE_URL=", send="env | grep -o '^ANTHROPIC_BASE_URL=' ; echo done\n")[1])
st, _ = api(f"/v1/ptys/{pid}", "DELETE"); time.sleep(0.8); st2, info = api(f"/v1/ptys/{pid}")
ok("kill", st == 202 and info and not info["alive"], json.dumps(info)[:100])
st, _ = api(f"/v1/ptys/{pid}", "DELETE"); ok("remove after exit", st == 204 and api(f"/v1/ptys/{pid}")[0] == 404)
print("ALL PASS" if all(res) else "SOME FAIL"); sys.exit(0 if all(res) else 1)
