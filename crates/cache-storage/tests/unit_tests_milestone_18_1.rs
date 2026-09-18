//! Unit Test Suite for Milestone 18.1 (Storage & Eviction)
//! Covers:
//! 5. Storage (Storage trait, put, put_file, get, exists, delete, batch ops, 256 shards)
//! 8. Eviction (LRU, FIFO, LFU, TTL expiration, reachability garbage collection)

use dcc_core::{ByteSize, CacheEntry, Computation, Digest, ExecutionMetadata, OutputManifestItem};
use dcc_storage::{CasStorage, EvictionStrategy, Pruner, Storage, StorageConfig};
use tempfile::tempdir;

// ============================================================================
// 5. STORAGE UNIT TESTS
// ============================================================================

#[test]
fn test_storage_trait_put_get_exists_delete_lifecycle() {
    let dir = tempdir().unwrap();
    let config = StorageConfig::new(dir.path()).with_max_size(ByteSize::mb(100));
    let storage = CasStorage::new(config).unwrap();

    let data = b"content-addressed storage verification payload";
    let (digest, size) = storage.put(data).expect("put bytes");
    assert_eq!(size, data.len() as u64);
    assert_eq!(digest, Digest::hash_bytes(data));

    // Exists check
    assert!(storage.exists(&digest));

    // Get bytes check
    let retrieved = storage.get_bytes(&digest).expect("get bytes");
    assert_eq!(retrieved, data);

    // Verify object integrity
    assert!(storage.verify(&digest).is_ok());

    // Delete object
    let deleted = storage.delete(&digest).expect("delete object");
    assert!(deleted);
    assert!(!storage.exists(&digest));
}

#[test]
fn test_storage_trait_batch_operations() {
    let dir = tempdir().unwrap();
    let config = StorageConfig::new(dir.path());
    let storage = CasStorage::new(config).unwrap();

    let items: Vec<&[u8]> = vec![b"batch item 1", b"batch item 2", b"batch item 3"];

    // Batch put
    let puts = storage.batch_put(&items).expect("batch put");
    assert_eq!(puts.len(), 3);

    let digests: Vec<Digest> = puts.into_iter().map(|(d, _)| d).collect();

    // Batch get
    let gets = storage.batch_get(&digests).expect("batch get");
    assert_eq!(gets.len(), 3);
    for (idx, (d, data_opt)) in gets.iter().enumerate() {
        assert_eq!(d, &digests[idx]);
        assert_eq!(data_opt.as_deref(), Some(items[idx]));
    }
}

#[test]
fn test_storage_256_shard_partitioning() {
    let dir = tempdir().unwrap();
    let config = StorageConfig::new(dir.path());
    let storage = CasStorage::new(config).unwrap();

    let payload = b"shard prefix test payload";
    let (digest, _) = storage.put(payload).unwrap();

    let prefix = digest.prefix(2);
    let expected_cas_path = dir
        .path()
        .join("objects")
        .join(prefix)
        .join(digest.as_str());

    assert!(
        expected_cas_path.exists(),
        "CAS blob must reside in 2-char hex shard directory"
    );
}

// ============================================================================
// 8. EVICTION & GARBAGE COLLECTION UNIT TESTS
// ============================================================================

#[test]
fn test_eviction_strategy_lru_order() {
    let dir = tempdir().unwrap();
    let config = StorageConfig::new(dir.path()).with_max_size(ByteSize::kb(50));
    let storage = CasStorage::new(config).unwrap();

    // Create 3 entries with distinct timestamps and outputs
    let mut entries = Vec::new();
    for i in 1..=3 {
        let comp = Computation::builder()
            .operation("op")
            .command(format!("cmd_{}", i))
            .build()
            .unwrap();
        let key = comp.compute_key().unwrap();
        let outputs = vec![OutputManifestItem {
            path: format!("out_{}.bin", i),
            digest: Digest::hash_bytes(format!("payload_{}", i).as_bytes()),
            size: 1000,
            is_executable: None,
        }];
        let exec = ExecutionMetadata {
            exit_code: 0,
            execution_time_ms: 10,
            stdout_digest: None,
            stderr_digest: None,
            timings: Default::default(),
        };
        let mut entry = CacheEntry::new(key.clone(), comp, outputs, exec);

        // Access timestamp staged sequentially
        entry.metadata.last_accessed_at =
            chrono::Utc::now() - chrono::Duration::seconds((10 - i * 2) as i64);
        storage.store_entry(&entry).unwrap();
        entries.push(entry);
    }

    let pruner = Pruner::new(&storage);
    let result = pruner
        .evict_with_strategy(EvictionStrategy::Lru, 1500)
        .unwrap();

    // At least 2 entries evicted (2000 bytes removed) to reach <= 1500
    assert!(result.deleted_entries >= 2);
    // Oldest accessed entry (entry 0) must have been deleted
    assert!(storage.get_entry(&entries[0].key).unwrap().is_none());
}

