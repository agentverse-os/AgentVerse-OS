#!/usr/bin/env bash
# Runs of agents_app_check.py on the stand by parts (docs/qa-plan.md §6). Take the smallest run that covers the change:
#   ONLY="voice|event hub" run_parallel.sh [N]   the named blocks (names: mkpart.py --list) — after an ordinary change
#   MODE=smoke run_parallel.sh                   head and the agentd restart — before a deploy that changes agentd or the shared parts
#   run_parallel.sh [N=4]                        the full run — after a platform update or before committing a large slice
#   WITH="memory watch" run_parallel.sh          the full run plus blocks that are otherwise run only on demand
# The EDGE_HOST, PROJECT, IMAGE variables are as for a single run. Parts and logs: parts/; the summary gives the time of every part.
set -u
N="${1:-4}"; HERE="$(cd "$(dirname "$0")" && pwd)"; cd "$HERE"
EDGE_HOST="${EDGE_HOST:-box.tail1234.ts.net}"; PROJECT="${PROJECT:-alpha}"; IMAGE="${IMAGE:-mcr.microsoft.com/playwright/python:v1.55.0-noble}"
MODE="${MODE:-full}"; ONLY="${ONLY:-}"; WITH="${WITH:-}"
# The memory of the box is shared with the other workspaces: a pool of parts next to a working conversation, followed
# by a restart of the e2e workspace, can starve the host. While a conversation outside the e2e project is working the
# parts go one at a time; BUSY_OK=1 keeps N.
if [ "$N" -gt 1 ] && [ "${BUSY_OK:-0}" != 1 ]; then
  busy=$(curl -s -m 20 "${AGENTS_API:-http://127.0.0.1:7300/agents/api}/workspaces" 2>/dev/null | PROJECT="$PROJECT" python3 -c '
import json, os, sys
try:
    d = json.load(sys.stdin)
    print(" ".join(sorted(p.get("name", "?") for p in d.get("projects", []) if p.get("name") != os.environ["PROJECT"] and any(s.get("status") == "working" for s in (p.get("sessions") or [])))))
except Exception:
    print("")' 2>/dev/null)
  if [ -n "$busy" ]; then echo "в рабочих местах владельца идёт работа ($busy): части пойдут по одной (N=1), чтобы не делить с ней память стенда; BUSY_OK=1 — запустить как есть"; N=1; fi
fi
OUT="$HERE/parts"; mkdir -p "$OUT"; rm -f "$OUT"/*.py "$OUT"/*.log "$OUT"/*.time
# POOL — independent blocks, N containers at once, the longest first so that the pool ends sooner. "+" joins related blocks
# into one part (agents v2 continues "agents"; the catalog and Oh My Pi are short). Every engine is a part of its own.
POOL=("agents, routines+agents v2" "pwa" "background after a turn" "engines: Gemini CLI" "editing and forking" "voice"
      "simple chat" "engines: Antigravity" "git panel" "live answer text" "feed on the device" "owner's servers"
      "engines: pi over pi-acp" "perimeter" "conversation search" "ACP engines+engines: Oh My Pi" "event hub" "subscription limits"
      "notification channels")
# LANE — blocks that change settings of the whole workspace (rules, MCP servers, spending limits) or of the "Chat" section:
# one after another, after the pool.
LANE=("permissions by rules" "MCP servers" "spending limits" "scratch chat")
# ON_DEMAND — not in the full run: "memory watch" brings the workspace to its memory limit and depends on how soon the
# kernel ends the process; it is needed when the watch itself changes and after kernel or Incus updates (WITH= or ONLY=).
ON_DEMAND=("memory watch")
# LAST — the agentd restart cuts every turn in the workspace: alone, at the very end.
LAST=("agentd restart")
slug() { python3 -c "
import sys, re
T = dict(zip('абвгдеёжзийклмнопрстуфхцчшщъыьэюя', ['a','b','v','g','d','e','e','zh','z','i','j','k','l','m','n','o','p','r','s','t','u','f','h','c','ch','sh','sch','','y','','e','yu','ya']))
s = ''.join(T.get(c, c) for c in sys.argv[1].lower()); print(re.sub(r'[^a-z0-9]+', '-', s).strip('-')[:40] or 'part')" "$1"; }
run_part() {  # run_part <file.py> <log>
  local t0; t0=$(date +%s)
  docker run --rm --network host -e EDGE_HOST="$EDGE_HOST" -e PROJECT="$PROJECT" -v "$HERE:/work" -w /work "$IMAGE" sh -c "pip install -q playwright==1.55.0 2>/dev/null; python3 parts/$(basename "$1")" > "$2" 2>&1
  local secs=$(( $(date +%s) - t0 )); echo "$secs" > "${2%.log}.time"
  echo "$(basename "$1" .py): $(grep -c '^PASS' "$2") PASS / $(grep -c '^FAIL' "$2") FAIL$(grep -q '^SKIP' "$2" && echo " / $(grep -c '^SKIP' "$2") SKIP")$(grep -q Traceback "$2" && echo ' (traceback)') — $secs с"
}
export -f run_part; export EDGE_HOST PROJECT IMAGE HERE
mk() { local f="$OUT/$(slug "$1").py"; IFS='+' read -r -a bl <<< "$1"; if python3 mkpart.py --mini "${bl[@]}" --out "$f" >/dev/null; then echo "$f"; else echo "часть «$1» не собрана" >&2; fi; }
pool() { local parts=() b f; for b in "$@"; do f=$(mk "$b"); [ -n "$f" ] && parts+=("$f"); done
  [ "${#parts[@]}" -gt 0 ] && printf '%s\n' "${parts[@]}" | xargs -P "$N" -I{} bash -c 'f="$1"; run_part "$f" "${f%.py}.log"' _ {}; }
lane() { local b f; for b in "$@"; do f=$(mk "$b"); [ -n "$f" ] && run_part "$f" "${f%.py}.log"; done; }
head_part() { python3 mkpart.py __head__ --out "$OUT/head.py" >/dev/null && run_part "$OUT/head.py" "$OUT/head.log"; }
# a block that must not share the workspace with other parts: it goes alone even when it is named in ONLY
solo() { local x; for x in "${LANE[@]}" "${ON_DEMAND[@]}" "${LAST[@]}"; do [[ "$x" == *"$1"* || "$1" == *"$x"* ]] && return 0; done; return 1; }
start=$(date +%s)
if [ -n "$ONLY" ]; then
  IFS='|' read -r -a sel <<< "$ONLY"; par=(); seq=()
  for b in "${sel[@]}"; do if solo "$b"; then seq+=("$b"); else par+=("$b"); fi; done
  [ "${#par[@]}" -gt 0 ] && pool "${par[@]}"
  [ "${#seq[@]}" -gt 0 ] && lane "${seq[@]}"
elif [ "$MODE" = smoke ]; then
  head_part; lane "${LAST[@]}"
else
  head_part; pool "${POOL[@]}"; lane "${LANE[@]}"
  if [ -n "$WITH" ]; then IFS='|' read -r -a extra <<< "$WITH"; lane "${extra[@]}"; fi
  lane "${LAST[@]}"
fi
pass=$(cat "$OUT"/*.log 2>/dev/null | grep -c '^PASS'); fail=$(cat "$OUT"/*.log 2>/dev/null | grep -c '^FAIL'); skip=$(cat "$OUT"/*.log 2>/dev/null | grep -c '^SKIP')
echo "---- итого за $(( $(date +%s) - start )) с: $pass PASS / $fail FAIL$([ "$skip" -gt 0 ] && echo " / $skip SKIP")"
grep -h '^FAIL\|^SKIP\|Traceback' "$OUT"/*.log 2>/dev/null | cut -c1-300
# waits that gave up (SLOW lines of the scenario): not failures, but time burnt — a hidden element wants state="attached"
slow=$(grep -h '^SLOW' "$OUT"/*.log 2>/dev/null | sort | uniq -c | sort -rn)
[ -n "$slow" ] && { echo "---- истёкшие ожидания (время впустую):"; echo "$slow" | cut -c1-200 | head -12; }
[ "$fail" -eq 0 ] && ! grep -qs Traceback "$OUT"/*.log
