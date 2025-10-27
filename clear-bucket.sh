#!/bin/bash

# Clear all objects from R2 bucket using Cloudflare API
# Usage: ./clear-bucket.sh [bucket-name]

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Load environment variables from .env file if it exists
if [ -f .env ]; then
    echo "📄 Loading environment from .env file..."
    # Export variables from .env file
    set -a
    source .env
    set +a
elif [ -f .dev.vars ]; then
    echo "📄 Loading environment from .dev.vars file..."
    # Export variables from .dev.vars file
    set -a
    source .dev.vars
    set +a
fi

# Default bucket name from wrangler.toml
BUCKET_NAME="${1:-rpc-logs-cache}"

echo -e "${YELLOW}╔════════════════════════════════════════╗${NC}"
echo -e "${YELLOW}║    R2 Bucket Cleanup Script           ║${NC}"
echo -e "${YELLOW}╚════════════════════════════════════════╝${NC}"
echo ""
echo -e "Bucket: ${GREEN}${BUCKET_NAME}${NC}"
echo ""

# Check if wrangler is available
if ! command -v wrangler &> /dev/null; then
    echo -e "${RED}❌ Error: wrangler CLI not found${NC}"
    echo -e "   Install with: ${BLUE}npm install -g wrangler${NC}"
    exit 1
fi

# Get Cloudflare credentials from wrangler config
echo "📋 Getting Cloudflare credentials..."

# Try to get account ID from wrangler.toml (skip comments)
ACCOUNT_ID=$(grep -E "^account_id\s*=" wrangler.toml 2>/dev/null | head -1 | cut -d'"' -f2 | tr -d ' ')

# Try from environment variable
if [ -z "$ACCOUNT_ID" ] && [ -n "$CLOUDFLARE_ACCOUNT_ID" ]; then
    ACCOUNT_ID="$CLOUDFLARE_ACCOUNT_ID"
fi

# Try from wrangler whoami
if [ -z "$ACCOUNT_ID" ]; then
    WHOAMI_OUTPUT=$(wrangler whoami 2>&1)
    ACCOUNT_ID=$(echo "$WHOAMI_OUTPUT" | grep "Account ID:" | sed 's/.*Account ID: //' | head -1)
fi

if [ -z "$ACCOUNT_ID" ]; then
    echo -e "${RED}❌ Cannot find Cloudflare Account ID${NC}"
    echo ""
    echo -e "${YELLOW}Please add your account_id to wrangler.toml:${NC}"
    echo -e "  ${BLUE}account_id = \"your-account-id-here\"${NC}"
    echo ""
    echo -e "Or get it from: ${BLUE}wrangler whoami${NC}"
    exit 1
fi

printf "   Account ID: ${GREEN}%s${NC}\n" "$ACCOUNT_ID"

# Check if we can get API token
if [ -z "$CLOUDFLARE_API_TOKEN" ]; then
    echo ""
    echo -e "${YELLOW}⚠️  No CLOUDFLARE_API_TOKEN found${NC}"
    echo ""
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${YELLOW}To delete objects via API, you need:${NC}"
    echo ""
    echo "1. Create an API token at:"
    echo "   https://dash.cloudflare.com/profile/api-tokens"
    echo ""
    echo "2. Token permissions needed:"
    echo "   • Account > R2 > Edit"
    echo ""
    echo "3. Add to .env file (recommended):"
    echo -e "   ${BLUE}echo 'CLOUDFLARE_API_TOKEN=your-token-here' >> .env${NC}"
    echo ""
    echo "   Or export directly:"
    echo -e "   ${BLUE}export CLOUDFLARE_API_TOKEN='your-token-here'${NC}"
    echo ""
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo -e "${YELLOW}Alternative: Delete via Cloudflare Dashboard${NC}"
    echo "   https://dash.cloudflare.com/ → R2 → ${BUCKET_NAME}"
    echo ""
    exit 1
fi

printf "   API Token: ${GREEN}Found${NC}\n"
echo ""

# Confirmation prompt
echo -e "${RED}⚠️  WARNING: This will delete ALL objects from '${BUCKET_NAME}'!${NC}"
read -p "Are you sure you want to continue? (yes/no): " -r
echo ""

if [[ ! $REPLY =~ ^[Yy][Ee][Ss]$ ]]; then
    echo -e "${YELLOW}❌ Cancelled${NC}"
    exit 0
fi

# List and delete objects using Cloudflare R2 API
# Note: Deletes one-by-one as R2 API doesn't support batch delete
echo "🗑️  Deleting objects..."
echo ""

API_URL="https://api.cloudflare.com/client/v4/accounts/${ACCOUNT_ID}/r2/buckets/${BUCKET_NAME}/objects"
DELETED=0
CURSOR=""

while true; do
    # List objects (max 1000 per request)
    if [ -z "$CURSOR" ]; then
        RESPONSE=$(curl -s -X GET "$API_URL?per_page=1000" \
            -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN")
    else
        RESPONSE=$(curl -s -X GET "$API_URL?per_page=1000&cursor=$CURSOR" \
            -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN")
    fi
    
    # Check for errors
    if echo "$RESPONSE" | grep -q '"success":false'; then
        ERROR=$(echo "$RESPONSE" | sed -n 's/.*"message":"\([^"]*\)".*/\1/p' | head -1)
        echo -e "${RED}❌ API Error: $ERROR${NC}"
        echo ""
        echo "Response: $RESPONSE"
        exit 1
    fi
    
    # Extract object keys (handle multiple keys in JSON)
    OBJECTS=$(echo "$RESPONSE" | grep -o '"key":"[^"]*"' | sed 's/"key":"//;s/"$//')
    
    if [ -z "$OBJECTS" ]; then
        break
    fi
    
    # Delete each object one by one
    # Note: R2 API doesn't support batch delete via Cloudflare API endpoint
    while IFS= read -r OBJECT_KEY; do
        if [ -n "$OBJECT_KEY" ]; then
            # Delete using R2 API: DELETE /objects/{key}
            DELETE_RESPONSE=$(curl -s -X DELETE "${API_URL}/${OBJECT_KEY}" \
                -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN")
            
            if echo "$DELETE_RESPONSE" | grep -q '"success":true'; then
                DELETED=$((DELETED + 1))
                echo -ne "  Deleted: ${GREEN}$DELETED${NC} objects\r"
            else
                echo ""
                echo -e "${YELLOW}⚠️  Failed to delete: $OBJECT_KEY${NC}"
                echo "Response: $DELETE_RESPONSE"
            fi
        fi
    done <<< "$OBJECTS"
    
    # Check if there are more pages
    CURSOR=$(echo "$RESPONSE" | sed -n 's/.*"cursor":"\([^"]*\)".*/\1/p' | head -1)
    if [ -z "$CURSOR" ] || [ "$CURSOR" == "null" ]; then
        break
    fi
done

echo ""
echo ""
echo -e "${GREEN}✅ Cleanup complete!${NC}"
echo -e "   Deleted ${GREEN}$DELETED${NC} objects from ${GREEN}$BUCKET_NAME${NC}"
echo ""
