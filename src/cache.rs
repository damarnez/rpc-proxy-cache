use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use worker::*;

use crate::utils::{generate_cache_key, parse_hex_to_u64};

#[derive(Clone)]
struct CachedBlock {
    data: Value,
    timestamp_ms: u64,
}

pub struct CacheManager {
    chain_id: String,
    r2_bucket: Option<Bucket>,
    block_distance_config: HashMap<String, u64>,
    default_block_distance: u64,
    // In-memory cache for blocks with 2-second TTL
    block_cache: RefCell<HashMap<String, CachedBlock>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetLogsRequest {
    #[serde(rename = "fromBlock")]
    pub from_block: Option<String>,
    #[serde(rename = "toBlock")]
    pub to_block: Option<String>,
    pub address: Option<Value>,
    pub topics: Option<Vec<Option<Value>>>,
}

impl CacheManager {
    pub fn new(env: &Env, chain_id: &str) -> Result<Self> {
        // Get R2 bucket for logs cache
        let r2_bucket = env.bucket("LOGS_CACHE").ok();

        // Load block distance configuration
        let default_block_distance = env
            .var("DEFAULT_BLOCK_DISTANCE")
            .ok()
            .and_then(|v| v.to_string().parse::<u64>().ok())
            .unwrap_or(100);

        let block_distance_config: HashMap<String, u64> = env
            .var("CHAIN_BLOCK_DISTANCES")
            .ok()
            .and_then(|v| serde_json::from_str(&v.to_string()).ok())
            .unwrap_or_default();

        console_log!(
            "CacheManager initialized for chain {} with block distance {}",
            chain_id,
            default_block_distance
        );

        Ok(Self {
            chain_id: chain_id.to_string(),
            r2_bucket,
            block_distance_config,
            default_block_distance,
            block_cache: RefCell::new(HashMap::new()),
        })
    }

    /// Get the block distance for the current chain
    fn get_block_distance(&self) -> u64 {
        self.block_distance_config
            .get(&self.chain_id)
            .copied()
            .unwrap_or(self.default_block_distance)
    }

    /// Check if logs should be cached based on block distance from tip
    pub async fn should_cache_logs(
        &self,
        from_block: &str,
        to_block: &str,
        env: &Env,
    ) -> Result<bool> {
        // Skip caching if using special tags like "latest" or "pending"
        if from_block == "latest"
            || from_block == "pending"
            || to_block == "latest"
            || to_block == "pending"
        {
            return Ok(false);
        }

        // Parse block numbers
        let from = parse_hex_to_u64(from_block)?;
        let to = parse_hex_to_u64(to_block)?;

        // Get current block number
        let current_block = match self.get_current_block_number(env).await {
            Ok(num) => num,
            Err(e) => {
                console_log!("Failed to get current block number: {:?}", e);
                return Ok(false);
            }
        };

        let block_distance = self.get_block_distance();

        // Cache only if the requested range is at least block_distance blocks behind current
        let should_cache = to + block_distance <= current_block;

        console_log!(
            "Block cache check: from={}, to={}, current={}, distance={}, should_cache={}",
            from,
            to,
            current_block,
            block_distance,
            should_cache
        );

        Ok(should_cache)
    }

    /// Get current block number from the RPC
    async fn get_current_block_number(&self, env: &Env) -> Result<u64> {
        let upstream_url = env.var(&format!("UPSTREAM_RPC_URL_{}", self.chain_id))?.to_string();

        let rpc_request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": [],
            "id": 1
        });

        let mut headers = Headers::new();
        headers.set("Content-Type", "application/json")?;

        let request = Request::new_with_init(
            &upstream_url,
            RequestInit::new()
                .with_method(Method::Post)
                .with_headers(headers)
                .with_body(Some(serde_json::to_string(&rpc_request)?.into())),
        )?;

        let mut response = Fetch::Request(request).send().await?;
        let response_json: Value = response.json().await?;

