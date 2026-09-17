use dcc_core::{ByteSize, CacheEntry, CacheError, Computation, Digest, ExecutionMetadata};
use dcc_storage::{CasStorage, EvictionPolicy, EvictionStrategy, Pruner, StorageConfig};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::sync::Arc;
use std::thread;

#[test]
fn test_storage_suite_empty_cache() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    let dummy_digest = Digest::from_bytes(b"non-existent");
    assert!(!storage.has_object(&dummy_digest));
    assert!(storage.verify_object(&dummy_digest).is_err());
    assert_eq!(storage.count_objects().unwrap(), 0);
    assert_eq!(storage.count_entries().unwrap(), 0);
    assert_eq!(storage.total_size_bytes().unwrap(), 0);
}

#[test]
fn test_storage_suite_one_object() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    let payload = b"single object data payload";
    let (digest, size) = storage.store_object_bytes(payload).unwrap();

    assert_eq!(size, payload.len() as u64);
    assert!(storage.has_object(&digest));
    assert!(storage.verify_object(&digest).is_ok());
    assert_eq!(storage.count_objects().unwrap(), 1);

    let mut reader = storage.get_object_reader(&digest).unwrap();
    let mut retrieved = Vec::new();
    reader.read_to_end(&mut retrieved).unwrap();
    assert_eq!(retrieved, payload);
}

#[test]
fn test_storage_suite_duplicate_object() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    let payload = b"duplicate test data payload";
    let (digest1, size1) = storage.store_object_bytes(payload).unwrap();
    let (digest2, size2) = storage.store_object_bytes(payload).unwrap();

    assert_eq!(digest1, digest2);
    assert_eq!(size1, size2);
    assert_eq!(
        storage.count_objects().unwrap(),
        1,
        "Deduplication must keep only 1 object"
    );
}

#[test]
fn test_storage_suite_corrupted_object() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    let payload = b"valid data to be corrupted";
    let (digest, _) = storage.store_object_bytes(payload).unwrap();
    let obj_path = storage.object_path(&digest);

    // Corrupt on-disk content
    fs::write(&obj_path, b"corrupted bytes").unwrap();

    let verify_res = storage.verify_object(&digest);
    assert!(verify_res.is_err());
    assert!(matches!(
        verify_res.unwrap_err(),
        CacheError::IntegrityError { .. }
    ));

    // Must be quarantined
    assert!(!obj_path.exists());
    assert!(obj_path.with_extension("corrupted").is_file());
}

#[test]
fn test_storage_suite_interrupted_write_simulation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    // Create a dangling .tmp file in tmp directory to simulate an interrupted/crashed process
    let tmp_file = storage.tmp_dir().join("interrupted_write_9999.tmp");
    fs::write(&tmp_file, b"partial incomplete write bytes").unwrap();
    assert!(tmp_file.is_file());

    // Normal CAS operations must remain completely unaffected and not see the incomplete .tmp file
    assert_eq!(storage.count_objects().unwrap(), 0);

    let payload = b"clean new object write";
    let (digest, _) = storage.store_object_bytes(payload).unwrap();
    assert!(storage.has_object(&digest));
    assert_eq!(storage.count_objects().unwrap(), 1);
}

#[test]
fn test_storage_suite_deletion() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    let payload = b"deletable object payload";
    let (digest, _) = storage.store_object_bytes(payload).unwrap();
    assert!(storage.has_object(&digest));

    let path = storage.object_path(&digest);
    assert!(fs::remove_file(path).is_ok());
    assert!(!storage.has_object(&digest));
    assert_eq!(storage.count_objects().unwrap(), 0);
}

