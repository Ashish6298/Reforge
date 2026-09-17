use crate::cas::CasStorage;
use chrono::{DateTime, Utc};
use dcc_core::{CacheEntry, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use walkdir::WalkDir;

/// Eviction strategies for managing cache capacity and lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EvictionStrategy {
    /// Least Recently Used: evict entries with the oldest `last_accessed_at` timestamp.
    #[default]
    Lru,
    /// First In First Out: evict entries with the oldest `created_at` timestamp.
    Fifo,
    /// Least Frequently Used: evict entries with the lowest `hit_count` (tie-broken by `last_accessed_at`).
    Lfu,
}

/// Eviction policy configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvictionPolicy {
    Lru,
    Fifo,
    Lfu,
    MaxSizeBytes(u64),
    StrategyAndLimit {
        strategy: EvictionStrategy,
        max_size_bytes: u64,
    },
}

/// Results of an eviction or pruning operation.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvictionResult {
    pub deleted_entries: usize,
    pub deleted_objects: usize,
    pub freed_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<EvictionStrategy>,
}

/// High-level garbage collection and eviction coordinator.
pub struct Pruner<'a> {
    storage: &'a CasStorage,
}

impl<'a> Pruner<'a> {
    pub fn new(storage: &'a CasStorage) -> Self {
        Self { storage }
    }

    /// Collect all digests referenced by valid cache entries (outputs, stdout, stderr).
    pub fn referenced_digests(&self) -> Result<HashSet<String>> {
        let mut referenced = HashSet::new();
        let entries_dir = self.storage.entries_dir();
        if entries_dir.exists() {
            for file_entry in WalkDir::new(entries_dir).into_iter().filter_map(|e| e.ok()) {
                if file_entry.file_type().is_file()
                    && file_entry.path().extension().and_then(|s| s.to_str()) == Some("json")
                {
                    if let Ok(file) = fs::File::open(file_entry.path()) {
                        if let Ok(entry) = serde_json::from_reader::<_, CacheEntry>(file) {
                            for out in entry.outputs {
                                referenced.insert(out.digest.as_str().to_string());
                            }
                            if let Some(stdout_digest) = entry.metadata.execution.stdout_digest {
                                referenced.insert(stdout_digest.as_str().to_string());
                            }
                            if let Some(stderr_digest) = entry.metadata.execution.stderr_digest {
                                referenced.insert(stderr_digest.as_str().to_string());
                            }
                        }
                    }
                }
            }
        }
        Ok(referenced)
    }

    /// Identify all unreferenced/orphaned CAS objects without deleting them.
    /// Returns a list of (digest_str, size_bytes, path).
    pub fn find_unreferenced_objects(&self) -> Result<Vec<(String, u64, PathBuf)>> {
        let referenced = self.referenced_digests()?;
        let mut unreferenced = Vec::new();
        let objects_dir = self.storage.objects_dir();
        if objects_dir.exists() {
            for obj in WalkDir::new(objects_dir).into_iter().filter_map(|e| e.ok()) {
                if obj.file_type().is_file() {
                    let file_name = obj.file_name().to_string_lossy().to_string();
                    if !referenced.contains(&file_name) {
                        let size = obj.metadata().map(|m| m.len()).unwrap_or(0);
                        unreferenced.push((file_name, size, obj.path().to_path_buf()));
                    }
                }
            }
        }
        Ok(unreferenced)
    }

    /// Convenience alias for pruning unreferenced objects.
    pub fn prune(&self) -> Result<EvictionResult> {
        self.prune_unreferenced_objects()
    }

    /// Prune unreferenced objects with dry-run support.
    pub fn prune_with_options(&self, dry_run: bool) -> Result<EvictionResult> {
        let unreferenced = self.find_unreferenced_objects()?;
        let mut result = EvictionResult::default();

        for (_digest, size, path) in unreferenced {
            result.freed_bytes += size;
            result.deleted_objects += 1;
            if !dry_run {
                let _ = fs::remove_file(path);
            }
        }

        Ok(result)
    }

    /// Prune CAS objects that are no longer referenced by any active cache entry.
    pub fn prune_unreferenced_objects(&self) -> Result<EvictionResult> {
        self.prune_with_options(false)
    }

    /// Enforce a maximum cache size using the default LRU strategy.
    pub fn enforce_max_size(&self, max_size_bytes: u64) -> Result<EvictionResult> {
        self.evict_with_strategy(EvictionStrategy::Lru, max_size_bytes)
    }

