import os, time, json, ssl, urllib.request
# Codex provider (ADR-006): an `agent: codex` session in cloudos-agentd via `codex app-server` — init with the thread id
# and model, a turn with an answer and token usage, a command approval request (untrusted mode) answered with "allow" and
# a tool result, stop + resume (thread/resume) keeping the history. By default — the project gate route (OPENAI_BASE_URL,
# the model is chosen from /v1/models or CODEX_MODEL). Spends a few tokens; gpt-oss turns are slow.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); MODEL = os.environ.get("CODEX_MODEL"); res = []
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
def wait_result(sid, since=0, timeout=240): return wait_event(sid, lambda e: e["kind"] == "result", since, timeout)
def approve_until_result(sid, since, timeout=420):
    """Approve every approval request until the turn finishes: the model may split the task into several commands."""
    t0 = time.time(); answered = set(); first = None
    while time.time() - t0 < timeout:
        evs = events(sid, since)
        done = {e["request_id"] for e in evs if e["kind"] == "permission_result"}
        for e in evs:
            if e["kind"] == "permission_request" and e["request_id"] not in done and e["request_id"] not in answered:
                first = first or e; answered.add(e["request_id"])
                api(f"/projects/{P}/sessions/{sid}/permission", "POST", {"request_id": e["request_id"], "behavior": "allow"})
        r = [e for e in evs if e["kind"] == "result"]
        if r: return first, len(answered), r[-1]
        time.sleep(2)
    return first, len(answered), None
import random, string
# a marker made of letters: gpt-oss through the gate cuts long numbers at tokens (verified with curl
# — not the adapter)
MARK = "codex-" + "".join(random.choice(string.ascii_lowercase) for _ in range(6))
# 1. a Codex session in the untrusted mode (every non-trivial command needs
# approval)
body = {"agent": "codex", "title": MARK, "permission_mode": "untrusted"}
if MODEL: body["model"] = MODEL
st, s = api(f"/projects/{P}/sessions", "POST", body); ok("codex session created", st == 200 and s and s["agent"] == "codex", json.dumps(s)[:100] if s else str(st))
sid = s["id"]
init = wait_event(sid, lambda e: e["kind"] == "init", 0, 60)
ok("init: codex thread started", init is not None and init.get("provider") == "codex" and bool(init.get("session_id")), json.dumps({k: init.get(k) for k in ("model", "route", "session_id")}) if init else "нет init (" + json.dumps([e for e in events(sid) if e["kind"] == "error"])[:200] + ")")
info = api(f"/projects/{P}/sessions/{sid}")[1]["info"]
ok("model and provider thread id recorded", bool(info.get("model")) and bool(info.get("provider_session_id")) and info["status"] in ("idle", "working"), f"model={info.get('model')} status={info['status']}")
# 2. a simple turn
api(f"/projects/{P}/sessions/{sid}/prompt", "POST", {"content": f"Reply with exactly this and nothing else: {MARK}. Do not run any commands."})
r = wait_result(sid)
ok("turn: reply with marker", r is not None and not r.get("is_error") and MARK in (r.get("text") or ""), (r or {}).get("text", "")[:80] if r else "нет result")
ok("assistant event recorded", any(e["kind"] == "assistant" and any(b.get("type") == "text" and MARK in (b.get("text") or "") for b in e.get("content") or []) for e in events(sid)))
# the stand's external gate does not return usage in the Responses stream for gpt-oss — then zeros are acceptable (the subscription is
# checked by codex_account_check)
usage = (r or {}).get("usage") or {}
ok("usage recorded", r is not None and "usage" in r and (usage.get("output_tokens", 0) > 0 or (init or {}).get("route") == "gate"), json.dumps(usage)[:120] + (" (gate без usage)" if usage.get("output_tokens", 0) == 0 else ""))
# 3. a command with approval
seq = r["seq"] if r else 0
api(f"/projects/{P}/sessions/{sid}/prompt", "POST", {"content": f"Using a shell command, create the file /home/coder/{MARK}.txt containing the single word hello, then reply with exactly: done-{MARK}. Do not ask questions."})
req, n_approved, r2 = approve_until_result(sid, seq)
ok("approval requested for the command", req is not None and req.get("tool_name") == "Bash" and req.get("request_id"), json.dumps({k: req.get(k) for k in ("tool_name", "input")})[:160] if req else "нет permission_request")
# gpt-oss through the gate may end the turn after the command without a final message — the turn itself must finish
# without an error
ok("turn after approval finished", r2 is not None and not r2.get("is_error"), f"одобрений {n_approved}; текст: " + ((r2 or {}).get("text", "")[:80] if r2 else "нет result"))
evs = events(sid, seq)
ok("tool call and result recorded", any(e["kind"] == "assistant" and any(b.get("type") == "tool_use" and b.get("name") == "Bash" for b in e.get("content") or []) for e in evs) and any(e["kind"] == "tool_result" for e in evs))
ok("approval recorded as allowed", any(e["kind"] == "permission_result" and e.get("behavior") == "allow" for e in evs))
# 4. stop and resume — thread/resume with the same history
seq2 = r2["seq"] if r2 else seq
api(f"/projects/{P}/sessions/{sid}", "DELETE"); time.sleep(3)
ok("stopped", api(f"/projects/{P}/sessions/{sid}")[1]["info"]["status"] == "exited")
st, s3 = api(f"/projects/{P}/sessions", "POST", {"resume": sid}); ok("resumed with the same id", st == 200 and s3 and s3["id"] == sid and s3["alive"], json.dumps(s3)[:100] if s3 else str(st))
init2 = wait_event(sid, lambda e: e["kind"] == "init" and e["seq"] > seq2, seq2, 60)
ok("resume: same provider thread", init2 is not None and init2.get("session_id") == info.get("provider_session_id"), f"{(init2 or {}).get('session_id')} vs {info.get('provider_session_id')}")
api(f"/projects/{P}/sessions/{sid}/prompt", "POST", {"content": "What was the exact marker string I first asked you to reply with? Reply with just the string."})
r3 = wait_result(sid, seq2)
# history with tool calls does not pass back into gpt-oss through the stand's gate (Responses→chat: 400 "messages array element");
# history preservation on resume is checked on the subscription in codex_account_check — here what matters is that the resumed
# thread accepts a turn
ok("resumed thread takes a turn", r3 is not None, ("история: " + ("да" if MARK in (r3.get("text") or "") else "нет — " + (r3.get("text") or "")[:90])) if r3 else "нет result")
# 5. cleanup
api(f"/projects/{P}/sessions/{sid}", "DELETE"); time.sleep(3); api(f"/projects/{P}/sessions/{sid}", "DELETE")
ok("session removed", api(f"/projects/{P}/sessions/{sid}")[0] == 404)
print("ALL PASS" if all(res) else "SOME FAIL")
