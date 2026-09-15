use dcc_core::Result;
use dcc_storage::{CasStorage, StorageConfig};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;

pub struct TestEnv {
    pub workspace_dir: TempDir,
    pub cache_dir: TempDir,
    pub storage: CasStorage,
}

impl TestEnv {
    pub fn new() -> Result<Self> {
        let workspace_dir = tempfile::tempdir()?;
        let cache_dir = tempfile::tempdir()?;
        let storage = CasStorage::new(StorageConfig {
            root_dir: cache_dir.path().to_path_buf(),
            max_size_bytes: None,
        })?;

        Ok(Self {
            workspace_dir,
            cache_dir,
            storage,
        })
    }

    pub fn create_input_file(&self, rel_path: &str, content: &[u8]) -> Result<PathBuf> {
        let full_path = self.workspace_dir.path().join(rel_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&full_path)?;
        file.write_all(content)?;
        Ok(full_path)
    }

    pub fn read_output_file(&self, rel_path: &str) -> Result<Vec<u8>> {
        let full_path = self.workspace_dir.path().join(rel_path);
        Ok(fs::read(full_path)?)
    }
}
