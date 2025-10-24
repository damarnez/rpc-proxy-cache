# RPC Proxy Cache

A high-performance CloudFlare Workers-based proxy for Ethereum RPC endpoints with intelligent caching using R2 and KV storage. Built with Rust for maximum performance and reliability.

## Features

- **Smart Caching**: Automatically caches specific RPC methods with different strategies
  - `eth_getLogs`: Cached in R2 storage with reorg-safe block distance validation
  - `eth_getBlockByNumber`: Cached in KV storage with 2-second TTL for fast access
- **Reorg Protection**: Configurable block distance per chain to avoid caching logs that might be affected by chain reorganizations
- **Multi-Chain Support**: Chain ID-based routing and configuration
- **Transparent Proxy**: All other RPC methods are proxied through without modification

## Architecture

```
Client Request /{chainId} → CloudFlare Worker → Cache Check → Upstream RPC
                                      ↓              ↓
                                  Chain ID      R2 (eth_getLogs)
                                  Routing       KV (eth_getBlockByNumber)
```

**Request Flow:**
1. Client sends RPC request to `/{chainId}` path (e.g., `/1` for Ethereum, `/137` for Polygon)
2. Worker extracts the first path segment as chain ID (defaults to `1` if path is `/`)
3. For cacheable methods (`eth_getLogs`, `eth_getBlockByNumber`):
   - Check if request should be cached based on block distance
   - Try to retrieve from cache (R2 for logs, KV for blocks)
   - On cache miss, fetch from upstream and store in cache
4. For other methods: Proxy directly to upstream RPC

### Caching Strategy

#### eth_getLogs (R2 Storage)
- Stores logs in CloudFlare R2 for long-term, cost-effective storage
- Only caches logs that are at least N blocks behind the current block (default: 100)
- Prevents caching of logs that might be affected by chain reorganizations
- Cache key includes chain ID and request parameters (fromBlock, toBlock, address, topics)

#### eth_getBlockByNumber (KV Storage)
- Stores blocks in CloudFlare KV for fast access
- 2-second TTL to balance freshness and cache efficiency
- Ideal for frequently requested recent blocks

## Setup

### Prerequisites