#[test]
fn test_storage_suite_concurrent_reads() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = Arc::new(CasStorage::new(config).unwrap());

    let payload = b"concurrent read test data";
    let (digest, _) = storage.store_object_bytes(payload).unwrap();

    let mut handles = Vec::new();
    for _ in 0..16 {
        let storage_clone = Arc::clone(&storage);
        let digest_clone = digest.clone();
        let handle = thread::spawn(move || {
            for _ in 0..10 {
                let mut reader = storage_clone.get_object_reader(&digest_clone).unwrap();
                let mut buf = Vec::new();
                reader.read_to_end(&mut buf).unwrap();
                assert_eq!(buf, payload);
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn test_storage_suite_concurrent_writes() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = Arc::new(CasStorage::new(config).unwrap());

    let payload = b"concurrent write identical data";
    let mut handles = Vec::new();

    for i in 0..16 {
        let storage_clone = Arc::clone(&storage);
        let handle = thread::spawn(move || {
            if i % 2 == 0 {
                // Racing writes of the exact same content (deduplication race)
                let (d, s) = storage_clone.store_object_bytes(payload).unwrap();
                assert_eq!(s, payload.len() as u64);
                assert!(storage_clone.verify_object(&d).is_ok());
            } else {
                // Racing writes of distinct content
                let unique_data = format!("unique concurrent data {}", i);
                let (d, s) = storage_clone
                    .store_object_bytes(unique_data.as_bytes())
                    .unwrap();
                assert_eq!(s, unique_data.len() as u64);
                assert!(storage_clone.verify_object(&d).is_ok());
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().unwrap();
    }

    // tmp dir must be clean after concurrent writes
    assert!(fs::read_dir(storage.tmp_dir()).unwrap().next().is_none());
}

#[test]
fn test_storage_suite_nested_directories() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    let nested_dir = temp_dir
        .path()
        .join("deeply")
        .join("nested")
        .join("sub")
        .join("path");
    fs::create_dir_all(&nested_dir).unwrap();
    let nested_file = nested_dir.join("artifact.bin");
    let payload = b"nested directory content";
    fs::write(&nested_file, payload).unwrap();

    let (digest, size) = storage.store_object_from_file(&nested_file).unwrap();
    assert_eq!(size, payload.len() as u64);
    assert!(storage.has_object(&digest));
    assert!(storage.verify_object(&digest).is_ok());
}

#[test]
fn test_storage_suite_very_large_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    // Create 8 MB synthetic chunked test file
    let chunk_size = 64 * 1024; // 64 KB
    let total_chunks = 128; // 128 * 64KB = 8 MB
    let large_file_path = temp_dir.path().join("large_file.dat");
    {
        let mut f = File::create(&large_file_path).unwrap();
        let chunk = vec![0xABu8; chunk_size];
        for _ in 0..total_chunks {
            f.write_all(&chunk).unwrap();
        }
        f.flush().unwrap();
    }

    let (digest, size) = storage.store_object_from_file(&large_file_path).unwrap();
    assert_eq!(size, (chunk_size * total_chunks) as u64);
    assert!(storage.has_object(&digest));
    assert!(storage.verify_object(&digest).is_ok());

    let (largest_size, largest_path) = storage.largest_object().unwrap();
    assert_eq!(largest_size, size);
    assert_eq!(largest_path, Some(storage.object_path(&digest)));
}

#[test]
fn test_storage_suite_binary_files() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    // Byte pattern covering all 0..=255 values, null bytes, unicode invalid sequences
    let mut binary_data = Vec::with_capacity(1024);
    for i in 0..1024 {
        binary_data.push((i % 256) as u8);
    }

    let (digest, size) = storage.store_object_bytes(&binary_data).unwrap();
    assert_eq!(size, binary_data.len() as u64);
    assert!(storage.has_object(&digest));
    assert!(storage.verify_object(&digest).is_ok());

    let mut reader = storage.get_object_reader(&digest).unwrap();
    let mut read_back = Vec::new();
    reader.read_to_end(&mut read_back).unwrap();
    assert_eq!(read_back, binary_data);
}

#[test]
fn test_storage_suite_max_size_limits_and_parsing() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Verify 500 MB, 2 GB, 10 GB configurations
    let cfg1 = StorageConfig::new(temp_dir.path())
        .with_max_size_str("500 MB")
        .unwrap();
    assert_eq!(cfg1.max_size_bytes, Some(500 * 1024 * 1024));
    assert_eq!(cfg1.max_size().unwrap(), ByteSize::mb(500));

    let cfg2 = StorageConfig::new(temp_dir.path())
        .with_max_size_str("2 GB")
        .unwrap();
    assert_eq!(cfg2.max_size_bytes, Some(2 * 1024 * 1024 * 1024));
    assert_eq!(cfg2.max_size().unwrap(), ByteSize::gb(2));

    let cfg3 = StorageConfig::new(temp_dir.path())
        .with_max_size_str("10 GB")
        .unwrap();
    assert_eq!(cfg3.max_size_bytes, Some(10 * 1024 * 1024 * 1024));
    assert_eq!(cfg3.max_size().unwrap(), ByteSize::gb(10));

    let storage = CasStorage::new(cfg1).unwrap();
    assert_eq!(storage.max_size().unwrap().to_human_readable(), "500.00 MB");
}

#[test]
fn test_storage_suite_max_size_enforcement_lru() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = CasStorage::new(StorageConfig::new(temp_dir.path())).unwrap();
    let pruner = Pruner::new(&storage);

    // Create 3 entries with distinct outputs
    // Entry 1: 100 bytes (oldest)
    let comp1 = Computation::builder("op1", "cmd1").build().unwrap();
    let key1 = comp1.compute_key().unwrap();
    let (d1, s1) = storage.store_object_bytes(&[1u8; 100]).unwrap();
    let out1 = dcc_core::OutputManifestItem {
        path: "out1.dat".to_string(),
        digest: d1,
        size: s1,
        is_executable: Some(false),
    };
    let mut entry1 = CacheEntry::new(
        key1.clone(),
        comp1,
        vec![out1],
        ExecutionMetadata::default(),
    );
    entry1.metadata.last_accessed_at = chrono::DateTime::from_timestamp(1000, 0).unwrap();
    storage.store_entry(&entry1).unwrap();

    // Entry 2: 100 bytes (middle)
    let comp2 = Computation::builder("op2", "cmd2").build().unwrap();
    let key2 = comp2.compute_key().unwrap();
    let (d2, s2) = storage.store_object_bytes(&[2u8; 100]).unwrap();
    let out2 = dcc_core::OutputManifestItem {
        path: "out2.dat".to_string(),
        digest: d2,
        size: s2,
        is_executable: Some(false),
    };
    let mut entry2 = CacheEntry::new(
        key2.clone(),
        comp2,
        vec![out2],
        ExecutionMetadata::default(),
    );
    entry2.metadata.last_accessed_at = chrono::DateTime::from_timestamp(2000, 0).unwrap();
    storage.store_entry(&entry2).unwrap();

    // Entry 3: 100 bytes (newest)
    let comp3 = Computation::builder("op3", "cmd3").build().unwrap();
    let key3 = comp3.compute_key().unwrap();
    let (d3, s3) = storage.store_object_bytes(&[3u8; 100]).unwrap();
    let out3 = dcc_core::OutputManifestItem {
        path: "out3.dat".to_string(),
        digest: d3,
        size: s3,
        is_executable: Some(false),
    };
    let mut entry3 = CacheEntry::new(
        key3.clone(),
        comp3,
        vec![out3],
        ExecutionMetadata::default(),
    );
    entry3.metadata.last_accessed_at = chrono::DateTime::from_timestamp(3000, 0).unwrap();
    storage.store_entry(&entry3).unwrap();

    // Total size of cache outputs is 300 bytes. Enforce max size of 150 bytes.
    let prune_res = pruner.enforce_max_size(150).unwrap();
    // It should evict entry1 (100b) and entry2 (100b) leaving entry3 (100b <= 150b)
    assert_eq!(prune_res.deleted_entries, 2);
    assert_eq!(prune_res.deleted_objects, 2);
    assert_eq!(prune_res.freed_bytes, 200);

    assert!(storage.get_entry(&key1).unwrap().is_none());
    assert!(storage.get_entry(&key2).unwrap().is_none());
    assert!(storage.get_entry(&key3).unwrap().is_some());
}

#[test]
fn test_storage_suite_eviction_strategy_fifo_vs_lru() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = CasStorage::new(StorageConfig::new(temp_dir.path())).unwrap();
    let pruner = Pruner::new(&storage);

    // Entry A: created at t=100, accessed at t=900 (recent access), size 100
    let comp_a = Computation::builder("opA", "cmdA").build().unwrap();
    let key_a = comp_a.compute_key().unwrap();
    let (da, sa) = storage.store_object_bytes(&[10u8; 100]).unwrap();
    let mut entry_a = CacheEntry::new(
        key_a.clone(),
        comp_a,
        vec![dcc_core::OutputManifestItem {
            path: "outA.dat".to_string(),
            digest: da,
            size: sa,
            is_executable: Some(false),
        }],
        ExecutionMetadata::default(),
    );
    entry_a.metadata.created_at = chrono::DateTime::from_timestamp(100, 0).unwrap();
    entry_a.metadata.last_accessed_at = chrono::DateTime::from_timestamp(900, 0).unwrap();
    storage.store_entry(&entry_a).unwrap();

    // Entry B: created at t=500, accessed at t=600 (older access), size 100
    let comp_b = Computation::builder("opB", "cmdB").build().unwrap();
    let key_b = comp_b.compute_key().unwrap();
    let (db, sb) = storage.store_object_bytes(&[20u8; 100]).unwrap();
    let mut entry_b = CacheEntry::new(
        key_b.clone(),
        comp_b,
        vec![dcc_core::OutputManifestItem {
            path: "outB.dat".to_string(),
            digest: db,
            size: sb,
            is_executable: Some(false),
        }],
        ExecutionMetadata::default(),
    );
    entry_b.metadata.created_at = chrono::DateTime::from_timestamp(500, 0).unwrap();
    entry_b.metadata.last_accessed_at = chrono::DateTime::from_timestamp(600, 0).unwrap();
    storage.store_entry(&entry_b).unwrap();

    // FIFO eviction with limit 150 bytes: Entry A is evicted (created first at t=100)
    let prune_fifo = pruner
        .evict_with_strategy(EvictionStrategy::Fifo, 150)
        .unwrap();
    assert_eq!(prune_fifo.deleted_entries, 1);
    assert_eq!(prune_fifo.strategy, Some(EvictionStrategy::Fifo));
    assert!(storage.get_entry(&key_a).unwrap().is_none());
    assert!(storage.get_entry(&key_b).unwrap().is_some());
}

