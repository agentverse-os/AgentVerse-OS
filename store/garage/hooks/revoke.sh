#!/usr/bin/env bash
# on_revoke storage.s3: delete the project key; the bucket and data stay (deleting data is an explicit user action, 3.15).
. "$(dirname "$0")/common.sh"
for id in $(api GET /v2/ListKeys | jq -r ".[] | select(.name==\"$KEYNAME\") | .id"); do
  api POST "/v2/DeleteKey?id=$id" >/dev/null && echo "key $id deleted" >&2
done
