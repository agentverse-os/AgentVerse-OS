#!/usr/bin/env bash
# AgentVerse OS · установка на чистый Ubuntu одной командой. Идемпотентно: повторный запуск ничего не ломает, любой шаг
# можно вызвать отдельно. Порядок: preflight → zfs → host (Docker, Incus, firewall) → tailscale (ссылка и QR на авторизацию,
# ожидание подтверждения, MagicDNS и HTTPS Certificates) → coder → komodo → edge → cloudd → catalog → template → verify.
#
#   sudo bootstrap/install.sh                                    # всё; дальше — Desktop в браузере
#   sudo bootstrap/install.sh <шаг>                              # preflight | zfs | host | tailscale | coder | komodo | edge | cloudd | catalog | template | verify
#   sudo DISKS="/dev/disk/by-id/…" bootstrap/install.sh          # создать пул ZFS на этих дисках (иначе пул не создаётся, Incus на dir)
#   sudo PACKAGE=agentverse-os-0.2.0-x86_64.tar.gz bootstrap/install.sh   # ядро из пакета обновления (файл или https-URL); он же — источник bootstrap/ и шаблона
#   sudo bootstrap/install.sh --help
#
# Переменные:
#   EDGE_HOST            имя ядра; по умолчанию — MagicDNS-имя узла Tailscale (когда узел авторизован, MagicDNS и HTTPS включены),
#                        иначе уже записанное в /etc/cloudos/cloudd.env, иначе IPv4 хоста (временно, внутренний CA)
#   DISKS                диски под пул tank (через пробел); DISKS_FORCE=1 — не спрашивать подтверждение (данные на дисках будут уничтожены)
#   PACKAGE              пакет agentverse-os-<версия>-<arch>.tar.gz (файл или https-URL); PACKAGE_SHA256 — ожидаемый sha256 файла
#   CLOUDD_BIN           собранный бинарь, если пакета нет (по умолчанию ../cloudd/target/release/cloudd или ../cloudd рядом с bootstrap/)
#   TAILSCALE_AUTH_KEY   авторизовать узел ключом (или TAILSCALE_AUTH_KEY_FILE — путь к файлу с ключом; ключ не попадает в ps и журнал)
#   TAILSCALE_HOSTNAME   имя узла в tailnet (по умолчанию hostname машины); задаётся через tailscale set после входа
#   TAILSCALE_WAIT       сколько ждать авторизацию и включение MagicDNS/HTTPS: Ns | Nm | Nh (по умолчанию 15m; 0 — не ждать)
#   NONINTERACTIVE=1     не ждать и не спрашивать (то же самое без терминала); ADMIN_EMAIL, ADMIN_PASSWORD (или ADMIN_PASSWORD_FILE) — администратор Coder/Komodo
#   IMPORT_CATALOG=1     импортировать каталог upstream (Runtipi, Coolify, Umbrel) даже если манифесты уже есть
#   ARC_MAX_GB           лимит ARC для ZFS (2 на 14 GB RAM, 6 на 64 GB); LOG — журнал (по умолчанию /var/log/agentverse-install.log, 0600)
# Секреты (пароль admin, токен Coder, ключ Komodo) генерируются один раз и живут в /etc/cloudos/cloudd.env (0600); в git не попадают.
set -Eeuo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
STATE=/var/lib/cloudos
ENV_FILE=/etc/cloudos/cloudd.env
KOMODO_DIR=/srv/apps/komodo
LOG="${LOG:-/var/log/agentverse-install.log}"
ADMIN_EMAIL_EXPLICIT="${ADMIN_EMAIL:-}"
ADMIN_EMAIL="${ADMIN_EMAIL:-admin@cloudos.local}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-}"
[[ -z "$ADMIN_PASSWORD" && -n "${ADMIN_PASSWORD_FILE:-}" && -r "${ADMIN_PASSWORD_FILE:-}" ]] && ADMIN_PASSWORD="$(tr -d '\r\n' < "$ADMIN_PASSWORD_FILE")"
ARC_MAX_GB="${ARC_MAX_GB:-2}"
CLOUDD_BIN="${CLOUDD_BIN:-}"
PACKAGE="${PACKAGE:-}"
TAILSCALE_WAIT="${TAILSCALE_WAIT:-15m}"
TAILSCALE_HOSTNAME="${TAILSCALE_HOSTNAME:-}"
TS_ADMIN_DNS="https://login.tailscale.com/admin/dns"
TS_ADMIN_MACHINES="https://login.tailscale.com/admin/machines"
EDGE_HOST_EXPLICIT="${EDGE_HOST:-}"
EDGE_HOST="${EDGE_HOST:-}"
EDGE_RESOLVED=0
TLS_INTERNAL=true
PKG_DIR=""
TEE_PID=""
STEP=""
TTY=0; [[ -z "${NONINTERACTIVE:-}" && -t 0 && -t 1 ]] && TTY=1   # до exec > >(tee): после него stdout — канал

