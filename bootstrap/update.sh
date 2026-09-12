#!/usr/bin/env bash
# AgentVerse OS · обновление с хоста: то же, что окно «Обновления» в Desktop, но из консоли и cron. Ставится как
# /usr/local/bin/agentverse-update (bootstrap/install.sh). Работает через API ядра на 127.0.0.1:7100; если ядро не отвечает,
# пакет ставится напрямую (с .prev и откатом). Нужны curl и jq.
#
#   sudo agentverse-update                                  # статус: версия, канал, что можно обновить (то же: --status)
#   sudo agentverse-update <пакет.tar.gz | https-URL>       # установить пакет agentverse-os-<версия>-<arch>.tar.gz (--sha256 <hex> — сверить файл)
#   sudo agentverse-update --channel https://…/stable.json  # задать канал обновлений, проверить и поставить новее, если есть
#   sudo agentverse-update --check                          # только проверить канал (код 0 — есть обновление, 1 — нет, 2 — канал недоступен)
#   sudo agentverse-update --apps                           # обновить приложения с изменениями в каталоге (--apps-all — все, включая плавающие теги)
#   sudo agentverse-update --components [проект]            # edge / Coder / Komodo / docker-прокси: docker compose pull && up -d (без проекта — все четыре)
#   sudo agentverse-update --catalog                        # переимпорт каталога Store из upstream
#   sudo agentverse-update --host                           # apt upgrade пакетов хоста (Docker, Incus, Tailscale не трогаются: --host-all)
#   sudo agentverse-update --rollback                       # вернуть предыдущую версию ядра (.prev)
#   sudo agentverse-update --all                            # пакет из канала (если новее) + приложения + компоненты + хост — для cron с --yes
#   --yes, -y                                               # без вопросов (без терминала вопросы считаются отказом)
#   API=http://host:port/api                                # другой адрес ядра (по умолчанию http://127.0.0.1:7100/api)
set -Eeuo pipefail
API="${API:-http://127.0.0.1:7100/api}"
BIN=/usr/local/bin/cloudd
STATE=/var/lib/cloudos
YES="${YES:-}"
SHA=""
KNOWN_COMPONENTS='^(Edge|Coder|Komodo|Docker-прокси)'

c_bold=$'\033[1m'; c_ok=$'\033[32m'; c_warn=$'\033[33m'; c_err=$'\033[31m'; c_off=$'\033[0m'
log()  { printf '\n%s== %s%s\n' "$c_bold" "$*" "$c_off"; }
ok()   { printf '%s✓%s %s\n' "$c_ok" "$c_off" "$*"; }
warn() { printf '%s! %s%s\n' "$c_warn" "$*" "$c_off"; }
die()  { printf '%s✗ %s%s\n' "$c_err" "$*" "$c_off" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }
need_root() { [[ $EUID -eq 0 ]] || die "запускать через sudo"; }
usage() { sed -n '2,/^set /{/^set /!p}' "$0" | sed 's/^# \{0,1\}//'; }
confirm() { # да — только явно: --yes или ответ в терминале; без терминала вопрос считается отказом
  [[ -n "$YES" ]] && return 0
  if [[ ! -t 0 ]]; then warn "нет терминала — добавьте --yes, чтобы подтвердить: $1"; return 1; fi
  local a=""; read -r -p "$1 [y/N] " a; [[ "$a" == [yY] ]]
}
for t in curl jq; do have "$t" || die "нужна команда $t (apt install $t)"; done
TMP=""; trap '[[ -n "$TMP" ]] && rm -rf "$TMP"' EXIT