#[test]
fn test_eviction_strategy_fifo_order() {
    let dir = tempdir().unwrap();
    let config = StorageConfig::new(dir.path());
    let storage = CasStorage::new(config).unwrap();

    let mut entries = Vec::new();
    for i in 1..=3 {
        let comp = Computation::builder()
            .operation("op")
            .command(format!("cmd_{}", i))
            .build()
            .unwrap();
        let key = comp.compute_key().unwrap();
        let outputs = vec![OutputManifestItem {
            path: format!("out_{}.bin", i),
            digest: Digest::hash_bytes(format!("payload_{}", i).as_bytes()),
            size: 1000,
            is_executable: None,
        }];
        let exec = ExecutionMetadata {
            exit_code: 0,
            execution_time_ms: 10,
            stdout_digest: None,
            stderr_digest: None,
            timings: Default::default(),
        };
        let mut entry = CacheEntry::new(key.clone(), comp, outputs, exec);

        entry.metadata.created_at =
            chrono::Utc::now() - chrono::Duration::hours((10 - i * 2) as i64);
        storage.store_entry(&entry).unwrap();
        entries.push(entry);
    }

    let pruner = Pruner::new(&storage);
    let result = pruner
        .evict_with_strategy(EvictionStrategy::Fifo, 1500)
        .unwrap();

    assert!(result.deleted_entries >= 2);
    // Oldest created entry (entry 0) must have been deleted
    assert!(storage.get_entry(&entries[0].key).unwrap().is_none());
}

#[test]
fn test_eviction_strategy_lfu_order() {
    let dir = tempdir().unwrap();
    let config = StorageConfig::new(dir.path());
    let storage = CasStorage::new(config).unwrap();

    let mut entries = Vec::new();
    let hit_counts = [10u64, 2u64, 50u64];
    for (i, &hits) in hit_counts.iter().enumerate() {
        let comp = Computation::builder()
            .operation("op")
            .command(format!("cmd_{}", i))
            .build()
            .unwrap();
        let key = comp.compute_key().unwrap();
        let outputs = vec![OutputManifestItem {
            path: format!("out_{}.bin", i),
            digest: Digest::hash_bytes(format!("payload_{}", i).as_bytes()),
            size: 1000,
            is_executable: None,
        }];
        let exec = ExecutionMetadata {
            exit_code: 0,
            execution_time_ms: 10,
            stdout_digest: None,
            stderr_digest: None,
            timings: Default::default(),
        };
        let mut entry = CacheEntry::new(key.clone(), comp, outputs, exec);
        entry.metadata.hit_count = hits;
        storage.store_entry(&entry).unwrap();
        entries.push(entry);
    }

    let pruner = Pruner::new(&storage);
    let result = pruner
        .evict_with_strategy(EvictionStrategy::Lfu, 2500)
        .unwrap();

    assert!(result.deleted_entries >= 1);
    // Entry with lowest hit count (entry 1 with 2 hits) must have been deleted first
    assert!(storage.get_entry(&entries[1].key).unwrap().is_none());
    // Entry with highest hit count (entry 2 with 50 hits) must be retained
    assert!(storage.get_entry(&entries[2].key).unwrap().is_some());
}

#[test]
fn test_garbage_collection_reachability_graph() {
    let dir = tempdir().unwrap();
    let config = StorageConfig::new(dir.path());
    let storage = CasStorage::new(config).unwrap();

    // 1. Put an active referenced object
    let active_data = b"active referenced object payload";
    let (active_digest, active_size) = storage.put(active_data).unwrap();

    // 2. Put an orphaned unreferenced object
    let orphan_data = b"orphaned unreferenced object payload";
    let (orphan_digest, _) = storage.put(orphan_data).unwrap();

    // 3. Create entry referencing only the active object
    let comp = Computation::builder()
        .operation("build")
        .command("rustc")
        .build()
        .unwrap();
    let key = comp.compute_key().unwrap();
    let outputs = vec![OutputManifestItem {
        path: "target/lib.rlib".into(),
        digest: active_digest.clone(),
        size: active_size,
        is_executable: None,
    }];
    let exec = ExecutionMetadata {
        exit_code: 0,
        execution_time_ms: 15,
        stdout_digest: None,
        stderr_digest: None,
        timings: Default::default(),
    };
    let entry = CacheEntry::new(key, comp, outputs, exec);
    storage.store_entry(&entry).unwrap();

    assert!(storage.exists(&active_digest));
    assert!(storage.exists(&orphan_digest));

    // 4. Prune unreferenced objects
    let pruner = Pruner::new(&storage);
    let result = pruner.prune_unreferenced_objects().unwrap();

    assert_eq!(
        result.deleted_objects, 1,
        "Only the orphan object should be pruned"
    );
    assert!(result.freed_bytes > 0);

    // Active object is retained; orphan object is removed
    assert!(storage.exists(&active_digest));
    assert!(!storage.exists(&orphan_digest));
}