# ------------------------------------------------------------------------------------------------------------ утилиты
c_bold=$'\033[1m'; c_ok=$'\033[32m'; c_warn=$'\033[33m'; c_err=$'\033[31m'; c_off=$'\033[0m'
log()  { printf '\n%s== %s%s\n' "$c_bold" "$*" "$c_off"; }
ok()   { printf '%s✓%s %s\n' "$c_ok" "$c_off" "$*"; }
warn() { printf '%s! %s%s\n' "$c_warn" "$*" "$c_off"; }
die()  { printf '%s✗ %s%s\n' "$c_err" "$*" "$c_off" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }
need_root() { [[ $EUID -eq 0 ]] || die "запускать через sudo"; }
rand() { openssl rand -hex "${1:-16}"; }
interactive() { [[ "$TTY" == 1 ]]; }
usage() { sed -n '2,/^set /{/^set /!p}' "$0" | sed 's/^# \{0,1\}//'; }
# длительность Ns | Nm | Nh | N (секунды) → секунды; иначе — ошибка
to_secs() {
  local v="${1:-0}"
  [[ "$v" =~ ^[0-9]+[smh]?$ ]] || die "TAILSCALE_WAIT=«$v»: ожидаю форму 15m, 90s, 2h или 0"
  case "$v" in *h) echo $(( ${v%h} * 3600 )) ;; *m) echo $(( ${v%m} * 60 )) ;; *s) echo "${v%s}" ;; *) echo "$v" ;; esac
}
human_dur() { # секунды → «15 мин» / «2 ч» / «90 с»
  local s="$1"
  if (( s >= 3600 && s % 3600 == 0 )); then echo "$((s / 3600)) ч"; elif (( s >= 60 && s % 60 == 0 )); then echo "$((s / 60)) мин"; else echo "$s с"; fi
}
env_get() { grep -E "^$1=" "$2" 2>/dev/null | head -1 | cut -d= -f2- || true; }
env_set() { # env_set FILE KEY VALUE — заменить строку KEY= на месте или дописать; без sed, спецсимволы в значении безопасны
  local f="$1" k="$2" v="$3" tmp
  mkdir -p "$(dirname "$f")"; [[ -f "$f" ]] || { : > "$f"; chmod 0600 "$f"; }
  tmp="$(mktemp "$f.XXXXXX")"
  K="$k" V="$v" awk 'index($0, ENVIRON["K"] "=") == 1 { if (!done) { print ENVIRON["K"] "=" ENVIRON["V"]; done = 1 }; next } { print } END { if (!done) print ENVIRON["K"] "=" ENVIRON["V"] }' "$f" > "$tmp"
  chmod 0600 "$tmp"; mv -f "$tmp" "$f"
}
lan_ip() { ip -4 route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i <= NF; i++) if ($i == "src") { print $(i + 1); exit }}' || true; }
is_ipv4() { [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; }
wait_for() { # wait_for <сек> <описание> <команда…> — ждать успеха команды
  local secs="$1" what="$2"; shift 2
  local end=$(( $(date +%s) + secs ))
  while ! "$@" >/dev/null 2>&1; do
    if (( $(date +%s) >= end )); then warn "не дождался: $what"; return 1; fi
    sleep 2
  done
}
confirm() { # confirm <вопрос> — да без вопросов только явно (NONINTERACTIVE не считается согласием)
  if ! interactive; then return 1; fi
  local a=""; read -r -p "$1 [y/N] " a </dev/tty; [[ "$a" == [yY] ]]
}
coder_cli() { # coder CLI под root: сессия — токен cloudd из env-файла, если он уже есть
  local tok; tok="$(env_get CLOUDD_CODER_TOKEN "$ENV_FILE")"
  if [[ -n "$tok" ]]; then CODER_URL=http://127.0.0.1:7080 CODER_SESSION_TOKEN="$tok" coder "$@" </dev/null; else coder "$@" </dev/null; fi
}
admin_password() { # заданный явно — сохраняется; иначе генерируется один раз и живёт в cloudd.env
  local p
  if [[ -n "$ADMIN_PASSWORD" ]]; then
    [[ "$(env_get CLOUDOS_ADMIN_PASSWORD "$ENV_FILE")" == "$ADMIN_PASSWORD" ]] || env_set "$ENV_FILE" CLOUDOS_ADMIN_PASSWORD "$ADMIN_PASSWORD"
    echo "$ADMIN_PASSWORD"; return
  fi
  p="$(env_get CLOUDOS_ADMIN_PASSWORD "$ENV_FILE")"
  if [[ -z "$p" ]]; then p="$(rand 12)"; env_set "$ENV_FILE" CLOUDOS_ADMIN_PASSWORD "$p"; fi
  echo "$p"
}
on_error() {
  local rc=$?
  printf '\n%s✗ шаг «%s» завершился с ошибкой (код %s). Журнал: %s.%s\n' "$c_err" "${STEP:-?}" "$rc" "$LOG" "$c_off" >&2
  printf '%s  После исправления повторите шаг: sudo %s %s — затем остальное: sudo %s (все шаги идемпотентны).%s\n' "$c_err" "$0" "${STEP:-}" "$0" "$c_off" >&2
}
on_exit() { [[ -n "$PKG_DIR" ]] && rm -rf "$PKG_DIR"; if [[ -n "$TEE_PID" ]]; then exec >&- 2>&-; wait "$TEE_PID" 2>/dev/null || true; fi; }
trap on_error ERR
trap on_exit EXIT

# ---------------------------------------------------------------------------------------------------------- tailscale
ts_json()     { local out; out="$(tailscale status --json 2>/dev/null || true)"; if [[ "$out" == \{* ]]; then printf '%s\n' "$out"; else echo '{}'; fi; }
ts_state()    { ts_json | jq -r '.BackendState // "NoState"'; }
ts_dnsname()  { ts_json | jq -r '.Self.DNSName // empty' | sed 's/\.$//'; }
ts_authurl()  { ts_json | jq -r '.AuthURL // empty'; }
ts_magicdns() { ts_json | jq -r '.CurrentTailnet.MagicDNSEnabled // false'; }
ts_https()    { ts_json | jq -r 'if (.CertDomains // []) | length > 0 then "true" else "false" end'; }
ts_ipv4()     { ts_json | jq -r '(.Self.TailscaleIPs // [])[]' | grep -m1 '\.' || true; }
ts_control_reachable() { curl -fsS -m 8 -o /dev/null https://controlplane.tailscale.com/ 2>/dev/null || curl -fsS -m 8 -o /dev/null https://login.tailscale.com/ 2>/dev/null; }
# после входа: имя узла и приём DNS из админки — через tailscale set (не требует перечислять все флаги, как up)
ts_apply_prefs() {
  tailscale set --accept-dns=true 2>/dev/null || true
  if [[ -n "$TAILSCALE_HOSTNAME" ]]; then tailscale set --hostname="$TAILSCALE_HOSTNAME" 2>/dev/null || warn "не удалось задать имя узла $TAILSCALE_HOSTNAME (tailscale set --hostname)"; sleep 2; fi
}
ts_report_state() { # что делать в текущем состоянии узла, если он не Running
  case "$(ts_state)" in
    NeedsMachineAuth) warn "узел вошёл в tailnet и ждёт одобрения администратора: $TS_ADMIN_MACHINES (Machines → Needs approval → Approve). После одобрения соединение поднимется само; затем выполните sudo $0 tailscale" ;;
    NeedsLogin) local url; url="$(ts_authurl)"
      if [[ -n "$url" ]]; then warn "узел не авторизован. Откройте ссылку и подтвердите узел (ссылка одноразовая; новую даст мастер первого запуска или sudo $0 tailscale): $url"
      else warn "узел не авторизован (tailscale status: NeedsLogin). Авторизуйте позже из мастера первого запуска в браузере или командой sudo $0 tailscale"; fi ;;
    *) warn "Tailscale не подключён (состояние $(ts_state)). Установка продолжится по IP; позже — sudo $0 tailscale или мастер первого запуска" ;;
  esac
}