# ------------------------------------------------------------------------------------------------------------ API
core_up() { curl -fs -m 4 "$API/status" >/dev/null 2>&1; }
api() { # api METHOD path [json] [timeout] → тело ответа в stdout; при неуспехе — текст ошибки в stdout и код 1 (без exit,
  # чтобы вызывать в подоболочке: if r="$(api …)"; then …; else warn "$r"; fi)
  local m="$1" p="$2" body="${3:-}" tmo="${4:-900}" out code
  if ! out="$(curl -s -m "$tmo" -X "$m" -H 'Content-Type: application/json' ${body:+-d "$body"} -w '\n%{http_code}' "$API$p")"; then printf 'ядро не отвечает на %s%s' "$API" "$p"; return 1; fi
  code="${out##*$'\n'}"; out="${out%$'\n'*}"
  if [[ "$code" != 2* ]]; then printf '%s %s → HTTP %s: %s' "$m" "$p" "$code" "$(echo "$out" | jq -r '.error // .' 2>/dev/null | head -c 400)"; return 1; fi
  printf '%s' "$out"
}
must() { local out; out="$(api "$@")" || die "$out"; printf '%s' "$out"; }    # то же, но с остановкой скрипта
jbody() { jq -cn "$@"; }
ver_of() { api GET /status | jq -r '"\(.version) (\(.build // "dev"))"' 2>/dev/null || echo "?"; }
wait_core() { # wait_core <сек> — дождаться ответа ядра
  local end=$(( $(date +%s) + ${1:-60} ))
  while ! core_up; do (( $(date +%s) >= end )) && return 1; sleep 2; done
}
# после apply/rollback: дождаться, когда ядро перезапустится и отчитается (update.applied.pending → false), а не просто ответит
wait_applied() { # wait_applied <сборка до> [сек] → печатает result (ok | version_mismatch | rolled_back | timeout)
  local before="$1" end=$(( $(date +%s) + ${2:-120} )) u pending build res
  sleep 3
  while (( $(date +%s) < end )); do
    if u="$(api GET /updates 2>/dev/null)"; then
      pending="$(echo "$u" | jq -r '.system.last_applied.pending // false')"
      build="$(echo "$u" | jq -r '.system.version + " " + .system.build')"
      res="$(echo "$u" | jq -r '.system.last_applied.result // empty')"
      if [[ "$(echo "$u" | jq -r '.system.last_applied.rolled_back // false')" == true && "$build" != "$before" ]]; then echo rolled_back; return 0; fi
      if [[ "$pending" == false && -n "$res" ]]; then echo "$res"; return 0; fi
    fi
    sleep 2
  done
  echo timeout
}
report_applied() { # report_applied <result> <ожидаемая версия>
  case "$1" in
    ok) ok "готово: $(ver_of)" ;;
    version_mismatch) warn "новая версия не поднялась — сторожок вернул предыдущую: работает $(ver_of), ожидалась $2. Причина — $STATE/updates/restart.log и journalctl -u cloudd -n 100; сломанный бинарь сохранён как $BIN.failed"; return 1 ;;
    rolled_back) ok "откат выполнен: $(ver_of)" ;;
    timeout) warn "ядро не отчиталось за 2 минуты (сейчас отвечает: $(ver_of)); смотрите $STATE/updates/restart.log"; return 1 ;;
    *) warn "результат установки: $1 (работает $(ver_of)); смотрите $STATE/updates/restart.log" ;;
  esac
}

