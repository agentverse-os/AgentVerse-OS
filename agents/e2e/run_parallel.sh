#!/usr/bin/env bash
# Parallel run of agents_app_check.py on the stand (docs/qa-plan.md): head separately, independent blocks — N containers at
# once, blocks with shared state — one after another, the agentd restart — last and alone. The result is a PASS/FAIL
# summary per part. run_parallel.sh [N=4] — the full run; the EDGE_HOST, PROJECT, IMAGE variables as for a single run.
# ONLY="event hub|voice" run_parallel.sh 2 — only the named blocks separated by "|" (with the mini-head), without head
set -u
N="${1:-4}"; HERE="$(cd "$(dirname "$0")" && pwd)"; cd "$HERE"
EDGE_HOST="${EDGE_HOST:-box.tail1234.ts.net}"; PROJECT="${PROJECT:-alpha}"; IMAGE="${IMAGE:-mcr.microsoft.com/playwright/python:v1.55.0-noble}"
OUT="$HERE/parts"; mkdir -p "$OUT"; rm -f "$OUT"/*.py "$OUT"/*.log
# groups: G1 — independent (in parallel), G2 — shared workspace state (one after another), G3 — the agentd restart (at
# the end). "+" — related blocks in one part (agents v2 continues "agents", "live answer text" takes the Gemini/pi
# sessions from the ACP block)
G1=("agents, routines+agents v2" "background after a turn" "editing and forking" "git panel" "subscription limits" "conversation search" "voice" "event hub" "feed on the device" "pwa" "ACP engines+live answer text")
G2=("permissions by rules" "MCP servers" "spending limits" "scratch chat" "simple chat")   # spending limits change the workspace's settings.limits — only one after another
G3=("agentd restart")
slug() { python3 -c "
import sys, re
T = dict(zip('абвгдеёжзийклмнопрстуфхцчшщъыьэюя', ['a','b','v','g','d','e','e','zh','z','i','j','k','l','m','n','o','p','r','s','t','u','f','h','c','ch','sh','sch','','y','','e','yu','ya']))
s = ''.join(T.get(c, c) for c in sys.argv[1].lower()); print(re.sub(r'[^a-z0-9]+', '-', s).strip('-')[:40] or 'part')" "$1"; }
run_part() {  # run_part <file.py> <log>
  docker run --rm --network host -e EDGE_HOST="$EDGE_HOST" -e PROJECT="$PROJECT" -v "$HERE:/work" -w /work "$IMAGE" sh -c "pip install -q playwright==1.55.0 2>/dev/null; python3 parts/$(basename "$1")" > "$2" 2>&1
  echo "$(basename "$1" .py): $(grep -c '^PASS' "$2") PASS / $(grep -c '^FAIL' "$2") FAIL$(grep -q Traceback "$2" && echo ' (traceback)')"
}
export -f run_part; export EDGE_HOST PROJECT IMAGE HERE
start=$(date +%s)
if [ -z "${ONLY:-}" ]; then
  python3 mkpart.py __head__ --out "$OUT/head.py" >/dev/null && run_part "$OUT/head.py" "$OUT/head.log"
fi
parts=()
if [ -n "${ONLY:-}" ]; then IFS='|' read -r -a sel <<< "$ONLY"; else sel=("${G1[@]}"); fi
mk() { local f="$OUT/$(slug "$1").py"; IFS='+' read -r -a bl <<< "$1"; python3 mkpart.py --mini "${bl[@]}" --out "$f" >/dev/null && echo "$f"; }
for b in "${sel[@]}"; do f=$(mk "$b"); [ -n "$f" ] && parts+=("$f"); done
printf '%s\n' "${parts[@]}" | xargs -P "$N" -I{} bash -c 'f="$1"; run_part "$f" "${f%.py}.log"' _ {}
if [ -z "${ONLY:-}" ]; then
  for b in "${G2[@]}" "${G3[@]}"; do f=$(mk "$b"); [ -n "$f" ] && run_part "$f" "${f%.py}.log"; done
fi
echo "---- итого за $(( $(date +%s) - start )) с: $(cat "$OUT"/*.log | grep -c '^PASS') PASS / $(cat "$OUT"/*.log | grep -c '^FAIL') FAIL"
grep -h '^FAIL\|Traceback' "$OUT"/*.log | cut -c1-300