1. [Rust](https://www.rust-lang.org/tools/install) (latest stable)
2. [Node.js](https://nodejs.org/) (v16 or later)
3. [Wrangler CLI](https://developers.cloudflare.com/workers/wrangler/install-and-update/)
   ```bash
   npm install -g wrangler
   ```
4. CloudFlare account with Workers enabled

### Installation

1. Clone the repository:
   ```bash
   cd rpc-proxy-cache
   ```

2. Install dependencies:
   ```bash
   # Install worker-build for building Rust workers
   cargo install worker-build
   
   # Install wasm32-unknown-unknown target
   rustup target add wasm32-unknown-unknown
   ```

3. Authenticate with CloudFlare:
   ```bash
   wrangler login
   ```

4. Create R2 bucket for logs cache:
   ```bash
   wrangler r2 bucket create rpc-logs-cache
   ```

5. Create KV namespace for blocks cache:
   ```bash
   wrangler kv namespace create "BLOCKS_CACHE"
   ```
   
   Copy the namespace ID from the output and update `wrangler.toml`:
   ```toml
   [[kv_namespaces]]
   binding = "BLOCKS_CACHE"
   id = "YOUR_KV_NAMESPACE_ID_HERE"
   ```

### Configuration

Edit `wrangler.toml` to configure:

1. **UPSTREAM_RPC_URL**: Your upstream RPC endpoint
   ```toml
   UPSTREAM_RPC_URL = "https://your-rpc-endpoint.com"
   ```

2. **DEFAULT_BLOCK_DISTANCE**: Default number of blocks behind tip before caching (default: 100)
   ```toml
   DEFAULT_BLOCK_DISTANCE = "100"
   ```

3. **CHAIN_BLOCK_DISTANCES**: Per-chain block distance configuration (JSON format)
   ```toml
   CHAIN_BLOCK_DISTANCES = "{\"1\": 100, \"137\": 200, \"56\": 150}"
   ```
   - Chain ID 1 (Ethereum): 100 blocks
   - Chain ID 137 (Polygon): 200 blocks
   - Chain ID 56 (BSC): 150 blocks

## How It Works

### URL Routing

The worker uses **path-based routing** to determine the chain ID:

```rust
// Code extracts the first path segment as chain ID
let chain_id = path
    .trim_start_matches('/')
    .split('/')
    .next()
    .filter(|s| !s.is_empty())
    .unwrap_or("1")  // Default to Ethereum mainnet
    .to_string();
```

**Examples:**
- Request to `/` → Chain ID: `1` (Ethereum)
- Request to `/137` → Chain ID: `137` (Polygon)  
- Request to `/56` → Chain ID: `56` (BSC)
- Request to `/42161/anything` → Chain ID: `42161` (Arbitrum) - trailing paths ignored

### Caching Logic

1. **Request arrives** with chain ID in path
2. **For `eth_getLogs`:**
   - Check if `toBlock + blockDistance <= currentBlock`
   - If yes: Check R2 cache → Return cached or fetch & store
   - If no: Skip cache, fetch from upstream only
3. **For `eth_getBlockByNumber`:**
   - Check KV cache (2s TTL)
   - Return cached or fetch & store with 2s expiration
4. **All other methods:** Direct proxy to upstream

## Project Structure

```
rpc-proxy-cache/
├── Cargo.toml          # Rust dependencies and build configuration
├── wrangler.toml       # CloudFlare Workers configuration
└── src/
    ├── lib.rs          # Main worker entry point and request handling
    ├── cache.rs        # Caching logic for R2 and KV storage
    ├── rpc.rs          # RPC request/response types
    └── utils.rs        # Utility functions
```

**Key Files:**
- `lib.rs`: Main entry point - handles HTTP requests, extracts chain ID from URL path, routes to appropriate handlers, manages CORS
- `cache.rs`: CacheManager implementation - handles R2 and KV storage operations, block distance validation, reorg protection logic
- `rpc.rs`: JSON-RPC data structures (RpcRequest, RpcResponse, RpcError)
- `utils.rs`: Helper functions for hex parsing and cache key generation
- `Cargo.toml`: Rust dependencies configured for WebAssembly compilation with aggressive size optimizations (opt-level="z", LTO)

## Development

### Local Development

Run the worker locally with hot-reloading:

```bash
# Development with default configuration
wrangler dev

# Or use production environment config
wrangler dev --env production
```

This compiles the Rust code to WebAssembly and starts a local development server at `http://localhost:8787`

**Note**: The first run may take a few minutes as it compiles the Rust code and installs `worker-build` if needed.

### Development Tips

**Check your code without building:**
```bash
cargo check --target wasm32-unknown-unknown
```

**Run clippy for linting:**
```bash
cargo clippy --target wasm32-unknown-unknown
```

**Format code:**
```bash
cargo fmt
```

**Recommended IDE setup:**
- VSCode with rust-analyzer extension
- IntelliJ IDEA with Rust plugin

**Run unit tests:**
```bash
cargo test
```

### Testing

Test the proxy with curl:

```bash
# eth_getLogs request (Ethereum mainnet - chain ID 1)
curl -X POST http://localhost:8787/1 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getLogs",
    "params": [{
      "fromBlock": "0x1000000",
      "toBlock": "0x1000010",
      "address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
    }],
    "id": 1
  }'

# eth_getBlockByNumber request (Polygon - chain ID 137)
curl -X POST http://localhost:8787/137 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getBlockByNumber",
    "params": ["0x1000000", false],
    "id": 1
  }'

# Other methods (proxied through) - BSC (chain ID 56)
curl -X POST http://localhost:8787/56 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_blockNumber",
    "params": [],
    "id": 1
  }'

# Request without params field (also valid)
curl -X POST http://localhost:8787/1 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_blockNumber",
    "id": 1
  }'
```

### Build

Build the Rust worker for production (compiles to optimized WebAssembly):

```bash
worker-build --release
```

This creates an optimized WASM binary in the `build` directory.

## Deployment

Deploy to CloudFlare Workers:

```bash
wrangler deploy --env production
```

After deployment, your worker will be available at:
```
https://rpc-proxy-cache.YOUR_SUBDOMAIN.workers.dev
```

## Usage

Once deployed, use the worker URL as your RPC endpoint with the chain ID in the path:

```javascript
// Example with ethers.js (Ethereum mainnet - chain ID 1)
const provider = new ethers.providers.JsonRpcProvider(
  'https://rpc-proxy-cache.YOUR_SUBDOMAIN.workers.dev/1'
);

// Example with web3.js (Polygon - chain ID 137)
const web3 = new Web3(
  'https://rpc-proxy-cache.YOUR_SUBDOMAIN.workers.dev/137'
);

// Example with viem
import { createPublicClient, http } from 'viem';
import { mainnet } from 'viem/chains';

const client = createPublicClient({
  chain: mainnet,
  transport: http('https://rpc-proxy-cache.YOUR_SUBDOMAIN.workers.dev/1')
});
```

### URL Format

**Base format:** `https://your-worker.workers.dev/{chainId}`

**How routing works:**
- The worker extracts the **first path segment** as the chain ID
- Path `/` → defaults to chain ID `1` (Ethereum Mainnet)
- Path `/137` → chain ID `137` (Polygon)
- Path `/56/anything` → chain ID `56` (BSC) - trailing segments are ignored

**Examples:**
```bash
# All of these use chain ID "1" (Ethereum):
POST https://your-worker.workers.dev/1
POST https://your-worker.workers.dev/

# Polygon (chain ID 137):
POST https://your-worker.workers.dev/137

# BSC (chain ID 56):
POST https://your-worker.workers.dev/56
```

**Common chain IDs:**
- `1` - Ethereum Mainnet
- `137` - Polygon
- `56` - Binance Smart Chain (BSC)
- `42161` - Arbitrum One
- `10` - Optimism
- `8453` - Base
- `43114` - Avalanche C-Chain

## API Reference

### Request Format

All requests should be standard JSON-RPC 2.0 POST requests:

```bash
POST /{chainId}
Content-Type: application/json

{
  "jsonrpc": "2.0",
  "method": "eth_methodName",
  "params": [...],  // Optional - defaults to [] if not provided
  "id": 1
}
```

**JSON-RPC 2.0 Compliance:**
- `jsonrpc`: Required - must be "2.0"
- `method`: Required - the RPC method name
- `params`: Optional - defaults to empty array `[]` if omitted
- `id`: Required - request identifier (can be string, number, or null)

### Cached Methods

**`eth_getLogs`**
- Storage: R2 (long-term, cost-effective)
- Cache Key: Based on chain ID + request parameters (fromBlock, toBlock, address, topics)
- Caching Rule: Only caches if `toBlock + blockDistance <= currentBlock`
- Use Case: Historical log queries

**`eth_getBlockByNumber`**
- Storage: KV (fast access)
- TTL: 2 seconds
- Cache Key: chain ID + block number
- Use Case: Frequently polled recent blocks

**All Other Methods**
- Directly proxied to upstream RPC
- No caching applied
- Examples: `eth_blockNumber`, `eth_call`, `eth_estimateGas`, etc.

## Configuration Examples

### High-throughput Chain (e.g., Polygon)
For chains with fast block times and frequent reorgs:
```toml
CHAIN_BLOCK_DISTANCES = "{\"137\": 256}"
```

### Ethereum Mainnet
More conservative caching:
```toml
CHAIN_BLOCK_DISTANCES = "{\"1\": 64}"
```

### BSC (Binance Smart Chain)
Balanced configuration:
```toml
CHAIN_BLOCK_DISTANCES = "{\"56\": 128}"
```

## Monitoring

### Worker Logs

Check worker logs in real-time:
```bash
# Development
wrangler tail

# Production
wrangler tail --env production
```

Filter logs by status:
```bash
wrangler tail --env production --status error
```

### Storage Usage

**View R2 storage:**
```bash
# List objects in R2 bucket
wrangler r2 object list rpc-logs-cache

# Get bucket info
wrangler r2 bucket info rpc-logs-cache
```

**View KV storage:**
```bash
# List all keys (limited to first 1000)
wrangler kv key list --binding=BLOCKS_CACHE --env production

# Get a specific key value
wrangler kv key get "block:1:0x1000000" --binding=BLOCKS_CACHE --env production
```

### Performance Metrics

Monitor in CloudFlare Dashboard:
- Worker analytics: Requests, errors, CPU time
- R2 analytics: Storage size, operations count
- KV analytics: Read/write operations

## Cost Optimization

### R2 Storage (eth_getLogs)
- **Class B Operations**: ~$0.36 per million requests
- **Storage**: $0.015 per GB-month
- **Best for**: Historical logs that don't change

### KV Storage (eth_getBlockByNumber)  
- **Read Operations**: ~$0.50 per million reads
- **Storage**: $0.50 per GB-month
- **Best for**: Frequently accessed, short-lived data

## Troubleshooting

### "R2 bucket not available" error
- Ensure R2 bucket is created and properly bound in `wrangler.toml`
- Check bucket name matches exactly

### "KV namespace not available" error
- Verify KV namespace ID is correct in `wrangler.toml`
- Ensure namespace exists with `wrangler kv:namespace list`

### Cache not working
- Check logs with `wrangler tail` or `wrangler tail --env production`
- Verify block distance configuration in `wrangler.toml`
- Ensure requests include chain ID in the URL path (e.g., `/1` for Ethereum)
- Verify R2 bucket and KV namespace are properly bound

### Build errors
- Ensure Rust toolchain is up to date: `rustup update`
- Install worker-build: `cargo install worker-build`
- Add wasm32 target: `rustup target add wasm32-unknown-unknown`
- Clear build cache: `cargo clean`
- Check `Cargo.toml` dependencies are compatible with `wasm32-unknown-unknown` target

## Performance Tips

1. **Block Distance Configuration**: Set appropriate block distances per chain based on:
   - Average block time
   - Historical reorg frequency
   - Data freshness requirements

2. **KV TTL**: The 2-second TTL for blocks balances:
   - Cache hit rate for frequently polled blocks
   - Freshness of block data
   - KV read costs

3. **Request Batching**: Batch multiple RPC calls in a single request when possible

## Security

- **Memory Safety**: Built with Rust for memory-safe execution
- **Type Safety**: Strong type system prevents common bugs
- CORS is enabled for all origins (adjust in `lib.rs` for production)
- No authentication is implemented (add if needed)
- Consider rate limiting for production deployments

## License

MIT

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## Support

For issues and questions:
- Open an issue on GitHub
- Check CloudFlare Workers documentation: https://developers.cloudflare.com/workers/

