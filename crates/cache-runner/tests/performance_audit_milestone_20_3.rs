//! Performance Audit Test & Benchmark Suite for Milestone 20.3
//! Measures real metrics across:
//! 1. Cold execution
//! 2. Cache lookup (metadata retrieval)
//! 3. Cache hit (overall turnaround)
//! 4. Cache restore (CAS blob to workspace)
//! 5. Cache store (workspace file to CAS)
//! 6. Large files (multi-megabyte throughput)
//! 7. Large cache (high-volume metadata index)
//! 8. Concurrent workloads (multi-threaded parallel access)

use dcc_core::{ByteSize, CacheEntry, Computation, Digest, ExecutionMetadata, OutputManifestItem};
use dcc_runner::{CommandSpec, EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_storage::{CasStorage, Storage, StorageConfig};
use dcc_test_utils::TestEnv;
use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Instant;
use tempfile::tempdir;

#[test]
fn test_audit_20_3_measure_performance_metrics() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("perf_in.txt", b"PERFORMANCE_TEST_INPUT")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('perf_out.txt', 'PERF_OUTPUT_DATA')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'PERF_OUTPUT_DATA' > perf_out.txt".to_string(),
        ],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("perf_in.txt")
        .output_path("perf_out.txt")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // 1. Cold execution measurement
    let start_cold = Instant::now();
    let res_cold = engine.execute_command(&spec).unwrap();
    let cold_duration = start_cold.elapsed();
    assert_eq!(res_cold.status, ExecutionStatus::Miss);

    // 2. Cache lookup measurement
    let start_lookup = Instant::now();
    let entry_opt = env.storage.get_entry(&res_cold.key).unwrap();
    let lookup_duration = start_lookup.elapsed();
    assert!(entry_opt.is_some());

    // 3. Cache hit measurement
    let start_hit = Instant::now();
    let res_hit = engine.execute_command(&spec).unwrap();
    let hit_duration = start_hit.elapsed();
    assert_eq!(res_hit.status, ExecutionStatus::Hit);

    // 4. Cache restore measurement
    let entry = entry_opt.unwrap();
    let out_digest = &entry.outputs[0].digest;
    let restore_target = env.workspace_dir.path().join("perf_restored.txt");
    let start_restore = Instant::now();
    env.storage
        .restore_object(out_digest, &restore_target)
        .unwrap();
    let restore_duration = start_restore.elapsed();
    assert!(restore_target.exists());

    // 5. Cache store measurement
    let store_payload = b"DIRECT_CACHE_STORE_BENCHMARK_BYTES_1234567890";
    let start_store = Instant::now();
    let stored_digest = env.storage.store_object_bytes(store_payload).unwrap();
    let store_duration = start_store.elapsed();
    assert_eq!(stored_digest, Digest::hash_bytes(store_payload));

    // 6. Large files measurement (5 MB streaming)
    let large_bytes = vec![0xAB; 5 * 1024 * 1024]; // 5 MB
    let start_large_hash = Instant::now();
    let large_digest = Digest::hash_bytes(&large_bytes);
    let large_hash_duration = start_large_hash.elapsed();

    let start_large_store = Instant::now();
    env.storage.store_object_bytes(&large_bytes).unwrap();
    let large_store_duration = start_large_store.elapsed();

    let large_restore_target = env.workspace_dir.path().join("large_restored.bin");
    let start_large_restore = Instant::now();
    env.storage
        .restore_object(&large_digest, &large_restore_target)
        .unwrap();
    let large_restore_duration = start_large_restore.elapsed();

    // 7. Large cache measurement (500 entries lookup & index traversal)
    let dir = tempdir().unwrap();
    let storage_large = CasStorage::new(StorageConfig::new(dir.path())).unwrap();
    for i in 0..500 {
        let comp = Computation::new("bench", vec![format!("arg_{}", i)]);
        let key = comp.compute_key().unwrap();
        let entry = CacheEntry::new(key, comp, vec![], ExecutionMetadata::default());
        storage_large.put_entry(entry).unwrap();
    }
    let start_large_cache_lookup = Instant::now();
    let query_comp = Computation::new("bench", vec!["arg_250".to_string()]);
    let query_key = query_comp.compute_key().unwrap();
    let found = storage_large.get_entry(&query_key).unwrap();
    let large_cache_lookup_duration = start_large_cache_lookup.elapsed();
    assert!(found.is_some());

    // 8. Concurrent workloads (8 threads parallel reading and writing)
    let num_threads = 8;
    let barrier = Arc::new(Barrier::new(num_threads));
    let env_arc = Arc::new(TestEnv::new().unwrap());
    let mut handles = vec![];
    let start_concurrent = Instant::now();

    for i in 0..num_threads {
        let env_c = Arc::clone(&env_arc);
        let b_c = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let data = format!("CONCURRENT_BENCHMARK_THREAD_{}", i).into_bytes();
            b_c.wait();
            let d = env_c.storage.store_object_bytes(&data).unwrap();
            let read_back = env_c.storage.get_object(&d).unwrap().unwrap();
            assert_eq!(read_back, data);
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
    let concurrent_duration = start_concurrent.elapsed();

    // Sanity validations on durations
    assert!(lookup_duration.as_millis() < 50);
    assert!(hit_duration.as_millis() < 100);
}
