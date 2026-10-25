#!/usr/bin/env bash
# on_grant notify: the user proj-<project> (a random password, not returned), rw rights on the topic, a token without expiry. stdout →
# the project env.
set -euo pipefail
C="ntfy-ntfy-1"; U="proj-${PROJECT}"; TOPIC="proj-${PROJECT}"
x() { docker exec -e NTFY_PASSWORD="$PW" "$C" ntfy "$@"; }
PW="$(openssl rand -hex 16)"
docker exec "$C" ntfy user list 2>/dev/null | grep -q "^user $U " || x user add --role=user "$U" >/dev/null
docker exec "$C" ntfy access "$U" "$TOPIC" rw >/dev/null
# old project tokens — revoke, issue a new one (the secret is known only at creation)
for t in $(docker exec "$C" ntfy token list "$U" 2>/dev/null | grep -oE 'tk_[A-Za-z0-9]+'); do docker exec "$C" ntfy token remove "$U" "$t" >/dev/null 2>&1 || true; done
TOKEN=$(docker exec "$C" ntfy token add --label "cloudos grant" "$U" | grep -oE 'tk_[A-Za-z0-9]+' | head -1)
echo "NOTIFY_URL=$GATE_URL"
echo "NOTIFY_TOPIC=$TOPIC"
echo "NOTIFY_TOKEN=$TOKEN"
echo "NOTIFY_PUBLIC_URL=https://${APP_DOMAIN}/$TOPIC"
