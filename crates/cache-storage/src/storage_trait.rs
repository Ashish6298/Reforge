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
    /// Returns a vector of tuples containing `(Digest, Option<Vec<u8>>)`.
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

/// Identifiers for Cache Hierarchy Tiers (Milestone 15.3).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum CacheTier {
    /// L1: Ultra-fast in-memory cache
    L1Memory,
    /// L2: Local persistent filesystem CAS on disk (v1 default)
    L2LocalDisk,
    /// L3: Remote distributed network/cloud cache (optional extension)
    L3RemoteCache,
}

/// Tiered Local-First Storage Coordinator (Milestone 15.3).
///
/// Implements a multi-tier local-first cache hierarchy:
/// - **L1 Memory**: Instantaneous RAM lookup for active in-process workflows.
/// - **L2 Local Disk**: Persistent content-addressed disk storage (primary engine).
/// - **L3 Remote Cache**: Optional remote storage tier for cross-machine artifact sharing.
///
/// In v1, operation defaults exclusively to L1 Memory + L2 Local Disk without requiring
/// external network services.
pub struct TieredCache {
    l1: RemoteStorage,
    l2: LocalFilesystemStorage,
    l3: Option<std::sync::Arc<dyn Storage>>,
}

impl TieredCache {
    /// Create a new local-first tiered cache with L1 Memory and L2 Local Disk.
    pub fn new(local_disk: LocalFilesystemStorage) -> Self {
        Self {
            l1: RemoteStorage::new(),
            l2: local_disk,
            l3: None,
        }
    }

    /// Attach an optional L3 remote cache backend.
    pub fn with_remote_tier(mut self, remote: std::sync::Arc<dyn Storage>) -> Self {
        self.l3 = Some(remote);
        self
    }

    /// Access the L1 in-memory tier.
    pub fn l1(&self) -> &RemoteStorage {
        &self.l1
    }

    /// Access the L2 local filesystem tier.
    pub fn l2(&self) -> &LocalFilesystemStorage {
        &self.l2
    }

    /// Access the optional L3 remote tier.
    pub fn l3(&self) -> Option<&std::sync::Arc<dyn Storage>> {
        self.l3.as_ref()
    }

    /// Find which tier contains the blob for a given digest.
    /// Traverses hierarchy in order: L1 Memory -> L2 Local Disk -> L3 Remote Cache.
    pub fn locate_tier(&self, digest: &Digest) -> Option<CacheTier> {
        if self.l1.exists(digest) {
            return Some(CacheTier::L1Memory);
        }
        if self.l2.exists(digest) {
            return Some(CacheTier::L2LocalDisk);
        }
        if let Some(ref remote) = self.l3 {
            if remote.exists(digest) {
                return Some(CacheTier::L3RemoteCache);
            }
        }
        None
    }
}

impl Storage for TieredCache {
    fn put(&self, bytes: &[u8]) -> Result<(Digest, u64)> {
        // Store in L1 RAM and L2 Local Disk
        let (digest, size) = self.l1.put(bytes)?;
        self.l2.put(bytes)?;
        // If L3 Remote is configured, opportunistically sync
        if let Some(ref remote) = self.l3 {
            let _ = remote.put(bytes);
        }
        Ok((digest, size))
    }

    fn put_file(&self, source_path: &Path) -> Result<(Digest, u64)> {
        let (digest, size) = self.l2.put_file(source_path)?;
        if let Ok(bytes) = std::fs::read(source_path) {
            let _ = self.l1.put(&bytes);
            if let Some(ref remote) = self.l3 {
                let _ = remote.put(&bytes);
            }
        }
        Ok((digest, size))
    }

