# Deployment Guide

## Prerequisites

```bash
# Install Wrangler
npm install -g wrangler

# Login to Cloudflare
wrangler login

# Verify setup
./check-setup.sh

# Create R2 bucket if needed
wrangler r2 bucket create rpc-logs-cache
```

## Configuration

### 1. Configure R2 Bucket

Edit `wrangler.toml`:
```toml
[[r2_buckets]]
binding = "LOGS_CACHE"
bucket_name = "rpc-logs-cache"
```

### 2. Set Environment Variables

Create `.dev.vars` for local development:
```bash
UPSTREAM_RPC_URL_1=https://eth-mainnet.example.com
UPSTREAM_RPC_URL_137=https://polygon.example.com
```

For production, use:
```bash
wrangler secret put UPSTREAM_RPC_URL_1
wrangler secret put UPSTREAM_RPC_URL_137
```

### 3. Configure Caching

```toml
[vars]
DEFAULT_BLOCK_DISTANCE = "100"
CHAIN_BLOCK_DISTANCES = '{"1": 100, "137": 200}'
```

## Deploy

### Test First
```bash
cargo test --all
```

### Deploy
```bash
wrangler deploy
```

### Verify
```bash
# Check deployment
wrangler deployments list

# Test request
curl -X POST https://your-worker.workers.dev/1 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

## Monitoring

### Watch Logs
```bash
wrangler tail
```

### Check R2 Storage
```bash
# View via Cloudflare Dashboard
# https://dash.cloudflare.com/ → R2 → rpc-logs-cache

# Clear all cached data
# Add API token to .env or .dev.vars first:
echo 'CLOUDFLARE_API_TOKEN=your-token-here' >> .env

# Then run:
./clear-bucket.sh

# Clear specific bucket
./clear-bucket.sh my-custom-bucket
```

### Look For

**Cache hits:**
```
eth_getLogs cache HIT for block 850
```

**Cache misses:**
```
eth_getLogs cache MISS for block 850
Stored logs in R2 cache
```

**Too recent (not cached):**
```
Block is too recent, skipping cache
```

## Troubleshooting

### Cache not working
1. Check R2 binding in `wrangler.toml`
2. Verify block distance settings
3. Check logs for errors

### High RPC costs
1. Verify cache hit rates in logs
2. Check block distance isn't too high
3. Monitor R2 storage growth

### Method not found
1. Verify method name spelling
2. Check deployment succeeded
3. Look for routing errors in logs

## Rollback

If issues occur:
```bash
# List deployments
wrangler deployments list

# Rollback to previous
wrangler rollback --deployment-id <id>
```

## Cost Optimization

### Monitor Usage
```bash
# In Cloudflare Dashboard
Workers > Your Worker > Metrics
R2 > Your Bucket > Metrics
```

### Adjust Block Distance
- Higher = More caching = Lower RPC costs
- Lower = Less caching = Fresher data

**Recommendation:** Start with 100, adjust based on chain block time and reorg frequency.

## Success Metrics

### Week 1
- ✅ All methods responding
- ✅ R2 folders created
- ✅ No errors in logs

### Month 1
- ✅ Cache hit rate > 50%
- ✅ RPC costs reduced
- ✅ R2 storage stable

