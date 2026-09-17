use crate::cas::CasStorage;
use dcc_core::entry::CacheEntry;
use dcc_core::Result;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
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
    pub total_requests: u64,
    pub total_hits: u64,
    pub total_misses: u64,
    pub hit_ratio: f64,
    pub execution_count: u64,
    pub cache_restore_count: u64,
    pub cache_store_count: u64,
    pub bytes_restored: u64,
    pub bytes_stored: u64,
    pub estimated_time_saved_ms: u64,
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

                    // Parse entry to extract hits, computation outputs size, and execution duration
                    if let Ok(file) = File::open(entry.path()) {
                        if let Ok(cache_entry) =
                            serde_json::from_reader::<_, CacheEntry>(BufReader::new(file))
                        {
                            let entry_output_size: u64 =
                                cache_entry.outputs.iter().map(|o| o.size).sum();
                            let hits = cache_entry.metadata.hit_count;
                            let exec_duration_ms = cache_entry.metadata.execution.execution_time_ms;

                            stats.total_hits += hits;
                            // Each stored entry represents 1 initial execution (miss) and 1 store event
                            stats.total_misses += 1;
                            stats.execution_count += 1;
                            stats.cache_store_count += 1;
                            stats.cache_restore_count += hits;
                            stats.bytes_stored += entry_output_size;
                            stats.bytes_restored += entry_output_size * hits;
                            stats.estimated_time_saved_ms += exec_duration_ms * hits;
                        }
                    }
                }
            }
        }

        stats.total_requests = stats.total_hits + stats.total_misses;
        if stats.total_requests > 0 {
            stats.hit_ratio = stats.total_hits as f64 / stats.total_requests as f64;
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
        let comp = Computation::builder_with("inspect", "cmd").build().unwrap();
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
        assert_eq!(stats.total_requests, 1);
        assert_eq!(stats.total_misses, 1);
        assert_eq!(stats.total_hits, 0);
        assert_eq!(stats.hit_ratio, 0.0);
        assert_eq!(stats.execution_count, 1);
        assert_eq!(stats.cache_store_count, 1);
        assert_eq!(stats.cache_restore_count, 0);
        assert_eq!(stats.bytes_stored, small_size + large_size);
        assert_eq!(stats.bytes_restored, 0);

        // Simulate 3 hits on the entry with 150ms execution time
        let mut entry_with_hits = entry.clone();
        entry_with_hits.metadata.hit_count = 3;
        entry_with_hits.metadata.execution.execution_time_ms = 150;
        storage.store_entry(&entry_with_hits).unwrap();

        let stats_after_hits = storage.stats().unwrap();
        assert_eq!(stats_after_hits.total_requests, 4);
        assert_eq!(stats_after_hits.total_hits, 3);
        assert_eq!(stats_after_hits.total_misses, 1);
        assert!((stats_after_hits.hit_ratio - 0.75).abs() < 1e-6);
        assert_eq!(stats_after_hits.execution_count, 1);
        assert_eq!(stats_after_hits.cache_store_count, 1);
        assert_eq!(stats_after_hits.cache_restore_count, 3);
        assert_eq!(stats_after_hits.bytes_stored, small_size + large_size);
        assert_eq!(
            stats_after_hits.bytes_restored,
            (small_size + large_size) * 3
        );
        assert_eq!(stats_after_hits.estimated_time_saved_ms, 150 * 3);
    }
}
