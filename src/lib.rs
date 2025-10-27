use serde_json::{json, Value};
use worker::*;

mod cache;
mod rpc;
mod utils;

use cache::CacheManager;
use rpc::RpcRequest;

#[event(fetch)]
async fn main(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();

    // Handle CORS preflight
    if req.method() == Method::Options {
        return Response::empty()
            .map(|res| {
                res.with_headers(get_cors_headers())
            });
    }

    // Extract chainId from URL path (e.g., /1, /137, /56)
    // Takes the first path segment as chain ID, defaults to "1" if not provided
    let url = req.url()?;
    let path = url.path();
    let chain_id = path
        .trim_start_matches('/')
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("1")
        .to_string();

    console_log!("Request received: method={}, path={}, chain_id={}", req.method(), path, chain_id);

    // Parse the RPC request
    let rpc_request: RpcRequest = match req.json().await {
        Ok(req) => req,
        Err(e) => {
            console_log!("ERROR: Failed to parse JSON-RPC request: {:?}", e);
            return Response::error("Invalid JSON-RPC request", 400);
        }
    };

    console_log!("RPC request parsed: method={}, id={:?}, params={}", rpc_request.method, rpc_request.id, rpc_request.params);

    // Initialize cache manager
    let cache_manager = match CacheManager::new(&env, &chain_id) {
        Ok(manager) => manager,
        Err(e) => {
            console_log!("ERROR: Failed to initialize cache manager: {:?}", e);
            return Err(e);
        }
    };

    // Handle different RPC methods
    let response = match rpc_request.method.as_str() {
        "eth_getLogs" => {
            console_log!("Handling eth_getLogs request");
            match handle_get_logs(&rpc_request, &cache_manager, &env, &chain_id).await {
                Ok(resp) => resp,
                Err(e) => {
                    console_log!("ERROR in eth_getLogs: {:?}", e);
                    return Err(e);
                }
            }
        }
        "eth_getBlockByNumber" => {
            console_log!("Handling eth_getBlockByNumber request");
            match handle_get_block_by_number(&rpc_request, &cache_manager, &env, &chain_id).await {
                Ok(resp) => resp,
                Err(e) => {
                    console_log!("ERROR in eth_getBlockByNumber: {:?}", e);
                    return Err(e);
                }
            }
        }
        "eth_getTransactionReceipt" => {
            console_log!("Handling eth_getTransactionReceipt request");
            match handle_get_transaction_receipt(&rpc_request, &cache_manager, &env, &chain_id).await {
                Ok(resp) => resp,
                Err(e) => {
                    console_log!("ERROR in eth_getTransactionReceipt: {:?}", e);
                    return Err(e);
                }
            }
        }
        "eth_getBlockByHash" => {
            console_log!("Handling eth_getBlockByHash request");
            match handle_get_block_by_hash(&rpc_request, &cache_manager, &env, &chain_id).await {
                Ok(resp) => resp,
                Err(e) => {
                    console_log!("ERROR in eth_getBlockByHash: {:?}", e);
                    return Err(e);
                }
            }
        }
        "eth_getBlockReceipts" => {
            console_log!("Handling eth_getBlockReceipts request");
            match handle_get_block_receipts(&rpc_request, &cache_manager, &env, &chain_id).await {
                Ok(resp) => resp,
                Err(e) => {
                    console_log!("ERROR in eth_getBlockReceipts: {:?}", e);
                    return Err(e);
                }
            }
        }
        "debug_traceBlockByNumber" => {
            console_log!("Handling debug_traceBlockByNumber request");
            match handle_debug_trace_block(&rpc_request, &cache_manager, &env, &chain_id, "debug_traceBlockByNumber").await {
                Ok(resp) => resp,
                Err(e) => {
                    console_log!("ERROR in debug_traceBlockByNumber: {:?}", e);
                    return Err(e);
                }
            }
        }
        "debug_traceBlockByHash" => {
            console_log!("Handling debug_traceBlockByHash request");
            match handle_debug_trace_block(&rpc_request, &cache_manager, &env, &chain_id, "debug_traceBlockByHash").await {
                Ok(resp) => resp,
                Err(e) => {
                    console_log!("ERROR in debug_traceBlockByHash: {:?}", e);
                    return Err(e);
                }
            }
        }
        _ => {
            console_log!("Proxying method: {}", rpc_request.method);
            match proxy_request(&rpc_request, &env, &chain_id).await {
                Ok(resp) => resp,
                Err(e) => {
                    console_log!("ERROR in proxy_request: {:?}", e);
                    return Err(e);
                }
            }
        }
    };

    console_log!("Request completed successfully for method: {}", rpc_request.method);
    
    Response::from_json(&response)
        .map(|res| res.with_headers(get_cors_headers()))
}

