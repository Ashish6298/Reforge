use chrono::Utc;
use dcc_core::{ByteSize, CacheEntry, CacheError, CacheKey, Digest, Result, SizeParseError};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub root_dir: PathBuf,
    pub max_size_bytes: Option<u64>,
}

impl StorageConfig {
    pub fn new(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            root_dir: root_dir.into(),
            max_size_bytes: Some(10 * 1024 * 1024 * 1024), // 10 GB default
        }
    }

    pub fn with_max_size(mut self, size: impl Into<ByteSize>) -> Self {
        self.max_size_bytes = Some(size.into().as_bytes());
        self
    }

    pub fn with_max_size_str(
        mut self,
        size_str: &str,
    ) -> std::result::Result<Self, SizeParseError> {
        let bs = ByteSize::parse(size_str)?;
        self.max_size_bytes = Some(bs.as_bytes());
        Ok(self)
    }

    pub fn max_size(&self) -> Option<ByteSize> {
        self.max_size_bytes.map(ByteSize::bytes)
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        let default_dir = dirs_or_fallback();
        Self {
            root_dir: default_dir.join(".dcc_cache"),
            max_size_bytes: Some(10 * 1024 * 1024 * 1024), // 10 GB default
        }
    }
}

fn dirs_or_fallback() -> PathBuf {
    if let Ok(dir) = std::env::var("DCC_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(user_dirs) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(user_dirs);
    }
    PathBuf::from(".")
}

#[derive(Debug, Clone)]
pub struct CasStorage {
    config: StorageConfig,
}

impl CasStorage {
    pub fn new(config: StorageConfig) -> Result<Self> {
        let storage = Self { config };
        storage.init_dirs()?;
        Ok(storage)
    }

    pub fn config(&self) -> &StorageConfig {
        &self.config
    }

    pub fn max_size(&self) -> Option<ByteSize> {
        self.config.max_size()
    }

    pub fn root_dir(&self) -> &Path {
        &self.config.root_dir
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.config.root_dir.join("objects")
    }

    pub fn entries_dir(&self) -> PathBuf {
        self.config.root_dir.join("entries")
    }

    pub fn metadata_dir(&self) -> PathBuf {
        self.config.root_dir.join("metadata")
    }

    pub fn index_dir(&self) -> PathBuf {
        self.config.root_dir.join("index")
    }

    pub fn tmp_dir(&self) -> PathBuf {
        self.config.root_dir.join("tmp")
    }

    pub fn locks_dir(&self) -> PathBuf {
        self.config.root_dir.join("locks")
    }

    pub fn init_dirs(&self) -> Result<()> {
        fs::create_dir_all(self.objects_dir())?;
        fs::create_dir_all(self.entries_dir())?;
        fs::create_dir_all(self.metadata_dir())?;
        fs::create_dir_all(self.index_dir())?;
        fs::create_dir_all(self.tmp_dir())?;
        fs::create_dir_all(self.locks_dir())?;
        Ok(())
    }

    pub fn object_path(&self, digest: &Digest) -> PathBuf {
        let prefix = digest.prefix(2);
        self.objects_dir().join(prefix).join(digest.as_str())
    }

    pub fn entry_path(&self, key: &CacheKey) -> PathBuf {
        let prefix = key.prefix(2);
        self.entries_dir()
            .join(prefix)
            .join(format!("{}.json", key.as_str()))
    }

    pub fn has_object(&self, digest: &Digest) -> bool {
        self.object_path(digest).is_file()
    }

    pub fn store_object_from_file(&self, source_path: &Path) -> Result<(Digest, u64)> {
        let file = File::open(source_path)?;
        let reader = BufReader::new(file);
        let digest = Digest::from_reader(reader)?;
        let metadata = fs::metadata(source_path)?;
        let size = metadata.len();

        let final_path = self.object_path(&digest);
        if final_path.exists() {
            // Deduplication: object already exists
            return Ok((digest, size));
        }

        let parent = final_path.parent().ok_or_else(|| {
            CacheError::ConfigurationError("Failed to get parent directory for CAS object".into())
        })?;
        fs::create_dir_all(parent)?;

        // Atomic write via tempfile
        let tmp_file_path = self.tmp_dir().join(format!("{}.tmp", uuid_like_nonce()));
        {
            let mut src = BufReader::new(File::open(source_path)?);
            let mut dst = BufWriter::new(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&tmp_file_path)?,
            );
            io::copy(&mut src, &mut dst)?;
            dst.flush()?;
            dst.get_ref().sync_all()?;
        }