    /// Enforce a maximum cache size using a specified eviction strategy (LRU, FIFO, LFU).
    pub fn evict_with_strategy(
        &self,
        strategy: EvictionStrategy,
        max_size_bytes: u64,
    ) -> Result<EvictionResult> {
        let mut result = EvictionResult {
            strategy: Some(strategy),
            ..Default::default()
        };
        let mut entries_list: Vec<(PathBuf, CacheEntry, u64)> = Vec::new();

        let entries_dir = self.storage.entries_dir();
        if entries_dir.exists() {
            for file_entry in WalkDir::new(entries_dir).into_iter().filter_map(|e| e.ok()) {
                if file_entry.file_type().is_file()
                    && file_entry.path().extension().and_then(|s| s.to_str()) == Some("json")
                {
                    if let Ok(file) = fs::File::open(file_entry.path()) {
                        if let Ok(entry) = serde_json::from_reader::<_, CacheEntry>(file) {
                            let size = entry.total_output_size();
                            entries_list.push((file_entry.path().to_path_buf(), entry, size));
                        }
                    }
                }
            }
        }

        // Sort candidates based on the chosen strategy (oldest/least valued candidates first)
        match strategy {
            EvictionStrategy::Lru => {
                // Least Recently Used: oldest last_accessed_at first
                entries_list.sort_by_key(|(_, entry, _)| entry.metadata.last_accessed_at);
            }
            EvictionStrategy::Fifo => {
                // First In First Out: oldest created_at first
                entries_list.sort_by_key(|(_, entry, _)| entry.metadata.created_at);
            }
            EvictionStrategy::Lfu => {
                // Least Frequently Used: lowest hit_count first, tie-break by oldest last_accessed_at
                entries_list.sort_by(|(_, a, _), (_, b, _)| {
                    a.metadata
                        .hit_count
                        .cmp(&b.metadata.hit_count)
                        .then_with(|| {
                            a.metadata
                                .last_accessed_at
                                .cmp(&b.metadata.last_accessed_at)
                        })
                });
            }
        }

        let mut current_size: u64 = entries_list.iter().map(|(_, _, s)| *s).sum();

        for (path, _, size) in entries_list {
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

    /// Enforce a configured `EvictionPolicy`.
    pub fn enforce_policy(&self, policy: EvictionPolicy) -> Result<EvictionResult> {
        match policy {
            EvictionPolicy::Lru => {
                let max_size = self
                    .storage
                    .max_size()
                    .map(|bs| bs.as_bytes())
                    .unwrap_or(u64::MAX);
                self.evict_with_strategy(EvictionStrategy::Lru, max_size)
            }
            EvictionPolicy::Fifo => {
                let max_size = self
                    .storage
                    .max_size()
                    .map(|bs| bs.as_bytes())
                    .unwrap_or(u64::MAX);
                self.evict_with_strategy(EvictionStrategy::Fifo, max_size)
            }
            EvictionPolicy::Lfu => {
                let max_size = self
                    .storage
                    .max_size()
                    .map(|bs| bs.as_bytes())
                    .unwrap_or(u64::MAX);
                self.evict_with_strategy(EvictionStrategy::Lfu, max_size)
            }
            EvictionPolicy::MaxSizeBytes(limit) => {
                self.evict_with_strategy(EvictionStrategy::Lru, limit)
            }
            EvictionPolicy::StrategyAndLimit {
                strategy,
                max_size_bytes,
            } => self.evict_with_strategy(strategy, max_size_bytes),
        }
    }

    /// Evict entries whose `last_accessed_at` is older than `max_age`.
    pub fn evict_expired(&self, max_age: Duration) -> Result<EvictionResult> {
        let mut result = EvictionResult::default();
        let now = Utc::now();
        let chrono_age = chrono::Duration::from_std(max_age).unwrap_or(chrono::Duration::zero());
        let cutoff: DateTime<Utc> = now - chrono_age;

        let entries_dir = self.storage.entries_dir();
        if entries_dir.exists() {
            for file_entry in WalkDir::new(entries_dir).into_iter().filter_map(|e| e.ok()) {
                if file_entry.file_type().is_file()
                    && file_entry.path().extension().and_then(|s| s.to_str()) == Some("json")
                {
                    if let Ok(file) = fs::File::open(file_entry.path()) {
                        if let Ok(entry) = serde_json::from_reader::<_, CacheEntry>(file) {
                            if entry.metadata.last_accessed_at < cutoff
                                && fs::remove_file(file_entry.path()).is_ok()
                            {
                                result.deleted_entries += 1;
                            }
                        }
                    }
                }
            }
        }

        let unreferenced = self.prune_unreferenced_objects()?;
        result.deleted_objects += unreferenced.deleted_objects;
        result.freed_bytes += unreferenced.freed_bytes;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::StorageConfig;
    use dcc_core::computation::Computation;
    use dcc_core::entry::{ExecutionMetadata, OutputManifestItem};

    #[test]
    fn test_eviction_strategy_fifo() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = CasStorage::new(StorageConfig::new(temp_dir.path())).unwrap();
        let pruner = Pruner::new(&storage);

        // Entry 1: created at t=1000, last accessed at t=5000, size 100
        let comp1 = Computation::builder("op1", "cmd1").build().unwrap();
        let key1 = comp1.compute_key().unwrap();
        let (d1, s1) = storage.store_object_bytes(&[1u8; 100]).unwrap();
        let mut entry1 = CacheEntry::new(
            key1.clone(),
            comp1,
            vec![OutputManifestItem {
                path: "out1.dat".to_string(),
                digest: d1,
                size: s1,
                is_executable: Some(false),
            }],
            ExecutionMetadata::default(),
        );
        entry1.metadata.created_at = DateTime::from_timestamp(1000, 0).unwrap();
        entry1.metadata.last_accessed_at = DateTime::from_timestamp(5000, 0).unwrap();
        storage.store_entry(&entry1).unwrap();

        // Entry 2: created at t=3000, last accessed at t=2000, size 100
        let comp2 = Computation::builder("op2", "cmd2").build().unwrap();
        let key2 = comp2.compute_key().unwrap();
        let (d2, s2) = storage.store_object_bytes(&[2u8; 100]).unwrap();
        let mut entry2 = CacheEntry::new(
            key2.clone(),
            comp2,
            vec![OutputManifestItem {
                path: "out2.dat".to_string(),
                digest: d2,
                size: s2,
                is_executable: Some(false),
            }],
            ExecutionMetadata::default(),
        );
        entry2.metadata.created_at = DateTime::from_timestamp(3000, 0).unwrap();
        entry2.metadata.last_accessed_at = DateTime::from_timestamp(2000, 0).unwrap();
        storage.store_entry(&entry2).unwrap();

        // Under FIFO with limit 150 bytes: entry1 (created at 1000) is evicted first despite recent access
        let res = pruner
            .evict_with_strategy(EvictionStrategy::Fifo, 150)
            .unwrap();
        assert_eq!(res.deleted_entries, 1);
        assert_eq!(res.deleted_objects, 1);
        assert_eq!(res.strategy, Some(EvictionStrategy::Fifo));

        assert!(storage.get_entry(&key1).unwrap().is_none());
        assert!(storage.get_entry(&key2).unwrap().is_some());
    }

    #[test]
    fn test_eviction_strategy_lfu() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = CasStorage::new(StorageConfig::new(temp_dir.path())).unwrap();
        let pruner = Pruner::new(&storage);

        // Entry 1: hits = 10, size 100
        let comp1 = Computation::builder("op1", "cmd1").build().unwrap();
        let key1 = comp1.compute_key().unwrap();
        let (d1, s1) = storage.store_object_bytes(&[1u8; 100]).unwrap();
        let mut entry1 = CacheEntry::new(
            key1.clone(),
            comp1,
            vec![OutputManifestItem {
                path: "out1.dat".to_string(),
                digest: d1,
                size: s1,
                is_executable: Some(false),
            }],
            ExecutionMetadata::default(),
        );
        entry1.metadata.hit_count = 10;
        storage.store_entry(&entry1).unwrap();

        // Entry 2: hits = 1, size 100
        let comp2 = Computation::builder("op2", "cmd2").build().unwrap();
        let key2 = comp2.compute_key().unwrap();
        let (d2, s2) = storage.store_object_bytes(&[2u8; 100]).unwrap();
        let mut entry2 = CacheEntry::new(
            key2.clone(),
            comp2,
            vec![OutputManifestItem {
                path: "out2.dat".to_string(),
                digest: d2,
                size: s2,
                is_executable: Some(false),
            }],
            ExecutionMetadata::default(),
        );
        entry2.metadata.hit_count = 1;
        storage.store_entry(&entry2).unwrap();

        // Under LFU with limit 150 bytes: entry2 (hit_count = 1) is evicted, preserving entry1 (hit_count = 10)
        let res = pruner
            .evict_with_strategy(EvictionStrategy::Lfu, 150)
            .unwrap();
        assert_eq!(res.deleted_entries, 1);
        assert_eq!(res.deleted_objects, 1);
        assert_eq!(res.strategy, Some(EvictionStrategy::Lfu));

        assert!(storage.get_entry(&key1).unwrap().is_some());
        assert!(storage.get_entry(&key2).unwrap().is_none());
    }

    #[test]
    fn test_eviction_expired_ttl() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = CasStorage::new(StorageConfig::new(temp_dir.path())).unwrap();
        let pruner = Pruner::new(&storage);

        let comp = Computation::builder("op", "cmd").build().unwrap();
        let key = comp.compute_key().unwrap();
        let (d, s) = storage.store_object_bytes(&[42u8; 50]).unwrap();
        let mut entry = CacheEntry::new(
            key.clone(),
            comp,
            vec![OutputManifestItem {
                path: "out.dat".to_string(),
                digest: d,
                size: s,
                is_executable: Some(false),
            }],
            ExecutionMetadata::default(),
        );
        // Accessed 10 hours ago
        entry.metadata.last_accessed_at = Utc::now() - chrono::Duration::hours(10);
        storage.store_entry(&entry).unwrap();

        // Evict older than 1 hour
        let res = pruner.evict_expired(Duration::from_secs(3600)).unwrap();
        assert_eq!(res.deleted_entries, 1);
        assert_eq!(res.deleted_objects, 1);
        assert!(storage.get_entry(&key).unwrap().is_none());
    }