#[test]
fn test_storage_suite_eviction_strategy_lfu_and_policy() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = CasStorage::new(StorageConfig::new(temp_dir.path())).unwrap();
    let pruner = Pruner::new(&storage);

    // Entry 1: hits = 50, size 100
    let comp1 = Computation::builder("op1", "cmd1").build().unwrap();
    let key1 = comp1.compute_key().unwrap();
    let (d1, s1) = storage.store_object_bytes(&[1u8; 100]).unwrap();
    let mut entry1 = CacheEntry::new(
        key1.clone(),
        comp1,
        vec![dcc_core::OutputManifestItem {
            path: "out1.dat".to_string(),
            digest: d1,
            size: s1,
            is_executable: Some(false),
        }],
        ExecutionMetadata::default(),
    );
    entry1.metadata.hit_count = 50;
    storage.store_entry(&entry1).unwrap();

    // Entry 2: hits = 2, size 100
    let comp2 = Computation::builder("op2", "cmd2").build().unwrap();
    let key2 = comp2.compute_key().unwrap();
    let (d2, s2) = storage.store_object_bytes(&[2u8; 100]).unwrap();
    let mut entry2 = CacheEntry::new(
        key2.clone(),
        comp2,
        vec![dcc_core::OutputManifestItem {
            path: "out2.dat".to_string(),
            digest: d2,
            size: s2,
            is_executable: Some(false),
        }],
        ExecutionMetadata::default(),
    );
    entry2.metadata.hit_count = 2;
    storage.store_entry(&entry2).unwrap();

    // Enforce Policy with LFU strategy and limit 150 bytes
    let policy = EvictionPolicy::StrategyAndLimit {
        strategy: EvictionStrategy::Lfu,
        max_size_bytes: 150,
    };
    let res = pruner.enforce_policy(policy).unwrap();
    assert_eq!(res.deleted_entries, 1);
    assert_eq!(res.strategy, Some(EvictionStrategy::Lfu));

    assert!(storage.get_entry(&key1).unwrap().is_some());
    assert!(storage.get_entry(&key2).unwrap().is_none());
}

