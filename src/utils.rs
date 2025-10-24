use worker::*;

/// Parse hex string to u64
/// Supports formats: "0x123", "latest", "earliest", "pending"
pub fn parse_hex_to_u64(hex_str: &str) -> Result<u64> {
    match hex_str {
        "latest" | "pending" => Err("Cannot parse special block tags to u64".into()),
        "earliest" => Ok(0),
        _ => {
            let hex_str = hex_str.trim_start_matches("0x");
            u64::from_str_radix(hex_str, 16)
                .map_err(|e| Error::RustError(format!("Failed to parse hex: {e}")))
        }
    }
}

/// Generate a cache key from the given data
pub fn generate_cache_key(chain_id: &str, data: &str) -> String {
    use sha2::{Digest, Sha256};
    
    let mut hasher = Sha256::new();
    hasher.update(chain_id.as_bytes());
    hasher.update(b":");
    hasher.update(data.as_bytes());
    let result = hasher.finalize();
    
    format!("{}:{}", chain_id, hex::encode(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_to_u64() {
        assert_eq!(parse_hex_to_u64("0x10").unwrap(), 16);
        assert_eq!(parse_hex_to_u64("0xFF").unwrap(), 255);
        assert_eq!(parse_hex_to_u64("earliest").unwrap(), 0);
        assert!(parse_hex_to_u64("latest").is_err());
    }

    #[test]
    fn test_generate_cache_key() {
        let key1 = generate_cache_key("1", "test");
        let key2 = generate_cache_key("1", "test");
        let key3 = generate_cache_key("137", "test");
        
        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }
}