# Авторизация узла. `tailscale up` без пользовательских флагов (иначе на узле с настройками он требует перечислить их все):
# печатает ссылку и QR и ждёт, пока узел не станет Running (или до --timeout). Без терминала — только получить ссылку.
ts_authorize() {
  local wait_s; wait_s="$(to_secs "$TAILSCALE_WAIT")"
  local key="${TAILSCALE_AUTH_KEY:-}"
  [[ -z "$key" && -n "${TAILSCALE_AUTH_KEY_FILE:-}" && -r "${TAILSCALE_AUTH_KEY_FILE:-}" ]] && key="file:$TAILSCALE_AUTH_KEY_FILE"
  if [[ -n "$key" && "$(ts_state)" != Running ]]; then
    log "Tailscale: авторизация ключом"
    if tailscale up --auth-key="$key" --timeout 2m; then ok "узел авторизован ключом"; else warn "tailscale up с ключом не удался — ниже обычная авторизация по ссылке"; fi
  fi
  if [[ "$(ts_state)" == Stopped ]]; then   # уже вошёл, но выключен: «простой up» без флагов
    if timeout 70 tailscale up --timeout 60s; then ok "Tailscale включён"; fi
  fi
  case "$(ts_state)" in Running) ts_apply_prefs; return 0 ;; NeedsMachineAuth) ts_report_state; return 1 ;; esac
  if ! ts_control_reachable; then
    warn "controlplane.tailscale.com недоступен — авторизация сейчас невозможна (нет интернета или закрыт исходящий HTTPS). Продолжаю без Tailscale."
    return 1
  fi
  if interactive && (( wait_s > 0 )); then
    printf '\n%sСервер нужно добавить в вашу сеть Tailscale.%s\n' "$c_bold" "$c_off"
    printf '  1. Откройте ссылку ниже (или отсканируйте QR с телефона) и войдите в аккаунт Tailscale.\n'
    printf '  2. Подтвердите добавление узла. Если в tailnet включено одобрение устройств — одобрите его: %s\n' "$TS_ADMIN_MACHINES"
    printf 'Жду до %s. Не успеете — установка продолжится без Tailscale (Desktop по IP, сертификат внутреннего CA), а подключить его\nможно позже из мастера первого запуска в браузере. Не ждать вообще: TAILSCALE_WAIT=0.\n\n' "$(human_dur "$wait_s")"
    if tailscale up --qr --timeout "${wait_s}s"; then
      ts_apply_prefs
      ok "узел авторизован: $(ts_dnsname) ($(ts_ipv4))"
      return 0
    fi
    if [[ "$(ts_state)" == NeedsMachineAuth ]]; then ts_report_state; return 1; fi
    warn "авторизация не завершена за $(human_dur "$wait_s"). Продолжаю установку без Tailscale: Desktop откроется по IP локальной сети (браузер предупредит о сертификате)."
    warn "Подключить Tailscale позже: мастер первого запуска в браузере или sudo $0 tailscale (шаг сам переключит ядро и Coder на имя узла)."
    return 1
  fi
  # без терминала (или TAILSCALE_WAIT=0): запустить вход (он остаётся ждать в tailscaled), забрать ссылку из status и продолжить
  timeout 10 tailscale up --timeout 6s >/dev/null 2>&1 || true
  ts_report_state
  return 1
}

# MagicDNS и HTTPS Certificates — настройки tailnet в админке; без них имя не резолвится на устройствах, а сертификат не выдаётся.
ts_wait_dns_https() {
  local wait_s; wait_s="$(to_secs "$TAILSCALE_WAIT")"; interactive || wait_s=0
  local end=$(( $(date +%s) + wait_s )) shown=0 magic https
  while :; do
    magic="$(ts_magicdns)"; https="$(ts_https)"
    if [[ "$magic" == true && "$https" == true ]]; then ok "MagicDNS и HTTPS Certificates включены: имя $(ts_dnsname), сертификат Let's Encrypt через tailscaled"; return 0; fi
    if (( shown == 0 )); then
      shown=1
      printf '\n%sДля имени с сертификатом Let'"'"'s Encrypt в tailnet должны быть включены:%s' "$c_bold" "$c_off"
      [[ "$magic" == true ]] || printf ' MagicDNS (устройства находят сервер по имени);'
      [[ "$https" == true ]] || printf ' HTTPS Certificates (сертификат для %s);' "$(ts_dnsname)"
      printf '\n  откройте %s → DNS: «MagicDNS» → Enable, «HTTPS Certificates» → Enable HTTPS.\n' "$TS_ADMIN_DNS"
      if (( wait_s > 0 )); then printf '  Жду до %s, проверяю каждые 5 с. Если переключатели уже включены, а статус не меняется больше минуты:\n  sudo systemctl restart tailscaled (туннель прервётся на секунды) и повторите sudo %s tailscale.\n' "$(human_dur "$wait_s")" "$0"; fi
    fi
    if (( $(date +%s) >= end )); then
      warn "MagicDNS и/или HTTPS Certificates не включены — ядро пока отвечает по IP с внутренним CA. Когда включите их, выполните sudo $0 tailscale (или в мастере первого запуска нажмите «Переключить ядро на $(ts_dnsname)»)."
      return 1
    fi
    sleep 5
  done
}