#[test]
fn test_storage_suite_garbage_collection_prune_unreferenced_lifecycle() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = CasStorage::new(StorageConfig::new(temp_dir.path())).unwrap();
    let pruner = Pruner::new(&storage);

    // 1. Initially empty cache has 0 unreferenced objects
    let res0 = pruner.prune_unreferenced_objects().unwrap();
    assert_eq!(res0.deleted_objects, 0);
    assert_eq!(res0.freed_bytes, 0);

    // 2. Put 3 objects directly into CAS (orphaned)
    let (d1, s1) = storage.store_object_bytes(b"orphan 1").unwrap();
    let (d2, s2) = storage.store_object_bytes(b"orphan 2").unwrap();
    let (d3, s3) = storage.store_object_bytes(b"active artifact").unwrap();

    // 3. Create entry referencing only d3
    let comp = Computation::builder("op", "cmd").build().unwrap();
    let key = comp.compute_key().unwrap();
    let entry = CacheEntry::new(
        key.clone(),
        comp,
        vec![dcc_core::OutputManifestItem {
            path: "active.txt".to_string(),
            digest: d3.clone(),
            size: s3,
            is_executable: Some(false),
        }],
        ExecutionMetadata::default(),
    );
    storage.store_entry(&entry).unwrap();

    // 4. Dry-run prune identifies d1 and d2 (s1 + s2 bytes)
    let dry_run = pruner.prune_with_options(true).unwrap();
    assert_eq!(dry_run.deleted_objects, 2);
    assert_eq!(dry_run.freed_bytes, s1 + s2);
    assert!(storage.has_object(&d1));
    assert!(storage.has_object(&d2));
    assert!(storage.has_object(&d3));

    // 5. Real prune deletes d1 and d2, keeping d3
    let gc_res = pruner.prune_unreferenced_objects().unwrap();
    assert_eq!(gc_res.deleted_objects, 2);
    assert_eq!(gc_res.freed_bytes, s1 + s2);
    assert!(!storage.has_object(&d1));
    assert!(!storage.has_object(&d2));
    assert!(storage.has_object(&d3));

    // 6. Delete the entry -> d3 becomes unreferenced
    storage.delete_entry(&key).unwrap();
    let gc_res2 = storage.prune_unreferenced().unwrap();
    assert_eq!(gc_res2.deleted_objects, 1);
    assert_eq!(gc_res2.freed_bytes, s3);
    assert!(!storage.has_object(&d3));
}