# ------------------------------------------------------------------------------------------------------------ статус
do_status() {
  core_up || die "ядро не отвечает на $API — проверьте systemctl status cloudd; пакет можно поставить напрямую: sudo $0 <пакет.tar.gz>"
  local u; u="$(must GET /updates)"
  printf '%sAgentVerse OS%s %s · сборка %s · %s\n' "$c_bold" "$c_off" "$(echo "$u" | jq -r .system.version)" "$(echo "$u" | jq -r '.system.build')" "$(echo "$u" | jq -r .system.binary)"
  local la; la="$(echo "$u" | jq -r '.system.last_applied.result // empty')"
  [[ -n "$la" && "$la" != ok ]] && warn "последняя установка: $la ($(echo "$u" | jq -r '.system.last_applied.at // "?"')) — см. $STATE/updates/restart.log"
  local ch; ch="$(echo "$u" | jq -r '.system.channel_url // empty')"
  if [[ -n "$ch" ]]; then
    printf 'канал: %s\n' "$ch"
    if echo "$u" | jq -e '.system.latest' >/dev/null; then
      if [[ "$(echo "$u" | jq -r .system.available)" == true ]]; then printf '  в канале: %s  ← доступно обновление (sudo agentverse-update --channel %s)\n' "$(echo "$u" | jq -r .system.latest.version)" "$ch"
      else printf '  в канале: %s (у вас актуальная)\n' "$(echo "$u" | jq -r .system.latest.version)"; fi
    else printf '  %s\n' "$(echo "$u" | jq -r '.system.check_error // "проверка ещё не выполнялась: sudo agentverse-update --check"')"; fi
  else printf 'канал обновлений не задан (sudo agentverse-update --channel URL или пакет файлом)\n'; fi
  echo "$u" | jq -r '.system.packages[] | "пакет: \(.file) — \(.version) (\(.build // "?")), \(.relation), манифестов Store: \(.store_apps | length)"'
  if [[ "$(echo "$u" | jq -r .system.can_rollback)" == true ]]; then
    local pv; pv="$(echo "$u" | jq -r '.system.prev_version // empty')"; [[ -n "$pv" ]] || pv="$("$BIN.prev" --version 2>/dev/null | awk '{print $2}' || echo '?')"
    printf 'откат доступен: предыдущая версия %s (sudo agentverse-update --rollback)\n' "$pv"
  fi
  printf '\n%sприложения%s: %s с обновлениями из %s установленных (каталог: %s манифестов)\n' "$c_bold" "$c_off" "$(echo "$u" | jq '[.apps[] | select(.has_update)] | length')" "$(echo "$u" | jq '.apps | length')" "$(echo "$u" | jq .catalog.manifests)"
  echo "$u" | jq -r '.apps[] | select(.has_update) | "  \(.name): \(.installed_version // "—") → \(.catalog_version // "—")\(if .compose_changed then ", изменился compose" else "" end)"'
  echo "$u" | jq -r '.apps[] | select((.has_update | not) and (.floating_tags | length > 0)) | "  \(.name): плавающие теги \(.floating_tags | join(", ")) — обновится по --apps-all"'
  printf '\n%sкомпоненты%s:\n' "$c_bold" "$c_off"
  echo "$u" | jq -r --arg re "$KNOWN_COMPONENTS" '.components[] | select(.title | test($re)) | "  \(.project): \(.title) — \(.containers | map("\(.name) [\(.state)]") | join(", "))\(if .updatable then "" else "  (не обновляется: \(.note // "?"))" end)"'
  local extra; extra="$(echo "$u" | jq -r --arg re "$KNOWN_COMPONENTS" '[.components[] | select(.title | test($re) | not) | .project] | join(", ")')"
  [[ -n "$extra" ]] && printf '  прочие compose-проекты хоста (не трогаются): %s\n' "$extra"
  return 0
}

do_check() { # 0 — есть обновление, 1 — нет, 2 — канал недоступен/не задан
  core_up || { warn "ядро не отвечает на $API"; return 2; }
  local s; if ! s="$(api POST /updates/check)"; then warn "$s"; return 2; fi
  if [[ "$(echo "$s" | jq -r .available)" == true ]]; then ok "доступно обновление $(echo "$s" | jq -r .latest.version) (сейчас $(echo "$s" | jq -r .version))"; return 0; fi
  if echo "$s" | jq -e '.latest' >/dev/null; then ok "в канале $(echo "$s" | jq -r .latest.version) — у вас актуальная $(echo "$s" | jq -r .version)"; return 1; fi
  warn "канал: $(echo "$s" | jq -r '.check_error // "нет данных"')"; return 2
}

