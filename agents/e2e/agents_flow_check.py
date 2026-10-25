import os, re, time, json, ssl, urllib.request, random, string
from playwright.sync_api import sync_playwright
# The "pair of agents" scenario (docs/qa-plan.md §3): "Reviewer" on Claude is created through the UI form, "Writer" on Gemini CLI
# (through the gate) — through the API; routines (manual, webhook, cron), mail owner → A → B → A's memory, reflection, "Inbox",
# screen checks on desktop and phone; everything is cleaned up at the end.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note, f"[{time.strftime('%H:%M:%S')}]", flush=True)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def raw(path, method="GET", body=None, ctype="application/json"):
    try:
        r = urllib.request.urlopen(urllib.request.Request(f"https://{HOST}{path}", data=body, method=method, headers={"Content-Type": ctype} if body is not None else {}), context=ctx_ssl, timeout=120); return r.status, r.read()
    except urllib.error.HTTPError as e: return e.code, e.read()
def get(path):
    try:
        r = urllib.request.urlopen(urllib.request.Request(f"https://{HOST}{path}"), context=ctx_ssl, timeout=60); t = r.read(); return r.status, (json.loads(t) if t and r.headers.get_content_type() == "application/json" else t[:200])
    except urllib.error.HTTPError as e: return e.code, None
def jpost(path, body): st, b = raw(path, "POST", json.dumps(body).encode()); return st, (json.loads(b or b"{}") if b else {})
def agent_by(name_or_id):
    for a in (get("/agents/api/agents")[1] or {}).get("agents", []):
        if a["id"] == name_or_id or a["name"] == name_or_id: return a
    return {}
