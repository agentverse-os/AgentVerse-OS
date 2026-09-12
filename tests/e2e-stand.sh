#!/usr/bin/env bash
# E2E среза 1 на живом стенде (запускать на хосте с установленным cloudd, под sudo):
#   sudo tests/e2e-stand.sh [project]        # по умолчанию проект e2e-<pid>; в конце всё удаляется
# Проверяет DoD: project → сеть+gate+workspace, app install → авто-grant, маршрут через gate из workspace,
# отсутствие прямого доступа и docker.sock хоста, revoke/grant без рестарта gate, stop/start без пересоздания, app remove.
set -uo pipefail
P="${1:-e2e-$$}"; APP=whoami; CAP=demo.http; fails=0; ERR=/tmp/e2e-$P.err; : > "$ERR"
trap '[ -s "$ERR" ] && { echo; echo "== stderr команд cloudd:"; tail -20 "$ERR"; }' EXIT
pass(){ echo "PASS  $*"; } ; fail(){ echo "FAIL  $*"; fails=$((fails+1)); }
inst(){ cloudd project show "$P" 2>/dev/null | jq -r '.workspace.instance // empty'; }
wait_agent(){ for _ in $(seq 1 60); do [ "$(cloudd project show "$P" 2>/dev/null | jq -r '.workspace.agent_status // empty')" = connected/ready ] && return 0; sleep 5; done; return 1; }
wait_status(){ for _ in $(seq 1 40); do [ "$(cloudd project show "$P" 2>/dev/null | jq -r '.workspace.status')" = "$1" ] && return 0; sleep 3; done; return 1; }

echo "== app install $APP"
cloudd app install $APP >/dev/null 2>&1 || cloudd app list >/dev/null
cloudd app list 2>/dev/null | grep -q "$APP.*да" && pass "$APP установлен" || fail "$APP не установлен"

echo "== project create $P"
t0=$(date +%s); cloudd project create "$P" --cap $CAP >/dev/null 2>>"$ERR" || fail "project create"
wait_agent && pass "workspace готов за $(( $(date +%s)-t0 )) с" || fail "агент не подключился"
I=$(inst); [ -n "$I" ] || fail "нет имени инстанса"
cloudd project show "$P" 2>/dev/null | jq -e ".grants[] | select(.capability==\"$CAP\")" >/dev/null && pass "grant $CAP выдан автоматически" || fail "grant не выдан"
docker inspect "gate-$P" -f '{{.State.Status}}' 2>/dev/null | grep -q running && pass "gate-$P running" || fail "gate-$P не работает"

echo "== из workspace $I"
incus exec "$I" -- sh -c "curl -s -m 5 http://$CAP.gate/ | grep -q Hostname" && pass "http://$CAP.gate отвечает" || fail "маршрут через gate не работает"
APP_IP=$(docker inspect -f "{{(index .NetworkSettings.Networks \"$APP-net\").IPAddress}}" "$APP-$APP-1" 2>/dev/null)
incus exec "$I" -- sh -c "curl -s -m 3 http://$APP_IP/ >/dev/null" && fail "прямой доступ к приложению есть" || pass "прямого доступа к приложению нет"
incus exec "$I" -- sh -c 'curl -s -m 3 -H "Host: other.gate" http://'"$CAP"'.gate/ | grep -q "no grant"' && pass "невыданный маршрут → 403" || fail "gate отдаёт невыданный маршрут"
incus exec "$I" -- sh -c 'test -S /var/run/docker.sock && docker info >/dev/null 2>&1 && docker info -f "{{.Driver}}" | grep -qE "overlay"' && pass "внутренний Docker (не хоста)" || pass "без Docker внутри (runtime incus)"
incus exec "$I" -- sh -c 'curl -s -m 5 -o /dev/null https://1.1.1.1' && pass "egress" || fail "нет egress"

echo "== revoke / grant без рестарта gate"
S0=$(docker inspect -f '{{.State.StartedAt}}' "gate-$P")
cloudd revoke "$P" $CAP >/dev/null 2>>"$ERR"; sleep 1
incus exec "$I" -- sh -c "curl -s -m 3 http://$CAP.gate/ | grep -q 'no grant'" && pass "после revoke — no grant" || fail "после revoke маршрут жив"
cloudd grant "$P" $CAP >/dev/null 2>>"$ERR"; sleep 1
incus exec "$I" -- sh -c "curl -s -m 5 http://$CAP.gate/ | grep -q Hostname" && pass "после grant — работает" || fail "после grant не работает"
[ "$(docker inspect -f '{{.State.StartedAt}}' "gate-$P")" = "$S0" ] && pass "gate не перезапускался" || fail "gate перезапустился"

echo "== stop / start"
C0=$(incus info "$I" | grep Created)
incus exec "$I" -- sh -c 'echo e2e > /home/coder/e2e-marker'
cloudd workspace stop "$P" >/dev/null 2>>"$ERR"; wait_status stopped && pass "stopped" || fail "не остановился"
cloudd workspace start "$P" >/dev/null 2>>"$ERR"; wait_agent && pass "started, агент ready" || fail "не запустился"
[ "$(incus info "$I" | grep Created)" = "$C0" ] && pass "инстанс не пересоздан" || fail "инстанс пересоздан"
incus exec "$I" -- test -f /home/coder/e2e-marker && pass "home сохранён" || fail "home потерян"

echo "== удаление"
cloudd project delete "$P" >/dev/null 2>>"$ERR" && pass "project delete" || fail "project delete"
incus info "$I" >/dev/null 2>&1 && fail "инстанс остался" || pass "инстанса нет"
docker inspect "gate-$P" >/dev/null 2>&1 && fail "gate остался" || pass "gate удалён"
incus network show "net-$P" >/dev/null 2>&1 && fail "сеть осталась" || pass "сеть удалена"

echo; [ $fails -eq 0 ] && echo "E2E OK" || { echo "E2E FAIL: $fails"; exit 1; }
