#!/usr/bin/env bash
# on_revoke storage.s3: удалить ключ проекта; бакет и данные остаются (удаление данных — явное действие пользователя, 3.15).
. "$(dirname "$0")/common.sh"
for id in $(api GET /v2/ListKeys | jq -r ".[] | select(.name==\"$KEYNAME\") | .id"); do
  api POST "/v2/DeleteKey?id=$id" >/dev/null && echo "key $id deleted" >&2
done
