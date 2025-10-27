# R2 Storage Structure

## Pattern

All cached data follows this structure:
```
{method}/{chain_id}/{identifier}
```

## Folder Layout

```
rpc-logs-cache/
├── eth_getLogs/
│   ├── 1/          (Ethereum)
│   ├── 137/        (Polygon)
│   └── 56/         (BSC)
│
├── eth_getTransactionReceipt/
│   ├── 1/
│   └── 137/
│
├── eth_getBlockByHash/
│   ├── 1/
│   └── 137/
│
├── eth_getBlockReceipts/
│   ├── 1/
│   └── 137/
│
├── debug_traceBlockByNumber/
│   ├── 1/
│   └── 137/
│
└── debug_traceBlockByHash/
    ├── 1/
    └── 137/
```

## Cache Keys

### eth_getLogs
```
eth_getLogs/{chain_id}/{params_hash}

Example:
eth_getLogs/1/a3f5b9c2e8d1f4a7...
```

### eth_getTransactionReceipt
```
eth_getTransactionReceipt/{chain_id}/{tx_hash}

Example:
eth_getTransactionReceipt/1/0xabc123...
```

### eth_getBlockByHash
```
eth_getBlockByHash/{chain_id}/{block_hash}

Example:
eth_getBlockByHash/1/0xdef456...
```

### eth_getBlockReceipts
```
eth_getBlockReceipts/{chain_id}/{block_id}

Examples:
eth_getBlockReceipts/1/0x64         (block number)
eth_getBlockReceipts/1/0x789abc...  (block hash)
```

### debug_traceBlockByNumber
```
debug_traceBlockByNumber/{chain_id}/{block_number}

Example:
debug_traceBlockByNumber/1/0xc8
```

### debug_traceBlockByHash
```
debug_traceBlockByHash/{chain_id}/{block_hash}

Example:
debug_traceBlockByHash/1/0xfed123...
```

## Management

### View cached items
```bash
# Via Cloudflare Dashboard
https://dash.cloudflare.com/ → R2 → rpc-logs-cache

# Browse folders:
# - eth_getLogs/
# - eth_getTransactionReceipt/
# - eth_getBlockByHash/
# - etc.
```

### Purge cache
```bash
# Setup (one-time):
# 1. Visit: https://dash.cloudflare.com/profile/api-tokens
# 2. Create token with: Account > R2 > Edit
# 3. Add to .env or .dev.vars:
echo 'CLOUDFLARE_API_TOKEN=your-token-here' >> .env

# Clear entire bucket:
./clear-bucket.sh

# Clear specific bucket:
./clear-bucket.sh my-other-bucket

# Or delete via dashboard:
# Visit: https://dash.cloudflare.com/ → R2 → rpc-logs-cache
# Select objects → Delete
```

## Benefits

### ✅ Organization
- Each method has its own folder
- Easy to find and manage
- No collisions between methods

### ✅ Scalability
- Add new methods easily
- Same pattern for all
- Clear separation

### ✅ Monitoring
- Track storage per method
- Analyze usage patterns
- Identify popular chains

### ✅ Management
- Purge by method or chain
- Easy cleanup
- Flexible control

