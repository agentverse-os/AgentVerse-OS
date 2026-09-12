#!/usr/bin/env bash
# Shell-тесты функций bootstrap/install.sh с шимом tailscale (tests/shell/tailscale). Ничего системного не трогает:
# функции извлекаются из install.sh без вызова main (строка `main "$@"` вырезается), ENV_FILE подменяется временным
# файлом, каждый случай идёт в отдельном процессе bash — в install.sh действуют set -euo pipefail и trap ERR, и они
# не должны ронять сам прогон. Внешние зависимости: bash ≥ 4.4, jq, coreutils (timeout, date, mktemp, stat), hostname.
#
#   tests/shell/install_functions_test.sh               # все проверки; код 0 — все прошли
#   VERBOSE=1 tests/shell/install_functions_test.sh     # печатать вывод каждого случая
#   ONLY=resolve tests/shell/install_functions_test.sh  # один раздел: to_secs | env_set | ts_shim | authorize | dns_https | resolve
# Проверки с пометкой «(доп.)» — сверх задания; их падение — повод посмотреть, но не блокер.
set -uo pipefail
TESTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$TESTS_DIR/../.." && pwd)"
INSTALL_SH="$REPO/bootstrap/install.sh"
SHIM="$TESTS_DIR/tailscale"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/install-fn-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
FUNCS="$WORK/install_functions.sh"
FAKE="$WORK/ts"
ENVF="$WORK/cloudd.env"
AUTH_URL="https://login.tailscale.com/a/test"
TS_NAME="box.tail1234.ts.net"

[[ -f "$INSTALL_SH" ]] || { echo "нет $INSTALL_SH" >&2; exit 2; }
[[ -x "$SHIM" ]] || { echo "шим $SHIM не исполняемый" >&2; exit 2; }
command -v jq >/dev/null || { echo "нужен jq" >&2; exit 2; }

# ---------------------------------------------------------------------------------------------- извлечение функций
sed '/^main "\$@"$/d' "$INSTALL_SH" > "$FUNCS"
grep -q '^main "\$@"$' "$FUNCS" && { echo "не удалось вырезать вызов main из install.sh" >&2; exit 2; }
grep -q '^main()' "$FUNCS" || { echo "в install.sh нет main() — тест устарел" >&2; exit 2; }

# Прелюдия каждого случая: шим впереди PATH, состояние шима в $FAKE, функции install.sh, подмена ENV_FILE/TTY/TAILSCALE_WAIT.
cat > "$WORK/prelude.sh" <<EOF
export PATH="$TESTS_DIR:\$PATH"
export TS_FAKE_DIR="$FAKE"
source "$FUNCS"
ENV_FILE="$ENVF"
TTY=0
TAILSCALE_WAIT=15m
EOF

# ------------------------------------------------------------------------------------------------------- каркас
PASS=0 FAIL=0 SKIP=0; FAILED=()
OUT="" RC=0 ELAPSED=0 CASE_ENV=() SECTION=""
fake_reset() { rm -rf "$FAKE" "$ENVF"; mkdir -p "$FAKE"; }
fake_state() { echo "$1" > "$FAKE/state"; }
section()    { SECTION="$1"; printf '\n== %s\n' "$1"; }
want()       { [[ -z "${ONLY:-}" || "$ONLY" == "$1" ]]; }
# run_case <фрагмент> — выполнить фрагмент после прелюдии в отдельном bash; результат в OUT (stdout+stderr), RC, ELAPSED.
# Переменные окружения для случая — через CASE_ENV=(K=V …) перед вызовом (сбрасываются после).
run_case() {
  local t0; t0=$(date +%s)
  OUT="$(env -u EDGE_HOST -u TAILSCALE_AUTH_KEY -u TAILSCALE_HOSTNAME -u TAILSCALE_WAIT -u NONINTERACTIVE -u LOG \
           ${CASE_ENV[@]+"${CASE_ENV[@]}"} bash -c "source '$WORK/prelude.sh'"$'\n'"$1" 2>&1)"; RC=$?
  ELAPSED=$(( $(date +%s) - t0 ))
  CASE_ENV=()
  [[ -n "${VERBOSE:-}" ]] && printf -- '--- rc=%s · %s с\n%s\n---\n' "$RC" "$ELAPSED" "$OUT"
  return 0
}
# условия для check — работают с OUT/RC/ELAPSED последнего run_case
contains()   { [[ "$OUT" == *"$1"* ]]; }
matches()    { [[ "$OUT" == $1 ]]; }             # glob; «*» покрывает и переводы строк
out_is()     { [[ "$OUT" == "$1" ]]; }
rc_is()      { [[ "$RC" == "$1" ]]; }
rc_contains(){ [[ "$RC" == "$1" && "$OUT" == *"$2"* ]]; }
faster()     { (( ELAPSED < $1 )); }
elapsed_in() { (( ELAPSED >= $1 && ELAPSED < $2 )); }
calls_match(){ grep -qE -- "$1" "$FAKE/calls.log" 2>/dev/null; }   # журнал вызовов шима
check() { # check <описание> <условие…>
  local what="$1"; shift
  if "$@"; then PASS=$((PASS+1)); printf '  ok    %s\n' "$what"
  else
    FAIL=$((FAIL+1)); FAILED+=("[$SECTION] $what")
    printf '  FAIL  %s\n' "$what"
    [[ -n "${VERBOSE:-}" ]] || printf '        rc=%s · %s с · вывод:\n%s\n' "$RC" "$ELAPSED" "$(sed 's/^/        | /' <<<"$OUT")"
  fi
}
skip() { SKIP=$((SKIP+1)); printf '  skip  %s\n' "$1"; }

