use chrono::Utc;
use dcc_core::{CacheEntry, CacheError, CacheKey, Digest, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub root_dir: PathBuf,
    pub max_size_bytes: Option<u64>,
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

    pub fn root_dir(&self) -> &Path {
        &self.config.root_dir
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.config.root_dir.join("objects")
    }

    pub fn entries_dir(&self) -> PathBuf {
        self.config.root_dir.join("entries")
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
        let file = File::open(path)?;
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

        let file = File::open(&path)?;
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
}
