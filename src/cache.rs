use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use worker::*;

use crate::utils::{generate_cache_key, parse_hex_to_u64};

pub struct CacheManager {
    chain_id: String,
    r2_bucket: Option<Bucket>,
    kv_namespace: Option<kv::KvStore>,
    block_distance_config: HashMap<String, u64>,
    default_block_distance: u64,
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

        // Get KV namespace for blocks cache
        let kv_namespace = env.kv("BLOCKS_CACHE").ok();

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
            kv_namespace,
            block_distance_config,
            default_block_distance,
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
        let upstream_url = env.var("UPSTREAM_RPC_URL")?.to_string();

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
        generate_cache_key(&self.chain_id, &format!("logs:{normalized}"))
    }

    /// Get block from KV cache
    pub async fn get_block_from_cache(&self, block_number: &str) -> Result<Option<Value>> {
        let kv = match &self.kv_namespace {
            Some(kv) => kv,
            None => return Ok(None),
        };

        let cache_key = self.generate_block_cache_key(block_number);

        match kv.get(&cache_key).json().await? {
            Some(block) => Ok(Some(block)),
            None => Ok(None),
        }
    }

    /// Store block in KV cache with 2 second expiration
    pub async fn store_block_in_cache(&self, block_number: &str, block: &Value) -> Result<()> {
        let kv = match &self.kv_namespace {
            Some(kv) => kv,
            None => return Err("KV namespace not available".into()),
        };

        let cache_key = self.generate_block_cache_key(block_number);

        // Store with 2 second expiration
        kv.put(&cache_key, block)?
            .expiration_ttl(2)
            .execute()
            .await?;

        console_log!(
            "Stored block {} in KV cache with 2s TTL, key: {}",
            block_number,
            cache_key
        );

        Ok(())
    }

    /// Generate cache key for eth_getBlockByNumber
    fn generate_block_cache_key(&self, block_number: &str) -> String {
        format!("block:{}:{}", self.chain_id, block_number)
    }
}