# ------------------------------------------------------------------------------------------------------- пакет
# через ядро: загрузить, установить, дождаться перезапуска и результата
apply_via_core() {
  local file="$1" name; name="$(basename "$file")"
  local before; before="$(api GET /status | jq -r '.version + " " + (.build // "dev")')"
  local p code
  p="$(curl -s -m 600 -X POST --data-binary @"$file" -w '\n%{http_code}' "$API/updates/upload?name=$name")" || die "загрузка пакета в ядро не удалась"
  code="${p##*$'\n'}"; p="${p%$'\n'*}"
  [[ "$code" == 2* ]] || die "пакет отклонён: $(echo "$p" | jq -r '.error // .')"
  local pkg ver rel; pkg="$(echo "$p" | jq -r .file)"; ver="$(echo "$p" | jq -r .version)"; rel="$(echo "$p" | jq -r .relation)"
  ok "пакет $pkg проверен: версия $ver ($rel относительно текущей), манифестов Store: $(echo "$p" | jq '.store_apps | length')"
  if [[ "$(echo "$p" | jq -r .arch_ok)" != true ]]; then api DELETE "/updates/packages/$pkg" >/dev/null || true; die "пакет собран для другой архитектуры ($(echo "$p" | jq -r .arch))"; fi
  if [[ "$rel" != newer ]] && ! confirm "Версия $ver не новее текущей ($before). Переустановить?"; then api DELETE "/updates/packages/$pkg" >/dev/null || true; die "отменено"; fi
  local r; r="$(must POST /updates/apply "$(jbody --arg f "$pkg" '{file: $f}')")"
  ok "установлено $(echo "$r" | jq -r .to), ядро перезапускается (автооткат, если не ответит за 45 с)…"
  local res; res="$(wait_applied "$before" 120)"
  if [[ "$res" == ok ]]; then api DELETE "/updates/packages/$pkg" >/dev/null 2>&1 || true; fi
  report_applied "$res" "$ver"
}

# напрямую (ядро не отвечает): распаковать, сверить sha256, .prev (только если текущий бинарь исправен), замена, перезапуск, откат
apply_direct() {
  need_root
  local src="$1"
  mkdir -p "$STATE/updates"; TMP="$(mktemp -d "$STATE/updates/direct.XXXXXX")"
  tar -xzf "$src" -C "$TMP" || die "пакет не распаковывается"
  [[ -f "$TMP/manifest.json" && -f "$TMP/cloudd" ]] || die "в пакете нет manifest.json или cloudd"
  local want got; want="$(jq -r .cloudd_sha256 "$TMP/manifest.json")"; got="$(sha256sum "$TMP/cloudd" | cut -d' ' -f1)"
  [[ "$want" == "$got" ]] || die "пакет повреждён: sha256 бинаря не совпадает"
  local arch; arch="$(jq -r '.arch // empty' "$TMP/manifest.json")"
  [[ -z "$arch" || "$arch" == "$(uname -m)"* ]] || die "пакет для $arch, а хост $(uname -m)"
  local ver; ver="$(jq -r .version "$TMP/manifest.json")"
  confirm "Установить AgentVerse OS $ver напрямую (ядро сейчас не отвечает)?" || die "отменено"
  chmod 0755 "$TMP/cloudd"
  "$TMP/cloudd" --version >/dev/null || die "новый cloudd не запускается"
  if [[ -d "$TMP/store" && -d "$STATE/repo/store" ]]; then rsync -a --exclude compose.override.yaml --exclude login.yaml "$TMP/store/" "$STATE/repo/store/" 2>/dev/null || cp -r "$TMP/store/." "$STATE/repo/store/"; fi
  if [[ -f "$BIN" ]]; then
    if "$BIN" --version >/dev/null 2>&1; then cp -f "$BIN" "$BIN.prev"; else warn "текущий бинарь не запускается — сохраняю его как $BIN.failed, а прежний $BIN.prev не трогаю"; cp -f "$BIN" "$BIN.failed"; fi
  fi
  install -m 0755 "$TMP/cloudd" "$BIN.new" && mv -f "$BIN.new" "$BIN"
  systemctl restart cloudd
  if wait_core 45; then ok "готово: $(ver_of) (установлено напрямую; окно «Обновления» узнает о .prev по факту наличия файла)"; return 0; fi
  warn "ядро не ответило за 45 с — откат на предыдущую версию"
  cp -f "$BIN" "$BIN.failed"
  [[ -f "$BIN.prev" ]] || die "нет $BIN.prev для отката: journalctl -u cloudd -n 100"
  install -m 0755 "$BIN.prev" "$BIN.new" && mv -f "$BIN.new" "$BIN"; systemctl restart cloudd
  if wait_core 45; then warn "откачено: $(ver_of); сломанный бинарь — $BIN.failed"; else die "ядро не поднялось и после отката: journalctl -u cloudd -n 100"; fi
  return 1
}