# ======================================================================================================== to_secs
if want to_secs; then
section "to_secs: длительность → секунды"
run_case 'to_secs 15m'; check "15m → 900"  out_is 900
run_case 'to_secs 90s'; check "90s → 90"   out_is 90
run_case 'to_secs 2h';  check "2h → 7200"  out_is 7200
run_case 'to_secs 0';   check "0 → 0"      out_is 0
fi

# ======================================================================================================== env_set
if want env_set; then
section "env_set: заменить или добавить ключ в env-файле"
fake_reset
run_case 'printf "A=1\nB=2\n" > "$ENV_FILE"; env_set "$ENV_FILE" A 9; cat "$ENV_FILE"'
check "замена: A=1 → A=9, соседний B=2 на месте" out_is $'A=9\nB=2'
run_case 'printf "A=1\n" > "$ENV_FILE"; env_set "$ENV_FILE" C 3; cat "$ENV_FILE"'
check "добавление: новый ключ дописывается в конец" out_is $'A=1\nC=3'
run_case 'printf "CLOUDD_EDGE_HOST=old\n" > "$ENV_FILE"; env_set "$ENV_FILE" CLOUDD_EDGE new; cat "$ENV_FILE"'
check "ключ-префикс: env_set CLOUDD_EDGE не задевает CLOUDD_EDGE_HOST" out_is $'CLOUDD_EDGE_HOST=old\nCLOUDD_EDGE=new'
run_case 'printf "CLOUDD_EDGE_ALT_HOSTS=1.2.3.4\nCLOUDD_EDGE_HOST=x\n" > "$ENV_FILE"; env_set "$ENV_FILE" CLOUDD_EDGE_HOST z; cat "$ENV_FILE"'
check "ключ-префикс: env_set CLOUDD_EDGE_HOST не задевает CLOUDD_EDGE_ALT_HOSTS" out_is $'CLOUDD_EDGE_ALT_HOSTS=1.2.3.4\nCLOUDD_EDGE_HOST=z'
run_case 'rm -f "$ENV_FILE"; env_set "$ENV_FILE" K V; cat "$ENV_FILE"; stat -c %a "$ENV_FILE"'
check "файла нет: создаётся с правами 0600" out_is $'K=V\n600'
run_case 'printf "A=1\nB=2\nA=1\n" > "$ENV_FILE"; env_set "$ENV_FILE" A 7; cat "$ENV_FILE"; echo "get=$(env_get A "$ENV_FILE")"'
check "повторный ключ: дубли схлопываются в одну строку A=, env_get читает её" out_is $'A=7\nB=2\nget=7'
run_case 'printf "K=old\n" > "$ENV_FILE"; env_set "$ENV_FILE" K "a&b"; cat "$ENV_FILE"'
check "(доп.) значение с «&» записывается буквально" out_is 'K=a&b'
run_case 'printf "K=old\n" > "$ENV_FILE"; env_set "$ENV_FILE" K "a|b"; cat "$ENV_FILE"'
check "(доп.) значение с «|» записывается буквально" out_is 'K=a|b'
fi

