use dcc_core::{CacheError, CacheKey, Result};
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct ComputationLock {
    lock_file: File,
    lock_path: PathBuf,
}

impl ComputationLock {
    pub fn acquire(locks_dir: &Path, key: &CacheKey, timeout: Duration) -> Result<Self> {
        let lock_path = locks_dir.join(format!("{}.lock", key.as_str()));
        let start = Instant::now();

        fs::create_dir_all(locks_dir)?;

        loop {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)?;

            match file.try_lock_exclusive() {
                Ok(()) => {
                    return Ok(Self {
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
}

impl Drop for ComputationLock {
    fn drop(&mut self) {
        let _ = self.lock_file.unlock();
        let _ = fs::remove_file(&self.lock_path);
    }
}