#[test]
fn test_storage_suite_manual_maintenance_operations() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = CasStorage::new(StorageConfig::new(temp_dir.path())).unwrap();
    let cache = dcc_storage::Cache::new(storage.clone());

    // 1. Initial Maintenance State: Stats & Verify on empty cache
    let initial_stats = cache.stats().unwrap();
    assert_eq!(initial_stats.total_objects, 0);
    assert_eq!(initial_stats.total_entries, 0);

    let initial_verify = cache.verify_all().unwrap();
    assert_eq!(initial_verify.total_objects, 0);
    assert_eq!(initial_verify.corrupted_objects, 0);
    assert_eq!(initial_verify.total_entries, 0);

    // 2. Populate Cache with 2 Computations and 2 Artifacts
    let comp1 = Computation::builder("op1", "cmd1").build().unwrap();
    let key1 = comp1.compute_key().unwrap();
    let (d1, s1) = cache.store_bytes(b"artifact payload 1").unwrap();
    let entry1 = CacheEntry::new(
        key1.clone(),
        comp1,
        vec![dcc_core::OutputManifestItem {
            path: "out1.txt".to_string(),
            digest: d1.clone(),
            size: s1,
            is_executable: Some(false),
        }],
        ExecutionMetadata::default(),
    );
    cache.store(&entry1).unwrap();

    let comp2 = Computation::builder("op2", "cmd2").build().unwrap();
    let key2 = comp2.compute_key().unwrap();
    let (d2, s2) = cache.store_bytes(b"artifact payload 2").unwrap();
    let entry2 = CacheEntry::new(
        key2.clone(),
        comp2,
        vec![dcc_core::OutputManifestItem {
            path: "out2.txt".to_string(),
            digest: d2.clone(),
            size: s2,
            is_executable: Some(false),
        }],
        ExecutionMetadata::default(),
    );
    cache.store(&entry2).unwrap();

    // 3. Maintenance Op: Stats
    let stats = cache.stats().unwrap();
    assert_eq!(stats.total_entries, 2);
    assert_eq!(stats.total_objects, 2);
    assert_eq!(stats.total_object_size_bytes, s1 + s2);

    // 4. Maintenance Op: Verify
    let verify = cache.verify_all().unwrap();
    assert_eq!(verify.total_objects, 2);
    assert_eq!(verify.verified_objects, 2);
    assert_eq!(verify.corrupted_objects, 0);
    assert_eq!(verify.total_entries, 2);
    assert_eq!(verify.valid_entries, 2);
    assert_eq!(verify.corrupted_entries, 0);

    // 5. Maintenance Op: Prune (with orphaned object)
    let (d_orphan, _) = cache.store_bytes(b"orphaned data").unwrap();
    assert_eq!(cache.stats().unwrap().total_objects, 3);
    let prune_res = cache.prune().unwrap();
    assert_eq!(prune_res.deleted_objects, 1);
    assert!(!cache.contains_blob(&d_orphan));
    assert!(cache.contains_blob(&d1));
    assert!(cache.contains_blob(&d2));

    // 6. Maintenance Op: Clean Specific Key
    let removed = cache.remove(&key1).unwrap();
    assert!(removed);
    assert!(cache.lookup(&key1).unwrap().is_none());
    assert!(cache.lookup(&key2).unwrap().is_some());

    // 7. Maintenance Op: Clean All
    cache.clean_all().unwrap();
    assert_eq!(cache.stats().unwrap().total_entries, 0);
    assert_eq!(cache.stats().unwrap().total_objects, 0);
    assert_eq!(cache.verify_all().unwrap().total_objects, 0);
}

