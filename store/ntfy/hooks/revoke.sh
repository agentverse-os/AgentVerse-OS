#!/usr/bin/env bash
# on_revoke notify: delete the project user (together with the tokens and
# rights).
set -euo pipefail
docker exec ntfy-ntfy-1 ntfy user remove "proj-${PROJECT}" >/dev/null 2>&1 || true
