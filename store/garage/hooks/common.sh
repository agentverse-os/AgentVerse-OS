# Общее для хуков Garage: Admin API v2 (3.6). Вход от cloudd: APP_IP, GARAGE_ADMIN_TOKEN, PROJECT, CAPABILITY, GATE_URL.
set -euo pipefail
ADMIN="http://${APP_IP}:3903"
api() { # api METHOD path [json]
  local m="$1" p="$2" d="${3:-}"
  if [[ -n "$d" ]]; then curl -fsS -X "$m" "$ADMIN$p" -H "Authorization: Bearer $GARAGE_ADMIN_TOKEN" -H 'Content-Type: application/json' -d "$d"
  else curl -fsS -X "$m" "$ADMIN$p" -H "Authorization: Bearer $GARAGE_ADMIN_TOKEN"; fi
}
BUCKET="proj-${PROJECT}"
KEYNAME="proj-${PROJECT}"
