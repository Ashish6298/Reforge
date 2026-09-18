use dcc_core::{Digest, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Metadata regarding a stored blob in Content-Addressed Storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobMetadata {
    pub digest: Digest,
    pub size_bytes: u64,
    pub path: Option<PathBuf>,
}

/// Abstract storage trait defining the physical storage engine interface.
///
/// Decouples *what* is cached from *where* and *how* cached bytes are stored.
/// The primary implementation is local filesystem CAS (`LocalCasStorage` / `CasStorage`),
/// while remaining future-extensible for alternative backends (e.g., S3, memory, network cache).
pub trait Storage: Send + Sync {
    /// Put raw bytes into the storage, returning the computed digest and stored size.
    fn put(&self, bytes: &[u8]) -> Result<(Digest, u64)>;

    /// Put content from a local file into storage, returning the computed digest and stored size.
    fn put_file(&self, source_path: &Path) -> Result<(Digest, u64)>;

    /// Retrieve a readable stream for a blob by digest after verifying its integrity.
    fn get(&self, digest: &Digest) -> Result<Box<dyn Read + Send>>;

    /// Retrieve raw bytes for a blob by digest.
    fn get_bytes(&self, digest: &Digest) -> Result<Vec<u8>> {
        let mut reader = self.get(digest)?;
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Check if a blob with the specified digest exists in storage.
    fn exists(&self, digest: &Digest) -> bool;

    /// Delete a blob by its digest. Returns true if the blob was present and deleted.
    fn delete(&self, digest: &Digest) -> Result<bool>;

    /// Retrieve metadata for a blob if present.
    fn metadata(&self, digest: &Digest) -> Result<Option<BlobMetadata>>;

    /// Verify cryptographic integrity of the blob against its expected digest.
    fn verify(&self, digest: &Digest) -> Result<()>;
}

impl Storage for crate::cas::CasStorage {
    fn put(&self, bytes: &[u8]) -> Result<(Digest, u64)> {
        self.store_object_bytes(bytes)
    }

    fn put_file(&self, source_path: &Path) -> Result<(Digest, u64)> {
        self.store_object_from_file(source_path)
    }

    fn get(&self, digest: &Digest) -> Result<Box<dyn Read + Send>> {
        let reader = self.get_object_reader(digest)?;
        Ok(Box::new(reader))
    }

    fn exists(&self, digest: &Digest) -> bool {
        self.has_object(digest)
    }

    fn delete(&self, digest: &Digest) -> Result<bool> {
        self.delete_object(digest)
    }

    fn metadata(&self, digest: &Digest) -> Result<Option<BlobMetadata>> {
        let path = self.object_path(digest);
        if !path.is_file() {
            return Ok(None);
        }
        let meta = std::fs::metadata(&path)?;
        Ok(Some(BlobMetadata {
            digest: digest.clone(),
            size_bytes: meta.len(),
            path: Some(path),
        }))
    }

    fn verify(&self, digest: &Digest) -> Result<()> {
        self.verify_object(digest)
    }
}

/// Type alias for local filesystem CAS storage implementing Storage backend trait (Milestone 15.1).
pub type LocalFilesystemStorage = crate::cas::CasStorage;

/// In-memory / Mock Remote Storage implementation preparing architecture for Milestone 15 Remote Cache.
/// Separates Action/Result Cache metadata mapping from Content-Addressable Storage (CAS).
#[derive(Debug, Default, Clone)]
pub struct RemoteStorage {
    blobs: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<Digest, Vec<u8>>>>,
}

impl RemoteStorage {
    /// Create a new empty remote storage instance.
    pub fn new() -> Self {
        Self {
            blobs: std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Number of blobs stored in the remote backend.
    pub fn blob_count(&self) -> usize {
        self.blobs.read().map(|b| b.len()).unwrap_or(0)
    }

    /// Clear all blobs in the remote backend.
    pub fn clear(&self) {
        if let Ok(mut b) = self.blobs.write() {
            b.clear();
        }
    }
}

impl Storage for RemoteStorage {
    fn put(&self, bytes: &[u8]) -> Result<(Digest, u64)> {
        let digest = Digest::from_bytes(bytes);
        let size = bytes.len() as u64;
        let mut blobs = self.blobs.write().map_err(|e| {
            dcc_core::CacheError::StorageError(std::io::Error::other(format!(
                "RemoteStorage lock poisoned: {}",
                e
            )))
        })?;
        blobs.insert(digest.clone(), bytes.to_vec());
        Ok((digest, size))
    }

    fn put_file(&self, source_path: &Path) -> Result<(Digest, u64)> {
        let bytes = std::fs::read(source_path)?;
        self.put(&bytes)
    }

    fn get(&self, digest: &Digest) -> Result<Box<dyn Read + Send>> {
        let blobs = self.blobs.read().map_err(|e| {
            dcc_core::CacheError::StorageError(std::io::Error::other(format!(
                "RemoteStorage lock poisoned: {}",
                e
            )))
        })?;
        let bytes = blobs.get(digest).ok_or_else(|| {
            dcc_core::CacheError::StorageError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Remote blob not found: {}", digest),
            ))
        })?;
        Ok(Box::new(std::io::Cursor::new(bytes.clone())))
    }

    fn exists(&self, digest: &Digest) -> bool {
        self.blobs
            .read()
            .map(|b| b.contains_key(digest))
            .unwrap_or(false)
    }

    fn delete(&self, digest: &Digest) -> Result<bool> {
        let mut blobs = self.blobs.write().map_err(|e| {
            dcc_core::CacheError::StorageError(std::io::Error::other(format!(
                "RemoteStorage lock poisoned: {}",
                e
            )))
        })?;
        Ok(blobs.remove(digest).is_some())
    }

    fn metadata(&self, digest: &Digest) -> Result<Option<BlobMetadata>> {
        let blobs = self.blobs.read().map_err(|e| {
            dcc_core::CacheError::StorageError(std::io::Error::other(format!(
                "RemoteStorage lock poisoned: {}",
                e
            )))
        })?;
        Ok(blobs.get(digest).map(|b| BlobMetadata {
            digest: digest.clone(),
            size_bytes: b.len() as u64,
            path: None,
        }))
    }

    fn verify(&self, digest: &Digest) -> Result<()> {
        let blobs = self.blobs.read().map_err(|e| {
            dcc_core::CacheError::StorageError(std::io::Error::other(format!(
                "RemoteStorage lock poisoned: {}",
                e
            )))
        })?;
        let bytes = blobs.get(digest).ok_or_else(|| {
            dcc_core::CacheError::StorageError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Remote blob not found: {}", digest),
            ))
        })?;
        let actual_digest = Digest::from_bytes(bytes);
        if &actual_digest != digest {
            return Err(dcc_core::CacheError::IntegrityError {
                expected: digest.as_str().to_string(),
                actual: actual_digest.as_str().to_string(),
                path: format!("remote://{}", digest),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::{CasStorage, StorageConfig};

    #[test]
    fn test_storage_trait_operations() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().to_path_buf(),
            max_size_bytes: None,
        };
        let storage: Box<dyn Storage> = Box::new(CasStorage::new(config).unwrap());

        let test_data = b"storage trait content payload";
        let (digest, size) = storage.put(test_data).unwrap();
        assert_eq!(size, test_data.len() as u64);

        // exists
        assert!(storage.exists(&digest));

        // metadata
        let meta = storage
            .metadata(&digest)
            .unwrap()
            .expect("should find metadata");
        assert_eq!(meta.digest, digest);
        assert_eq!(meta.size_bytes, size);

        // get_bytes & get
        let retrieved = storage.get_bytes(&digest).unwrap();
        assert_eq!(retrieved, test_data);

        // verify
        assert!(storage.verify(&digest).is_ok());

        // delete
        assert!(storage.delete(&digest).unwrap());
        assert!(!storage.exists(&digest));
        assert_eq!(storage.metadata(&digest).unwrap(), None);
    }

    #[test]
    fn test_storage_put_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().join("storage"),
            max_size_bytes: None,
        };
        let storage = CasStorage::new(config).unwrap();

        let src_file = temp_dir.path().join("source.txt");
        let content = b"file-based storage put test";
        std::fs::write(&src_file, content).unwrap();

        let (digest, size) = storage.put_file(&src_file).unwrap();
        assert_eq!(size, content.len() as u64);
        assert!(storage.exists(&digest));
        assert_eq!(storage.get_bytes(&digest).unwrap(), content);
    }

    #[test]
    fn test_milestone_15_1_remote_storage_backend_trait() {
        let remote = RemoteStorage::new();
        let test_payload = b"milestone 15 remote storage blob data";

        // 1. Put raw bytes
        let (digest, size) = remote.put(test_payload).unwrap();
        assert_eq!(size, test_payload.len() as u64);
        assert_eq!(remote.blob_count(), 1);

        // 2. Exists & Metadata
        assert!(remote.exists(&digest));
        let meta = remote.metadata(&digest).unwrap().unwrap();
        assert_eq!(meta.digest, digest);
        assert_eq!(meta.size_bytes, size);
        assert!(meta.path.is_none());

        // 3. Get bytes & Verify
        let retrieved = remote.get_bytes(&digest).unwrap();
        assert_eq!(retrieved, test_payload);
        assert!(remote.verify(&digest).is_ok());

        // 4. Put file
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("remote_test.txt");
        std::fs::write(&file_path, b"file content for remote").unwrap();
        let (file_digest, file_size) = remote.put_file(&file_path).unwrap();
        assert_eq!(file_size, 23);
        assert!(remote.exists(&file_digest));
        assert_eq!(remote.blob_count(), 2);

        // 5. Delete & Verify disappearance
        assert!(remote.delete(&digest).unwrap());
        assert!(!remote.exists(&digest));
        assert!(remote.get(&digest).is_err());
        assert_eq!(remote.blob_count(), 1);

        // 6. Dynamic dispatch via Box<dyn Storage>
        let boxed: Box<dyn Storage> = Box::new(remote);
        assert!(boxed.exists(&file_digest));
        assert_eq!(
            boxed.get_bytes(&file_digest).unwrap(),
            b"file content for remote"
        );
    }
}