def wait_run(rid, secs=180):
    r = None
    for _ in range(max(1, secs // 3)):
        time.sleep(3); r = next((x for x in (get("/agents/api/runs?limit=60")[1] or {}).get("runs", []) if x["id"] == rid), None)
        if r and r["status"] != "running": break
    return r or {}
MARK = "flow-" + "".join(random.choice(string.ascii_lowercase) for _ in range(6))
A_NAME, B_NAME = f"Ревьюер {MARK}", f"Писатель {MARK}"
cleanup = {"agents": [], "routines": [], "runs": [], "sessions": []}

def scenario(p):
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1500, "height": 900})
    ctx.add_init_script("try { if (!localStorage.getItem('agents.lang')) localStorage.setItem('agents.lang', 'ru'); localStorage.removeItem('agents.ui'); } catch (e) {}")
    pg = ctx.new_page(); errors = []; pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"https://{HOST}/agents/", wait_until="load"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(1.5)
    # ---- agent A through the UI form ----
    pg.locator(".rail .nav[data-tip='Агенты']").click(); pg.wait_for_selector(".top .seg button:has-text('Агенты')", timeout=20000); time.sleep(0.8)
    pg.locator(".top .seg button:has-text('Агенты')").click(); time.sleep(0.4); pg.locator(".top button.primary:has-text('Новый агент')").click(); pg.wait_for_selector(".rt-form", timeout=10000); time.sleep(0.5)
    form = pg.locator(".rt-form")
    form.locator("input.sm").first.fill(A_NAME)
    form.locator("input[name='prov'][value='claude']").check(force=True)
    ws_sel = form.locator("select").filter(has=pg.locator(f"option[value='{P}']")).first
    if ws_sel.count(): ws_sel.select_option(P)
    form.locator("textarea:not(.memory)").first.fill("Ты ревьюер кода в проекте. Отвечай коротко и по делу; инструменты используй только когда просят. Пиши по-русски.")
    form.locator("input.place").fill(f"review-{MARK}")
    if not form.locator("input.can-mail").is_checked(): form.locator("input.can-mail").check(force=True)
    sk = form.locator(".skills input[data-skill='report']")
    if sk.count() and not sk.is_checked(): sk.check(force=True)
    form.locator(".acts button.primary").first.click(); time.sleep(2.5)
    agA = agent_by(A_NAME); cleanup["agents"].append(agA.get("id"))
    ok("ui: agent A created through the form — name, Claude, workspace, brief, place, mail and a skill saved", bool(agA.get("id")) and agA.get("provider") == "claude" and agA.get("workspace") == P and agA.get("place") == f"review-{MARK}" and agA.get("can_mail") is True and ("report" in (agA.get("skills") or []) or not sk.count()), str({k: agA.get(k) for k in ("id", "provider", "workspace", "place", "can_mail", "skills")})[:200])
    ok("ui: the new agent appears in the list and its card is selected", pg.locator(f".rt-item[data-agent='{agA.get('id')}']").count() == 1 and pg.locator(".rt-form input.sm").first.input_value() == A_NAME)
    pg.screenshot(path="shots/flow1-agent-a.png")
    # ---- agent B (Gemini CLI through the gate) through
    # the API ----
    st_b, agB = jpost("/agents/api/agents", {"name": B_NAME, "brief": "Ты технический писатель. Отвечай одной короткой строкой по-русски; инструменты не используй, если не просят.", "provider": "gemini", "workspace": P, "mode": "auto", "can_mail": True, "remember": True, "place": f"docs-{MARK}", "skills": ["lean-code"]})
    cleanup["agents"].append(agB.get("id"))
    ok("api: agent B created on Gemini CLI with place, mail and a skill", st_b == 200 and agB.get("id", "").startswith("ag_") and agB.get("provider") == "gemini" and agB.get("place") == f"docs-{MARK}", f"{st_b} {str(agB)[:160]}")
    pg.reload(wait_until="load"); pg.wait_for_selector(".app .rail", timeout=30000); time.sleep(1)
    pg.locator(".rail .nav[data-tip='Агенты']").click(); pg.wait_for_selector(".rt", timeout=20000); time.sleep(0.8)
    pg.locator(f".rt-item[data-agent='{agB.get('id')}']").click(); time.sleep(0.8)
    ok("ui: agent B's card shows the Gemini CLI provider and its place", pg.locator(".rt-form input[name='prov'][value='gemini']").is_checked() and pg.locator(".rt-form input.place").input_value() == f"docs-{MARK}" and pg.locator(".rt-item[data-agent]").count() >= 2)
    # ---- routines: A manually, B by webhook
    # ----
    st_r1, rtA = jpost("/agents/api/routines", {"name": f"Ревью {MARK}", "agent": agA.get("id"), "prompt": f"Reply with exactly: review-{MARK}. Do not run tools.", "trigger": {"kind": "manual"}, "mode": "auto"}); cleanup["routines"].append(rtA.get("id"))
    st_r2, rtB = jpost("/agents/api/routines", {"name": f"Текст {MARK}", "agent": agB.get("id"), "prompt": f"Reply with exactly: write-{MARK} and then the value of field x from this payload: {{payload}}. Do not run tools.", "trigger": {"kind": "webhook", "cron": "", "event": "", "contains": ""}, "mode": "auto"}); cleanup["routines"].append(rtB.get("id"))
    ok("api: routines for both agents — manual for A, webhook with a token for B", st_r1 == 200 and rtA.get("id") and st_r2 == 200 and rtB.get("token", "").startswith("rt_"), f"{st_r1} {st_r2}")
    st_run, rr = jpost(f"/agents/api/routines/{rtA.get('id')}/run", {}); rA = rr.get("run", {}); cleanup["runs"].append(rA.get("id")); cleanup["sessions"].append(rA.get("session"))
    st_h, hk = jpost(f"/agents/api/hooks/{rtB.get('token')}", {"x": f"payload-{MARK}"}); cleanup["runs"].append(hk.get("run"))
    ok("api: A's manual run and B's webhook run start in the agents' places", st_run == 200 and rA.get("status") == "running" and st_h == 202 and hk.get("status") == "running", f"{st_run} {st_h}")
    rAf = wait_run(rA.get("id")); rBf = wait_run(hk.get("run")); cleanup["sessions"].append(rBf.get("session"))
    sA = (get(f"/agents/api/core/projects/{P}/sessions/{rA.get('session')}")[1] or {}).get("info", {}); sB = (get(f"/agents/api/core/projects/{P}/sessions/{rBf.get('session')}")[1] or {}).get("info", {})
    ok("run: A (Claude) finished with the marker in the summary, in its place", rAf.get("status") == "done" and f"review-{MARK}" in (rAf.get("summary") or "") and sA.get("cwd", "").endswith(f"review-{MARK}"), f"{rAf.get('status')} {str(rAf.get('summary'))[:80]} cwd={sA.get('cwd')}")
    ok("run: B (Gemini CLI via the gate) finished with the marker and the payload, session agent gemini, in its place", rBf.get("status") == "done" and f"write-{MARK}" in (rBf.get("summary") or "") and f"payload-{MARK}" in (rBf.get("summary") or "") and sB.get("agent") == "gemini" and sB.get("cwd", "").endswith(f"docs-{MARK}"), f"{rBf.get('status')} {str(rBf.get('summary'))[:100]} agent={sB.get('agent')} cwd={sB.get('cwd')}")
    # ---- mail: owner → A; A → B by the "ПОЧТА:" rule (Gemini's answer into A's memory)
    # ----
    st_ml, ml = jpost(f"/agents/api/agents/{agA.get('id')}/mail", {"text": f"Reply with exactly: hello-{MARK}"}); ml = ml.get("run", {}); cleanup["runs"].append(ml.get("id")); cleanup["sessions"].append(ml.get("session"))
    mlf = wait_run(ml.get("id"))
    ok("mail: the owner's letter to A is answered", st_ml == 200 and mlf.get("status") == "done" and f"hello-{MARK}" in (mlf.get("summary") or ""), str(mlf.get("summary"))[:80])
    raw(f"/agents/api/routines/{rtA.get('id')}", "PUT", json.dumps({"prompt": f"Reply with exactly: chain-{MARK}. Then add a separate last line exactly like this: ПОЧТА: {B_NAME}: Reply with exactly: pong-{MARK}", "mode": "auto"}).encode())
    st_r5, r5 = jpost(f"/agents/api/routines/{rtA.get('id')}/run", {}); r5 = r5.get("run", {}); cleanup["runs"].append(r5.get("id")); cleanup["sessions"].append(r5.get("session")); r5f = wait_run(r5.get("id"))
    rB2 = None
    for _ in range(25):
        time.sleep(3); rB2 = next((x for x in (get("/agents/api/runs?limit=60")[1] or {}).get("runs", []) if x.get("agent") == agB.get("id") and x.get("reply_to") == r5.get("id")), None)
        if rB2: break
    if rB2: cleanup["runs"].append(rB2["id"]); cleanup["sessions"].append(rB2.get("session"))
    rB2f = wait_run((rB2 or {}).get("id", "none"))
    agA2 = {}
    for _ in range(12):
        time.sleep(2); agA2 = agent_by(agA.get("id"))
        if f"pong-{MARK}" in agA2.get("memory", ""): break
    ok("mail: A's «ПОЧТА:» line starts a mail run for B (Gemini) and B's reply lands in A's memory", r5f.get("status") == "done" and bool(rB2) and rB2f.get("status") == "done" and f"pong-{MARK}" in (rB2f.get("summary") or "") and f"pong-{MARK}" in agA2.get("memory", ""), f"A={r5f.get('status')} B={rB2f.get('status')} {str(rB2f.get('summary'))[:60]} mem={agA2.get('memory', '')[-80:]!r}")
    # ---- cron on B: the nearest minute ----
    st_c, _ = raw(f"/agents/api/routines/{rtB.get('id')}", "PUT", json.dumps({"trigger": {"kind": "schedule", "cron": "* * * * *", "event": "", "contains": ""}, "prompt": f"Reply with exactly: tick-{MARK}. Do not run tools."}).encode())
    r3 = None
    for _ in range(32):
        time.sleep(3); r3 = next((x for x in (get(f"/agents/api/runs?routine={rtB.get('id')}&limit=10")[1] or {}).get("runs", []) if x["trigger"] == "schedule"), None)
        if r3: break
    raw(f"/agents/api/routines/{rtB.get('id')}", "PUT", json.dumps({"enabled": False}).encode())
    if r3: cleanup["runs"].append(r3["id"]); cleanup["sessions"].append(r3.get("session"))
    r3f = wait_run((r3 or {}).get("id", "none"), 120)
    ok("cron: the scheduler fires B's every-minute routine (Gemini) and the run completes", st_c == 200 and bool(r3) and r3f.get("status") == "done" and f"tick-{MARK}" in (r3f.get("summary") or ""), str(r3f)[:160] if r3 else "no scheduled run in 96 s")
    # ---- reflection on A ----
    raw(f"/agents/api/agents/{agA.get('id')}", "PUT", json.dumps({"reflect": True, "memory_budget": 1500}).encode()); t_refl = time.time()
    st_r6, r6 = jpost(f"/agents/api/agents/{agA.get('id')}/run", {"prompt": f"Reply with exactly: reflect-{MARK}. Also state that the review queue is empty today. Do not run tools."}); r6 = r6.get("run", {}); cleanup["runs"].append(r6.get("id")); cleanup["sessions"].append(r6.get("session")); r6f = wait_run(r6.get("id"))
    agR = {}
    for _ in range(30):
        time.sleep(3); agR = agent_by(agA.get("id"))
        if (agR.get("reflected_at") or 0) >= int(t_refl): break
    mlines = [l for l in agR.get("memory", "").splitlines() if l.strip()]
    ok("reflection: A's memory is rewritten into dated lines within the budget after the run", r6f.get("status") == "done" and (agR.get("reflected_at") or 0) >= int(t_refl) and mlines and all(re.match(r"^\d{4}-\d{2}-\d{2}:", l) for l in mlines) and len(agR.get("memory", "")) <= 1500, f"lines={len(mlines)} {agR.get('memory', '')[:100]!r}")
    # ---- inbox and desktop screens ----
    st_i, inbox = get("/agents/api/inbox"); done_titles = [((x.get("session") or {}).get("title") or "") for x in (inbox or {}).get("done", [])]
    ok("inbox: finished run sessions of both agents are listed as done (titled after the routines)", st_i == 200 and any(f"Ревью {MARK}" in t_ for t_ in done_titles) and any(f"Текст {MARK}" in t_ for t_ in done_titles), f"done={len(done_titles)} {[t_[:30] for t_ in done_titles if MARK in t_][:3]}")
    pg.locator(".rail .nav[data-tip='Агенты']").click(); time.sleep(0.8); pg.locator(".top .seg button:has-text('Запуски')").click(); time.sleep(1)
    rows = pg.locator(".runs .tr"); row_txt = " ".join(rows.all_inner_texts())
    ok("ui: the runs tab lists runs of both agents including the Gemini ones", rows.count() >= 5 and A_NAME in row_txt and B_NAME in row_txt and ("вебхук" in row_txt.lower() or "webhook" in row_txt.lower()), f"rows={rows.count()}")
    pg.screenshot(path="shots/flow2-runs.png")
    pg.locator(f".runs .tr[data-run='{rBf.get('id')}'] button, .runs .tr[data-run='{rBf.get('id')}'] a").first.click() if pg.locator(f".runs .tr[data-run='{rBf.get('id')}'] button").count() else None; time.sleep(1.5)
    opened = pg.locator(".thread .t-head .av.gemini").count() >= 1 and f"Текст {MARK}" in (pg.locator(".thread .t-head h2").inner_text() if pg.locator(".thread .t-head h2").count() else "")
    ok("ui: «open session» from B's run shows the Gemini session in Thread", opened, pg.locator(".thread .t-head h2").inner_text()[:60] if pg.locator(".thread .t-head h2").count() else "no thread")
    pg.locator(".rail .nav[data-tip='Входящие']").click()
    try: pg.wait_for_selector("main.main .item", timeout=30000)
    except Exception: pass
    time.sleep(0.5)
    ok("ui: the inbox shows the agents' finished run sessions", MARK in pg.locator("main.main").inner_text(), pg.locator("main.main").inner_text()[:80].replace("\n", " "))
    pg.screenshot(path="shots/flow3-inbox.png")
    ok("ui: no page errors on desktop", not errors, str(errors[:2])[:160]); ctx.close()
    # ---- phone ----
    ctx2 = b.new_context(**p.devices["iPhone 13"], locale="ru-RU", ignore_https_errors=True)
    ctx2.add_init_script("try { if (!localStorage.getItem('agents.lang')) localStorage.setItem('agents.lang', 'ru'); localStorage.removeItem('agents.ui'); } catch (e) {}")
    pp = ctx2.new_page(); perr = []; pp.on("pageerror", lambda e: perr.append(str(e)))
    pp.goto(f"https://{HOST}/agents/", wait_until="load"); pp.wait_for_selector(".app", timeout=30000); time.sleep(1.5)
    pp.locator(".tabbar .tb:has-text('Ещё')").click(); pp.wait_for_selector(".sheet .mi", timeout=5000); pp.locator(".sheet .mi:has-text('Агенты')").click(); time.sleep(1)
    try: pp.wait_for_selector(f".rt-item[data-agent='{agA.get('id')}']", timeout=15000)
    except Exception: pass
    item = pp.locator(f".rt-item[data-agent='{agA.get('id')}']")
    ok("phone: the agents list is reachable without a drawer and the item is visible", item.count() == 1 and item.is_visible(), f"count={item.count()}")
    item.scroll_into_view_if_needed(); item.click(timeout=10000); time.sleep(0.8)
    fits = pp.evaluate("document.documentElement.scrollWidth <= window.innerWidth + 1")
    ok("phone: the agents section lists both agents; A's card opens and fits the screen", pp.locator(".rt-form input.sm").first.input_value() == A_NAME and fits, f"fits={fits}")
    pp.screenshot(path="shots/flow4-phone-agent.png")
    pp.locator("main.main .top .seg button:has-text('Запуски')").click(); time.sleep(1)
    ok("phone: the runs tab lists the runs and fits", pp.locator(".runs .tr").count() >= 3 and pp.evaluate("document.documentElement.scrollWidth <= window.innerWidth + 1"))
    pp.screenshot(path="shots/flow5-phone-runs.png")
    pp.locator(".tabbar .tb:has-text('Входящие')").click(); time.sleep(1.2)
    ok("phone: the inbox renders and fits", pp.locator("main.main").count() == 1 and pp.evaluate("document.documentElement.scrollWidth <= window.innerWidth + 1"))
    ok("phone: no page errors", not perr, str(perr[:2])[:160]); ctx2.close(); b.close()

with sync_playwright() as p:
    try: scenario(p)
    except Exception as e: ok("scenario completed without exceptions", False, str(e).split("\n")[0][:220])
# ---- cleanup ----
for rid in cleanup["routines"]:
    if rid: raw(f"/agents/api/routines/{rid}", "DELETE")
for x in cleanup["runs"]:
    if x: raw(f"/agents/api/runs/{x}", "DELETE")
for sid in set(s for s in cleanup["sessions"] if s):
    for _ in range(3):
        if raw(f"/agents/api/core/projects/{P}/sessions/{sid}", "DELETE")[0] in (204, 404): break
        time.sleep(2)
for aid in cleanup["agents"]:
    if aid: raw(f"/agents/api/agents/{aid}", "DELETE")
raw(f"/agents/api/core/projects/{P}/fs?path=review-{MARK}&recursive=true", "DELETE"); raw(f"/agents/api/core/projects/{P}/fs?path=docs-{MARK}&recursive=true", "DELETE")
left = [a for a in (get("/agents/api/agents")[1] or {}).get("agents", []) if a["id"] in cleanup["agents"]]
ok("cleanup: agents, routines, runs and sessions removed", not left)
print("ALL PASS" if all(res) else "SOME FAIL")