    #[test]
    fn test_garbage_collection_prune_unreferenced_and_dry_run() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = CasStorage::new(StorageConfig::new(temp_dir.path())).unwrap();
        let pruner = Pruner::new(&storage);

        // Store 2 CAS objects
        let (d1, _s1) = storage.store_object_bytes(b"referenced payload 1").unwrap();
        let (d2, s2) = storage.store_object_bytes(b"orphaned payload 2").unwrap();

        // Create an entry that only references d1
        let comp = Computation::builder("op", "cmd").build().unwrap();
        let key = comp.compute_key().unwrap();
        let entry = CacheEntry::new(
            key,
            comp,
            vec![OutputManifestItem {
                path: "out1.txt".to_string(),
                digest: d1.clone(),
                size: 20,
                is_executable: Some(false),
            }],
            ExecutionMetadata::default(),
        );
        storage.store_entry(&entry).unwrap();

        // Verify find_unreferenced_objects finds exactly d2
        let unref = pruner.find_unreferenced_objects().unwrap();
        assert_eq!(unref.len(), 1);
        assert_eq!(unref[0].0, d2.as_str());
        assert_eq!(unref[0].1, s2);

        // Dry-run prune must not delete d2
        let dry_res = pruner.prune_with_options(true).unwrap();
        assert_eq!(dry_res.deleted_objects, 1);
        assert_eq!(dry_res.freed_bytes, s2);
        assert!(storage.has_object(&d2));

