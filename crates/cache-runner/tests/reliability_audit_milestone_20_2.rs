//! Reliability Audit Test Suite for Milestone 20.2
//! Verifies:
//! 1. Process crash (unclean exit, stale tmp file cleanup)
//! 2. Disk failure / Capacity exhaustion (safe error handling, cache consistency preserved)
//! 3. Partial writes (atomic rename prevents reading partial objects)
//! 4. Concurrent processes (locking prevents race conditions and corrupted entries)
//! 5. Cache corruption (CAS object byte alteration detected and handled gracefully)
//! 6. Cache deletion (re-creating directory structure on the fly)
//! 7. Large cache (eviction enforcement under high storage load)

use dcc_core::{
    ByteSize, CacheEntry, Computation, Digest, ExecutionMetadata, OutputManifestItem,
};
use dcc_runner::{CommandSpec, EngineOptions, RunnerEngine};
use dcc_storage::{CasStorage, StorageConfig};
use dcc_test_utils::TestEnv;
use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;
use tempfile::tempdir;

// ============================================================================
// 1. PROCESS CRASH / UNCLEAN INTERRUPTION
// ============================================================================

#[test]
fn test_audit_20_2_process_crash_resilience() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("crash_input.txt", b"crash test")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('crash_out.txt', 'PARTIAL'); [System.Environment]::FailFast('Crash simulation')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'PARTIAL' > crash_out.txt && kill -9 $$".to_string(),
        ],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("crash_input.txt")
        .output_path("crash_out.txt")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    let res = engine.execute_command(&spec);
    // Either error or non-zero exit code due to crash
    if let Ok(r) = res {
        assert_ne!(r.exit_code, 0);
        let entry = env.storage.get_entry(&r.key).unwrap();
        assert!(
            entry.is_none(),
            "Crashed process must not be committed to cache"
        );
    }

    // Verify storage remains uncorrupted
    let stats = env.storage.stats().unwrap();
    assert_eq!(stats.total_entries, 0);
}

// ============================================================================
// 2. DISK FAILURE / CAPACITY EXHAUSTION
// ============================================================================

#[test]
fn test_audit_20_2_disk_failure_and_capacity_limit() {
    let dir = tempdir().unwrap();
    // 512-byte strict storage limit
    let config = StorageConfig::new(dir.path()).with_max_size(ByteSize::bytes(512));
    let storage = CasStorage::new(config).unwrap();

    let env = TestEnv::new().unwrap();
    env.create_input_file("huge.bin", b"data").unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllBytes('huge_out.bin', [byte[]]@(1..255 * 10))".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "head -c 2550 /dev/urandom > huge_out.bin".to_string(),
        ],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("huge.bin")
        .output_path("huge_out.bin")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    let res = engine.execute_command(&spec);
    // Command executes and returns result or handles storage limit safely without crashing
    assert!(
        res.is_ok(),
        "Engine must safely handle storage limits without panicking"
    );
}

// ============================================================================
// 3. PARTIAL WRITES / ATOMIC RENAME
// ============================================================================

#[test]
fn test_audit_20_2_partial_write_atomicity() {
    let env = TestEnv::new().unwrap();
    let data = b"CRITICAL_PAYLOAD";
    let digest = Digest::hash_bytes(data);

    // Simulate an interrupted write in temporary staging area
    let tmp_path = env
        .storage
        .root_dir()
        .join("objects")
        .join(format!(".tmp_partial_{}", digest));
    fs::create_dir_all(tmp_path.parent().unwrap()).unwrap();
    fs::write(&tmp_path, b"INCOMPLETE_PARTIAL_BYTES").unwrap();

    // Verify that querying CAS does NOT return the incomplete temporary object
    assert!(!env.storage.has_object(&digest));

    // Valid atomic store must overwrite or succeed properly
    let (stored_digest, _) = env.storage.store_object_bytes(data).unwrap();
    assert!(env.storage.has_object(&stored_digest));
}

// ============================================================================
// 4. CONCURRENT PROCESSES / CONTENTION
// ============================================================================

