use crate::error::{CacheError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as Sha256Digest, Sha256};
use std::fmt;
use std::io::Read;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Digest(String);

impl Digest {
    pub fn new(hash: impl Into<String>) -> Result<Self> {
        let s = hash.into().to_lowercase();
        if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(CacheError::InvalidDigest(s));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let result = hasher.finalize();
        Self(hex::encode(result))
    }

    pub fn from_reader<R: Read>(mut reader: R) -> std::io::Result<Self> {
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024]; // 64KB chunk buffer for streaming
        loop {
            let bytes_read = reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }
        let result = hasher.finalize();
        Ok(Self(hex::encode(result)))
    }

    pub fn prefix(&self, len: usize) -> &str {
        let end = len.min(self.0.len());
        &self.0[..end]
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CacheKey(Digest);

impl CacheKey {
    pub fn new(digest: Digest) -> Self {
        Self(digest)
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(Digest::from_bytes(bytes))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn digest(&self) -> &Digest {
        &self.0
    }

    pub fn prefix(&self, len: usize) -> &str {
        self.0.prefix(len)
    }
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_digest() {
        let empty_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let d = Digest::new(empty_hash).unwrap();
        assert_eq!(d.as_str(), empty_hash);
        assert_eq!(d.prefix(2), "e3");
    }

    #[test]
    fn test_from_bytes() {
        let d = Digest::from_bytes(b"hello world");
        assert_eq!(
            d.as_str(),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_invalid_digest() {
        assert!(Digest::new("not_hex").is_err());
        assert!(Digest::new("abcd").is_err());
    }
}
