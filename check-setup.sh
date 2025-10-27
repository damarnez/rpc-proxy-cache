#!/bin/bash

# Check wrangler and R2 setup
# Usage: ./check-setup.sh

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}╔════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║    Setup Check                        ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════╝${NC}"
echo ""

# Check wrangler
echo -n "1. Checking wrangler CLI... "
if command -v wrangler &> /dev/null; then
    VERSION=$(wrangler --version 2>&1 | head -n 1)
    echo -e "${GREEN}✓${NC} ($VERSION)"
else
    echo -e "${RED}✗${NC}"
    echo -e "   ${YELLOW}Install with:${NC} npm install -g wrangler"
    exit 1
fi

# Check authentication
echo -n "2. Checking authentication... "
AUTH_CHECK=$(wrangler whoami 2>&1)
if echo "$AUTH_CHECK" | grep -q "not authenticated"; then
    echo -e "${RED}✗${NC}"
    echo -e "   ${YELLOW}Login with:${NC} wrangler login"
    exit 1
else
    echo -e "${GREEN}✓${NC}"
    if echo "$AUTH_CHECK" | grep -q "You are logged in"; then
        EMAIL=$(echo "$AUTH_CHECK" | grep -oP '(?<=as ).*' || echo "")
        if [ -n "$EMAIL" ]; then
            echo "   Logged in as: $EMAIL"
        fi
    fi
fi

# List R2 buckets
echo ""
echo "3. R2 Buckets:"
BUCKETS=$(wrangler r2 bucket list 2>&1)

if [ -z "$BUCKETS" ] || echo "$BUCKETS" | grep -q "No R2 buckets"; then
    echo -e "   ${YELLOW}No buckets found${NC}"
    echo ""
    echo -e "   Create with: ${BLUE}wrangler r2 bucket create rpc-logs-cache${NC}"
else
    echo "$BUCKETS" | while read -r line; do
        if [ -n "$line" ]; then
            if echo "$line" | grep -q "rpc-logs-cache"; then
                echo -e "   ${GREEN}✓${NC} $line"
            else
                echo "   - $line"
            fi
        fi
    done
fi

echo ""
echo -e "${GREEN}Setup check complete!${NC}"

