#!/bin/bash
source .env
ACCOUNT_ID=$(grep -E "^account_id\s*=" wrangler.toml | cut -d'"' -f2)
BUCKET_NAME="rpc-logs-cache"
API_URL="https://api.cloudflare.com/client/v4/accounts/${ACCOUNT_ID}/r2/buckets/${BUCKET_NAME}/objects"

# List first 10 objects to see the structure
echo "Current objects in bucket:"
curl -s -X GET "$API_URL?per_page=10" \
    -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" | jq -r '.result[] | .key' | head -10
