# RPC Proxy Cache

An Ethereum RPC proxy that caches responses in Cloudflare R2 to reduce costs and improve performance.

## Features

- ✅ **6 cached RPC methods** (eth_getLogs, eth_getTransactionReceipt, eth_getBlockByHash, eth_getBlockReceipts, debug_traceBlockByNumber, debug_traceBlockByHash)
- ✅ **Smart caching** - Only caches old blocks to avoid reorgs
- ✅ **R2 storage** - Organized by method and chain
- ✅ **Multi-chain support** - Different block distances per chain
- ✅ **Cost savings** - Up to 99% reduction on repeat queries

## Quick Start

```bash
# Install dependencies
npm install -g wrangler

# Login to Cloudflare
wrangler login

# Check setup
./check-setup.sh

# Test
cargo test --all

# Deploy
wrangler deploy

# Monitor logs
wrangler tail
```

## How It Works

### Caching Rule
```
Cache if: block_number + block_distance ≤ current_block
```

**Example:**
- Current block: 1000
- Block distance: 100
- Request block: 850
- Decision: 850 + 100 = 950 ≤ 1000 ✓ **CACHE**

### Storage Structure
```
{method}/{chain_id}/{identifier}

Examples:
eth_getLogs/1/abc123...
eth_getTransactionReceipt/1/0xdef456...
debug_traceBlockByNumber/1/0x64
```

## Configuration

Edit `wrangler.toml`:

```toml
[vars]
DEFAULT_BLOCK_DISTANCE = "100"  # 100 blocks (~20 min on Ethereum)
CHAIN_BLOCK_DISTANCES = '{"1": 100, "137": 200}'  # Per-chain config
```

## Cached Methods

| Method | Description | Cache Duration |
|--------|-------------|----------------|
| `eth_getLogs` | Event logs | Permanent (old blocks) |
| `eth_getTransactionReceipt` | Transaction receipts | Permanent (confirmed) |
| `eth_getBlockByHash` | Block by hash | Permanent (old blocks) |
| `eth_getBlockByNumber` | Block by number | 2 seconds (memory) |
| `eth_getBlockReceipts` | All block receipts | Permanent (old blocks) |
| `debug_traceBlockByNumber` | Debug traces | Permanent (old blocks) |
| `debug_traceBlockByHash` | Debug traces | Permanent (old blocks) |

## Documentation

- 📖 [Caching Logic](docs/caching-logic.md) - How caching decisions work
- 📖 [R2 Structure](docs/r2-structure.md) - Storage organization
- 📖 [Deployment Guide](docs/deployment.md) - Deploy and monitor

## Testing

```bash
# Run all tests (60 tests)
cargo test --all

# Run specific tests
cargo test block_receipts
cargo test debug_trace
```

## Monitoring

```bash
# Watch live logs
wrangler tail

# Check R2 storage (via dashboard)
# Visit: https://dash.cloudflare.com/ → R2 → rpc-logs-cache

# Clear all cached data
# 1. Add API token to .env or .dev.vars:
echo 'CLOUDFLARE_API_TOKEN=your-token-here' >> .env

# 2. Run cleanup script:
./clear-bucket.sh
```

## Cost Savings

### Example: Debug Trace
```
Without cache: 100 queries × $0.05 = $5.00
With cache:    1 × $0.05 + 99 × $0.0001 = $0.06
Savings:       $4.94 (98.8%)
```

## License

MIT
