# Caching Logic

## Core Rule

```
Cache if: block_number + block_distance ≤ current_block
```

This ensures we only cache blocks that are far enough from the chain tip to avoid reorganization issues.

## Block Distance

**Default:** 100 blocks (~20 minutes on Ethereum)

**Per-chain configuration:**
```json
{
  "1": 100,    // Ethereum: 100 blocks
  "137": 200,  // Polygon: 200 blocks (faster block time)
  "56": 150    // BSC: 150 blocks
}
```

## Caching Decisions

### ✅ Will Cache

**Old blocks (safe from reorgs):**
```
Current: 1000
Distance: 100
Request: 850

Check: 850 + 100 = 950 ≤ 1000 ✓
Result: CACHE
```

### ❌ Won't Cache

**Recent blocks (reorg risk):**
```
Current: 1000
Distance: 100
Request: 950

Check: 950 + 100 = 1050 > 1000 ✗
Result: DON'T CACHE
```

**Special tags:**
- `latest` - always fetch fresh
- `pending` - always fetch fresh
- `earliest` - always fetch fresh

## Block Hash vs Block Number

### Block Number (Known)
```
Request: eth_getBlockReceipts("0x64")
→ Parse: 100
→ Check: 100 + 100 ≤ 1000 ✓
→ Decision: CACHE
```

### Block Hash (Unknown)
```
Request: eth_getBlockReceipts("0xabc...")
→ Fetch from RPC
→ Extract blockNumber from response: 100
→ Check: 100 + 100 ≤ 1000 ✓
→ Decision: CACHE
```

**Detection:** Block hash = 66 characters (0x + 64 hex)

## Method-Specific Rules

### eth_getLogs
- **Cache:** Old block ranges only
- **Key:** Hash of parameters (from, to, address, topics)
- **Example:** `eth_getLogs/1/abc123...`

### eth_getTransactionReceipt
- **Cache:** Confirmed transactions only (has blockNumber)
- **Never cache:** Pending (null receipt)
- **Example:** `eth_getTransactionReceipt/1/0xdef456...`

### eth_getBlockByHash
- **Cache:** After checking block number from response
- **Example:** `eth_getBlockByHash/1/0x789abc...`

### eth_getBlockByNumber
- **Cache:** In-memory only, 2-second TTL
- **Why:** Frequent queries, short-lived cache

### eth_getBlockReceipts
- **Cache:** Old blocks only
- **Supports:** Block number OR block hash
- **Example:** `eth_getBlockReceipts/1/0x64`

### debug_traceBlockByNumber
- **Cache:** Old blocks only (expensive to generate!)
- **Example:** `debug_traceBlockByNumber/1/0xc8`

### debug_traceBlockByHash
- **Cache:** After checking block number from response
- **Example:** `debug_traceBlockByHash/1/0xfed...`

## Response Extraction

When we need to extract block number from response:

### For Receipts
```json
{
  "result": [
    {
      "blockNumber": "0x64",  ← Extract this
      "transactionHash": "0x...",
      ...
    }
  ]
}
```

### For Blocks
```json
{
  "result": {
    "number": "0x64",  ← Extract this
    "hash": "0x...",
    ...
  }
}
```

## Edge Cases

### Empty Response
```json
{"result": null}
→ Don't cache
```

### No Block Number
```json
{"result": {"data": "..."}}  // No blockNumber field
→ Don't cache
```

### Pending Transaction
```json
{"result": null}  // Transaction not mined
→ Don't cache
```

## Logging

```bash
# Cache hit
"eth_getLogs cache HIT for block 850"

# Cache miss + store
"eth_getLogs cache MISS for block 850"
"Block number 850 check: current=1000, distance=100, should_cache=true"
"Stored logs in R2 cache"

# Too recent
"Block number 950 check: current=1000, distance=100, should_cache=false"
"Block is too recent, skipping cache"
```

