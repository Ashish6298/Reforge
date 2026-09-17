use crate::cas::CasStorage;
use crate::eviction::Pruner;
use crate::lock::ComputationLock;
use crate::stats::StorageStats;
use dcc_core::{CacheEntry, CacheError, CacheKey, Digest, Result};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// High-level, library-level Cache API.
///
/// Designed to decouple cache consumers (such as CLI, build tools, runners)
/// from the internal CAS implementation details.
///
/// Provides:
/// - `lookup(key)`: Retrieve a cached entry if present.
/// - `store(entry)`: Persist computation metadata and associate with CAS blobs.
/// - `restore(entry, dest_dir)`: Atomically restore output blobs to target directory with integrity checking.
/// - `remove(key)`: Delete a cached entry by key.
/// - `contains(key)`: Check if a valid entry exists for the given key.
/// - `verify(key)`: Verify metadata and all referenced output blobs for integrity.
#[derive(Debug, Clone)]
pub struct Cache {
    storage: Arc<CasStorage>,
}

impl Cache {
    /// Open a Cache instance at the given root directory path, creating required directories if needed.
    pub fn open(cache_dir: impl AsRef<Path>) -> Result<Self> {
        let storage = CasStorage::new(crate::cas::StorageConfig {
            root_dir: cache_dir.as_ref().to_path_buf(),
            max_size_bytes: None,
        })?;
        storage.init_dirs()?;
        Ok(Self::new(storage))
    }

    /// Open a Cache instance with custom storage configuration.
    pub fn open_with_config(config: crate::cas::StorageConfig) -> Result<Self> {
        let storage = CasStorage::new(config)?;
        storage.init_dirs()?;
        Ok(Self::new(storage))
    }

    /// Create a new Cache library instance wrapping the provided CAS storage.
    pub fn new(storage: CasStorage) -> Self {
        Self {
            storage: Arc::new(storage),
        }
    }

    /// Create a new Cache library instance with an existing Arc-wrapped CAS storage.
    pub fn from_arc(storage: Arc<CasStorage>) -> Self {
        Self { storage }
    }

    /// Access the underlying CAS storage reference.
    pub fn storage(&self) -> &CasStorage {
        &self.storage
    }

    /// Lookup a cache entry by key.
    ///
    /// Updates access statistics (last_accessed_at, hit_count) if found.
    pub fn lookup(&self, key: &CacheKey) -> Result<Option<CacheEntry>> {
        self.storage.get_entry(key)
    }

    /// Store a cache entry into the storage.
    ///
    /// Ensures atomic write of entry metadata.
    pub fn store(&self, entry: &CacheEntry) -> Result<()> {
        self.storage.store_entry(entry)
    }