async fn handle_get_logs(
    rpc_request: &RpcRequest,
    cache_manager: &CacheManager,
    env: &Env,
    chain_id: &str,
) -> Result<Value> {
    // Parse the eth_getLogs parameters
    let params = match rpc_request.params.as_array() {
        Some(arr) if !arr.is_empty() => &arr[0],
        _ => {
            return Ok(json!({
                "jsonrpc": "2.0",
                "id": rpc_request.id,
                "error": {
                    "code": -32602,
                    "message": "Invalid params"
                }
            }));
        }
    };

    // Extract block range
    let from_block = params.get("fromBlock").and_then(|v| v.as_str());
    let to_block = params.get("toBlock").and_then(|v| v.as_str());

    // Check if we should cache this request
    if let (Some(from), Some(to)) = (from_block, to_block) {
        // Check if the block range is far enough from the tip to avoid reorgs
        if let Ok(should_cache) = cache_manager.should_cache_logs(from, to, env).await {
            if should_cache {
                // Try to get from cache
                if let Ok(Some(cached)) = cache_manager.get_logs_from_cache(params).await {
                    console_log!("eth_getLogs cache HIT");
                    return Ok(json!({
                        "jsonrpc": "2.0",
                        "id": rpc_request.id,
                        "result": cached
                    }));
                }
                console_log!("eth_getLogs cache MISS");
            } else {
                console_log!("eth_getLogs: blocks too recent, skipping cache");
            }
        }
    }

    // Cache miss or not cacheable - fetch from upstream
    let result = proxy_request(rpc_request, env, chain_id).await?;

    // Store in cache if applicable
    if let (Some(from), Some(to)) = (from_block, to_block) {
        if let Ok(should_cache) = cache_manager.should_cache_logs(from, to, env).await {
            if should_cache {
                if let Some(logs) = result.get("result") {
                    let _ = cache_manager.store_logs_in_cache(params, logs).await;
                }
            }
        }
    }

    Ok(result)
}

async fn handle_get_block_by_number(
    rpc_request: &RpcRequest,
    cache_manager: &CacheManager,
    env: &Env,
    chain_id: &str,
) -> Result<Value> {
    // Extract block number from params
    let block_number = match rpc_request.params.as_array() {
        Some(arr) if !arr.is_empty() => arr[0].as_str().unwrap_or("latest"),
        _ => "latest",
    };

    // Try to get from in-memory cache (2 second TTL)
    if let Some(cached) = cache_manager.get_block_from_cache(block_number) {
        console_log!("eth_getBlockByNumber cache HIT for block {}", block_number);
        return Ok(json!({
            "jsonrpc": "2.0",
            "id": rpc_request.id,
            "result": cached
        }));
    }

    console_log!("eth_getBlockByNumber cache MISS for block {}", block_number);

    // Fetch from upstream
    let result = proxy_request(rpc_request, env, chain_id).await?;

    // Store in memory cache with 2 second TTL
    if let Some(block) = result.get("result") {
        cache_manager.store_block_in_cache(block_number, block);
    }

    Ok(result)
}

async fn handle_get_transaction_receipt(
    rpc_request: &RpcRequest,
    cache_manager: &CacheManager,
    env: &Env,
    chain_id: &str,
) -> Result<Value> {
    // Extract transaction hash from params
    let tx_hash = match rpc_request.params.as_array() {
        Some(arr) if !arr.is_empty() => {
            arr[0].as_str().ok_or("Transaction hash must be a string")?
        }
        _ => {
            return Ok(json!({
                "jsonrpc": "2.0",
                "id": rpc_request.id,
                "error": {
                    "code": -32602,
                    "message": "Invalid params: missing transaction hash"
                }
            }));
        }
    };

    // Try to get from R2 cache
    if let Ok(Some(cached)) = cache_manager.get_tx_receipt_from_cache(tx_hash).await {
        console_log!("eth_getTransactionReceipt cache HIT for tx {}", tx_hash);
        return Ok(json!({
            "jsonrpc": "2.0",
            "id": rpc_request.id,
            "result": cached
        }));
    }

    console_log!("eth_getTransactionReceipt cache MISS for tx {}", tx_hash);

    // Fetch from upstream
    let result = proxy_request(rpc_request, env, chain_id).await?;

    // Store in R2 cache if receipt is confirmed (has blockNumber)
    if let Some(receipt) = result.get("result") {
        if cache_manager.should_cache_tx_receipt(receipt) {
            console_log!("Transaction receipt is confirmed, storing in cache");
            let _ = cache_manager.store_tx_receipt_in_cache(tx_hash, receipt).await;
        } else {
            console_log!("Transaction receipt not confirmed yet, skipping cache");
        }
    }

    Ok(result)
}