# ======================================================================================================== ts_* на шиме
if want ts_shim; then
section "ts_state/ts_dnsname/ts_https/ts_magicdns/ts_ipv4/ts_authurl на шиме"
fake_reset
run_case 'echo "ver=$(tailscale version | head -1) state=$(ts_state) dns=[$(ts_dnsname)] https=$(ts_https) magic=$(ts_magicdns) ip=[$(ts_ipv4)] url=$(ts_authurl)"'
check "шим найден первым в PATH (version 1.100.0)"     contains "ver=1.100.0"
check "без файла state: ts_state → NeedsLogin"          contains "state=NeedsLogin"
check "NeedsLogin: ts_dnsname пусто"                    contains "dns=[]"
check "без файла https: ts_https → false"               contains "https=false"
check "без файла magic: ts_magicdns → false"            contains "magic=false"
check "NeedsLogin: ts_ipv4 пусто"                       contains "ip=[]"
check "NeedsLogin: ts_authurl → ссылка"                 contains "url=$AUTH_URL"
fake_state Running; touch "$FAKE/https" "$FAKE/magic"
run_case 'echo "state=$(ts_state) dns=[$(ts_dnsname)] https=$(ts_https) magic=$(ts_magicdns) ip=[$(ts_ipv4)] url=[$(ts_authurl)]"'
check "state=Running: ts_state → Running"               contains "state=Running"
check "Running: ts_dnsname без завершающей точки"       contains "dns=[$TS_NAME]"
check "файл https: ts_https → true"                     contains "https=true"
check "файл magic: ts_magicdns → true"                  contains "magic=true"
check "Running: ts_ipv4 → 100.100.1.2 (IPv6 отброшен)"  contains "ip=[100.100.1.2]"
check "Running: ts_authurl пусто"                       contains "url=[]"
fake_reset; fake_state Stopped
run_case 'echo "state=$(ts_state)"'
check "state=Stopped: ts_state → Stopped"               contains "state=Stopped"
run_case 'PATH=/usr/bin:/bin; echo "state=$(ts_state) https=$(ts_https)"'
check "tailscale недоступен: ts_json → {} → NoState/false, без падения" contains "state=NoState https=false"
fi

# ======================================================================================================== ts_authorize
if want authorize; then
section "ts_authorize: ссылка на авторизацию и ожидание подтверждения"
fake_reset; echo 2 > "$FAKE/auth_after"
run_case 'TTY=1; TAILSCALE_WAIT=6s; rc=0; ts_authorize || rc=$?; echo "state_after=$(ts_state)"; exit $rc'
check "TTY=1, wait 6s, подтверждение через 2 с → код 0"            rc_is 0
check "  в выводе «узел авторизован»"                              contains "узел авторизован"
check "  в выводе ссылка на авторизацию"                           contains "$AUTH_URL"
check "  имя и IP узла в отчёте"                                   contains "$TS_NAME (100.100.1.2)"
check "  состояние шима стало Running"                             contains "state_after=Running"
check "  ждал подтверждения (≥ 2 с), но меньше таймаута (< 6 с)"   elapsed_in 2 6
check "  up вызван с --qr и --timeout 6s (без пользовательских флагов)" calls_match '^up --qr --timeout 6s$'
check "  после входа prefs через set --accept-dns"                calls_match '^set --accept-dns=true$'

fake_reset
run_case 'TTY=1; TAILSCALE_WAIT=3s; rc=0; ts_authorize || rc=$?; exit $rc'
check "TTY=1, wait 3s, без подтверждения → код 1"                  rc_is 1
check "  предупреждение «не завершена»"                            contains "не завершена"
check "  подсказка, как подключить позже (шаг tailscale / мастер)" matches "*tailscale*мастера первого запуска*"
check "  ждал до таймаута (≥ 3 с) и вышел (< 15 с)"                elapsed_in 3 15

fake_reset
run_case 'TTY=0; TAILSCALE_WAIT=15m; rc=0; ts_authorize || rc=$?; exit $rc'
check "TTY=0 → код 1"                                              rc_is 1
check "  быстро (< 30 с)"                                          faster 30
check "  в выводе ссылка $AUTH_URL"                                contains "$AUTH_URL"
check "  сказано, что узел не авторизован"                         contains "узел не авторизован"
check "  up вызван с --timeout 6s (без login: он сбрасывал бы профиль)" calls_match '^up --timeout 6s$'
check "  (login не вызывался)"                                     bash -c "! grep -q '^login' '$FAKE/calls.log'"

fake_reset
run_case 'TTY=1; TAILSCALE_WAIT=0; rc=0; ts_authorize || rc=$?; exit $rc'
check "TTY=1, TAILSCALE_WAIT=0 → не ждёт: код 1"                   rc_is 1
check "  ссылка в выводе"                                          contains "$AUTH_URL"
check "  быстро (< 10 с)"                                          faster 10