    fn get(&self, digest: &Digest) -> Result<Box<dyn Read + Send>> {
        // 1. Try L1 Memory
        if self.l1.exists(digest) {
            return self.l1.get(digest);
        }

        // 2. Try L2 Local Disk
        if self.l2.exists(digest) {
            let bytes = self.l2.get_bytes(digest)?;
            // Promote to L1 Memory
            let _ = self.l1.put(&bytes);
            return Ok(Box::new(std::io::Cursor::new(bytes)));
        }

        // 3. Try L3 Remote Cache (if attached)
        if let Some(ref remote) = self.l3 {
            if remote.exists(digest) {
                let bytes = remote.get_bytes(digest)?;
                // Populate both L2 Local Disk and L1 Memory (local-first promotion)
                let _ = self.l2.put(&bytes);
                let _ = self.l1.put(&bytes);
                return Ok(Box::new(std::io::Cursor::new(bytes)));
            }
        }

        Err(dcc_core::CacheError::StorageError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Blob not found in any cache tier: {}", digest),
        )))
    }

    fn exists(&self, digest: &Digest) -> bool {
        self.locate_tier(digest).is_some()
    }

    fn delete(&self, digest: &Digest) -> Result<bool> {
        let mut deleted = false;
        if self.l1.delete(digest)? {
            deleted = true;
        }
        if self.l2.delete(digest)? {
            deleted = true;
        }
        if let Some(ref remote) = self.l3 {
            if remote.delete(digest)? {
                deleted = true;
            }
        }
        Ok(deleted)
    }

    fn metadata(&self, digest: &Digest) -> Result<Option<BlobMetadata>> {
        if let Ok(Some(meta)) = self.l1.metadata(digest) {
            return Ok(Some(meta));
        }
        if let Ok(Some(meta)) = self.l2.metadata(digest) {
            return Ok(Some(meta));
        }
        if let Some(ref remote) = self.l3 {
            if let Ok(Some(meta)) = remote.metadata(digest) {
                return Ok(Some(meta));
            }
        }
        Ok(None)
    }

    fn verify(&self, digest: &Digest) -> Result<()> {
        if self.l1.exists(digest) {
            self.l1.verify(digest)?;
        }
        if self.l2.exists(digest) {
            self.l2.verify(digest)?;
        }
        if let Some(ref remote) = self.l3 {
            if remote.exists(digest) {
                remote.verify(digest)?;
            }
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

    #[test]
    fn test_milestone_15_3_local_first_tiered_cache() {
        let temp_dir = tempfile::tempdir().unwrap();
        let local_disk = CasStorage::new(StorageConfig {
            root_dir: temp_dir.path().to_path_buf(),
            max_size_bytes: None,
        })
        .unwrap();

        // 1. Instantiate TieredCache with L1 Memory and L2 Local Disk
        let tiered = TieredCache::new(local_disk);
        let payload = b"tiered local-first cache data payload";

        // Store into tiered cache (populates L1 RAM and L2 Disk)
        let (digest, size) = tiered.put(payload).unwrap();
        assert_eq!(size, payload.len() as u64);

        // Verify tier location
        assert_eq!(tiered.locate_tier(&digest), Some(CacheTier::L1Memory));
        assert!(tiered.exists(&digest));

        // 2. Clear L1 Memory to test L2 Local Disk retrieval and automatic L1 promotion
        tiered.l1().clear();
        assert_eq!(tiered.locate_tier(&digest), Some(CacheTier::L2LocalDisk));

        let retrieved_from_l2 = tiered.get_bytes(&digest).unwrap();
        assert_eq!(retrieved_from_l2, payload);

        // Verification after retrieval confirms promotion back to L1 Memory
        assert_eq!(tiered.locate_tier(&digest), Some(CacheTier::L1Memory));

        // 3. Attach optional L3 Remote Cache tier
        let remote_tier = std::sync::Arc::new(RemoteStorage::new());
        let remote_payload = b"payload originally in remote cloud";
        let (remote_digest, _) = remote_tier.put(remote_payload).unwrap();

        let tiered_with_l3 = tiered.with_remote_tier(remote_tier);
        assert_eq!(
            tiered_with_l3.locate_tier(&remote_digest),
            Some(CacheTier::L3RemoteCache)
        );

        // Retrieve from L3 -> promotes to both L2 Local Disk and L1 Memory
        let retrieved_remote = tiered_with_l3.get_bytes(&remote_digest).unwrap();
        assert_eq!(retrieved_remote, remote_payload);

        // Now local tiers contain it
        assert!(tiered_with_l3.l1().exists(&remote_digest));
        assert!(tiered_with_l3.l2().exists(&remote_digest));
        assert_eq!(
            tiered_with_l3.locate_tier(&remote_digest),
            Some(CacheTier::L1Memory)
        );
    }
}
