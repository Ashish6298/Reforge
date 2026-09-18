#![allow(clippy::incompatible_msrv)]

use chrono::{DateTime, Utc};
use dcc_core::{CacheError, CacheKey, Digest, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockMetadata {
    pub pid: u32,
    pub created_at: DateTime<Utc>,
    pub key: String,
}

#[derive(Debug)]
pub struct ComputationLock {
    pub metadata: LockMetadata,
    lock_file: File,
    lock_path: PathBuf,
}

impl ComputationLock {
    pub fn acquire(locks_dir: &Path, key: &CacheKey, timeout: Duration) -> Result<Self> {
        let lock_path = locks_dir.join(format!("{}.lock", key.as_str()));
        let start = Instant::now();

        fs::create_dir_all(locks_dir)?;

        loop {
            let mut file = match OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
            {
                Ok(f) => f,
                Err(e) => {
                    if start.elapsed() >= timeout {
                        return Err(CacheError::LockError(format!(
                            "Failed to open lock file {}: {}",
                            lock_path.display(),
                            e
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
            };

            match file.try_lock_exclusive() {
                Ok(()) => {
                    // Lock acquired. Write/update structured lock metadata (clearing any old/corrupted data)
                    let meta = LockMetadata {
                        pid: std::process::id(),
                        created_at: Utc::now(),
                        key: key.as_str().to_string(),
                    };
                    if let Ok(serialized) = serde_json::to_vec(&meta) {
                        let _ = file.set_len(0);
                        let _ = file.seek(SeekFrom::Start(0));
                        let _ = file.write_all(&serialized);
                        let _ = file.flush();
                    }

                    return Ok(Self {
                        metadata: meta,
                        lock_file: file,
                        lock_path,
                    });
                }
                Err(_) => {
                    if start.elapsed() >= timeout {
                        return Err(CacheError::LockError(format!(
                            "Timed out acquiring lock for key {} after {:?}",
                            key.as_str(),
                            timeout
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }

    /// Access the lock metadata of the currently held lock.
    pub fn metadata(&self) -> &LockMetadata {
        &self.metadata
    }

    /// Safely read metadata from an unlocked lock file on disk. Returns `None` if absent, unreadable, or corrupted.
    pub fn read_metadata(lock_path: &Path) -> Option<LockMetadata> {
        let mut file = File::open(lock_path).ok()?;
        let mut content = Vec::new();
        file.read_to_end(&mut content).ok()?;
        serde_json::from_slice(&content).ok()
    }

    /// Scan locks directory and remove stale lock files that are not actively held by any process.
    pub fn clean_stale_locks(locks_dir: &Path, max_age: Duration) -> Result<usize> {
        if !locks_dir.is_dir() {
            return Ok(0);
        }

        let mut cleaned = 0;
        let now = Utc::now();

        for entry in fs::read_dir(locks_dir)? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("lock") {
                let is_stale = if let Some(meta) = Self::read_metadata(&path) {
                    let age = now.signed_duration_since(meta.created_at);
                    age.to_std().map(|d| d >= max_age).unwrap_or(false)
                } else if let Ok(fs_meta) = entry.metadata() {
                    if let Ok(mtime) = fs_meta.modified() {
                        mtime.elapsed().map(|d| d >= max_age).unwrap_or(false)
                    } else {
                        false
                    }
                } else {
                    false
                };

                if is_stale {
                    if let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) {
                        if file.try_lock_exclusive().is_ok() {
                            let _ = file.unlock();
                            if fs::remove_file(&path).is_ok() {
                                cleaned += 1;
                            }
                        }
                    }
                }
            }
        }

        Ok(cleaned)
    }
}

impl Drop for ComputationLock {
    fn drop(&mut self) {
        let _ = self.lock_file.unlock();
        let _ = fs::remove_file(&self.lock_path);
    }
}

/// An object lock coordinates concurrent reader access with safe deletion.
///
/// Multiple readers can acquire shared read locks on the same CAS object.
/// Eviction, pruning, and cleanup routines acquire exclusive deletion locks
/// before deleting any CAS object, ensuring no object in active use is ever deleted.
#[derive(Debug)]
pub struct ObjectLock {
    lock_file: File,
    lock_path: PathBuf,
    is_exclusive: bool,
}

impl ObjectLock {
    /// Acquire a shared read lock for a CAS object. Multiple processes/threads can hold shared locks concurrently.
    pub fn acquire_shared(locks_dir: &Path, digest: &Digest, timeout: Duration) -> Result<Self> {
        let lock_path = locks_dir.join(format!("obj_{}.lock", digest.as_str()));
        let start = Instant::now();

        fs::create_dir_all(locks_dir)?;

        loop {
            let file = match OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
            {
                Ok(f) => f,
                Err(e) => {
                    if start.elapsed() >= timeout {
                        return Err(CacheError::LockError(format!(
                            "Failed to open object lock file {}: {}",
                            lock_path.display(),
                            e
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
            };

            match file.try_lock_shared() {
                Ok(()) => {
                    return Ok(Self {
                        lock_file: file,
                        lock_path,
                        is_exclusive: false,
                    });
                }
                Err(_) => {
                    if start.elapsed() >= timeout {
                        return Err(CacheError::LockError(format!(
                            "Timed out acquiring shared lock for object {} after {:?}",
                            digest.as_str(),
                            timeout
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    /// Try to acquire an exclusive lock for safe deletion of a CAS object without blocking.
    ///
    /// If another process is currently reading or locking this object, returns `Ok(None)`.
    pub fn try_acquire_exclusive(locks_dir: &Path, digest: &Digest) -> Result<Option<Self>> {
        let lock_path = locks_dir.join(format!("obj_{}.lock", digest.as_str()));
        fs::create_dir_all(locks_dir)?;

        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
        {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };

        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self {
                lock_file: file,
                lock_path,
                is_exclusive: true,
            })),
            Err(_) => Ok(None),
        }
    }

    /// Acquire an exclusive lock for safe deletion with a timeout.
    pub fn acquire_exclusive(locks_dir: &Path, digest: &Digest, timeout: Duration) -> Result<Self> {
        let lock_path = locks_dir.join(format!("obj_{}.lock", digest.as_str()));
        let start = Instant::now();

        fs::create_dir_all(locks_dir)?;

        loop {
            let file = match OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
            {
                Ok(f) => f,
                Err(e) => {
                    if start.elapsed() >= timeout {
                        return Err(CacheError::LockError(format!(
                            "Failed to open object lock file {}: {}",
                            lock_path.display(),
                            e
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
            };

            match file.try_lock_exclusive() {
                Ok(()) => {
                    return Ok(Self {
                        lock_file: file,
                        lock_path,
                        is_exclusive: true,
                    });
                }
                Err(_) => {
                    if start.elapsed() >= timeout {
                        return Err(CacheError::LockError(format!(
                            "Timed out acquiring exclusive lock for object {} after {:?}",
                            digest.as_str(),
                            timeout
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }
}

impl Drop for ObjectLock {
    fn drop(&mut self) {
        let _ = self.lock_file.unlock();
        if self.is_exclusive {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_acquire_and_metadata() {
        let temp_dir = tempfile::tempdir().unwrap();
        let key = CacheKey::from_bytes(b"test-lock-key");

        {
            let lock =
                ComputationLock::acquire(temp_dir.path(), &key, Duration::from_secs(2)).unwrap();
            assert!(lock.lock_path.is_file());

            let meta = lock.metadata();
            assert_eq!(meta.pid, std::process::id());
            assert_eq!(meta.key, key.as_str());
        }

        // After drop, lockfile should be cleanly unlocked and removed
        assert!(!temp_dir
            .path()
            .join(format!("{}.lock", key.as_str()))
            .exists());
    }

    #[test]
    fn test_corrupted_lock_metadata_recovery() {
        let temp_dir = tempfile::tempdir().unwrap();
        let key = CacheKey::from_bytes(b"corrupted-meta-lock-key");
        let lock_path = temp_dir.path().join(format!("{}.lock", key.as_str()));

        // Simulate leftover/corrupted lock file with junk bytes
        fs::write(&lock_path, b"MALFORMED_GARBAGE_JSON_DATA_!!!###").unwrap();
        assert!(ComputationLock::read_metadata(&lock_path).is_none());

        // A new process acquiring the lock should succeed immediately and overwrite the corrupted metadata
        let lock = ComputationLock::acquire(temp_dir.path(), &key, Duration::from_secs(2)).unwrap();
        let meta = lock.metadata();
        assert_eq!(meta.pid, std::process::id());
        assert_eq!(meta.key, key.as_str());
    }

    #[test]
    fn test_stale_lock_cleanup() {
        let temp_dir = tempfile::tempdir().unwrap();
        let lock_path = temp_dir.path().join("old_dead_process.lock");

        // Write an old lock metadata file simulating a dead process from 1 hour ago
        let old_meta = LockMetadata {
            pid: 999999,
            created_at: Utc::now() - chrono::Duration::hours(2),
            key: "old-key".into(),
        };
        fs::write(&lock_path, serde_json::to_vec(&old_meta).unwrap()).unwrap();
        assert!(lock_path.is_file());

        // Clean stale locks with threshold of 1 hour
        let cleaned =
            ComputationLock::clean_stale_locks(temp_dir.path(), Duration::from_secs(3600)).unwrap();
        assert_eq!(cleaned, 1);
        assert!(!lock_path.exists());
    }

    #[test]
    fn test_lock_timeout_handling() {
        let temp_dir = tempfile::tempdir().unwrap();
        let key = CacheKey::from_bytes(b"timeout-lock-key");

        // Process 1 holds the lock
        let _lock1 =
            ComputationLock::acquire(temp_dir.path(), &key, Duration::from_secs(5)).unwrap();

        // Process 2 attempts to acquire with very short timeout (100ms)
        let res = ComputationLock::acquire(temp_dir.path(), &key, Duration::from_millis(100));
        assert!(res.is_err(), "Must return error on timeout");
        match res {
            Err(CacheError::LockError(msg)) => {
                assert!(msg.contains("Timed out acquiring lock"));
            }
            other => panic!("Expected LockError, got {:?}", other),
        }
    }

    #[test]
    fn test_object_lock_shared_and_exclusive() {
        let temp_dir = tempfile::tempdir().unwrap();
        let digest = Digest::from_bytes(b"object-safe-deletion-test");

        // 1. Multiple shared readers can acquire simultaneously
        let r1 =
            ObjectLock::acquire_shared(temp_dir.path(), &digest, Duration::from_secs(2)).unwrap();
        let r2 =
            ObjectLock::acquire_shared(temp_dir.path(), &digest, Duration::from_secs(2)).unwrap();

        // 2. Exclusive deletion attempt while readers are active fails / returns None
        let excl_try = ObjectLock::try_acquire_exclusive(temp_dir.path(), &digest).unwrap();
        assert!(
            excl_try.is_none(),
            "Cannot acquire exclusive deletion lock while active readers exist"
        );

        // 3. Drop all readers
        drop(r1);
        drop(r2);

        // 4. Exclusive deletion lock succeeds now
        let excl_lock = ObjectLock::try_acquire_exclusive(temp_dir.path(), &digest).unwrap();
        assert!(
            excl_lock.is_some(),
            "Exclusive deletion lock must succeed once readers are gone"
        );

        // 5. While exclusive lock is held, new readers cannot acquire
        let reader_attempt =
            ObjectLock::acquire_shared(temp_dir.path(), &digest, Duration::from_millis(50));
        assert!(
            reader_attempt.is_err(),
            "Reader must not acquire while exclusive deletion lock is held"
        );

        // 6. Dropping exclusive lock removes the lockfile and cleans up
        drop(excl_lock);
        assert!(!temp_dir
            .path()
            .join(format!("obj_{}.lock", digest.as_str()))
            .exists());
    }
}
