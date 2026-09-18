use crate::error::{CacheError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as Sha256Digest, Sha256};
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

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
        Self::hash_bytes(bytes)
    }

    pub fn hash_bytes(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let result = hasher.finalize();
        Self(hex::encode(result))
    }

    pub fn from_reader<R: Read>(reader: R) -> std::io::Result<Self> {
        Self::hash_reader(reader)
    }

    pub fn hash_reader<R: Read>(mut reader: R) -> std::io::Result<Self> {
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

    pub fn hash_file(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        Self::hash_reader(reader)
    }

    /// Recursively hash a directory deterministically across platforms.
    /// Traverses directory entries, sorts relative paths canonically, and hashes
    /// relative paths together with file contents.
    pub fn hash_directory(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let base_dir = path.as_ref();
        let mut entries = Vec::new();

        for entry in walkdir::WalkDir::new(base_dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                if let Ok(rel) = entry.path().strip_prefix(base_dir) {
                    let norm_rel = crate::paths::PathUtils::to_normalized_string(rel);
                    let file_digest = Self::hash_file(entry.path())?;
                    entries.push((norm_rel, file_digest));
                }
            }
        }

        // Sort canonically by relative path
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut hasher = Sha256::new();
        for (rel_path, file_digest) in entries {
            hasher.update(rel_path.as_bytes());
            hasher.update(b":");
            hasher.update(file_digest.as_str().as_bytes());
            hasher.update(b"\n");
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
    use std::io::Write;

    #[test]
    fn test_valid_digest() {
        let empty_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let d = Digest::new(empty_hash).unwrap();
        assert_eq!(d.as_str(), empty_hash);
        assert_eq!(d.prefix(2), "e3");
    }

    #[test]
    fn test_from_bytes_and_hash_bytes() {
        let d1 = Digest::from_bytes(b"hello world");
        let d2 = Digest::hash_bytes(b"hello world");
        assert_eq!(d1, d2);
        assert_eq!(
            d1.as_str(),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_hash_reader_streaming() {
        let data = b"streaming content for hashing test";
        let cursor = std::io::Cursor::new(data);
        let digest = Digest::hash_reader(cursor).unwrap();
        assert_eq!(digest, Digest::hash_bytes(data));
    }

    #[test]
    fn test_hash_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test_hash.txt");
        let payload = b"large chunk of test bytes for file hashing verification";
        {
            let mut file = File::create(&file_path).unwrap();
            file.write_all(payload).unwrap();
        }

        let file_digest = Digest::hash_file(&file_path).unwrap();
        let memory_digest = Digest::hash_bytes(payload);
        assert_eq!(file_digest, memory_digest);
    }

    #[test]
    fn test_hash_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sub = temp_dir.path().join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(temp_dir.path().join("file_a.txt"), b"aaa").unwrap();
        std::fs::write(sub.join("file_b.txt"), b"bbb").unwrap();

        let dir_digest = Digest::hash_directory(temp_dir.path()).unwrap();
        assert_eq!(dir_digest.as_str().len(), 64);

        // Modifying one file changes directory hash
        std::fs::write(sub.join("file_b.txt"), b"bbb_modified").unwrap();
        let dir_digest2 = Digest::hash_directory(temp_dir.path()).unwrap();
        assert_ne!(dir_digest, dir_digest2);
    }

    #[test]
    fn test_invalid_digest() {
        assert!(Digest::new("not_hex").is_err());
        assert!(Digest::new("abcd").is_err());
    }
}
