use crate::cas::CasStorage;
use dcc_core::{CacheEntry, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvictionPolicy {
    Lru,
    MaxSizeBytes(u64),
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct EvictionResult {
    pub deleted_entries: usize,
    pub deleted_objects: usize,
    pub freed_bytes: u64,
}

pub struct Pruner<'a> {
    storage: &'a CasStorage,
}

impl<'a> Pruner<'a> {
    pub fn new(storage: &'a CasStorage) -> Self {
        Self { storage }
    }

    pub fn prune_unreferenced_objects(&self) -> Result<EvictionResult> {
        let mut referenced_digests = HashSet::new();

        // 1. Gather all digests from valid entries
        let entries_dir = self.storage.entries_dir();
        if entries_dir.exists() {
            for file_entry in WalkDir::new(entries_dir).into_iter().filter_map(|e| e.ok()) {
                if file_entry.file_type().is_file()
                    && file_entry.path().extension().and_then(|s| s.to_str()) == Some("json")
                {
                    if let Ok(file) = fs::File::open(file_entry.path()) {
                        if let Ok(entry) = serde_json::from_reader::<_, CacheEntry>(file) {
                            for out in entry.outputs {
                                referenced_digests.insert(out.digest.as_str().to_string());
                            }
                            if let Some(stdout_digest) = entry.metadata.execution.stdout_digest {
                                referenced_digests.insert(stdout_digest.as_str().to_string());
                            }
                            if let Some(stderr_digest) = entry.metadata.execution.stderr_digest {
                                referenced_digests.insert(stderr_digest.as_str().to_string());
                            }
                        }
                    }
                }
            }
        }

        // 2. Scan all objects and delete those not referenced
        let mut result = EvictionResult::default();
        let objects_dir = self.storage.objects_dir();
        if objects_dir.exists() {
            for obj in WalkDir::new(objects_dir).into_iter().filter_map(|e| e.ok()) {
                if obj.file_type().is_file() {
                    let file_name = obj.file_name().to_string_lossy().to_string();
                    if !referenced_digests.contains(&file_name) {
                        if let Ok(meta) = obj.metadata() {
                            result.freed_bytes += meta.len();
                        }
                        if fs::remove_file(obj.path()).is_ok() {
                            result.deleted_objects += 1;
                        }
                    }
                }
            }
        }

        Ok(result)
    }

    pub fn enforce_max_size(&self, max_size_bytes: u64) -> Result<EvictionResult> {
        let mut result = EvictionResult::default();
        let mut entries_with_access: Vec<(PathBuf, CacheEntry, u64)> = Vec::new();

        let entries_dir = self.storage.entries_dir();
        if entries_dir.exists() {
            for file_entry in WalkDir::new(entries_dir).into_iter().filter_map(|e| e.ok()) {
                if file_entry.file_type().is_file()
                    && file_entry.path().extension().and_then(|s| s.to_str()) == Some("json")
                {
                    if let Ok(file) = fs::File::open(file_entry.path()) {
                        if let Ok(entry) = serde_json::from_reader::<_, CacheEntry>(file) {
                            let size = entry.total_output_size();
                            entries_with_access.push((
                                file_entry.path().to_path_buf(),
                                entry,
                                size,
                            ));
                        }
                    }
                }
            }
        }

        // Sort by LRU: oldest last_accessed_at first
        entries_with_access.sort_by_key(|(_, entry, _)| entry.metadata.last_accessed_at);

        let mut current_size: u64 = entries_with_access.iter().map(|(_, _, s)| *s).sum();

        for (path, _, size) in entries_with_access {
            if current_size <= max_size_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                result.deleted_entries += 1;
                current_size = current_size.saturating_sub(size);
            }
        }

        // Clean unreferenced objects freed by entry removal
        let unreferenced = self.prune_unreferenced_objects()?;
        result.deleted_objects += unreferenced.deleted_objects;
        result.freed_bytes += unreferenced.freed_bytes;

        Ok(result)
    }
}