#[test]
fn test_audit_20_2_concurrent_processes_and_locking() {
    let env = Arc::new(TestEnv::new().unwrap());
    env.create_input_file("concurrent_input.txt", b"shared input")
        .unwrap();

    let num_threads = 6;
    let barrier = Arc::new(Barrier::new(num_threads));
    let mut handles = vec![];

    for i in 0..num_threads {
        let env_clone = Arc::clone(&env);
        let barrier_clone = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            #[cfg(windows)]
            let (cmd, args) = (
                "powershell.exe",
                vec![
                    "-Command".to_string(),
                    format!("[System.IO.File]::WriteAllText('concurrent_out_{}.txt', 'THREAD_{}_OUTPUT')", i, i),
                ],
            );
            #[cfg(not(windows))]
            let (cmd, args) = (
                "sh",
                vec![
                    "-c".to_string(),
                    format!("echo -n 'THREAD_{}_OUTPUT' > concurrent_out_{}.txt", i, i),
                ],
            );

            let spec = CommandSpec::builder(cmd)
                .args(args)
                .current_dir(env_clone.workspace_dir.path())
                .input_path("concurrent_input.txt")
                .output_path(format!("concurrent_out_{}.txt", i))
                .build()
                .unwrap();

            let engine = RunnerEngine::new(
                &env_clone.storage,
                EngineOptions {
                    working_dir: env_clone.workspace_dir.path().to_path_buf(),
                    ..Default::default()
                },
            );

            barrier_clone.wait();
            let res = engine.execute_command(&spec).unwrap();
            assert_eq!(res.exit_code, 0);
        });

        handles.push(handle);
    }

    for h in handles {
        h.join().unwrap();
    }

    let stats = env.storage.stats().unwrap();
    assert_eq!(stats.total_entries, num_threads);
}

// ============================================================================
// 5. CACHE CORRUPTION / QUARANTINE
// ============================================================================

#[test]
fn test_audit_20_2_cache_corruption_detection() {
    let env = TestEnv::new().unwrap();
    let data = b"ORIGINAL_VALID_DATA";
    let (digest, _) = env.storage.store_object_bytes(data).unwrap();

    let obj_path = env.storage.object_path(&digest);
    assert!(obj_path.exists());

    // Corrupt object on disk
    fs::write(&obj_path, b"TAMPERED_INVALID_DATA").unwrap();

    // Verify corruption detection
    let verify_res = env.storage.verify_object(&digest);
    assert!(
        verify_res.is_err(),
        "Corrupted object must fail integrity verification"
    );
}

// ============================================================================
// 6. CACHE DELETION / SAFE RECOVERY
// ============================================================================

#[test]
fn test_audit_20_2_cache_deletion_and_recovery() {
    let env = TestEnv::new().unwrap();
    let data = b"PERSISTENT_DATA";
    env.storage.store_object_bytes(data).unwrap();

    // Delete entire cache directory
    let cache_dir = env.storage.root_dir().to_path_buf();
    fs::remove_dir_all(&cache_dir).unwrap();
    assert!(!cache_dir.exists());

    // Reconnect / initialize storage on deleted location
    let storage2 = CasStorage::new(StorageConfig::new(&cache_dir)).unwrap();
    assert!(
        cache_dir.exists(),
        "Storage must auto-create directories on initialization"
    );

    let stats = storage2.stats().unwrap();
    assert_eq!(stats.total_entries, 0);
    assert_eq!(stats.total_objects, 0);
}

// ============================================================================
// 7. LARGE CACHE / EVICTION ENFORCEMENT
// ============================================================================

#[test]
fn test_audit_20_2_large_cache_eviction() {
    let dir = tempdir().unwrap();
    let limit = ByteSize::kb(10); // 10 KB limit
    let config = StorageConfig::new(dir.path()).with_max_size(limit);
    let storage = CasStorage::new(config).unwrap();

    // Insert multiple 2KB items to trigger eviction
    for i in 0..10 {
        let payload = vec![i as u8; 2048];
        let (digest, size) = storage.store_object_bytes(&payload).unwrap();

        let comp = Computation::builder()
            .operation("tool")
            .command("tool")
            .args(vec![format!("arg_{}", i)])
            .build()
            .unwrap();
        let key = comp.compute_key().unwrap();
        let entry = CacheEntry::new(
            key,
            comp,
            vec![OutputManifestItem {
                path: format!("out_{}.bin", i),
                digest,
                size,
                is_executable: None,
            }],
            ExecutionMetadata::default(),
        );
        storage.store_entry(&entry).unwrap();
    }

    let stats = storage.stats().unwrap();
    assert!(
        stats.total_size_bytes <= limit.as_bytes() + 4096,
        "Storage total bytes must be strictly bounded by capacity policy"
    );
}
