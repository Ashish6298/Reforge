use crate::cas::CasStorage;
use dcc_core::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use walkdir::WalkDir;

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
                if entry.file_type().is_file()
                    && entry.path().extension().and_then(|s| s.to_str()) == Some("json")
                {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::StorageConfig;
    use dcc_core::computation::Computation;
    use dcc_core::entry::{CacheEntry, ExecutionMetadata, OutputManifestItem};

    #[test]
    fn test_storage_inspection_apis() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().to_path_buf(),
            max_size_bytes: None,
        };
        let storage = CasStorage::new(config).unwrap();

        // Initially empty
        assert_eq!(storage.count_objects().unwrap(), 0);
        assert_eq!(storage.count_entries().unwrap(), 0);
        assert_eq!(storage.total_size_bytes().unwrap(), 0);
        let (largest_size, largest_path) = storage.largest_object().unwrap();
        assert_eq!(largest_size, 0);
        assert_eq!(largest_path, None);

        // Store 2 objects of differing sizes
        let small_blob = b"small blob";
        let large_blob = b"this is a significantly larger blob payload for inspection testing";

        let (small_digest, small_size) = storage.store_object_bytes(small_blob).unwrap();
        let (large_digest, large_size) = storage.store_object_bytes(large_blob).unwrap();

        assert_eq!(storage.count_objects().unwrap(), 2);
        let (found_largest_size, found_largest_path) = storage.largest_object().unwrap();
        assert_eq!(found_largest_size, large_size);
        assert_eq!(found_largest_path, Some(storage.object_path(&large_digest)));

        // Store a CacheEntry
        let comp = Computation::builder("inspect", "cmd").build().unwrap();
        let key = comp.compute_key().unwrap();
        let entry = CacheEntry::new(
            key.clone(),
            comp,
            vec![
                OutputManifestItem {
                    path: "small.bin".into(),
                    digest: small_digest,
                    size: small_size,
                    is_executable: None,
                },
                OutputManifestItem {
                    path: "large.bin".into(),
                    digest: large_digest,
                    size: large_size,
                    is_executable: None,
                },
            ],
            ExecutionMetadata::default(),
        );
        storage.store_entry(&entry).unwrap();

        assert_eq!(storage.count_entries().unwrap(), 1);
        let total_size = storage.total_size_bytes().unwrap();
        assert!(total_size >= (small_size + large_size));

        let stats = storage.stats().unwrap();
        assert_eq!(stats.total_objects, 2);
        assert_eq!(stats.total_entries, 1);
        assert_eq!(stats.total_object_size_bytes, small_size + large_size);
        assert_eq!(stats.largest_object_size_bytes, large_size);
    }
}
