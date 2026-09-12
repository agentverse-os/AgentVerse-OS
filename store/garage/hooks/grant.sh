#!/usr/bin/env bash
# on_grant storage.s3: бакет и ключ проекта (идемпотентно), права ключа только на этот бакет. stdout → env проекта.
. "$(dirname "$0")/common.sh"

# бакет
if ! bid=$(api GET "/v2/GetBucketInfo?globalAlias=$BUCKET" 2>/dev/null | jq -r '.id // empty'); then bid=""; fi
if [[ -z "$bid" ]]; then
  bid=$(api POST /v2/CreateBucket "{\"globalAlias\":\"$BUCKET\"}" | jq -r .id)
fi
# ключ: пересоздаём на каждый grant, чтобы секрет был известен (Admin API отдаёт secret только при создании / с showSecretKey)
existing=$(api GET /v2/ListKeys | jq -r ".[] | select(.name==\"$KEYNAME\") | .id" | head -1 || true)
if [[ -n "$existing" ]]; then
  key=$(api GET "/v2/GetKeyInfo?id=$existing&showSecretKey=true")
else
  key=$(api POST /v2/CreateKey "{\"name\":\"$KEYNAME\"}")
fi
kid=$(echo "$key" | jq -r .accessKeyId); ksec=$(echo "$key" | jq -r .secretAccessKey)
api POST /v2/AllowBucketKey "{\"bucketId\":\"$bid\",\"accessKeyId\":\"$kid\",\"permissions\":{\"read\":true,\"write\":true,\"owner\":true}}" >/dev/null

echo "S3_ENDPOINT=$GATE_URL"
echo "S3_REGION=garage"
echo "S3_BUCKET=$BUCKET"
echo "S3_ACCESS_KEY=$kid"
echo "S3_SECRET_KEY=$ksec"
echo "S3_FORCE_PATH_STYLE=true"
