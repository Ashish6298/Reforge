use dcc_core::{Digest, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Backend storage capabilities descriptor (Milestone 15.2).
///
/// Defines fine-grained capability flags supported by a storage backend:
/// `read`, `write`, `delete`, `exists`, `stream`, `batch_get`, `batch_put`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageCapabilities {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
    pub exists: bool,
    pub stream: bool,
    pub batch_get: bool,
    pub batch_put: bool,
}

impl StorageCapabilities {
    /// Full capabilities supported by standard read-write storage backends.
    pub const fn all() -> Self {
        Self {
            read: true,
            write: true,
            delete: true,
            exists: true,
            stream: true,
            batch_get: true,
            batch_put: true,
        }
    }

    /// Read-only capabilities (read, exists, stream, batch_get).
    pub const fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            delete: false,
            exists: true,
            stream: true,
            batch_get: true,
            batch_put: false,
        }
    }

    /// Basic storage capabilities (read, write, delete, exists, stream) without batch acceleration.
    pub const fn basic() -> Self {
        Self {
            read: true,
            write: true,
            delete: true,
            exists: true,
            stream: true,
            batch_get: false,
            batch_put: false,
        }
    }
}

impl Default for StorageCapabilities {
    fn default() -> Self {
        Self::all()
    }
}

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
/// Supports both individual and batch operations with capability interrogation.
pub trait Storage: Send + Sync {
    /// Return the capability flags supported by this backend (Milestone 15.2).
    fn capabilities(&self) -> StorageCapabilities {
        StorageCapabilities::all()
    }

    /// Put raw bytes into the storage, returning the computed digest and stored size (Capability: `write`).
    fn put(&self, bytes: &[u8]) -> Result<(Digest, u64)>;

    /// Put content from a local file into storage, returning the computed digest and stored size (Capability: `write`, `stream`).
    fn put_file(&self, source_path: &Path) -> Result<(Digest, u64)>;

    /// Retrieve a readable stream for a blob by digest after verifying its integrity (Capability: `read`, `stream`).
    fn get(&self, digest: &Digest) -> Result<Box<dyn Read + Send>>;

    /// Retrieve raw bytes for a blob by digest (Capability: `read`).
    fn get_bytes(&self, digest: &Digest) -> Result<Vec<u8>> {
        let mut reader = self.get(digest)?;
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Check if a blob with the specified digest exists in storage (Capability: `exists`).
    fn exists(&self, digest: &Digest) -> bool;

    /// Delete a blob by its digest. Returns true if the blob was present and deleted (Capability: `delete`).
    fn delete(&self, digest: &Digest) -> Result<bool>;

    /// Retrieve metadata for a blob if present.
    fn metadata(&self, digest: &Digest) -> Result<Option<BlobMetadata>>;

    /// Verify cryptographic integrity of the blob against its expected digest.
    fn verify(&self, digest: &Digest) -> Result<()>;

    /// Batch retrieve multiple blobs in one operation (Milestone 15.2: `batch_get`).
    /// Returns a vector of tuples containing (Digest, Option<Vec<u8>>).
    fn batch_get(&self, digests: &[Digest]) -> Result<Vec<(Digest, Option<Vec<u8>>)>> {
        let mut results = Vec::with_capacity(digests.len());
        for digest in digests {
            let data = self.get_bytes(digest).ok();
            results.push((digest.clone(), data));
        }
        Ok(results)
    }

    /// Batch store multiple byte slices in one operation (Milestone 15.2: `batch_put`).
    /// Returns a vector of tuples containing computed (Digest, size_in_bytes).
    fn batch_put(&self, items: &[&[u8]]) -> Result<Vec<(Digest, u64)>> {
        let mut results = Vec::with_capacity(items.len());
        for item in items {
            results.push(self.put(item)?);
        }
        Ok(results)
    }
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

    #[test]
    fn test_milestone_15_2_backend_capabilities() {
        let local_storage = CasStorage::new(StorageConfig {
            root_dir: tempfile::tempdir().unwrap().path().to_path_buf(),
            max_size_bytes: None,
        })
        .unwrap();
        let remote_storage = RemoteStorage::new();

        // 1. Verify capability descriptor queries
        let local_caps = local_storage.capabilities();
        assert!(local_caps.read);
        assert!(local_caps.write);
        assert!(local_caps.delete);
        assert!(local_caps.exists);
        assert!(local_caps.stream);
        assert!(local_caps.batch_get);
        assert!(local_caps.batch_put);

        let remote_caps = remote_storage.capabilities();
        assert_eq!(remote_caps, StorageCapabilities::all());

        // 2. Test batch_put capability
        let items: Vec<&[u8]> = vec![b"batch item alpha", b"batch item beta", b"batch item gamma"];
        let put_results = remote_storage.batch_put(&items).unwrap();
        assert_eq!(put_results.len(), 3);
        let digests: Vec<Digest> = put_results.iter().map(|(d, _)| d.clone()).collect();

        // 3. Test batch_get capability
        let mut query_digests = digests.clone();
        query_digests.push(Digest::from_bytes(b"non-existent-digest"));

        let get_results = remote_storage.batch_get(&query_digests).unwrap();
        assert_eq!(get_results.len(), 4);
        assert_eq!(
            get_results[0].1.as_deref(),
            Some(b"batch item alpha".as_slice())
        );
        assert_eq!(
            get_results[1].1.as_deref(),
            Some(b"batch item beta".as_slice())
        );
        assert_eq!(
            get_results[2].1.as_deref(),
            Some(b"batch item gamma".as_slice())
        );
        assert_eq!(get_results[3].1, None); // non-existent item returns None
    }
}