        // Rename atomically
        if let Err(e) = fs::rename(&tmp_file_path, &final_path) {
            let _ = fs::remove_file(&tmp_file_path);
            if !final_path.exists() {
                return Err(CacheError::StorageError(e));
            }
        }

        Ok((digest, size))
    }

    pub fn store_object_bytes(&self, bytes: &[u8]) -> Result<(Digest, u64)> {
        let digest = Digest::from_bytes(bytes);
        let size = bytes.len() as u64;
        let final_path = self.object_path(&digest);
        if final_path.exists() {
            return Ok((digest, size));
        }

        let parent = final_path.parent().ok_or_else(|| {
            CacheError::ConfigurationError("Failed to get parent directory for CAS object".into())
        })?;
        fs::create_dir_all(parent)?;

        let tmp_file_path = self.tmp_dir().join(format!("{}.tmp", uuid_like_nonce()));
        {
            let mut dst = BufWriter::new(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&tmp_file_path)?,
            );
            dst.write_all(bytes)?;
            dst.flush()?;
            dst.get_ref().sync_all()?;
        }

        if let Err(e) = fs::rename(&tmp_file_path, &final_path) {
            let _ = fs::remove_file(&tmp_file_path);
            if !final_path.exists() {
                return Err(CacheError::StorageError(e));
            }
        }

        Ok((digest, size))
    }

    pub fn verify_object(&self, digest: &Digest) -> Result<()> {
        let path = self.object_path(digest);
        if !path.exists() {
            return Err(CacheError::IntegrityError {
                expected: digest.as_str().to_string(),
                actual: "<missing>".to_string(),
                path: path.display().to_string(),
            });
        }

        let file = File::open(&path)?;
        let actual = Digest::from_reader(BufReader::new(file))?;
        if &actual != digest {
            // Quarantine corrupted object
            let corrupted_path = path.with_extension("corrupted");
            let _ = fs::rename(&path, corrupted_path);
            return Err(CacheError::IntegrityError {
                expected: digest.as_str().to_string(),
                actual: actual.as_str().to_string(),
                path: path.display().to_string(),
            });
        }
        Ok(())
    }

    pub fn get_object_reader(&self, digest: &Digest) -> Result<BufReader<File>> {
        self.verify_object(digest)?;
        let path = self.object_path(digest);
        let mut attempts = 0;
        let file = loop {
            match File::open(&path) {
                Ok(f) => break f,
                Err(e) if attempts < 5 && e.kind() == io::ErrorKind::PermissionDenied => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => return Err(CacheError::StorageError(e)),
            }
        };
        Ok(BufReader::new(file))
    }

    pub fn store_entry(&self, entry: &CacheEntry) -> Result<()> {
        let path = self.entry_path(&entry.key);
        let parent = path.parent().ok_or_else(|| {
            CacheError::ConfigurationError("Failed to get parent directory for cache entry".into())
        })?;
        fs::create_dir_all(parent)?;

        let serialized = serde_json::to_string_pretty(entry)?;
        let tmp_file_path = self.tmp_dir().join(format!("{}.tmp", uuid_like_nonce()));

        {
            let mut dst = BufWriter::new(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&tmp_file_path)?,
            );
            dst.write_all(serialized.as_bytes())?;
            dst.flush()?;
            dst.get_ref().sync_all()?;
        }

        if let Err(e) = fs::rename(&tmp_file_path, &path) {
            let _ = fs::remove_file(&tmp_file_path);
            if !path.exists() {
                return Err(CacheError::StorageError(e));
            }
        }

        Ok(())
    }

    pub fn get_entry(&self, key: &CacheKey) -> Result<Option<CacheEntry>> {
        let path = self.entry_path(key);
        if !path.is_file() {
            return Ok(None);
        }

        let mut attempts = 0;
        let file = loop {
            match File::open(&path) {
                Ok(f) => break f,
                Err(e) if attempts < 5 && e.kind() == io::ErrorKind::PermissionDenied => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => {
                    if !path.exists() {
                        return Ok(None);
                    }
                    return Err(CacheError::StorageError(e));
                }
            }
        };

        let mut entry: CacheEntry = match serde_json::from_reader(BufReader::new(file)) {
            Ok(e) => e,
            Err(e) => {
                let _ = fs::remove_file(&path);
                return Err(CacheError::CorruptedEntry(path, e.to_string()));
            }
        };

        // Update last accessed time and hit count
        entry.metadata.last_accessed_at = Utc::now();
        entry.metadata.hit_count = entry.metadata.hit_count.saturating_add(1);
        let _ = self.store_entry(&entry);

        Ok(Some(entry))
    }

    pub fn delete_entry(&self, key: &CacheKey) -> Result<bool> {
        let path = self.entry_path(key);
        if path.is_file() {
            fs::remove_file(path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Inspect storage and collect comprehensive metrics.
    pub fn stats(&self) -> Result<crate::stats::StorageStats> {
        crate::stats::StorageStats::collect(self)
    }

    /// Returns the total number of physical CAS objects currently stored.
    pub fn count_objects(&self) -> Result<usize> {
        Ok(self.stats()?.total_objects)
    }

    /// Returns the total number of cached computation entries currently stored.
    pub fn count_entries(&self) -> Result<usize> {
        Ok(self.stats()?.total_entries)
    }

    /// Returns the total disk space consumed by CAS objects and entry records in bytes.
    pub fn total_size_bytes(&self) -> Result<u64> {
        Ok(self.stats()?.total_size_bytes)
    }

    /// Returns the size in bytes and optional path of the largest stored CAS object.
    pub fn largest_object(&self) -> Result<(u64, Option<PathBuf>)> {
        let stats = self.stats()?;
        Ok((stats.largest_object_size_bytes, stats.largest_object_path))
    }
}

fn uuid_like_nonce() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    format!("{}_{}_{:x}", pid, now, rand_simple())
}

fn rand_simple() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcc_core::computation::Computation;
    use dcc_core::entry::ExecutionMetadata;
    use std::io::Read;

    #[test]
    fn test_cas_store_and_verify() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().to_path_buf(),
            max_size_bytes: None,
        };
        let storage = CasStorage::new(config).unwrap();

        let data = b"deterministic computation payload";
        let (digest, size) = storage.store_object_bytes(data).unwrap();

        assert_eq!(size, data.len() as u64);
        assert!(storage.has_object(&digest));
        assert!(storage.verify_object(&digest).is_ok());

        let mut reader = storage.get_object_reader(&digest).unwrap();
        let mut read_data = Vec::new();
        reader.read_to_end(&mut read_data).unwrap();
        assert_eq!(read_data, data);
    }

    #[test]
    fn test_cas_entry_lifecycle() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().to_path_buf(),
            max_size_bytes: None,
        };
        let storage = CasStorage::new(config).unwrap();

        let comp = Computation::builder("op", "cmd").build().unwrap();
        let key = CacheKey::from_bytes(b"key-data");
        let entry = CacheEntry::new(
            key.clone(),
            comp,
            Vec::new(),
            ExecutionMetadata {
                exit_code: 0,
                execution_time_ms: 120,
                stdout_digest: None,
                stderr_digest: None,
            },
        );

        storage.store_entry(&entry).unwrap();
        let retrieved = storage.get_entry(&key).unwrap().expect("should find entry");
        assert_eq!(retrieved.key, key);
        assert_eq!(retrieved.metadata.execution.execution_time_ms, 120);

        assert!(storage.delete_entry(&key).unwrap());
        assert!(storage.get_entry(&key).unwrap().is_none());
    }

    #[test]
    fn test_storage_layout_sharding() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().to_path_buf(),
            max_size_bytes: None,
        };
        let storage = CasStorage::new(config).unwrap();

        // 1. Verify directory layout creation
        assert!(storage.objects_dir().is_dir());
        assert!(storage.entries_dir().is_dir());
        assert!(storage.metadata_dir().is_dir());
        assert!(storage.index_dir().is_dir());
        assert!(storage.tmp_dir().is_dir());
        assert!(storage.locks_dir().is_dir());

        // 2. Verify objects sharded by 2-character hex prefix
        let (digest, _) = storage.store_object_bytes(b"shard test data").unwrap();
        let expected_obj_path = storage
            .objects_dir()
            .join(digest.prefix(2))
            .join(digest.as_str());
        assert_eq!(storage.object_path(&digest), expected_obj_path);
        assert!(expected_obj_path.is_file());

        // 3. Verify entries sharded by 2-character hex prefix
        let comp = Computation::builder("op", "cmd").build().unwrap();
        let key = comp.compute_key().unwrap();
        let entry = CacheEntry::new(key.clone(), comp, Vec::new(), ExecutionMetadata::default());
        storage.store_entry(&entry).unwrap();

        let expected_entry_path = storage
            .entries_dir()
            .join(key.prefix(2))
            .join(format!("{}.json", key.as_str()));
        assert_eq!(storage.entry_path(&key), expected_entry_path);
        assert!(expected_entry_path.is_file());
    }

    #[test]
    fn test_atomic_writes_pipeline() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().to_path_buf(),
            max_size_bytes: None,
        };
        let storage = CasStorage::new(config).unwrap();

        // Storing an object should write to tmp, flush, sync, rename, and leave tmp clean
        let payload = b"atomic write verification content payload";
        let (digest, size) = storage.store_object_bytes(payload).unwrap();
        assert_eq!(size, payload.len() as u64);

        let final_obj_path = storage.object_path(&digest);
        assert!(final_obj_path.is_file());

        // Verify no leftover .tmp files in tmp_dir
        let mut tmp_entries = fs::read_dir(storage.tmp_dir()).unwrap();
        assert!(
            tmp_entries.next().is_none(),
            "tmp directory must be empty after successful atomic store"
        );

        // Deduplication test: re-storing identical object does not create temporary or duplicate files
        let (digest2, _) = storage.store_object_bytes(payload).unwrap();
        assert_eq!(digest, digest2);
        assert!(
            fs::read_dir(storage.tmp_dir()).unwrap().next().is_none(),
            "tmp directory must remain clean after deduplicated store"
        );

        // Entry atomic write test
        let comp = Computation::builder("atomic_test", "echo").build().unwrap();
        let key = comp.compute_key().unwrap();
        let entry = CacheEntry::new(key.clone(), comp, Vec::new(), ExecutionMetadata::default());
        storage.store_entry(&entry).unwrap();

        let final_entry_path = storage.entry_path(&key);
        assert!(final_entry_path.is_file());
        assert!(
            fs::read_dir(storage.tmp_dir()).unwrap().next().is_none(),
            "tmp directory must remain clean after atomic entry write"
        );
    }

    #[test]
    fn test_corruption_detection_and_quarantine() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().to_path_buf(),
            max_size_bytes: None,
        };
        let storage = CasStorage::new(config).unwrap();

        let original_data = b"integrity verified payload data";
        let (digest, _) = storage.store_object_bytes(original_data).unwrap();

        let obj_path = storage.object_path(&digest);
        assert!(obj_path.is_file());
        assert!(storage.verify_object(&digest).is_ok());

        // Simulate bitrot / corruption by modifying file content directly
        fs::write(&obj_path, b"bitrot corrupted payload data").unwrap();

        // Verification must detect the corruption
        let verify_res = storage.verify_object(&digest);
        assert!(
            verify_res.is_err(),
            "Corrupted object must fail verification"
        );

        match verify_res {
            Err(CacheError::IntegrityError {
                expected, actual, ..
            }) => {
                assert_eq!(expected, digest.as_str());
                assert_ne!(actual, digest.as_str());
            }
            other => panic!("Expected IntegrityError, got: {:?}", other),
        }

        // Must quarantine the corrupted file so it is no longer at the valid object_path
        assert!(
            !obj_path.exists(),
            "Corrupted object must be quarantined from primary path"
        );
        let quarantined_path = obj_path.with_extension("corrupted");
        assert!(
            quarantined_path.is_file(),
            "Quarantined file must exist with .corrupted extension"
        );

        // Attempting to read via get_object_reader should also fail
        assert!(storage.get_object_reader(&digest).is_err());
    }

    #[test]
    fn test_storage_config_max_size_support() {
        let temp_dir = tempfile::tempdir().unwrap();

        // 1. Default config has 10 GB max_size
        let default_cfg = StorageConfig::default();
        assert_eq!(default_cfg.max_size_bytes, Some(10 * 1024 * 1024 * 1024));
        assert_eq!(
            default_cfg.max_size().unwrap().as_bytes(),
            10 * 1024 * 1024 * 1024
        );
        assert_eq!(
            default_cfg.max_size().unwrap().to_human_readable(),
            "10.00 GB"
        );

        // 2. Custom config with 500 MB
        let cfg_500mb = StorageConfig::new(temp_dir.path())
            .with_max_size_str("500 MB")
            .unwrap();
        assert_eq!(cfg_500mb.max_size_bytes, Some(500 * 1024 * 1024));
        assert_eq!(cfg_500mb.max_size().unwrap().as_bytes(), 500 * 1024 * 1024);

        // 3. Custom config with 2 GB
        let cfg_2gb = StorageConfig::new(temp_dir.path()).with_max_size(ByteSize::gb(2));
        assert_eq!(cfg_2gb.max_size_bytes, Some(2 * 1024 * 1024 * 1024));
        assert_eq!(
            cfg_2gb.max_size().unwrap().as_bytes(),
            2 * 1024 * 1024 * 1024
        );

        // 4. CasStorage reflects configured max_size
        let storage = CasStorage::new(cfg_500mb).unwrap();
        assert_eq!(storage.max_size().unwrap().to_human_readable(), "500.00 MB");
    }
}
