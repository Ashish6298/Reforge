use std::fs;
use std::path::PathBuf;
use dcc_core::Result;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;
use crate::cas::CasStorage;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct StorageStats {
    pub total_entries: usize,
    pub total_objects: usize,
    pub total_object_size_bytes: u64,
    pub total_entry_size_bytes: u64,
    pub total_size_bytes: u64,
    pub largest_object_size_bytes: u64,
    pub largest_object_path: Option<PathBuf>,
}

impl StorageStats {
    pub fn collect(storage: &CasStorage) -> Result<Self> {
        let mut stats = Self::default();

        let entries_dir = storage.entries_dir();
        if entries_dir.exists() {
            for entry in WalkDir::new(entries_dir).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file() && entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                    stats.total_entries += 1;
                    if let Ok(meta) = entry.metadata() {
                        stats.total_entry_size_bytes += meta.len();
                    }
                }
            }
        }

        let objects_dir = storage.objects_dir();
        if objects_dir.exists() {
            for obj in WalkDir::new(objects_dir).into_iter().filter_map(|e| e.ok()) {
                if obj.file_type().is_file() {
                    stats.total_objects += 1;
                    if let Ok(meta) = obj.metadata() {
                        let len = meta.len();
                        stats.total_object_size_bytes += len;
                        if len > stats.largest_object_size_bytes {
                            stats.largest_object_size_bytes = len;
                            stats.largest_object_path = Some(obj.path().to_path_buf());
                        }
                    }
                }
            }
        }

        stats.total_size_bytes = stats.total_object_size_bytes + stats.total_entry_size_bytes;
        Ok(stats)
    }
}