do_package() {
  local src="$1"
  if [[ "$src" =~ ^https:// ]]; then
    mkdir -p "$STATE/updates" 2>/dev/null || true; TMP="$(mktemp -d "${STATE:-/tmp}/updates/dl.XXXXXX" 2>/dev/null || mktemp -d)"
    log "скачиваю $src"; curl -fSL --progress-bar "$src" -o "$TMP/pkg.tar.gz" || die "не удалось скачать"; src="$TMP/pkg.tar.gz"
  elif [[ "$src" =~ ^http:// ]]; then die "пакет по http:// не принимаю (подмена по пути даёт root на сервере) — используйте https:// или файл"; fi
  [[ -f "$src" ]] || die "пакет $src не найден"
  if [[ -n "$SHA" ]]; then [[ "$(sha256sum "$src" | cut -d' ' -f1)" == "${SHA,,}" ]] || die "sha256 файла не совпадает с --sha256"; fi
  if core_up; then log "установка пакета через ядро"; apply_via_core "$src"; else log "ядро не отвечает — ставлю пакет напрямую"; apply_direct "$src"; fi
}

do_channel() {
  local url="$1"
  core_up || die "ядро не отвечает на $API"
  log "канал обновлений: $url"
  local s; s="$(must PUT /updates/channel "$(jbody --arg u "$url" '{url: $u}')")"
  local err; err="$(echo "$s" | jq -r '.check_error // empty')"
  [[ -z "$err" ]] || die "канал сохранён, но не отвечает: $err"
  if [[ "$(echo "$s" | jq -r .available)" != true ]]; then ok "в канале $(echo "$s" | jq -r '.latest.version // "?"') — у вас актуальная $(echo "$s" | jq -r .version)"; return 0; fi
  local ver; ver="$(echo "$s" | jq -r .latest.version)"
  ok "доступно $ver (сейчас $(echo "$s" | jq -r .version))"
  confirm "Скачать и установить $ver?" || return 0
  install_from_channel "$ver"
}
install_from_channel() { # скачать пакет из канала через ядро и установить
  local ver="$1" before p r res
  before="$(api GET /status | jq -r '.version + " " + (.build // "dev")')"
  p="$(must POST /updates/download)"
  ok "скачан $(echo "$p" | jq -r .file) ($(( $(echo "$p" | jq -r .size) / 1048576 )) МБ)"
  r="$(must POST /updates/apply "$(jbody --arg f "$(echo "$p" | jq -r .file)" '{file: $f}')")"
  ok "установлено $(echo "$r" | jq -r .to), ядро перезапускается…"
  res="$(wait_applied "$before" 120)"
  [[ "$res" == ok ]] && { api DELETE "/updates/packages/$(echo "$p" | jq -r .file)" >/dev/null 2>&1 || true; }
  report_applied "$res" "$ver"
}

# ------------------------------------------------------------------------------------------ приложения и компоненты
do_apps() { # do_apps all|changed → код 1, если что-то не обновилось
  core_up || { warn "ядро не отвечает на $API — приложения пропущены"; return 1; }
  log "приложения из каталога"
  local u names; u="$(must GET /updates)"
  if [[ "$1" == all ]]; then names="$(echo "$u" | jq -r '.apps[] | select(.in_catalog and (.has_update or (.floating_tags | length > 0))) | .name')"
  else names="$(echo "$u" | jq -r '.apps[] | select(.in_catalog and .has_update) | .name')"; fi
  [[ -n "$names" ]] || { ok "обновлять нечего"; return 0; }
  echo "обновляю: $(echo "$names" | tr '\n' ' ')"
  local n fails=0 r
  for n in $names; do
    printf '  %s… ' "$n"
    if r="$(api POST "/apps/$n/update" "" 1200)"; then ok "обновлено"; else warn "ошибка: $r"; fails=$((fails + 1)); fi
  done
  (( fails == 0 )) || { warn "не обновилось: $fails"; return 1; }
}

do_components() { # do_components [проект] → код 1 при ошибке или неизвестном проекте
  core_up || { warn "ядро не отвечает на $API — компоненты пропущены"; return 1; }
  log "компоненты (edge, Coder, Komodo, docker-прокси)"
  local u list p fails=0; u="$(must GET /updates)"
  if [[ -n "${1:-}" ]]; then list="$1"
  else list="$(echo "$u" | jq -r --arg re "$KNOWN_COMPONENTS" '.components[] | select(.updatable and (.title | test($re))) | .project')"; fi
  [[ -n "$list" ]] || { ok "обновляемых компонентов не найдено"; return 0; }
  for p in $list; do
    local title; title="$(echo "$u" | jq -r --arg p "$p" '.components[] | select(.project == $p) | .title')"
    [[ -n "$title" ]] || { warn "компонент $p не найден (проекты: $(echo "$u" | jq -r '[.components[].project] | join(", ")'))"; fails=$((fails + 1)); continue; }
    [[ "$title" == Edge* ]] && warn "обновление edge пересоздаст Caddy — Desktop прервётся на несколько секунд"
    printf '  %s (%s): pull && up -d…\n' "$title" "$p"
    local r; if r="$(api POST "/updates/components/$p" "" 1700)"; then
      local ch; ch="$(echo "$r" | jq -r '.changed | join(", ")')"
      if [[ -n "$ch" ]]; then ok "$title: обновлены $ch"; else ok "$title: уже актуально"; fi
    else warn "$title: $r"; fails=$((fails + 1)); fi
  done
  (( fails == 0 )) || return 1
}

do_catalog() {
  core_up || die "ядро не отвечает на $API"
  log "каталог Store из upstream (Runtipi, Coolify, Umbrel)"
  must POST /updates/catalog '{"source":"all"}' >/dev/null
  local i; for i in $(seq 1 100); do
    sleep 3
    [[ "$(api GET /updates | jq -r .catalog.importing 2>/dev/null)" == true ]] || break
    (( i % 10 == 0 )) && echo "  импорт идёт ($((i * 3)) с)…"
  done
  must GET /updates | jq -r '.catalog | "манифестов: \(.manifests); последний импорт: \(.last_import.at // "?") — \(if .last_import.ok then "\(.last_import.imported) импортировано" else "ошибка: \(.last_import.error // "?")" end)"'
}

do_host() { # do_host [all] — пакеты хоста; Docker, Incus и Tailscale по умолчанию удерживаются (их обновление перезапускает сервисы)
  need_root
  log "пакеты хоста"
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -q
  local hold=() pkg
  if [[ "${1:-}" != all ]]; then
    for pkg in docker-ce docker-ce-cli containerd.io docker-compose-plugin incus incus-client tailscale; do dpkg -s "$pkg" >/dev/null 2>&1 && hold+=("$pkg"); done
    (( ${#hold[@]} )) && apt-mark hold "${hold[@]}" >/dev/null
  else
    warn "обновляются и Docker, Incus, Tailscale: перезапуск docker.service перезапустит cloudd и прервёт операции; tailscaled — разорвёт туннель на секунды"
    if [[ "${SSH_CONNECTION:-}" =~ ^100\. ]]; then warn "сессия SSH идёт через Tailscale — при обновлении tailscale она может прерваться, обновление продолжится"; fi
  fi
  apt-get -y -q -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold upgrade
  (( ${#hold[@]} )) && apt-mark unhold "${hold[@]}" >/dev/null
  if have restic; then restic self-update >/dev/null 2>&1 || true; fi
  if [[ -f /var/run/reboot-required ]]; then warn "хост просит перезагрузку (ядро Linux обновлено): sudo reboot в удобное время"; else ok "пакеты хоста актуальны"; fi
  (( ${#hold[@]} )) && echo "не обновлялись (удержаны): ${hold[*]} — обновить их: sudo agentverse-update --host-all"
  return 0
}

do_rollback() {
  if core_up; then
    local u; u="$(must GET /updates)"
    [[ "$(echo "$u" | jq -r .system.can_rollback)" == true ]] || die "предыдущей версии нет (.prev отсутствует)"
    local pv; pv="$(echo "$u" | jq -r '.system.prev_version // empty')"; [[ -n "$pv" ]] || pv="$("$BIN.prev" --version 2>/dev/null | awk '{print $2}' || echo '?')"
    confirm "Вернуть версию $pv и перезапустить ядро?" || die "отменено"
    local before; before="$(echo "$u" | jq -r '.system.version + " " + .system.build')"
    must POST /updates/rollback >/dev/null
    report_applied "$(wait_applied "$before" 120)" "$pv"
  else
    need_root
    [[ -f "$BIN.prev" ]] || die "ядро не отвечает и $BIN.prev нет — переустановите пакетом: sudo $0 <пакет.tar.gz>"
    confirm "Ядро не отвечает. Вернуть $BIN.prev и перезапустить?" || die "отменено"
    cp -f "$BIN" "$BIN.failed"; install -m 0755 "$BIN.prev" "$BIN.new" && mv -f "$BIN.new" "$BIN"; systemctl restart cloudd
    if wait_core 60; then ok "откачено: $(ver_of)"; else die "ядро не поднялось: journalctl -u cloudd -n 100"; fi
  fi
}

do_all() { # для cron: sudo agentverse-update --all --yes
  local rc=0
  if core_up && [[ -n "$(api GET /updates | jq -r '.system.channel_url // empty')" ]]; then
    local c=0; do_check || c=$?
    case "$c" in
      0) local ver; ver="$(api GET /updates | jq -r '.system.latest.version // "?"')"; if confirm "Установить $ver?"; then install_from_channel "$ver" || rc=1; wait_core 60 || true; fi ;;
      2) warn "канал недоступен — продолжаю остальное"; rc=1 ;;
    esac
  fi
  do_apps changed || rc=1
  do_components || rc=1
  do_host || rc=1
  return $rc
}

# -------------------------------------------------------------------------------------------------------- разбор
ACTION=status; ARG=""
while [[ $# -gt 0 ]]; do case "$1" in
  --yes|-y) YES=1 ;;
  --sha256) SHA="${2:?hex}"; shift ;;
  --status) ACTION=status ;;
  --check) ACTION=check ;;
  --channel) ACTION=channel; ARG="${2:?URL канала}"; shift ;;
  --apps) ACTION=apps; ARG=changed ;;
  --apps-all) ACTION=apps; ARG=all ;;
  --components) ACTION=components; if [[ -n "${2:-}" && "${2:0:1}" != - ]]; then ARG="$2"; shift; fi ;;
  --catalog) ACTION=catalog ;;
  --host) ACTION=host; ARG="" ;;
  --host-all) ACTION=host; ARG=all ;;
  --rollback) ACTION=rollback ;;
  --all) ACTION=all ;;
  -h|--help|help) usage; exit 0 ;;
  -*) usage; die "неизвестный флаг $1" ;;
  *) ACTION=package; ARG="$1" ;;
esac; shift; done

case "$ACTION" in
  status) do_status ;;
  check) do_check ;;
  channel) do_channel "$ARG" ;;
  package) do_package "$ARG" ;;
  apps) do_apps "$ARG" ;;
  components) do_components "$ARG" ;;
  catalog) do_catalog ;;
  host) do_host "$ARG" ;;
  rollback) do_rollback ;;
  all) do_all ;;
esac