#[test]
fn test_storage_suite_safe_deletion_coordination() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = CasStorage::new(StorageConfig::new(temp_dir.path())).unwrap();
    let cache = dcc_storage::Cache::new(storage.clone());

    // 1. Store test blob in CAS
    let data = b"active stream computation artifact payload";
    let (digest, size) = cache.store_bytes(data).unwrap();
    assert_eq!(size, data.len() as u64);
    assert!(cache.contains_blob(&digest));

    // 2. Simulate an active reader process acquiring a shared lock on the object
    let reader_lock = cache
        .lock_object(&digest, std::time::Duration::from_secs(2))
        .unwrap();

    // 3. Attempt to prune unreferenced objects:
    // Because reader_lock is actively held, safe deletion skips deleting this object
    let prune_res = cache.prune().unwrap();
    assert_eq!(prune_res.deleted_objects, 0);
    assert_eq!(prune_res.freed_bytes, 0);
    assert!(
        cache.contains_blob(&digest),
        "Object must not be deleted while active reader holds shared lock"
    );

    // 4. Also verify non-blocking delete_object_safe returns false without deleting
    let delete_attempt = storage.delete_object_safe(&digest, None).unwrap();
    assert!(
        !delete_attempt,
        "delete_object_safe must return false and preserve object when reader is active"
    );
    assert!(cache.contains_blob(&digest));

    // 5. Release / drop reader lock
    drop(reader_lock);

    // 6. Prune again: now that reader lock is released, pruning successfully deletes the unreferenced object
    let prune_res2 = cache.prune().unwrap();
    assert_eq!(prune_res2.deleted_objects, 1);
    assert_eq!(prune_res2.freed_bytes, size);
    assert!(
        !cache.contains_blob(&digest),
        "Object must be cleanly deleted once reader lock is released"
    );
}
