#!/usr/bin/env bash
# on_grant storage.s3: the project's bucket and key (idempotent), the key's rights only on this bucket. stdout →
# the project env.
. "$(dirname "$0")/common.sh"

# bucket
if ! bid=$(api GET "/v2/GetBucketInfo?globalAlias=$BUCKET" 2>/dev/null | jq -r '.id // empty'); then bid=""; fi
if [[ -z "$bid" ]]; then
  bid=$(api POST /v2/CreateBucket "{\"globalAlias\":\"$BUCKET\"}" | jq -r .id)
fi
# key: recreated on every grant so the secret is known (the Admin API returns the secret only at creation / with
# showSecretKey)
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