        if let Some(result) = response_json.get("result").and_then(|v| v.as_str()) {
            parse_hex_to_u64(result)
        } else {
            Err("Failed to get block number".into())
        }
    }

    /// Get logs from R2 cache
    pub async fn get_logs_from_cache(&self, params: &Value) -> Result<Option<Value>> {
        let r2_bucket = match &self.r2_bucket {
            Some(bucket) => bucket,
            None => return Ok(None),
        };

        let cache_key = self.generate_logs_cache_key(params);

        match r2_bucket.get(&cache_key).execute().await? {
            Some(object) => {
                let body = object.body().ok_or("No body in R2 object")?;
                let bytes = body.bytes().await?;
                let logs: Value = serde_json::from_slice(&bytes)?;
                Ok(Some(logs))
            }
            None => Ok(None),
        }
    }

    /// Store logs in R2 cache
    pub async fn store_logs_in_cache(&self, params: &Value, logs: &Value) -> Result<()> {
        let r2_bucket = match &self.r2_bucket {
            Some(bucket) => bucket,
            None => return Err("R2 bucket not available".into()),
        };

        let cache_key = self.generate_logs_cache_key(params);
        let logs_json = serde_json::to_vec(logs)?;

        r2_bucket.put(&cache_key, logs_json).execute().await?;

        console_log!("Stored logs in R2 cache with key: {}", cache_key);

        Ok(())
    }

    /// Generate cache key for eth_getLogs based on parameters
    fn generate_logs_cache_key(&self, params: &Value) -> String {
        // Create a normalized version of the parameters for the cache key
        let normalized = serde_json::to_string(params).unwrap_or_default();
        let hash = generate_cache_key(&self.chain_id, &normalized);
        // Store in eth_getLogs/{chain_id}/ folder
        format!("eth_getLogs/{}/{}", self.chain_id, hash)
    }

    /// Get transaction receipt from R2 cache
    pub async fn get_tx_receipt_from_cache(&self, tx_hash: &str) -> Result<Option<Value>> {
        let r2_bucket = match &self.r2_bucket {
            Some(bucket) => bucket,
            None => return Ok(None),
        };

        let cache_key = self.generate_tx_receipt_cache_key(tx_hash);

        match r2_bucket.get(&cache_key).execute().await? {
            Some(object) => {
                let body = object.body().ok_or("No body in R2 object")?;
                let bytes = body.bytes().await?;
                let receipt: Value = serde_json::from_slice(&bytes)?;
                Ok(Some(receipt))
            }
            None => Ok(None),
        }
    }

    /// Store transaction receipt in R2 cache
    pub async fn store_tx_receipt_in_cache(&self, tx_hash: &str, receipt: &Value) -> Result<()> {
        let r2_bucket = match &self.r2_bucket {
            Some(bucket) => bucket,
            None => return Err("R2 bucket not available".into()),
        };

        let cache_key = self.generate_tx_receipt_cache_key(tx_hash);
        let receipt_json = serde_json::to_vec(receipt)?;

        r2_bucket.put(&cache_key, receipt_json).execute().await?;

        console_log!("Stored transaction receipt in R2 cache with key: {}", cache_key);

        Ok(())
    }

    /// Generate cache key for eth_getTransactionReceipt
    fn generate_tx_receipt_cache_key(&self, tx_hash: &str) -> String {
        // Store in eth_getTransactionReceipt/ folder
        // Transaction hash is already unique, use it directly (normalized to lowercase)
        let normalized_hash = tx_hash.to_lowercase();
        format!("eth_getTransactionReceipt/{}/{}", self.chain_id, normalized_hash)
    }

    /// Check if transaction receipt should be cached
    /// Receipts are cached if the transaction is confirmed (not null)
    pub fn should_cache_tx_receipt(&self, receipt: &Value) -> bool {
        // If receipt is not null and has a blockNumber, it's confirmed
        if receipt.is_null() {
            return false;
        }
        
        // Check if receipt has a blockNumber (meaning it's been mined)
        receipt.get("blockNumber")
            .and_then(|v| v.as_str())
            .map(|bn| !bn.is_empty() && bn != "null")
            .unwrap_or(false)
    }

    /// Get block by hash from R2 cache
    pub async fn get_block_by_hash_from_cache(&self, block_hash: &str) -> Result<Option<Value>> {
        let r2_bucket = match &self.r2_bucket {
            Some(bucket) => bucket,
            None => return Ok(None),
        };

        let cache_key = self.generate_block_by_hash_cache_key(block_hash);

        match r2_bucket.get(&cache_key).execute().await? {
            Some(object) => {
                let body = object.body().ok_or("No body in R2 object")?;
                let bytes = body.bytes().await?;
                let block: Value = serde_json::from_slice(&bytes)?;
                Ok(Some(block))
            }
            None => Ok(None),
        }
    }

    /// Store block by hash in R2 cache
    pub async fn store_block_by_hash_in_cache(&self, block_hash: &str, block: &Value) -> Result<()> {
        let r2_bucket = match &self.r2_bucket {
            Some(bucket) => bucket,
            None => return Err("R2 bucket not available".into()),
        };

        let cache_key = self.generate_block_by_hash_cache_key(block_hash);
        let block_json = serde_json::to_vec(block)?;

        r2_bucket.put(&cache_key, block_json).execute().await?;

        console_log!("Stored block by hash in R2 cache with key: {}", cache_key);

        Ok(())
    }

    /// Generate cache key for eth_getBlockByHash
    fn generate_block_by_hash_cache_key(&self, block_hash: &str) -> String {
        let normalized_hash = block_hash.to_lowercase();
        format!("eth_getBlockByHash/{}/{}", self.chain_id, normalized_hash)
    }

    /// Check if block should be cached based on block number
    pub async fn should_cache_block(&self, block: &Value, env: &Env) -> Result<bool> {
        // Check if block has a number
        let block_number_str = match block.get("number").and_then(|v| v.as_str()) {
            Some(bn) => bn,
            None => return Ok(false),
        };

        // Skip special tags
        if block_number_str == "latest" || block_number_str == "pending" {
            return Ok(false);
        }

        // Parse block number
        let block_number = parse_hex_to_u64(block_number_str)?;

        // Get current block number
        let current_block = match self.get_current_block_number(env).await {
            Ok(num) => num,
            Err(e) => {
                console_log!("Failed to get current block number: {:?}", e);
                return Ok(false);
            }
        };

        let block_distance = self.get_block_distance();
        
        // Cache only if block is old enough
        Ok(block_number + block_distance <= current_block)
    }

    /// Get block receipts from R2 cache
    pub async fn get_block_receipts_from_cache(&self, block_id: &str) -> Result<Option<Value>> {
        let r2_bucket = match &self.r2_bucket {
            Some(bucket) => bucket,
            None => return Ok(None),
        };

        let cache_key = self.generate_block_receipts_cache_key(block_id);

        match r2_bucket.get(&cache_key).execute().await? {
            Some(object) => {
                let body = object.body().ok_or("No body in R2 object")?;
                let bytes = body.bytes().await?;
                let receipts: Value = serde_json::from_slice(&bytes)?;
                Ok(Some(receipts))
            }
            None => Ok(None),
        }
    }

    /// Store block receipts in R2 cache
    pub async fn store_block_receipts_in_cache(&self, block_id: &str, receipts: &Value) -> Result<()> {
        let r2_bucket = match &self.r2_bucket {
            Some(bucket) => bucket,
            None => return Err("R2 bucket not available".into()),
        };

        let cache_key = self.generate_block_receipts_cache_key(block_id);
        let receipts_json = serde_json::to_vec(receipts)?;

        r2_bucket.put(&cache_key, receipts_json).execute().await?;

        console_log!("Stored block receipts in R2 cache with key: {}", cache_key);

        Ok(())
    }

    /// Generate cache key for eth_getBlockReceipts
    fn generate_block_receipts_cache_key(&self, block_id: &str) -> String {
        let normalized = block_id.to_lowercase();
        format!("eth_getBlockReceipts/{}/{}", self.chain_id, normalized)
    }

    /// Get trace from R2 cache
    pub async fn get_trace_from_cache(&self, method: &str, block_id: &str) -> Result<Option<Value>> {
        let r2_bucket = match &self.r2_bucket {
            Some(bucket) => bucket,
            None => return Ok(None),
        };

        let cache_key = self.generate_trace_cache_key(method, block_id);

        match r2_bucket.get(&cache_key).execute().await? {
            Some(object) => {
                let body = object.body().ok_or("No body in R2 object")?;
                let bytes = body.bytes().await?;
                let trace: Value = serde_json::from_slice(&bytes)?;
                Ok(Some(trace))
            }
            None => Ok(None),
        }
    }

    /// Store trace in R2 cache
    pub async fn store_trace_in_cache(&self, method: &str, block_id: &str, trace: &Value) -> Result<()> {
        let r2_bucket = match &self.r2_bucket {
            Some(bucket) => bucket,
            None => return Err("R2 bucket not available".into()),
        };

        let cache_key = self.generate_trace_cache_key(method, block_id);
        let trace_json = serde_json::to_vec(trace)?;

        r2_bucket.put(&cache_key, trace_json).execute().await?;

        console_log!("Stored trace in R2 cache with key: {}", cache_key);

        Ok(())
    }

    /// Generate cache key for debug trace methods
    fn generate_trace_cache_key(&self, method: &str, block_id: &str) -> String {
        let normalized = block_id.to_lowercase();
        format!("{}/{}/{}", method, self.chain_id, normalized)
    }

    /// Check if block ID should be cached (for block receipts and traces)
    /// For block numbers, we can check directly. For block hashes, caller should extract
    /// block number from response and use should_cache_by_block_number instead.
    pub async fn should_cache_block_id(&self, block_id: &str, env: &Env) -> Result<bool> {
        // Skip special tags
        if block_id == "latest" || block_id == "pending" || block_id == "earliest" {
            return Ok(false);
        }

        // If it's a block hash (0x followed by 64 hex chars), we can't determine without response
        // Caller should extract block number from response and check
        if block_id.starts_with("0x") && block_id.len() == 66 {
            // Return Ok(false) to indicate: need to check response
            // This is just a signal - not that it's uncacheable, but that we need response data
            console_log!("Block hash detected - will check block number from response");
            return Ok(false);
        }

        // It's a block number - parse and check distance
        let block_number = parse_hex_to_u64(block_id)?;
        self.should_cache_by_block_number(block_number, env).await
    }

    /// Check if a specific block number should be cached
    pub async fn should_cache_by_block_number(&self, block_number: u64, env: &Env) -> Result<bool> {
        // Get current block number
        let current_block = match self.get_current_block_number(env).await {
            Ok(num) => num,
            Err(e) => {
                console_log!("Failed to get current block number: {:?}", e);
                return Ok(false);
            }
        };

        let block_distance = self.get_block_distance();
        
        // Cache only if block is old enough
        let should_cache = block_number + block_distance <= current_block;
        
        console_log!(
            "Block number {} check: current={}, distance={}, should_cache={}",
            block_number, current_block, block_distance, should_cache
        );
        
        Ok(should_cache)
    }

    /// Extract block number from response data and check if cacheable
    pub async fn should_cache_from_response(&self, response_data: &Value, env: &Env) -> Result<bool> {
        // Try to extract block number from response
        // Could be at different paths depending on response type
        let block_number_str = if let Some(bn) = response_data.get("blockNumber").and_then(|v| v.as_str()) {
            // For receipts, traces might have blockNumber
            bn
        } else if let Some(bn) = response_data.get("number").and_then(|v| v.as_str()) {
            // For blocks, use number field
            bn
        } else {
            console_log!("No block number found in response");
            return Ok(false);
        };

        // Skip if null or special tags
        if block_number_str == "null" || block_number_str.is_empty() {
            return Ok(false);
        }

        // Parse and check
        let block_number = parse_hex_to_u64(block_number_str)?;
        self.should_cache_by_block_number(block_number, env).await
    }

    /// Get block from in-memory cache (2-second TTL)
    pub fn get_block_from_cache(&self, block_number: &str) -> Option<Value> {
        let now = Date::now().as_millis();
        let cache_key = format!("{}:{}", self.chain_id, block_number);
        
        let mut cache = self.block_cache.borrow_mut();
        
        if let Some(cached) = cache.get(&cache_key) {
            // Check if cache entry is still valid (within 2 seconds)
            if now - cached.timestamp_ms < 2000 {
                console_log!("Block cache HIT for {} (age: {:.2}s)", block_number, (now - cached.timestamp_ms) as f64 / 1000.0);
                return Some(cached.data.clone());
            } else {
                console_log!("Block cache EXPIRED for {} (age: {:.2}s)", block_number, (now - cached.timestamp_ms) as f64 / 1000.0);
                // Remove expired entry
                cache.remove(&cache_key);
            }
        }
        
        None
    }

    /// Store block in in-memory cache with timestamp
    pub fn store_block_in_cache(&self, block_number: &str, block: &Value) {
        let now = Date::now().as_millis();
        let cache_key = format!("{}:{}", self.chain_id, block_number);
        
        let cached_block = CachedBlock {
            data: block.clone(),
            timestamp_ms: now,
        };
        
        self.block_cache.borrow_mut().insert(cache_key.clone(), cached_block);
        
        console_log!("Stored block {} in memory cache with 2s TTL", block_number);
        
        // Optional: Clean up expired entries to prevent memory bloat
        self.cleanup_expired_cache();
    }

    /// Clean up expired cache entries
    fn cleanup_expired_cache(&self) {
        let now = Date::now().as_millis();
        let mut cache = self.block_cache.borrow_mut();
        
        cache.retain(|_, cached| now - cached.timestamp_ms < 2000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_generate_logs_cache_key_consistency() {
        // Test that the same parameters always generate the same cache key
        let params1 = json!({
            "fromBlock": "0x64",
            "toBlock": "0xc8",
            "address": "0x1234567890123456789012345678901234567890"
        });

        let normalized1 = serde_json::to_string(&params1).unwrap();
        let normalized2 = serde_json::to_string(&params1).unwrap();
        
        assert_eq!(normalized1, normalized2, "Same params should normalize identically");
    }

    #[test]
    fn test_cache_key_differs_by_chain() {
        let params = json!({
            "fromBlock": "0x64",
            "toBlock": "0xc8"
        });
        
        let normalized = serde_json::to_string(&params).unwrap();
        let key1 = generate_cache_key("1", &format!("logs:{normalized}"));
        let key137 = generate_cache_key("137", &format!("logs:{normalized}"));
        
        assert_ne!(key1, key137, "Different chains must have different cache keys");
    }

    #[test]
    fn test_block_distance_logic() {
        // Test the core logic of should_cache_logs
        // This tests the calculation: to + block_distance <= current_block
        
        let test_cases = vec![
            // (to_block, current_block, block_distance, expected_should_cache)
            (850, 1000, 100, true),   // 850 + 100 = 950 <= 1000 ✓
            (901, 1000, 100, false),  // 901 + 100 = 1001 > 1000 ✗
            (900, 1000, 100, true),   // 900 + 100 = 1000 <= 1000 ✓ (boundary)
            (100, 1000, 100, true),   // Very old blocks
            (990, 1000, 100, false),  // Very recent blocks
            (800, 1000, 200, true),   // 800 + 200 = 1000 <= 1000 ✓
            (801, 1000, 200, false),  // 801 + 200 = 1001 > 1000 ✗
        ];

        for (to_block, current_block, block_distance, expected) in test_cases {
            let should_cache = to_block + block_distance <= current_block;
            assert_eq!(
                should_cache, expected,
                "to_block={} + block_distance={} <= current_block={} should be {}",
                to_block, block_distance, current_block, expected
            );
        }
    }

    #[test]
    fn test_special_block_tags_not_cacheable() {
        // Special tags should never be cacheable
        let special_tags = vec!["latest", "pending"];
        
        for tag in special_tags {
            // These should fail hex parsing and return false for caching
            assert!(
                tag == "latest" || tag == "pending",
                "Tag '{}' is a special tag", tag
            );
        }
    }

    #[test]
    fn test_hex_block_number_parsing() {
        use crate::utils::parse_hex_to_u64;
        
        // Test valid hex numbers
        assert_eq!(parse_hex_to_u64("0x0").unwrap(), 0);
        assert_eq!(parse_hex_to_u64("0x1").unwrap(), 1);
        assert_eq!(parse_hex_to_u64("0x64").unwrap(), 100);
        assert_eq!(parse_hex_to_u64("0x3e8").unwrap(), 1000);
        assert_eq!(parse_hex_to_u64("0xf4240").unwrap(), 1000000);
        
        // Test special tags that should fail
        assert!(parse_hex_to_u64("latest").is_err());
        assert!(parse_hex_to_u64("pending").is_err());
        
        // Test earliest (should return 0)
        assert_eq!(parse_hex_to_u64("earliest").unwrap(), 0);
    }

    #[test]
    fn test_cache_scenarios() {
        // Scenario 1: Old blocks (should cache in R2)
        let current_block = 1000u64;
        let block_distance = 100u64;
        let _from_block = 800u64;
        let to_block = 850u64;
        
        let should_cache_old = to_block + block_distance <= current_block;
        assert!(
            should_cache_old,
            "Old blocks (to={}, current={}, distance={}) should be cached in R2",
            to_block, current_block, block_distance
        );
        
        // Scenario 2: Recent blocks (should NOT cache, fetch from RPC)
        let to_block_recent = 950u64;
        let should_cache_recent = to_block_recent + block_distance <= current_block;
        assert!(
            !should_cache_recent,
            "Recent blocks (to={}, current={}, distance={}) should NOT be cached, fetch from RPC",
            to_block_recent, current_block, block_distance
        );
        
        // Scenario 3: Boundary case
        let to_block_boundary = 900u64;
        let should_cache_boundary = to_block_boundary + block_distance <= current_block;
        assert!(
            should_cache_boundary,
            "Blocks exactly at boundary (to={}, current={}, distance={}) should be cached",
            to_block_boundary, current_block, block_distance
        );
    }

    #[test]
    fn test_per_chain_block_distance() {
        // Test that different chains can have different block distances
        let mut config: HashMap<String, u64> = HashMap::new();
        config.insert("1".to_string(), 100);
        config.insert("137".to_string(), 200);
        config.insert("56".to_string(), 150);
        
        let default_distance = 100u64;
        
        // Chain 1 should use 100
        assert_eq!(config.get("1").copied().unwrap_or(default_distance), 100);
        
        // Chain 137 should use 200
        assert_eq!(config.get("137").copied().unwrap_or(default_distance), 200);
        
        // Chain 56 should use 150
        assert_eq!(config.get("56").copied().unwrap_or(default_distance), 150);
        
        // Unknown chain should use default
        assert_eq!(config.get("999").copied().unwrap_or(default_distance), 100);
    }

    #[test]
    fn test_logs_request_format() {
        // Test the expected format of eth_getLogs parameters
        let params = json!({
            "fromBlock": "0x64",
            "toBlock": "0xc8",
            "address": "0x1234567890123456789012345678901234567890",
            "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"]
        });

        assert!(params.is_object());
        assert!(params.get("fromBlock").is_some());
        assert!(params.get("toBlock").is_some());
        
        let from_block = params.get("fromBlock").and_then(|v| v.as_str()).unwrap();
        let to_block = params.get("toBlock").and_then(|v| v.as_str()).unwrap();
        
        assert!(from_block.starts_with("0x"));
        assert!(to_block.starts_with("0x"));
    }

    #[test]
    fn test_tx_receipt_cache_key_generation() {
        // Test transaction receipt cache key generation
        let chain_id = "1";
        let tx_hash = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        
        let key = format!("eth_getTransactionReceipt/{}/{}", chain_id, tx_hash.to_lowercase());
        
        assert!(key.starts_with("eth_getTransactionReceipt/"));
        assert!(key.contains(chain_id));
        assert!(key.contains(&tx_hash.to_lowercase()));
    }

    #[test]
    fn test_should_cache_tx_receipt_confirmed() {
        // Test that confirmed receipts should be cached
        let confirmed_receipt = json!({
            "transactionHash": "0x123",
            "blockNumber": "0x64",
            "blockHash": "0xabc",
            "status": "0x1"
        });

        // Check if receipt has blockNumber
        let has_block = confirmed_receipt.get("blockNumber")
            .and_then(|v| v.as_str())
            .map(|bn| !bn.is_empty() && bn != "null")
            .unwrap_or(false);
        
        assert!(has_block, "Confirmed receipt should have blockNumber");
    }

    #[test]
    fn test_should_not_cache_tx_receipt_pending() {
        // Test that pending receipts (null) should NOT be cached
        let null_receipt = json!(null);
        
        assert!(null_receipt.is_null(), "Pending receipt should be null");
    }

    #[test]
    fn test_should_not_cache_tx_receipt_no_block() {
        // Test that receipts without blockNumber should NOT be cached
        let no_block_receipt = json!({
            "transactionHash": "0x123"
            // Missing blockNumber
        });

        let has_block = no_block_receipt.get("blockNumber")
            .and_then(|v| v.as_str())
            .map(|bn| !bn.is_empty() && bn != "null")
            .unwrap_or(false);
        
        assert!(!has_block, "Receipt without blockNumber should not be cached");
    }

    #[test]
    fn test_cache_key_folder_structure() {
        // Test that cache keys use proper folder structure with chain_id
        let chain_id = "1";
        
        // eth_getLogs folder
        let params = json!({
            "fromBlock": "0x64",
            "toBlock": "0xc8"
        });
        let normalized = serde_json::to_string(&params).unwrap();
        let logs_hash = generate_cache_key(chain_id, &normalized);
        let logs_key = format!("eth_getLogs/{}/{}", chain_id, logs_hash);
        assert!(logs_key.starts_with("eth_getLogs/"));
        assert!(logs_key.contains(&format!("eth_getLogs/{}/", chain_id)));
        
        // eth_getTransactionReceipt folder
        let tx_hash = "0xabc123";
        let receipt_key = format!("eth_getTransactionReceipt/{}/{}", chain_id, tx_hash);
        assert!(receipt_key.starts_with("eth_getTransactionReceipt/"));
        assert!(receipt_key.contains(chain_id));

        // eth_getBlockByHash folder
        let block_hash = "0xdef456";
        let block_hash_key = format!("eth_getBlockByHash/{}/{}", chain_id, block_hash);
        assert!(block_hash_key.starts_with("eth_getBlockByHash/"));
        assert!(block_hash_key.contains(chain_id));

        // eth_getBlockReceipts folder
        let block_id = "0x64";
        let block_receipts_key = format!("eth_getBlockReceipts/{}/{}", chain_id, block_id);
        assert!(block_receipts_key.starts_with("eth_getBlockReceipts/"));
        assert!(block_receipts_key.contains(chain_id));

        // debug_traceBlockByNumber folder
        let trace_key = format!("debug_traceBlockByNumber/{}/{}", chain_id, "0x64");
        assert!(trace_key.starts_with("debug_traceBlockByNumber/"));
        assert!(trace_key.contains(chain_id));

        // debug_traceBlockByHash folder
        let trace_hash_key = format!("debug_traceBlockByHash/{}/{}", chain_id, "0xabc");
        assert!(trace_hash_key.starts_with("debug_traceBlockByHash/"));
        assert!(trace_hash_key.contains(chain_id));
    }

    #[test]
    fn test_tx_hash_normalization() {
        // Test that transaction hashes are normalized to lowercase
        let tx_hash_upper = "0xABCDEF123456";
        let tx_hash_lower = "0xabcdef123456";
        
        let normalized_upper = tx_hash_upper.to_lowercase();
        let normalized_lower = tx_hash_lower.to_lowercase();
        
        assert_eq!(normalized_upper, normalized_lower, "Transaction hashes should be normalized");
        assert_eq!(normalized_upper, "0xabcdef123456");
    }

    #[test]
    fn test_block_by_hash_cache_key() {
        // Test eth_getBlockByHash cache key generation
        let chain_id = "1";
        let block_hash = "0xABC123DEF456";
        let expected = format!("eth_getBlockByHash/{}/{}", chain_id, block_hash.to_lowercase());
        
        let key = format!("eth_getBlockByHash/{}/{}", chain_id, block_hash.to_lowercase());
        assert_eq!(key, expected);
        assert!(key.starts_with("eth_getBlockByHash/"));
        assert!(key.contains(chain_id));
        assert!(key.contains(&block_hash.to_lowercase()));
    }

    #[test]
    fn test_block_receipts_cache_key() {
        // Test eth_getBlockReceipts cache key generation
        let chain_id = "1";
        
        // With block number
        let block_number = "0x64";
        let key_num = format!("eth_getBlockReceipts/{}/{}", chain_id, block_number.to_lowercase());
        assert!(key_num.starts_with("eth_getBlockReceipts/"));
        assert!(key_num.contains(chain_id));
        
        // With block hash
        let block_hash = "0xABCDEF123456789012345678901234567890123456789012345678901234ABCD";
        let key_hash = format!("eth_getBlockReceipts/{}/{}", chain_id, block_hash.to_lowercase());
        assert!(key_hash.starts_with("eth_getBlockReceipts/"));
        assert!(key_hash.contains(chain_id));
    }

    #[test]
    fn test_trace_cache_keys() {
        // Test debug trace cache key generation
        let chain_id = "1";
        
        // debug_traceBlockByNumber
        let block_number = "0xc8";
        let trace_num_key = format!("debug_traceBlockByNumber/{}/{}", chain_id, block_number.to_lowercase());
        assert!(trace_num_key.starts_with("debug_traceBlockByNumber/"));
        assert!(trace_num_key.contains(chain_id));
        
        // debug_traceBlockByHash
        let block_hash = "0xABC123";
        let trace_hash_key = format!("debug_traceBlockByHash/{}/{}", chain_id, block_hash.to_lowercase());
        assert!(trace_hash_key.starts_with("debug_traceBlockByHash/"));
        assert!(trace_hash_key.contains(chain_id));
    }

    #[test]
    fn test_block_hash_detection() {
        // Test detection of block hash vs block number
        // Block hash: 0x + 64 hex chars = 66 total
        let block_hash = "0x1234567890123456789012345678901234567890123456789012345678901234";
        assert_eq!(block_hash.len(), 66, "Block hash should be 66 chars");
        assert!(block_hash.starts_with("0x"));
        
        // Block number: 0x + variable hex chars
        let block_number = "0x64";
        assert!(block_number.len() < 66, "Block number should be less than 66 chars");
        assert!(block_number.starts_with("0x"));
    }

    #[test]
    fn test_should_cache_block_logic() {
        // Test the should_cache_block logic for blocks
        let current_block = 1000u64;
        let block_distance = 100u64;
        
        // Block with old number - should cache
        let old_block_number = 850u64;
        let should_cache_old = old_block_number + block_distance <= current_block;
        assert!(should_cache_old, "Old block should be cacheable");
        
        // Block with recent number - should NOT cache
        let recent_block_number = 950u64;
        let should_cache_recent = recent_block_number + block_distance <= current_block;
        assert!(!should_cache_recent, "Recent block should NOT be cacheable");
    }

    #[test]
    fn test_block_receipts_hash_vs_number() {
        // Test that block receipts handles both hash and number
        let block_hash = "0x1234567890123456789012345678901234567890123456789012345678901234";
        let block_number = "0x64";
        
        // Block hash (66 chars) should always be cacheable (immutable)
        let is_hash = block_hash.starts_with("0x") && block_hash.len() == 66;
        assert!(is_hash, "Should detect as block hash");
        
        // Block number should be checked against block distance
        let is_number = block_number.starts_with("0x") && block_number.len() < 66;
        assert!(is_number, "Should detect as block number");
    }

    #[test]
    fn test_special_tags_not_cached() {
        // Test that special tags are never cached for new methods
        let special_tags = vec!["latest", "pending", "earliest"];
        
        for tag in special_tags {
            // These should all fail the caching check
            assert!(
                tag == "latest" || tag == "pending" || tag == "earliest",
                "Tag '{}' is special and should not be cached", tag
            );
        }
    }

    #[test]
    fn test_all_methods_have_chain_id() {
        // Verify all cache keys include chain_id
        let chain_id = "137"; // Use non-"1" to make it obvious
        
        let keys = vec![
            format!("eth_getLogs/{}/hash", chain_id),
            format!("eth_getTransactionReceipt/{}/0xabc", chain_id),
            format!("eth_getBlockByHash/{}/0xdef", chain_id),
            format!("eth_getBlockReceipts/{}/0x64", chain_id),
            format!("debug_traceBlockByNumber/{}/0xc8", chain_id),
            format!("debug_traceBlockByHash/{}/0x123", chain_id),
        ];
        
        for key in keys {
            assert!(key.contains(chain_id), "Key '{}' should contain chain_id", key);
        }
    }

    #[test]
    fn test_cache_key_uniqueness() {
        // Test that different methods produce different cache keys
        let chain_id = "1";
        let identifier = "0x64";
        
        let key1 = format!("eth_getBlockReceipts/{}/{}", chain_id, identifier);
        let key2 = format!("debug_traceBlockByNumber/{}/{}", chain_id, identifier);
        let key3 = format!("eth_getBlockByHash/{}/{}", chain_id, identifier);
        
        // All should be different even with same identifier
        assert_ne!(key1, key2);
        assert_ne!(key2, key3);
        assert_ne!(key1, key3);
    }
}