async fn handle_get_block_by_hash(
    rpc_request: &RpcRequest,
    cache_manager: &CacheManager,
    env: &Env,
    chain_id: &str,
) -> Result<Value> {
    // Extract block hash from params
    let block_hash = match rpc_request.params.as_array() {
        Some(arr) if !arr.is_empty() => {
            arr[0].as_str().ok_or("Block hash must be a string")?
        }
        _ => {
            return Ok(json!({
                "jsonrpc": "2.0",
                "id": rpc_request.id,
                "error": {
                    "code": -32602,
                    "message": "Invalid params: missing block hash"
                }
            }));
        }
    };

    // Try to get from R2 cache
    if let Ok(Some(cached)) = cache_manager.get_block_by_hash_from_cache(block_hash).await {
        console_log!("eth_getBlockByHash cache HIT for block {}", block_hash);
        return Ok(json!({
            "jsonrpc": "2.0",
            "id": rpc_request.id,
            "result": cached
        }));
    }

    console_log!("eth_getBlockByHash cache MISS for block {}", block_hash);

    // Fetch from upstream
    let result = proxy_request(rpc_request, env, chain_id).await?;

    // Store in R2 cache if block is old enough
    if let Some(block) = result.get("result") {
        if !block.is_null() {
            if let Ok(should_cache) = cache_manager.should_cache_block(block, env).await {
                if should_cache {
                    console_log!("Block is old enough, storing in cache");
                    let _ = cache_manager.store_block_by_hash_in_cache(block_hash, block).await;
                } else {
                    console_log!("Block is too recent, skipping cache");
                }
            }
        }
    }

    Ok(result)
}