# Имя ядра — один раз за запуск: явный EDGE_HOST → своё имя из cloudd.env (не IP и не устаревшее *.ts.net) → имя Tailscale
# (Running, MagicDNS и HTTPS включены) → уже записанный IP → IPv4 хоста. С IP на имя переходим сами, со своего имени — нет.
resolve_edge_host() {
  if (( EDGE_RESOLVED )); then return 0; fi
  local existing tsname=""
  existing="$(env_get CLOUDD_EDGE_HOST "$ENV_FILE")"
  if [[ "$(ts_state)" == Running && "$(ts_magicdns)" == true && "$(ts_https)" == true ]]; then tsname="$(ts_dnsname)"; fi
  if [[ -n "$EDGE_HOST_EXPLICIT" ]]; then EDGE_HOST="$EDGE_HOST_EXPLICIT"
  elif [[ -n "$existing" && "$existing" == *.ts.net && -n "$tsname" && "$tsname" != "$existing" ]]; then
    EDGE_HOST="$tsname"; warn "узел Tailscale переименован: $existing → $tsname — ядро переходит на новое имя"
  elif [[ -n "$existing" ]] && ! is_ipv4 "$existing"; then
    EDGE_HOST="$existing"
    if [[ -n "$tsname" && "$tsname" != "$existing" ]]; then warn "имя узла Tailscale $tsname отличается от своего имени ядра $existing — оставляю прежнее (сменить: EDGE_HOST=$tsname sudo $0 cloudd coder)"; fi
  elif [[ -n "$tsname" ]]; then EDGE_HOST="$tsname"
  elif [[ -n "$existing" ]]; then EDGE_HOST="$existing"
  else EDGE_HOST="$(lan_ip)"; fi
  [[ -n "$EDGE_HOST" ]] || die "не удалось определить адрес хоста — задайте EDGE_HOST"
  if [[ "$EDGE_HOST" == *.ts.net ]]; then TLS_INTERNAL=false; else TLS_INTERNAL=true; fi
  EDGE_RESOLVED=1
}

# Записать имя ядра в cloudd.env: EDGE_HOST, TLS_INTERNAL, запасные входы по IP (объединяются с уже записанными).
write_edge_env() {
  resolve_edge_host
  env_set "$ENV_FILE" CLOUDD_EDGE_HOST "$EDGE_HOST"
  env_set "$ENV_FILE" CLOUDD_TLS_INTERNAL "$TLS_INTERNAL"
  local alt=() h lan ts; lan="$(lan_ip)"; ts="$(ts_ipv4)"
  IFS=',' read -r -a alt <<< "$(env_get CLOUDD_EDGE_ALT_HOSTS "$ENV_FILE")"
  for h in "$lan" "$ts"; do
    [[ -n "$h" ]] || continue
    local dup=0 x; for x in "${alt[@]}"; do [[ "$x" == "$h" ]] && dup=1; done
    (( dup )) || alt+=("$h")
  done
  local out=(); for h in "${alt[@]}"; do [[ -n "$h" && "$h" != "$EDGE_HOST" ]] && out+=("$h"); done
  env_set "$ENV_FILE" CLOUDD_EDGE_ALT_HOSTS "$(IFS=,; echo "${out[*]}")"
}

