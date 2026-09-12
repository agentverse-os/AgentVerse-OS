#!/usr/bin/env bash
# on_revoke notify: удалить пользователя проекта (вместе с токенами и правами).
set -euo pipefail
docker exec ntfy-ntfy-1 ntfy user remove "proj-${PROJECT}" >/dev/null 2>&1 || true
