import os, time, json, ssl, urllib.request, random, string, tempfile
from playwright.sync_api import sync_playwright
# Session card renderings (ADR-006): the agentd files and git API through the core (list, write/read/download, folder, rename,
# delete, ".." rejected, git status/diff/log on a repository created via a PTY), then in Desktop: the Chat | Terminal | Files |
# Git tabs on a session, a file upload in the "Files" tab, the session shell (a PTY with session=id) and its cleanup together
# with the session, choosing the permission mode in the composer. No model turns — no tokens spent.
HOST = os.environ.get("EDGE_HOST", "box.tail1234.ts.net"); P = os.environ.get("PROJECT", "alpha"); os.makedirs("shots", exist_ok=True); res = []
def ok(n, c, note=""): res.append(bool(c)); print(("PASS  " if c else "FAIL  ") + n, note, flush=True)
ctx_ssl = ssl.create_default_context(); ctx_ssl.check_hostname = False; ctx_ssl.verify_mode = ssl.CERT_NONE
def raw(path, method="GET", data=None, headers=None):
    req = urllib.request.Request(f"https://{HOST}/api{path}", method=method, data=data, headers=headers or {})
    try:
        r = urllib.request.urlopen(req, context=ctx_ssl, timeout=120); return r.status, dict(r.headers), r.read()
    except urllib.error.HTTPError as e: return e.code, dict(e.headers), e.read()
def api(path, method="GET", body=None):
    st, h, t = raw(path, method, json.dumps(body).encode() if body is not None else None, {"content-type": "application/json"})
    try: return st, (json.loads(t) if t else None)
    except Exception: return st, None
MARK = "panes-" + "".join(random.choice(string.ascii_lowercase) for _ in range(6)); D = f"/home/coder/{MARK}"
# ---- files ----
st, l = api(f"/projects/{P}/fs?path="); ok("fs: home listed", st == 200 and l and l["path"] == "/home/coder" and isinstance(l["entries"], list), f"{st} {len((l or {}).get('entries', []))} записей")
ok("fs: write into a missing folder fails", raw(f"/projects/{P}/fs/file?path={D}/hello.txt", "PUT", b"x")[0] in (404, 400, 500))
ok("fs: mkdir", api(f"/projects/{P}/fs/mkdir", "POST", {"path": D})[0] == 200)
st, h, t = raw(f"/projects/{P}/fs/file?path={D}/hello.txt", "PUT", "привет, мир\n".encode()); ok("fs: write file", st == 200 and json.loads(t)["size"] == len("привет, мир\n".encode()), str(st))
st, h, t = raw(f"/projects/{P}/fs/file?path={D}/hello.txt"); ok("fs: read back with text content-type", st == 200 and t.decode() == "привет, мир\n" and "text/plain" in h.get("Content-Type", ""), h.get("Content-Type"))
st, h, t = raw(f"/projects/{P}/fs/file?path={D}/hello.txt&download=1"); ok("fs: download disposition", "attachment" in h.get("Content-Disposition", ""), h.get("Content-Disposition"))
ok("fs: rename", api(f"/projects/{P}/fs/rename", "POST", {"from": f"{D}/hello.txt", "to": f"{D}/hi.txt"})[0] == 200 and [e["name"] for e in api(f"/projects/{P}/fs?path={D}")[1]["entries"]] == ["hi.txt"])
ok("fs: '..' rejected", api(f"/projects/{P}/fs?path=/home/coder/../etc")[0] == 400)
ok("fs: delete file and folder", api(f"/projects/{P}/fs?path={D}/hi.txt", "DELETE")[0] == 204 and api(f"/projects/{P}/fs?path={D}", "DELETE")[0] == 204 and api(f"/projects/{P}/fs?path={D}")[0] == 404)
# ---- git: a repository via a PTY ----
G = f"/tmp/gtest-{MARK}"
st, pty = api(f"/projects/{P}/ptys", "POST", {"cmd": ["bash", "-lc", f"mkdir -p {G} && cd {G} && git init -q && echo a > a.txt && git add a.txt && git -c user.email=t@t -c user.name=tester commit -qm init && echo b >> a.txt && echo new > b.txt; exit 0"], "title": "git-setup"})
for _ in range(30):
    time.sleep(1)
    if not api(f"/projects/{P}/ptys/{pty['id']}")[1]["alive"]: break