    /// Restore all outputs recorded in a cache entry to the destination directory.
    ///
    /// Validates cryptographic checksums before and after restoration, prevents
    /// directory traversal vulnerabilities, and writes atomically.
    pub fn restore(&self, entry: &CacheEntry, destination_dir: &Path) -> Result<()> {
        for output in &entry.outputs {
            let target_path = self.sanitize_path(destination_dir, &output.path)?;

            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }

            // Verify CAS object exists and is valid before restoration
            self.storage.verify_object(&output.digest)?;

            let cas_path = self.storage.object_path(&output.digest);
            let mut src = BufReader::new(File::open(cas_path)?);

            let parent_dir = target_path.parent().unwrap_or(destination_dir);
            let tmp_path = parent_dir.join(format!(".tmp_restore_{}", output.digest.prefix(8)));

            {
                let mut dst = BufWriter::new(
                    OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(&tmp_path)?,
                );
                io::copy(&mut src, &mut dst)?;
                dst.flush()?;
                dst.get_ref().sync_all()?;
            }

            // Validate restored output digest
            let check_file = File::open(&tmp_path)?;
            let check_digest = Digest::from_reader(BufReader::new(check_file))?;
            if check_digest != output.digest {
                let _ = fs::remove_file(&tmp_path);
                return Err(CacheError::IntegrityError {
                    expected: output.digest.as_str().to_string(),
                    actual: check_digest.as_str().to_string(),
                    path: target_path.display().to_string(),
                });
            }

            // Atomically replace target path
            fs::rename(&tmp_path, &target_path)?;

            #[cfg(unix)]
            if let Some(true) = output.is_executable {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&target_path)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&target_path, perms)?;
            }
        }

        Ok(())
    }

    /// Check if a cache entry exists for the given key without loading the entire entry.
    pub fn contains(&self, key: &CacheKey) -> bool {
        self.storage.entry_path(key).is_file()
    }

    /// Remove a cache entry by key. Returns `true` if the entry was present and removed.
    pub fn remove(&self, key: &CacheKey) -> Result<bool> {
        self.storage.delete_entry(key)
    }

    /// Verify a cache entry and all of its referenced output blobs in CAS.
    ///
    /// Fails if the entry is missing or corrupted, or if any referenced output blob
    /// is missing or has a hash mismatch.
    pub fn verify(&self, key: &CacheKey) -> Result<()> {
        let entry = self.storage.get_entry(key)?.ok_or_else(|| {
            CacheError::StorageError(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Cache entry not found for key: {}", key),
            ))
        })?;

        // 1. Verify entry identity matches its embedded computation
        entry.verify_identity()?;

        // 2. Verify each referenced output blob in CAS
        for output in &entry.outputs {
            self.storage.verify_object(&output.digest)?;
        }

        // 3. Verify stdout / stderr blobs if present
        if let Some(stdout_digest) = &entry.metadata.execution.stdout_digest {
            self.storage.verify_object(stdout_digest)?;
        }
        if let Some(stderr_digest) = &entry.metadata.execution.stderr_digest {
            self.storage.verify_object(stderr_digest)?;
        }

        Ok(())
    }

    /// Store raw object bytes into CAS storage, returning its digest and size.
    pub fn store_bytes(&self, bytes: &[u8]) -> Result<(Digest, u64)> {
        self.storage.store_object_bytes(bytes)
    }

    /// Store a file into CAS storage, returning its digest and size.
    pub fn store_file(&self, source_path: &Path) -> Result<(Digest, u64)> {
        self.storage.store_object_from_file(source_path)
    }

    /// Check if a blob with the given digest exists in CAS.
    pub fn contains_blob(&self, digest: &Digest) -> bool {
        self.storage.has_object(digest)
    }

    /// Verify a specific blob in CAS.
    pub fn verify_blob(&self, digest: &Digest) -> Result<()> {
        self.storage.verify_object(digest)
    }

    /// Safely delete a specific blob in CAS, coordinating with its ObjectLock.
    pub fn delete_blob(&self, digest: &Digest) -> Result<bool> {
        self.storage.delete_object(digest)
    }

    /// Acquire a shared read lock on a CAS object.
    pub fn lock_object(
        &self,
        digest: &Digest,
        timeout: Duration,
    ) -> Result<crate::lock::ObjectLock> {
        crate::lock::ObjectLock::acquire_shared(&self.storage.locks_dir(), digest, timeout)
    }

    /// Acquire a computation concurrency lock for a key.
    pub fn lock(&self, key: &CacheKey, timeout: Duration) -> Result<ComputationLock> {
        ComputationLock::acquire(&self.storage.locks_dir(), key, timeout)
    }

    /// Collect storage statistics.
    pub fn stats(&self) -> Result<StorageStats> {
        StorageStats::collect(&self.storage)
    }

    /// Prune unreferenced objects or enforce maximum storage size.
    pub fn pruner(&self) -> Pruner<'_> {
        Pruner::new(&self.storage)
    }

    /// Convenience method to garbage-collect all unreferenced objects.
    pub fn prune(&self) -> Result<crate::eviction::EvictionResult> {
        self.pruner().prune_unreferenced_objects()
    }

    /// Clean the entire cache by deleting all stored objects and metadata entries.
    pub fn clean_all(&self) -> Result<()> {
        self.storage.clean_all()
    }

    /// Verify integrity of all stored CAS objects and entry records.
    pub fn verify_all(&self) -> Result<crate::cas::VerifyResult> {
        self.storage.verify_all()
    }

    fn sanitize_path(&self, base_dir: &Path, rel_path: &str) -> Result<PathBuf> {
        let norm = rel_path.replace('\\', "/");
        if norm.starts_with('/') || norm.starts_with("../") || norm.contains("/../") || norm == ".."
        {
            return Err(CacheError::PathTraversal(format!(
                "Illegal path component in output path: {}",
                rel_path
            )));
        }

        let full_path = base_dir.join(rel_path);
        if !full_path.starts_with(base_dir) {
            return Err(CacheError::PathTraversal(format!(
                "Path {} escapes base directory {}",
                rel_path,
                base_dir.display()
            )));
        }

        Ok(full_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::StorageConfig;
    use dcc_core::computation::Computation;
    use dcc_core::entry::{ExecutionMetadata, OutputManifestItem};

    #[test]
    fn test_cache_api_full_lifecycle() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().join("cache"),
            max_size_bytes: None,
        };
        let storage = CasStorage::new(config).unwrap();
        let cache = Cache::new(storage);

        // 1. Prepare sample output blob and file
        let output_content = b"generated artifact 123";
        let (output_digest, output_size) = cache.store_bytes(output_content).unwrap();
        assert!(cache.contains_blob(&output_digest));
        assert!(cache.verify_blob(&output_digest).is_ok());

        // 2. Build CacheEntry
        let comp = Computation::builder("compile", "rustc")
            .arg("main.rs")
            .build()
            .unwrap();
        let key = comp.compute_key().unwrap();

        let entry = CacheEntry::new(
            key.clone(),
            comp,
            vec![OutputManifestItem {
                path: "bin/app.exe".to_string(),
                digest: output_digest.clone(),
                size: output_size,
                is_executable: Some(true),
            }],
            ExecutionMetadata {
                exit_code: 0,
                execution_time_ms: 45,
                stdout_digest: None,
                stderr_digest: None,
                timings: Default::default(),
            },
        );

        // Verify not yet present
        assert!(!cache.contains(&key));
        assert_eq!(cache.lookup(&key).unwrap(), None);

        // 3. store(entry)
        cache.store(&entry).unwrap();
        assert!(cache.contains(&key));

        // 4. lookup(key)
        let fetched = cache.lookup(&key).unwrap().expect("should find entry");
        assert_eq!(fetched.key, key);
        assert_eq!(fetched.outputs.len(), 1);
        assert_eq!(fetched.outputs[0].digest, output_digest);

        // 5. verify(key)
        assert!(cache.verify(&key).is_ok());

        // 6. restore(entry)
        let restore_dir = temp_dir.path().join("workspace");
        fs::create_dir_all(&restore_dir).unwrap();
        cache.restore(&fetched, &restore_dir).unwrap();

        let restored_file = restore_dir.join("bin").join("app.exe");
        assert!(restored_file.is_file());
        assert_eq!(fs::read(&restored_file).unwrap(), output_content);

        // 7. remove(key)
        assert!(cache.remove(&key).unwrap());
        assert!(!cache.contains(&key));
        assert_eq!(cache.lookup(&key).unwrap(), None);
        assert!(cache.verify(&key).is_err());
    }

    #[test]
    fn test_cache_verify_corrupted_blob() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().join("cache"),
            max_size_bytes: None,
        };
        let storage = CasStorage::new(config).unwrap();
        let cache = Cache::new(storage);

        let content = b"valid content";
        let (digest, size) = cache.store_bytes(content).unwrap();

        let comp = Computation::builder("test", "echo").build().unwrap();
        let key = comp.compute_key().unwrap();
        let entry = CacheEntry::new(
            key.clone(),
            comp,
            vec![OutputManifestItem {
                path: "out.txt".to_string(),
                digest: digest.clone(),
                size,
                is_executable: None,
            }],
            ExecutionMetadata::default(),
        );

        cache.store(&entry).unwrap();
        assert!(cache.verify(&key).is_ok());

        // Corrupt the CAS object file directly
        let obj_path = cache.storage().object_path(&digest);
        fs::write(&obj_path, b"corrupted payload").unwrap();

        // verify() must now detect integrity failure
        assert!(cache.verify(&key).is_err());
    }

    #[test]
    fn test_milestone_2_7_exit_criteria_without_external_commands() {
        // 1. Create a deterministic computation
        let input_bytes = b"input source code content";
        let input_digest = Digest::from_bytes(input_bytes);

        let comp = Computation::builder("compile", "rustc")
            .arg("--crate-type=lib")
            .arg("lib.rs")
            .input("src/lib.rs", input_digest.clone(), input_bytes.len() as u64)
            .env("RUST_BACKTRACE", "1")
            .build()
            .unwrap();

        // 2. Generate a deterministic key
        let key = comp.compute_key().unwrap();
        let same_key = comp.compute_key().unwrap();
        assert_eq!(key, same_key, "Key generation must be deterministic");

        // 3. Create a cache entry referencing CAS blobs
        let output_bytes = b"compiled rlib binary blob";
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().join("cache"),
            max_size_bytes: None,
        };
        let storage = CasStorage::new(config).unwrap();
        let cache = Cache::new(storage);

        let (out_digest, out_size) = cache.store_bytes(output_bytes).unwrap();
        let entry = CacheEntry::new(
            key.clone(),
            comp,
            vec![OutputManifestItem {
                path: "libfoo.rlib".to_string(),
                digest: out_digest.clone(),
                size: out_size,
                is_executable: None,
            }],
            ExecutionMetadata {
                exit_code: 0,
                execution_time_ms: 85,
                stdout_digest: None,
                stderr_digest: None,
                timings: Default::default(),
            },
        );

        cache.store(&entry).unwrap();

        // 4. Retrieve the entry
        let retrieved = cache
            .lookup(&key)
            .unwrap()
            .expect("Entry must be retrieved");
        assert_eq!(retrieved.key, key);
        assert_eq!(retrieved.outputs.len(), 1);
        assert_eq!(retrieved.outputs[0].digest, out_digest);

        // 5. Verify its identity
        assert!(retrieved.verify_identity().is_ok());
        assert!(cache.verify(&key).is_ok());

        // 6. Detect corrupted metadata
        // A) Corrupt the JSON file on disk
        let entry_file = cache.storage().entry_path(&key);
        fs::write(&entry_file, b"{ invalid json metadata payload").unwrap();
        let lookup_res = cache.lookup(&key);
        assert!(
            lookup_res.is_err(),
            "Corrupted JSON metadata must trigger an error on lookup"
        );

        // B) Detect identity tampering (modified command in entry but key kept same)
        let mut tampered_comp = retrieved.computation.clone();
        tampered_comp.command = "malicious_binary".to_string();
        let tampered_entry = CacheEntry::new(
            key.clone(), // Kept old key but changed computation
            tampered_comp,
            retrieved.outputs.clone(),
            ExecutionMetadata::default(),
        );
        assert!(
            tampered_entry.verify_identity().is_err(),
            "Tampered computation identity must be detected"
        );
    }

    #[test]
    fn test_milestone_10_1_public_library_api() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_path = temp_dir.path().join("dcc_cache");

        // 1. Cache::open(path)
        let cache = Cache::open(&cache_path).unwrap();
        assert!(cache_path.exists());

        // 2. ComputationBuilder fluent construction
        let comp = Computation::builder("test_op", "echo")
            .arg("hello world")
            .output("out.txt", true)
            .build()
            .unwrap();

        let key = comp.compute_key().unwrap();

        // 3. Cache::lookup
        assert!(cache.lookup(&key).unwrap().is_none());

        // 4. Cache::store
        let (out_digest, out_size) = cache.store_bytes(b"hello world").unwrap();
        let entry = CacheEntry::new(
            key.clone(),
            comp,
            vec![OutputManifestItem {
                path: "out.txt".into(),
                digest: out_digest,
                size: out_size,
                is_executable: None,
            }],
            ExecutionMetadata::default(),
        );
        cache.store(&entry).unwrap();

        // 5. Cache::lookup after store
        let found = cache.lookup(&key).unwrap().expect("should find entry");
        assert_eq!(found.key, key);

        // 6. Cache::verify
        assert!(cache.verify(&key).is_ok());

        // 7. Cache::remove
        assert!(cache.remove(&key).unwrap());
        assert!(cache.lookup(&key).unwrap().is_none());
    }
}