        // Real prune must delete d2 and keep d1
        let prune_res = pruner.prune_unreferenced_objects().unwrap();
        assert_eq!(prune_res.deleted_objects, 1);
        assert_eq!(prune_res.freed_bytes, s2);
        assert!(!storage.has_object(&d2));
        assert!(storage.has_object(&d1));
    }

    #[test]
    fn test_garbage_collection_preserves_shared_objects_and_streams() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = CasStorage::new(StorageConfig::new(temp_dir.path())).unwrap();
        let pruner = Pruner::new(&storage);

        // Shared blob referenced by entry 1 and entry 2
        let (shared_digest, _) = storage.store_object_bytes(b"shared artifact blob").unwrap();
        let (stdout_digest, _) = storage.store_object_bytes(b"stdout capture").unwrap();

        // Entry 1
        let comp1 = Computation::builder("op1", "cmd1").build().unwrap();
        let key1 = comp1.compute_key().unwrap();
        let entry1 = CacheEntry::new(
            key1.clone(),
            comp1,
            vec![OutputManifestItem {
                path: "out.bin".to_string(),
                digest: shared_digest.clone(),
                size: 20,
                is_executable: Some(false),
            }],
            ExecutionMetadata {
                exit_code: 0,
                execution_time_ms: 10,
                stdout_digest: Some(stdout_digest.clone()),
                stderr_digest: None,
            },
        );
        storage.store_entry(&entry1).unwrap();

        // Entry 2
        let comp2 = Computation::builder("op2", "cmd2").build().unwrap();
        let key2 = comp2.compute_key().unwrap();
        let entry2 = CacheEntry::new(
            key2.clone(),
            comp2,
            vec![OutputManifestItem {
                path: "different_path.bin".to_string(),
                digest: shared_digest.clone(),
                size: 20,
                is_executable: Some(false),
            }],
            ExecutionMetadata::default(),
        );
        storage.store_entry(&entry2).unwrap();

        // Initial prune: nothing unreferenced
        let res0 = pruner.prune_unreferenced_objects().unwrap();
        assert_eq!(res0.deleted_objects, 0);

        // Delete entry 1: shared_digest is still referenced by entry 2! stdout_digest becomes orphaned
        storage.delete_entry(&key1).unwrap();
        let res1 = pruner.prune_unreferenced_objects().unwrap();
        assert_eq!(res1.deleted_objects, 1); // stdout_digest pruned
        assert!(storage.has_object(&shared_digest));
        assert!(!storage.has_object(&stdout_digest));

        // Delete entry 2: now shared_digest becomes orphaned and is pruned
        storage.delete_entry(&key2).unwrap();
        let res2 = pruner.prune_unreferenced_objects().unwrap();
        assert_eq!(res2.deleted_objects, 1); // shared_digest pruned
        assert!(!storage.has_object(&shared_digest));
    }
}