st, g = api(f"/projects/{P}/git/status?cwd={G}"); ok("git: status of a repo", st == 200 and g and g["repo"] and g["head"] in ("master", "main") and {e["path"]: e["kind"] for e in g["entries"]} == {"a.txt": "changed", "b.txt": "untracked"}, json.dumps(g)[:200] if g else str(st))
st, d = api(f"/projects/{P}/git/diff?cwd={G}&path=a.txt"); ok("git: diff of a modified file", st == 200 and d and "+b" in d["diff"] and not d["truncated"])
st, d2 = api(f"/projects/{P}/git/diff?cwd={G}&path=b.txt&untracked=true"); ok("git: diff of an untracked file", st == 200 and d2 and "+new" in d2["diff"])
st, lg = api(f"/projects/{P}/git/log?cwd={G}"); ok("git: log", st == 200 and lg and len(lg["entries"]) == 1 and lg["entries"][0]["subject"] == "init" and lg["entries"][0]["author"] == "tester")
st, ng = api(f"/projects/{P}/git/status?cwd=/tmp"); ok("git: non-repo answers repo=false", st == 200 and ng and ng["repo"] is False)
api(f"/projects/{P}/fs?path={G}&recursive=true", "DELETE"); api(f"/projects/{P}/ptys/{pty['id']}", "DELETE")
# ---- Desktop ----
before = {s["id"] for s in (api(f"/projects/{P}/sessions")[1] or [])}
sid = None
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(locale="ru-RU", ignore_https_errors=True, viewport={"width": 1500, "height": 900}); pg = ctx.new_page()
    pg.goto(f"https://{HOST}/?app=agents", wait_until="networkidle"); pg.wait_for_selector(".agents", timeout=30000); time.sleep(1.5)
    pg.locator(".agents .modes button:text-is('Thread')").click()
    proj = pg.locator(f".agents .proj:has-text('{P}')"); proj.locator("button.add").click(); proj.locator(".menu button:text-is('💬 Claude')").click()
    pg.wait_for_selector(".agents .pane .chat", timeout=30000); time.sleep(2)
    new = [s for s in (api(f"/projects/{P}/sessions")[1] or []) if s["id"] not in before]; sid = new[0]["id"] if new else None
    tabs = [x.strip() for x in pg.locator(".agents .pane .tabs button").all_inner_texts()][:4]
    ok("pane: tabs Chat | Terminal | Files | Git", tabs == ["💬 Чат", "⌨ Терминал", "📁 Файлы", "⎇ Git"], str(tabs))
    ok("composer: model input and permission mode select", pg.locator(".chat .tools input.model").count() == 1 and pg.locator(".chat .tools select.mode").count() == 1)
    pg.locator(".chat .tools select.mode").select_option("plan"); time.sleep(2)
    ok("composer: permission mode applied to the session", sid is not None and api(f"/projects/{P}/sessions/{sid}")[1]["info"]["permission_mode"] == "plan")
    # Files: the home listing and a file
    # upload
    pg.locator(".agents .pane .tabs button:has-text('Файлы')").click(); pg.wait_for_selector(".files .list .row", timeout=20000)
    ok("files: home listed in the pane", pg.locator(".files .crumb:has-text('coder')").count() == 1)
    tmp = tempfile.NamedTemporaryFile("w", suffix=f"-{MARK}.txt", delete=False); tmp.write("uploaded from the desktop\n"); tmp.close()
    pg.locator(".files input[type=file]").set_input_files(tmp.name); pg.wait_for_selector(f".files .row:has-text('{MARK}')", timeout=20000)
    ok("files: uploaded file appears", True); pg.screenshot(path="shots/pn1-files.png")
    pg.locator(f".files .row:has-text('{MARK}') button.open").click(); pg.wait_for_selector(".files .preview pre", timeout=10000)
    ok("files: text preview", "uploaded from the desktop" in pg.locator(".files .preview pre").inner_text())
    api(f"/projects/{P}/fs?path=/home/coder/{os.path.basename(tmp.name)}", "DELETE")
    # Git: home is not a repository
    pg.locator(".agents .pane .tabs button:has-text('Git')").click(); pg.wait_for_selector(".git .hint:has-text('git'), .git .bar, .git .err", timeout=20000)
    ok("git: pane answers for the home directory", pg.locator(".git .hint:has-text('Не git-репозиторий')").count() == 1 or pg.locator(".git .bar").count() == 1, pg.locator(".git").inner_text()[:120])
    # Session terminal: a PTY with session =
    # id
    pg.locator(".agents .pane .tabs button:has-text('Терминал')").click(); pg.wait_for_selector(".agents .pane .xterm", timeout=30000); time.sleep(2)
    shells = [x for x in (api(f"/projects/{P}/ptys")[1] or []) if x.get("session") == sid]
    ok("shell: PTY linked to the session", len(shells) == 1 and shells[0]["alive"] and shells[0]["cwd"] == "/home/coder", json.dumps(shells)[:160])
    ok("shell: linked PTY hidden from the tree", pg.locator(f".agents .proj:has-text('{P}') .sess:has-text('shell ·')").count() == 0)
    pg.screenshot(path="shots/pn2-shell.png")
    b.close()
# cleanup: the session (stop + delete) removes its
# shell too
if sid:
    api(f"/projects/{P}/sessions/{sid}", "DELETE"); time.sleep(3); api(f"/projects/{P}/sessions/{sid}", "DELETE"); time.sleep(2)
    left = [x for x in (api(f"/projects/{P}/ptys")[1] or []) if x.get("session") == sid]
    ok("shell: removed together with the session", api(f"/projects/{P}/sessions/{sid}")[0] == 404 and not left, json.dumps(left)[:120])
print("ALL PASS" if all(res) else "SOME FAIL")