fake_reset; fake_state Stopped
run_case 'TTY=0; rc=0; ts_authorize || rc=$?; exit $rc'
check "(доп.) Stopped → включается: код 0, «Tailscale включён»"    rc_contains 0 "Tailscale включён"

fake_reset
CASE_ENV=(TAILSCALE_AUTH_KEY=tskey-auth-test TAILSCALE_HOSTNAME=srv1)
run_case 'TTY=0; rc=0; ts_authorize || rc=$?; exit $rc'
check "(доп.) TAILSCALE_AUTH_KEY → код 0, «авторизован ключом»"     rc_contains 0 "авторизован ключом"
check "  up получил --auth-key без прочих флагов"                  calls_match '^up --auth-key=tskey-auth-test --timeout 2m$'
check "  имя узла задано через set --hostname"                      calls_match '^set --hostname=srv1$'
fi

# ======================================================================================================== ts_wait_dns_https
if want dns_https; then
section "ts_wait_dns_https: MagicDNS и HTTPS Certificates"
fake_reset; fake_state Running
run_case 'TTY=0; rc=0; ts_wait_dns_https || rc=$?; exit $rc'
check "TTY=0 без magic/https → код 1"                              rc_is 1
check "  подсказка про админку (login.tailscale.com/admin/dns)"    contains "login.tailscale.com/admin/dns"
check "  названы обе настройки"                                    matches "*MagicDNS*HTTPS Certificates*"
check "  не ждёт (< 5 с)"                                          faster 5

fake_reset; fake_state Running; touch "$FAKE/magic"
run_case 'TTY=0; rc=0; ts_wait_dns_https || rc=$?; exit $rc'
check "только magic → код 1, упомянут HTTPS Certificates"          rc_contains 1 "HTTPS Certificates"

fake_reset; fake_state Running; touch "$FAKE/magic" "$FAKE/https"
run_case 'TTY=0; rc=0; ts_wait_dns_https || rc=$?; exit $rc'
check "оба файла → код 0"                                          rc_is 0
check "  «включены» и имя узла"                                    matches "*включены*$TS_NAME*"

fake_reset; fake_state Running
run_case '( sleep 1; touch "$TS_FAKE_DIR/magic" "$TS_FAKE_DIR/https" ) & TTY=1; TAILSCALE_WAIT=20s; rc=0; ts_wait_dns_https || rc=$?; wait; exit $rc'
check "(доп.) TTY=1: настройки включили во время ожидания → код 0"  rc_is 0
check "  за один цикл опроса (< 12 с)"                             faster 12
fi

# ======================================================================================================== resolve_edge_host
if want resolve; then
section "resolve_edge_host: явный EDGE_HOST → имя Tailscale → CLOUDD_EDGE_HOST из env-файла → IPv4 хоста"
# Логика выбора имени проверяется через `|| rc=$?` (set -e внутри функции при этом отключён), а код возврата — отдельно:
# в install.sh функция вызывается напрямую под set -e (step_tailscale/coder/cloudd/verify), и ненулевой код = обрыв установки.
R='rc=0; resolve_edge_host || rc=$?; echo "EDGE_HOST=$EDGE_HOST TLS_INTERNAL=$TLS_INTERNAL rc=$rc"'
D='resolve_edge_host; echo "direct EDGE_HOST=$EDGE_HOST TLS_INTERNAL=$TLS_INTERNAL"'   # как в step_*: напрямую под set -e

fake_reset; fake_state Running; touch "$FAKE/magic" "$FAKE/https"; echo "CLOUDD_EDGE_HOST=old.tail1.ts.net" > "$ENVF"
CASE_ENV=(EDGE_HOST=my.example.com); run_case "$R"
check "явный EDGE_HOST (из окружения) побеждает имя Tailscale и env-файл" contains "EDGE_HOST=my.example.com "
check "  не *.ts.net → TLS_INTERNAL=true"                          contains "TLS_INTERNAL=true"
check "  (код возврата) функция вернула 0"                         contains "rc=0"
CASE_ENV=(EDGE_HOST=custom.tail99.ts.net); run_case "$R"
check "явный EDGE_HOST *.ts.net → TLS_INTERNAL=false"              contains "EDGE_HOST=custom.tail99.ts.net TLS_INTERNAL=false"
check "  (код возврата) функция вернула 0"                         contains "rc=0"
run_case 'EDGE_HOST_EXPLICIT=192.168.1.10; '"$R"
check "явный IPv4 → TLS_INTERNAL=true"                             contains "EDGE_HOST=192.168.1.10 TLS_INTERNAL=true"
check "  (код возврата) функция вернула 0"                         contains "rc=0"