async fn handle_get_block_receipts(
    rpc_request: &RpcRequest,
    cache_manager: &CacheManager,
    env: &Env,
    chain_id: &str,
) -> Result<Value> {
    // Extract block identifier from params (can be block number or hash)
    let block_id = match rpc_request.params.as_array() {
        Some(arr) if !arr.is_empty() => {
            arr[0].as_str().ok_or("Block identifier must be a string")?
        }
        _ => {
            return Ok(json!({
                "jsonrpc": "2.0",
                "id": rpc_request.id,
                "error": {
                    "code": -32602,
                    "message": "Invalid params: missing block identifier"
                }
            }));
        }
    };

    // Detect if it's a block hash (66 chars) or block number
    let is_block_hash = block_id.starts_with("0x") && block_id.len() == 66;

    // Try to get from R2 cache
    if let Ok(Some(cached)) = cache_manager.get_block_receipts_from_cache(block_id).await {
        console_log!("eth_getBlockReceipts cache HIT for block {}", block_id);
        return Ok(json!({
            "jsonrpc": "2.0",
            "id": rpc_request.id,
            "result": cached
        }));
    }

    console_log!("eth_getBlockReceipts cache MISS for block {}", block_id);

    // Fetch from upstream
    let result = proxy_request(rpc_request, env, chain_id).await?;

    // Store in R2 cache if block is old enough
    if let Some(receipts) = result.get("result") {
        if !receipts.is_null() {
            // For block hash, check block number from response
            // For block number, check directly
            let should_cache = if is_block_hash {
                console_log!("Block hash provided - checking block number from response");
                // Try to extract block number from first receipt
                if let Some(receipts_array) = receipts.as_array() {
                    if let Some(first_receipt) = receipts_array.first() {
                        cache_manager.should_cache_from_response(first_receipt, env).await.unwrap_or(false)
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                cache_manager.should_cache_block_id(block_id, env).await.unwrap_or(false)
            };

            if should_cache {
                console_log!("Block receipts are for old block, storing in cache");
                let _ = cache_manager.store_block_receipts_in_cache(block_id, receipts).await;
            } else {
                console_log!("Block is too recent or no block number found, skipping cache");
            }
        }
    }

    Ok(result)
}

async fn handle_debug_trace_block(
    rpc_request: &RpcRequest,
    cache_manager: &CacheManager,
    env: &Env,
    chain_id: &str,
    method: &str,
) -> Result<Value> {
    // Extract block identifier from params (can be block number or hash)
    let block_id = match rpc_request.params.as_array() {
        Some(arr) if !arr.is_empty() => {
            arr[0].as_str().ok_or("Block identifier must be a string")?
        }
        _ => {
            return Ok(json!({
                "jsonrpc": "2.0",
                "id": rpc_request.id,
                "error": {
                    "code": -32602,
                    "message": "Invalid params: missing block identifier"
                }
            }));
        }
    };

    // Detect if it's a block hash (66 chars) or block number
    let is_block_hash = block_id.starts_with("0x") && block_id.len() == 66;

    // Try to get from R2 cache
    if let Ok(Some(cached)) = cache_manager.get_trace_from_cache(method, block_id).await {
        console_log!("{} cache HIT for block {}", method, block_id);
        return Ok(json!({
            "jsonrpc": "2.0",
            "id": rpc_request.id,
            "result": cached
        }));
    }

    console_log!("{} cache MISS for block {}", method, block_id);

    // Fetch from upstream
    let result = proxy_request(rpc_request, env, chain_id).await?;

    // Store in R2 cache if block is old enough
    if let Some(trace) = result.get("result") {
        if !trace.is_null() {
            // For block hash, check block number from response
            // For block number, check directly
            let should_cache = if is_block_hash {
                console_log!("Block hash provided - checking block number from response");
                // Debug traces might have block info at different locations
                // Try to extract from trace structure
                if let Some(block_obj) = trace.as_object() {
                    // Look for block number in trace result
                    if let Some(_struct_logs) = block_obj.get("structLogs") {
                        // It's a transaction trace, might not have block number directly
                        // For now, don't cache block hash traces unless we can extract block number
                        console_log!("Debug trace by hash - cannot determine block age, skipping cache");
                        false
                    } else {
                        // Try direct check
                        cache_manager.should_cache_from_response(trace, env).await.unwrap_or(false)
                    }
                } else {
                    false
                }
            } else {
                cache_manager.should_cache_block_id(block_id, env).await.unwrap_or(false)
            };

            if should_cache {
                console_log!("Block trace is for old block, storing in cache");
                let _ = cache_manager.store_trace_in_cache(method, block_id, trace).await;
            } else {
                console_log!("Block is too recent or cannot determine age, skipping cache");
            }
        }
    }

    Ok(result)
}

async fn proxy_request(rpc_request: &RpcRequest, env: &Env, chain_id: &str) -> Result<Value> {
    let upstream_url = env
        .var(&format!("UPSTREAM_RPC_URL_{}", chain_id))?
        .to_string();

    console_log!("Proxying to upstream: {}", upstream_url);

    let mut headers = Headers::new();
    headers.set("Content-Type", "application/json")?;

    let request_body = match serde_json::to_string(rpc_request) {
        Ok(body) => body,
        Err(e) => {
            console_log!("ERROR: Failed to serialize RPC request: {:?}", e);
            return Err(e.to_string().into());
        }
    };

    console_log!("Upstream request body: {}", request_body);

    let request = Request::new_with_init(
        &upstream_url,
        RequestInit::new()
            .with_method(Method::Post)
            .with_headers(headers)
            .with_body(Some(request_body.into())),
    )?;

    let mut response = match Fetch::Request(request).send().await {
        Ok(resp) => resp,
        Err(e) => {
            console_log!("ERROR: Failed to send request to upstream: {:?}", e);
            return Err(e);
        }
    };

    let status = response.status_code();
    console_log!("Upstream response status: {}", status);

    let response_json: Value = match response.json().await {
        Ok(json) => json,
        Err(e) => {
            console_log!("ERROR: Failed to parse upstream response as JSON: {:?}", e);
            return Err(e);
        }
    };

    console_log!("Upstream response: {}", response_json);

    Ok(response_json)
}

fn get_cors_headers() -> Headers {
    let mut headers = Headers::new();
    let _ = headers.set("Access-Control-Allow-Origin", "*");
    let _ = headers.set("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
    let _ = headers.set("Access-Control-Allow-Headers", "Content-Type");
    headers
}

