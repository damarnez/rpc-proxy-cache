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
            match handle_get_logs(&rpc_request, &cache_manager, &env).await {
                Ok(resp) => resp,
                Err(e) => {
                    console_log!("ERROR in eth_getLogs: {:?}", e);
                    return Err(e);
                }
            }
        }
        "eth_getBlockByNumber" => {
            console_log!("Handling eth_getBlockByNumber request");
            match handle_get_block_by_number(&rpc_request, &cache_manager, &env).await {
                Ok(resp) => resp,
                Err(e) => {
                    console_log!("ERROR in eth_getBlockByNumber: {:?}", e);
                    return Err(e);
                }
            }
        }
        _ => {
            console_log!("Proxying method: {}", rpc_request.method);
            match proxy_request(&rpc_request, &env).await {
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
    let result = proxy_request(rpc_request, env).await?;

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
) -> Result<Value> {
    // Extract block number from params
    let block_number = match rpc_request.params.as_array() {
        Some(arr) if !arr.is_empty() => arr[0].as_str().unwrap_or("latest"),
        _ => "latest",
    };

    // Try to get from KV cache (2 second TTL)
    if let Ok(Some(cached)) = cache_manager.get_block_from_cache(block_number).await {
        console_log!("eth_getBlockByNumber cache HIT for block {}", block_number);
        return Ok(json!({
            "jsonrpc": "2.0",
            "id": rpc_request.id,
            "result": cached
        }));
    }

    console_log!("eth_getBlockByNumber cache MISS for block {}", block_number);

    // Fetch from upstream
    let result = proxy_request(rpc_request, env).await?;

    // Store in KV cache with 2 second expiration
    if let Some(block) = result.get("result") {
        let _ = cache_manager.store_block_in_cache(block_number, block).await;
    }

    Ok(result)
}

async fn proxy_request(rpc_request: &RpcRequest, env: &Env) -> Result<Value> {
    let upstream_url = env
        .var("UPSTREAM_RPC_URL")?
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