fake_reset; fake_state Running; touch "$FAKE/magic" "$FAKE/https"; echo "CLOUDD_EDGE_HOST=10.0.0.5" > "$ENVF"
run_case "$R"
check "Running+magic+https → имя Tailscale, а не старый CLOUDD_EDGE_HOST" contains "EDGE_HOST=$TS_NAME "
check "  имя Tailscale → TLS_INTERNAL=false"                       contains "TLS_INTERNAL=false"
check "  (код возврата) функция вернула 0"                         contains "rc=0"

fake_reset; fake_state Running; touch "$FAKE/magic"; echo "CLOUDD_EDGE_HOST=10.0.0.5" > "$ENVF"
run_case "$R"
check "Running без https → существующий CLOUDD_EDGE_HOST из env-файла" contains "EDGE_HOST=10.0.0.5 TLS_INTERNAL=true"
check "  (код возврата) функция вернула 0"                         contains "rc=0"
fake_reset; fake_state Running; touch "$FAKE/https"; echo "CLOUDD_EDGE_HOST=10.0.0.5" > "$ENVF"
run_case "$R"
check "Running без magic → существующий CLOUDD_EDGE_HOST из env-файла" contains "EDGE_HOST=10.0.0.5 TLS_INTERNAL=true"

fake_reset; echo "CLOUDD_EDGE_HOST=$TS_NAME" > "$ENVF"
run_case "$R"
check "NeedsLogin, в env-файле имя *.ts.net (выбрано раньше мастером) → берётся оно" contains "EDGE_HOST=$TS_NAME "
check "  и TLS_INTERNAL=false"                                     contains "TLS_INTERNAL=false"
check "  (код возврата) функция вернула 0"                         contains "rc=0"

fake_reset
LAN_IP="$(hostname -I 2>/dev/null | awk '{print $1}')"
run_case "$R"
if [[ -n "$LAN_IP" ]]; then
  check "ничего нет (первая установка, env-файла ещё нет) → IPv4 хоста ($LAN_IP), TLS_INTERNAL=true" contains "EDGE_HOST=$LAN_IP TLS_INTERNAL=true"
  check "  это IPv4"                                                bash -c "[[ '$LAN_IP' =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]"
  check "  (код возврата) функция вернула 0"                        contains "rc=0"
else
  skip "IPv4 хоста: hostname -I ничего не вернул на этой машине — ветка lan_ip проверена только на die"
  check "ничего нет и IP не определить → die «задайте EDGE_HOST», код 1" rc_contains 1 "задайте EDGE_HOST"
fi
run_case 'lan_ip() { echo; }; '"$R"
check "lan_ip пустой → die «задайте EDGE_HOST», код 1"             rc_contains 1 "задайте EDGE_HOST"

# Прямой вызов под set -e — ровно так функция вызывается в step_tailscale/coder/cloudd/verify.
fake_reset; fake_state Running; touch "$FAKE/magic" "$FAKE/https"; echo "CLOUDD_EDGE_HOST=old.example.com" > "$ENVF"
run_case "$D"
check "прямой вызов под set -e: своё имя в env-файле остаётся, скрипт идёт дальше"     rc_contains 0 "direct EDGE_HOST=old.example.com"
fake_reset; fake_state Running; touch "$FAKE/magic" "$FAKE/https"; echo "CLOUDD_EDGE_HOST=oldname.tail1234.ts.net" > "$ENVF"
run_case "$D"
check "прямой вызов под set -e: устаревшее имя *.ts.net → новое имя узла"           rc_contains 0 "direct EDGE_HOST=$TS_NAME"
fake_reset; echo "CLOUDD_EDGE_HOST=10.0.0.5" > "$ENVF"
run_case "$D"
check "прямой вызов под set -e: IPv4 из env-файла → скрипт идёт дальше"              rc_contains 0 "direct EDGE_HOST=10.0.0.5"
fake_reset
run_case "$D"
check "прямой вызов под set -e: первая установка без env-файла → скрипт идёт дальше"  rc_contains 0 "direct EDGE_HOST="
fi

# ======================================================================================================== итог
printf '\n== итог: %d ok, %d FAIL, %d skip\n' "$PASS" "$FAIL" "$SKIP"
if (( FAIL )); then printf '  падения:\n'; printf '   - %s\n' "${FAILED[@]}"; exit 1; fi
