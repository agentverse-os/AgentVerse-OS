#!/usr/bin/env python3
"""Building a part of the run out of agents_app_check.py (docs/qa-plan.md, "parallelizing e2e").

The check file is one scenario: the preamble (the service, browser launch), the head block (creating the `sid` session and the basic
checks) and blocks separated by `    # ---- …` lines. A part = the preamble + (the head block | the mini-head) + the chosen blocks + the tail (b.close()).
The mini-head creates a session through the API and opens it in Thread — the state the blocks after head start from.
Function definitions from all blocks are moved to the start of the part: the blocks use each other's helpers.

  mkpart.py __head__                       head only
  mkpart.py "event hub" "voice"            head + two blocks
  mkpart.py --mini "event hub"             the mini-head + a block (for the parallel run)
  mkpart.py --list                         the list of blocks
  --src path --out path                    the source and the result (by default next to each other: agents_app_check.py → e2e_part.py)
"""
import os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
HEAD_END = 'ok("ui: shell with six sections"'
MINI = r'''    # the mini-head of a parallel part: its own session through the API, opened in Thread, the "Chat" tab (the state after head)
    _ms = json.loads(raw(f"/agents/api/core/projects/{P}/sessions", "POST", json.dumps({"agent": "claude", "title": f"e2e part {MARK}", "permission_mode": "default"}).encode())[1] or b"{}")
    sid = _ms.get("id"); ok("mini-head: session created for this part", bool(sid), str(_ms)[:120])
    if sid:
        pg.locator(".rail .nav[data-tip='Рабочие места']").click(); time.sleep(0.5); tree_expand(pg, P)   # under a parallel run there are more than eight rows
        try:
            pg.wait_for_selector(f".panel .sess[data-id='{sid}']", timeout=20000)
            if pg.locator(".top .seg button:has-text('Thread')").count(): pg.locator(".top .seg button:has-text('Thread')").click(); time.sleep(0.3)
            pg.locator(f".panel .sess[data-id='{sid}']").click()
            pg.wait_for_selector(".thread .pane .tabs", timeout=20000); pg.locator(".thread .pane .tabs .tab:has-text('Чат')").click(); pg.wait_for_selector(".thread .chat textarea", timeout=20000); time.sleep(0.8)
        except Exception as e: ok("mini-head: session opened in Thread", False, str(e)[:120])
        # one turn, as in head: the blocks count on a conversation with an agent answer (a bubble,
        # history for a fork)
        raw(f"/agents/api/core/projects/{P}/sessions/{sid}/prompt", "POST", json.dumps({"content": f"Reply with exactly: part-{MARK}"}).encode())
        for _ in range(120):
            time.sleep(1); _mi = (get(f"/agents/api/core/projects/{P}/sessions/{sid}")[1] or {}).get("info", {})
            if _mi.get("status") == "idle" and f"part-{MARK}" in (_mi.get("last_text") or ""): break
        ok("mini-head: the session answered its first prompt", f"part-{MARK}" in (_mi.get("last_text") or ""), str(_mi.get("status")))
        time.sleep(1)
'''
# cleanup of the mini-head session is the scenario's shared tail (after b.close(): two DELETEs and "cleanup:
# session removed")

def defs_of(lines, lo, hi):
    """`    def …` / `    class …` definitions in a line range (with the body, by indent)"""
    out = []; i = lo
    while i < hi:
        l = lines[i]
        if re.match(r"    (def|class) \w+", l):
            j = i + 1
            while j < hi and (lines[j].strip() == "" or lines[j].startswith("     ")): j += 1
            while j > i + 1 and lines[j - 1].strip() == "": j -= 1
            out.append((re.match(r"    (?:def|class) (\w+)", l).group(1), lines[i:j])); i = j
        else: i += 1
    return out

def main():
    args = sys.argv[1:]
    src = os.path.join(HERE, "agents_app_check.py"); out = os.path.join(HERE, "e2e_part.py"); mini = False; want = []
    while args:
        a = args.pop(0)
        if a == "--src": src = args.pop(0)
        elif a == "--out": out = args.pop(0)
        elif a == "--mini": mini = True
        elif a == "--list": want = ["--list"]
        else: want.append(a)
    lines = open(src, encoding="utf-8").read().split("\n")
    head_end = next(i for i, l in enumerate(lines) if HEAD_END in l)
    tail_start = next(i for i, l in enumerate(lines) if l.startswith("    b.close()"))
    marks = [i for i, l in enumerate(lines) if l.startswith("    # ---- ")]
    blocks = list(zip(marks, marks[1:] + [tail_start]))
    if want == ["--list"]:
        for a, b in blocks: print(f"{lines[a][11:80].strip()}  ({b - a} lines)")
        return
    part = lines[:head_end + 1]
    if "__head__" in want: part += lines[head_end + 1:marks[0]]
    else:
        # helpers from head and all blocks — up front, one per name
        seen = set()
        for name, body in defs_of(lines, head_end + 1, tail_start):
            if name in seen: continue
            seen.add(name); part += body
        part += MINI.rstrip("\n").split("\n") if mini else ["    try: pg.locator(\".top .seg button:has-text('Thread')\").click(); time.sleep(0.5)", "    except Exception: pass"]
    chosen = [(a, b) for a, b in blocks if any(w != "__head__" and w in lines[a] for w in want)]
    for a, b in chosen: part += lines[a:b]
    part += lines[tail_start:]
    open(out, "w", encoding="utf-8").write("\n".join(part))
    print(f"{len(part)} строк; head={'да' if '__head__' in want else ('мини' if mini else 'нет')}; блоки: {[lines[a][11:60].strip() for a, b in chosen]}")

if __name__ == "__main__": main()