# Пакет обновления (PACKAGE): скачать, распаковать во временный каталог, сверить sha256 бинаря с manifest.json (и файла с PACKAGE_SHA256).
resolve_package() {
  [[ -n "$PACKAGE" && -z "$PKG_DIR" ]] || return 0
  local src="$PACKAGE"
  mkdir -p "$STATE/updates"
  PKG_DIR="$(mktemp -d "$STATE/updates/install.XXXXXX")"
  if [[ "$src" =~ ^https:// ]]; then curl -fsSL "$src" -o "$PKG_DIR/pkg.tar.gz" || die "не удалось скачать $src"; src="$PKG_DIR/pkg.tar.gz"
  elif [[ "$src" =~ ^http:// ]]; then die "пакет по http:// не принимаю (подмена по пути даёт root на сервере) — используйте https:// или файл"; fi
  [[ -f "$src" ]] || die "пакет $src не найден"
  if [[ -n "${PACKAGE_SHA256:-}" ]]; then
    local got; got="$(sha256sum "$src" | cut -d' ' -f1)"
    [[ "$got" == "${PACKAGE_SHA256,,}" ]] || die "sha256 пакета не совпадает с PACKAGE_SHA256"
  fi
  tar -xzf "$src" -C "$PKG_DIR" || die "пакет не распаковывается (нужен tar.gz из scripts/release.sh)"
  [[ -f "$PKG_DIR/manifest.json" && -f "$PKG_DIR/cloudd" ]] || die "в пакете нет manifest.json или cloudd"
  local want got; want="$(jq -r .cloudd_sha256 "$PKG_DIR/manifest.json")"; got="$(sha256sum "$PKG_DIR/cloudd" | cut -d' ' -f1)"
  [[ "$want" == "$got" ]] || die "пакет повреждён: sha256 бинаря не совпадает с manifest.json"
  local arch; arch="$(jq -r '.arch // empty' "$PKG_DIR/manifest.json")"
  [[ -z "$arch" || "$arch" == "$(uname -m)"* ]] || die "пакет для $arch, а хост $(uname -m)"
  chmod 0755 "$PKG_DIR/cloudd"
  ok "пакет: AgentVerse OS $(jq -r .version "$PKG_DIR/manifest.json") ($(jq -r '.build // "?"' "$PKG_DIR/manifest.json"))"
}
# источник бинаря cloudd: пакет → CLOUDD_BIN → сборка в checkout → бинарь в корне распакованного пакета
cloudd_source() {
  if [[ -n "$PKG_DIR" ]]; then echo "$PKG_DIR/cloudd"; return; fi
  for c in "$CLOUDD_BIN" "$REPO/cloudd/target/release/cloudd" "$REPO/cloudd"; do [[ -n "$c" && -f "$c" && -x "$c" ]] && { echo "$c"; return; }; done
  echo ""
}
templates_dir() { for d in "$REPO/templates/incus" "${PKG_DIR:+$PKG_DIR/templates/incus}"; do [[ -n "$d" && -f "$d/main.tf" ]] && { echo "$d"; return; }; done; echo ""; }

# -------------------------------------------------------------------------------------------------------------- шаги
step_preflight() {
  log "проверка хоста"
  need_root
  local id ver
  # shellcheck source=/dev/null
  id="$(. /etc/os-release && echo "${ID:-}")"; ver="$(. /etc/os-release && echo "${VERSION_ID:-0}")"
  if [[ "$id" == ubuntu ]]; then (( ${ver%%.*} >= 22 )) || die "нужна Ubuntu 22.04 или новее (здесь $ver)"; ok "Ubuntu $ver"
  else warn "проверено на Ubuntu, здесь $id $ver — продолжаю на ваш страх и риск"; fi
  case "$(uname -m)" in x86_64|aarch64) ok "архитектура $(uname -m)" ;; *) die "архитектура $(uname -m) не поддерживается" ;; esac
  have systemctl || die "нужен systemd"
  for f in host/install-host.sh host/zfs-layout.sh host/docker-daemon.json coder/docker-compose.yml docker-proxy/docker-compose.yml edge/docker-compose.yml edge/Caddyfile.bootstrap systemd/cloudd.service komodo/ferretdb.compose.yaml komodo/compose.env.template komodo/compose.override.yaml update.sh; do
    [[ -f "$HERE/$f" ]] || die "нет файла $HERE/$f — нужен полный каталог bootstrap/ (git-checkout или распакованный пакет обновления)"
  done
  local mem free
  mem="$(awk '/MemTotal/ {print int($2/1024/1024)}' /proc/meminfo)"
  if (( mem >= 4 )); then ok "память ${mem} ГБ"; else warn "памяти ${mem} ГБ — мало для Coder, Komodo и workspaces (рекомендуется от 8 ГБ)"; fi
  free="$(df -BG --output=avail / | tail -1 | tr -dc 0-9)"
  if (( free >= 20 )); then ok "свободно на /: ${free} ГБ"; else warn "на / свободно ${free} ГБ — мало (рекомендуется от 20 ГБ; образы Docker живут в /var/lib/docker, если не на ZFS)"; fi
  if curl -fsS -m 8 -o /dev/null https://tailscale.com/ && curl -fsS -m 8 -o /dev/null https://download.docker.com/; then ok "интернет доступен"; else warn "нет доступа к tailscale.com или download.docker.com — установка пакетов может не пройти"; fi
  local p
  for p in 80 443 8444; do
    if ss -ltn 2>/dev/null | awk '{print $4}' | grep -qE "[:.]$p\$" && ! docker ps --format '{{.Names}}' 2>/dev/null | grep -q '^cloudos-edge$'; then warn "порт $p уже занят — edge Caddy не поднимется"; fi
  done
  resolve_package
  local src; src="$(cloudd_source)"
  [[ -n "$src" ]] || die "нет ни PACKAGE, ни собранного бинаря cloudd. Соберите пакет на машине разработчика (scripts/release.sh → dist/agentverse-os-*.tar.gz) и задайте PACKAGE=…, либо собранный бинарь: CLOUDD_BIN=…"
  ok "источник ядра: $src"
}

step_zfs() {
  log "ZFS: пул и datasets"
  export DEBIAN_FRONTEND=noninteractive
  have zpool || { apt-get update -q && apt-get install -y -q zfsutils-linux; }
  if zpool list tank >/dev/null 2>&1; then
    ARC_MAX_GB="$ARC_MAX_GB" "$HERE/host/zfs-layout.sh" datasets
  elif [[ -n "${DISKS:-}" ]]; then
    printf '%sПул tank будет создан на дисках (все данные на них будут уничтожены):%s\n' "$c_bold" "$c_off"
    # shellcheck disable=SC2086
    lsblk -o NAME,SIZE,MODEL,FSTYPE,MOUNTPOINTS $DISKS 2>/dev/null || true
    if [[ "${DISKS_FORCE:-}" != 1 ]]; then confirm "Продолжить?" || die "отменено (без вопросов: DISKS_FORCE=1)"; fi
    DISKS="$DISKS" ARC_MAX_GB="$ARC_MAX_GB" "$HERE/host/zfs-layout.sh" create
  else
    warn "пула tank нет и DISKS не задан — пропускаю: данные лягут в корневую ФС, Incus на драйвере dir; снимки и бэкапы ZFS работать не будут"
    mkdir -p /srv/apps /srv/postgres /srv/backups /var/lib/docker
  fi
}

step_host() {
  log "хост: Docker, Incus, sysctl, firewall"
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -q && apt-get install -y -q curl ca-certificates gnupg jq zfsutils-linux restic rsync openssl iproute2
  have docker || "$HERE/host/install-host.sh" docker
  have incus  || "$HERE/host/install-host.sh" incus
  incus storage show default >/dev/null 2>&1 || "$HERE/host/install-host.sh" incus
  "$HERE/host/install-host.sh" firewall
  usermod -aG docker,incus-admin "${SUDO_USER:-root}" 2>/dev/null || true
  ok "Docker $(docker --version | awk '{print $3}' | tr -d ,) · Incus $(incus version 2>/dev/null | head -1)"
}

step_tailscale() {
  log "Tailscale: overlay-сеть, имя ядра и сертификат"
  if ! have tailscale; then
    local sh; sh="$(mktemp)"
    if curl -fsSL https://tailscale.com/install.sh -o "$sh" && sh "$sh"; then ok "Tailscale установлен"; else warn "Tailscale не установлен (нет сети или репозиторий недоступен) — продолжаю по IP; позже: sudo $0 tailscale"; rm -f "$sh"; resolve_edge_host; return 0; fi
    rm -f "$sh"
  fi
  systemctl enable --now tailscaled
  wait_for 20 "tailscaled поднялся" tailscale status --json || true
  if [[ "$(ts_state)" != Running ]]; then ts_authorize || true; fi
  if [[ "$(ts_state)" == Running ]]; then
    ts_apply_prefs
    ok "узел в tailnet: $(ts_dnsname) · $(ts_ipv4) · Tailscale $(tailscale version | head -1)"
    ts_wait_dns_https || true
  fi
  resolve_edge_host
  ok "адрес ядра: https://$EDGE_HOST/ (сертификат: $([[ $TLS_INTERNAL == false ]] && echo "Let's Encrypt через tailscaled" || echo 'внутренний CA — временно, до имени Tailscale'))"
  # шаг вызван отдельно после установки: применить новое имя к ядру и Coder
  if [[ -f "$ENV_FILE" && -n "$(env_get CLOUDD_EDGE_HOST "$ENV_FILE")" && "$(env_get CLOUDD_EDGE_HOST "$ENV_FILE")" != "$EDGE_HOST" ]]; then
    warn "имя ядра меняется: $(env_get CLOUDD_EDGE_HOST "$ENV_FILE") → $EDGE_HOST — обновляю cloudd.env, перезапускаю cloudd и Coder"
    write_edge_env
    systemctl try-restart cloudd 2>/dev/null || true
    if docker ps --format '{{.Names}}' | grep -q coder; then step_coder_compose; warn "у Coder новый access URL — запущенные workspace нужно перезапустить (карточка проекта → стоп → старт)"; fi
  fi
}

step_coder_compose() { # поднять/пересобрать compose Coder с текущим EDGE_HOST
  resolve_edge_host
  mkdir -p /srv/postgres/coder
  export EDGE_HOST INCUS_ADMIN_GID; INCUS_ADMIN_GID="$(getent group incus-admin | cut -d: -f3)"
  docker compose -p cloudos-coder -f "$HERE/coder/docker-compose.yml" up -d
  wait_for 120 "Coder /healthz" curl -fs -m 2 http://127.0.0.1:7080/healthz
}

step_coder() {
  log "Coder + PostgreSQL"
  step_coder_compose
  have coder || { local sh; sh="$(mktemp)"; curl -fsSL https://coder.com/install.sh -o "$sh" && sh "$sh" --method standalone >/dev/null; rm -f "$sh"; }
  local pw; pw="$(admin_password)"
  if ! coder_cli users list >/dev/null 2>&1; then
    coder login http://127.0.0.1:7080 --first-user-username admin --first-user-email "$ADMIN_EMAIL" --first-user-password "$pw" --first-user-trial=false </dev/null >/dev/null 2>&1 || true
  fi
  if [[ -z "$(env_get CLOUDD_CODER_TOKEN "$ENV_FILE")" ]]; then
    # сессия без интерактива: логин по паролю администратора через API, затем токен на год с уникальным именем
    local sess tok
    sess="$(curl -fsS -m 10 -X POST http://127.0.0.1:7080/api/v2/users/login -H 'Content-Type: application/json' -d "$(jq -cn --arg e "$ADMIN_EMAIL" --arg p "$pw" '{email: $e, password: $p}')" 2>/dev/null | jq -r '.session_token // empty' || true)"
    if [[ -n "$sess" ]]; then
      tok="$(CODER_URL=http://127.0.0.1:7080 CODER_SESSION_TOKEN="$sess" coder tokens create --lifetime 8760h --name "cloudd-$(date +%s)" </dev/null 2>/dev/null | grep -oE '[A-Za-z0-9]{10}-[A-Za-z0-9]{22}' | head -1 || true)"
    else
      tok="$(coder tokens create --lifetime 8760h --name "cloudd-$(date +%s)" </dev/null 2>/dev/null | grep -oE '[A-Za-z0-9]{10}-[A-Za-z0-9]{22}' | head -1 || true)"
    fi
    if [[ -n "$tok" ]]; then env_set "$ENV_FILE" CLOUDD_CODER_TOKEN "$tok"; ok "токен Coder для cloudd создан"
    else warn "токен Coder не создан (вход администратора $ADMIN_EMAIL не удался). Проверьте пароль CLOUDOS_ADMIN_PASSWORD в $ENV_FILE и повторите: sudo $0 coder"; fi
  fi
}

step_komodo() {
  log "Komodo (App Runtime)"
  mkdir -p "$KOMODO_DIR/periphery"
  [[ -f "$KOMODO_DIR/ferretdb.compose.yaml" ]] || install -m 0644 "$HERE/komodo/ferretdb.compose.yaml" "$KOMODO_DIR/ferretdb.compose.yaml"
  install -m 0644 "$HERE/komodo/compose.override.yaml" "$KOMODO_DIR/compose.override.yaml"
  if [[ ! -f "$KOMODO_DIR/compose.env" ]]; then
    install -m 0600 "$HERE/komodo/compose.env.template" "$KOMODO_DIR/compose.env"
    local pw; pw="$(admin_password)"
    env_set "$KOMODO_DIR/compose.env" KOMODO_HOST http://127.0.0.1:9120
    env_set "$KOMODO_DIR/compose.env" KOMODO_INIT_ADMIN_PASSWORD "$pw"
    env_set "$KOMODO_DIR/compose.env" KOMODO_WEBHOOK_SECRET "$(rand)"
    env_set "$KOMODO_DIR/compose.env" KOMODO_JWT_SECRET "$(rand 24)"
    env_set "$KOMODO_DIR/compose.env" KOMODO_DATABASE_PASSWORD "$(rand 12)"
    env_set "$KOMODO_DIR/compose.env" PERIPHERY_ROOT_DIRECTORY "$KOMODO_DIR/periphery"
    env_set "$KOMODO_DIR/compose.env" KOMODO_DISABLE_USER_REGISTRATION true
    grep -qE 'changeme|a_random' "$KOMODO_DIR/compose.env" && die "в compose.env остались значения по умолчанию — шаблон bootstrap/komodo/compose.env.template расходится с ожиданиями установщика"
  fi
  (cd "$KOMODO_DIR" && docker compose -p komodo --env-file compose.env -f ferretdb.compose.yaml -f compose.override.yaml up -d)
  wait_for 180 "Komodo на :9120" curl -fs -m 2 http://127.0.0.1:9120/
  local cur; cur="$(env_get CLOUDD_KOMODO_KEY "$ENV_FILE")"
  if [[ -z "$cur" || "$cur" == null ]]; then
    sleep 5
    local pw jwt resp k s; pw="$(env_get KOMODO_INIT_ADMIN_PASSWORD "$KOMODO_DIR/compose.env")"
    jwt="$(curl -fsS -m 15 -X POST http://127.0.0.1:9120/auth/login -H 'Content-Type: application/json' -d "$(jq -cn --arg p "$pw" '{type: "LoginLocalUser", params: {username: "admin", password: $p}}')" | jq -r '.data.jwt // empty' || true)"
    [[ -n "$jwt" ]] || die "не удалось войти в Komodo (пароль admin из $KOMODO_DIR/compose.env)"
    resp="$(curl -fsS -m 15 -X POST http://127.0.0.1:9120/auth/manage -H "Authorization: Bearer $jwt" -H 'Content-Type: application/json' -d '{"type":"CreateApiKey","params":{"name":"cloudd","expires":0}}' || true)"
    k="$(jq -r '.key // empty' <<< "$resp")"; s="$(jq -r '.secret // empty' <<< "$resp")"
    [[ -n "$k" && -n "$s" ]] || die "Komodo не выдал ключ API: ${resp:0:300}"
    env_set "$ENV_FILE" CLOUDD_KOMODO_KEY "$k"
    env_set "$ENV_FILE" CLOUDD_KOMODO_SECRET "$s"
    ok "ключ API Komodo для cloudd создан"
  fi
}

step_edge() {
  log "read-only docker-прокси для приложений Store"
  docker compose -p cloudos-docker-proxy -f "$HERE/docker-proxy/docker-compose.yml" up -d
  log "edge Caddy (конфигурацию загружает cloudd)"
  docker network inspect coder-net >/dev/null 2>&1 || docker network create coder-net
  mkdir -p /var/run/tailscale   # каталог сокета tailscaled монтируется в edge; без Tailscale он пустой
  docker compose -p cloudos-edge -f "$HERE/edge/docker-compose.yml" up -d
  wait_for 60 "Admin API Caddy" curl -fs -m 2 http://127.0.0.1:2019/config/ || die "edge Caddy не поднялся: docker logs cloudos-edge"
  ok "edge поднят (корень внутреннего CA появится после загрузки конфигурации ядром — шаг cloudd)"
}

capture_ca() { # корень внутреннего CA Caddy → $STATE/ca.crt (создаётся Caddy при первой конфигурации с tls internal)
  mkdir -p "$STATE"
  if wait_for 60 "корень внутреннего CA" docker cp cloudos-edge:/data/caddy/pki/authorities/local/root.crt "$STATE/ca.crt"; then
    chmod 0644 "$STATE/ca.crt"; ok "корень CA: $STATE/ca.crt (для устройств: https://$EDGE_HOST/api/ca.crt)"
  else warn "корень внутреннего CA не получен — устройствам пока придётся принимать сертификат вручную"; fi
}

step_cloudd() {
  log "cloudd (ядро и Desktop)"
  resolve_package
  local src; src="$(cloudd_source)"
  [[ -n "$src" ]] || die "нет бинаря cloudd: задайте PACKAGE=…tar.gz (scripts/release.sh) или CLOUDD_BIN=…"
  install -m 0755 "$src" /usr/local/bin/cloudd
  mkdir -p "$STATE/repo/store" "$STATE/repo/projects" /etc/cloudos
  # каталог Store: из checkout — только чего ещё нет (не откатывать более свежие манифесты рантайма); из пакета — рукописные манифесты, они авторитетны
  if [[ -d "$REPO/store" ]]; then rsync -a --ignore-existing --exclude compose.override.yaml --exclude login.yaml "$REPO/store/" "$STATE/repo/store/"; fi
  if [[ -n "$PKG_DIR" && -d "$PKG_DIR/store" ]]; then rsync -a --exclude compose.override.yaml --exclude login.yaml "$PKG_DIR/store/" "$STATE/repo/store/"; fi
  # projects/ — состояние рантайма, из checkout не копируется (примеры воскрешали бы удалённые проекты)
  write_edge_env
  env_set "$ENV_FILE" CLOUDD_REPO_DIR "$STATE/repo"
  env_set "$ENV_FILE" CLOUDD_STATE_DIR "$STATE"
  env_set "$ENV_FILE" CLOUDD_CODER_URL http://127.0.0.1:7080
  env_set "$ENV_FILE" CLOUDD_CODER_PORT 8444
  env_set "$ENV_FILE" CLOUDD_CODER_TEMPLATE cloudos-incus
  env_set "$ENV_FILE" CLOUDD_KOMODO_URL http://127.0.0.1:9120
  env_set "$ENV_FILE" CLOUDD_KOMODO_SERVER Local
  env_set "$ENV_FILE" CLOUDD_EDGE_CONTAINER cloudos-edge
  if [[ -n "$ADMIN_EMAIL_EXPLICIT" || -z "$(env_get CLOUDOS_ADMIN_EMAIL "$ENV_FILE")" ]]; then env_set "$ENV_FILE" CLOUDOS_ADMIN_EMAIL "$ADMIN_EMAIL"; fi
  admin_password >/dev/null
  chmod 0600 "$ENV_FILE"
  install -m 0644 "$HERE/systemd/cloudd.service" /etc/systemd/system/cloudd.service
  install -m 0755 "$HERE/update.sh" /usr/local/bin/agentverse-update
  systemctl daemon-reload && systemctl enable --now cloudd && systemctl restart cloudd
  wait_for 60 "cloudd на :7100" curl -fs -m 2 http://127.0.0.1:7100/api/status || die "cloudd не поднялся: journalctl -u cloudd -n 50"
  ok "cloudd $(curl -s http://127.0.0.1:7100/api/status | jq -r '"\(.version) (\(.build // "dev"))"') слушает 127.0.0.1:7100; обновления: sudo agentverse-update"
  if [[ "$TLS_INTERNAL" == true ]]; then capture_ca; fi
}

step_catalog() {
  log "каталог Store"
  local n; n="$(find "$STATE/repo/store" -mindepth 2 -maxdepth 2 -name manifest.yaml 2>/dev/null | wc -l)"
  if [[ "${IMPORT_CATALOG:-}" == 1 || "$n" -lt 50 ]]; then
    echo "манифестов: $n — импортирую каталоги Runtipi, Coolify и Umbrel (1–3 минуты, нужен интернет)…"
    CLOUDD_REPO_DIR="$STATE/repo" timeout 15m cloudd store import all --dir "$STATE/repo/store" || warn "импорт каталога не удался — повторить позже: sudo agentverse-update --catalog"
    n="$(find "$STATE/repo/store" -mindepth 2 -maxdepth 2 -name manifest.yaml | wc -l)"
  else
    echo "каталог из поставки (upstream не опрашивался); обновить позже: sudo agentverse-update --catalog"
  fi
  ok "манифестов в каталоге: $n"
}

step_template() {
  log "шаблон Coder → Incus"
  local dir; dir="$(templates_dir)"
  [[ -n "$dir" ]] || die "нет templates/incus ни в checkout, ни в пакете — шаблон cloudos-incus не создать; возьмите полный пакет обновления или git-checkout"
  [[ -f "$STATE/ca.crt" ]] && cp "$STATE/ca.crt" "$dir/ca.crt"
  rm -rf "$dir/.terraform" "$dir/"*.tfstate*
  if coder_cli templates push cloudos-incus -d "$dir" -y --message "bootstrap $(date -Is)" >/dev/null; then ok "шаблон cloudos-incus обновлён"; else warn "шаблон не загружен — повторите: sudo $0 template"; fi
}

step_verify() {
  log "проверка"
  resolve_edge_host
  cloudd status || true
  local cacert=(); [[ "$TLS_INTERNAL" == true && -f "$STATE/ca.crt" ]] && cacert=(--cacert "$STATE/ca.crt")
  curl -s "${cacert[@]}" -m 8 -o /dev/null -w "Desktop https://$EDGE_HOST/ → %{http_code}\n" "https://$EDGE_HOST/" || warn "Desktop по https://$EDGE_HOST/ не ответил (сертификат ещё выдаётся или имя не резолвится на хосте)"
  curl -s "${cacert[@]}" -m 8 -o /dev/null -w "Coder   https://$EDGE_HOST:8444/ → %{http_code}\n" "https://$EDGE_HOST:8444/healthz" || true
  printf '\n%s┌ AgentVerse OS установлен%s\n' "$c_bold" "$c_off"
  printf '│ Desktop:    https://%s/\n' "$EDGE_HOST"
  printf '│ Coder:      https://%s:8444/  (вход: %s; пароль: sudo grep CLOUDOS_ADMIN_PASSWORD %s — он же у admin в Komodo)\n' "$EDGE_HOST" "$ADMIN_EMAIL" "$ENV_FILE"
  if [[ "$(ts_state)" == Running ]]; then printf '│ Tailscale:  %s · %s\n' "$(ts_dnsname)" "$(ts_ipv4)"; else printf '│ Tailscale:  не подключён — мастер первого запуска даст ссылку на авторизацию\n'; fi
  if [[ "$TLS_INTERNAL" == true ]]; then
    printf '│ Сертификат: внутренний CA (браузер предупредит; корень: https://%s/api/ca.crt).\n' "$EDGE_HOST"
    if [[ "$(ts_state)" == Running ]]; then printf '│             Включите MagicDNS и HTTPS Certificates в %s, затем sudo %s tailscale — ядро перейдёт на %s с Let'"'"'s Encrypt.\n' "$TS_ADMIN_DNS" "$0" "$(ts_dnsname)"
    else printf '│             После подключения Tailscale (мастер первого запуска или sudo %s tailscale) ядро перейдёт на имя узла с Let'"'"'s Encrypt.\n' "$0"; fi
  else printf '│ Сертификат: Let'"'"'s Encrypt через tailscaled\n'; fi
  printf '│ Дальше:     откройте Desktop — мастер первого запуска проверит сеть и создаст первый проект.\n'
  printf '│ Обновления: окно «Обновления» в Desktop или sudo agentverse-update (пакет, канал, приложения, компоненты, хост).\n'
  printf '└ Журнал:     %s\n' "$LOG"
}

# ------------------------------------------------------------------------------------------------------------- запуск
main() {
  local arg="${1:-all}"
  case "$arg" in -h|--help|help) usage; exit 0 ;; esac
  need_root
  local steps
  case "$arg" in
    preflight|zfs|host|tailscale|coder|komodo|edge|cloudd|catalog|template|verify) steps=("$@") ;;
    all) steps=(preflight zfs host tailscale coder komodo edge cloudd catalog template verify) ;;
    *) usage; die "неизвестный шаг «$arg»" ;;
  esac
  for s in "${steps[@]}"; do [[ "$s" =~ ^(preflight|zfs|host|tailscale|coder|komodo|edge|cloudd|catalog|template|verify)$ ]] || die "неизвестный шаг «$s»"; done
  mkdir -p "$(dirname "$LOG")"; [[ -f "$LOG" ]] || install -m 0600 /dev/null "$LOG"; chmod 0600 "$LOG"
  exec > >(tee -a "$LOG") 2>&1; TEE_PID=$!
  printf '\n### %s · agentverse install %s\n' "$(date -Is)" "${steps[*]}"
  to_secs "$TAILSCALE_WAIT" >/dev/null
  for STEP in "${steps[@]}"; do "step_$STEP"; done
  STEP=""
}
main "$@"
