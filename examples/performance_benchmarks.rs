use dcc_core::{
    CacheEntry, CacheKey, Computation, Digest, ExecutionMetadata, OutputManifestItem, TimingMetrics,
};
use dcc_storage::Cache;
use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    println!("===============================================================");
    println!("=== DCC MILESTONE 13.1: PERFORMANCE ENGINEERING BENCHMARKS ===");
    println!("===============================================================\n");

    let temp_dir = tempfile::tempdir()?;
    let cache_dir = temp_dir.path().join(".dcc_cache");
    let ws_dir = temp_dir.path().join("workspace");
    fs::create_dir_all(&ws_dir)?;

    let cache = Cache::open(&cache_dir)?;

    // -------------------------------------------------------------
    // 1. Hash Small File (4 KB)
    // -------------------------------------------------------------
    let small_file = ws_dir.join("small_file.bin");
    let small_data = vec![0xABu8; 4 * 1024];
    fs::write(&small_file, &small_data)?;

    let iters = 1000;
    let t0 = Instant::now();
    for _ in 0..iters {
        let _ = Digest::hash_file(&small_file)?;
    }
    let dur_hash_small = t0.elapsed();
    let avg_hash_small_us = dur_hash_small.as_micros() as f64 / iters as f64;

    // -------------------------------------------------------------
    // 2. Hash Large File (10 MB)
    // -------------------------------------------------------------
    let large_file = ws_dir.join("large_file.bin");
    let large_data = vec![0x55u8; 10 * 1024 * 1024];
    fs::write(&large_file, &large_data)?;

    let large_iters = 10;
    let t0 = Instant::now();
    for _ in 0..large_iters {
        let _ = Digest::hash_file(&large_file)?;
    }
    let dur_hash_large = t0.elapsed();
    let avg_hash_large_ms = dur_hash_large.as_millis() as f64 / large_iters as f64;
    let throughput_mb_s = (10.0 * large_iters as f64) / dur_hash_large.as_secs_f64();

    // -------------------------------------------------------------
    // 3. Hash Directory (100 files in nested structure)
    // -------------------------------------------------------------
    let dir_tree = ws_dir.join("tree");
    for i in 0..10 {
        let sub = dir_tree.join(format!("sub_{}", i));
        fs::create_dir_all(&sub)?;
        for j in 0..10 {
            fs::write(
                sub.join(format!("file_{}.txt", j)),
                format!("content_{}_{}", i, j),
            )?;
        }
    }

    let dir_iters = 50;
    let t0 = Instant::now();
    for _ in 0..dir_iters {
        let _ = Digest::hash_directory(&dir_tree)?;
    }
    let dur_hash_dir = t0.elapsed();
    let avg_hash_dir_ms = dur_hash_dir.as_millis() as f64 / dir_iters as f64;

    // -------------------------------------------------------------
    // 4. Generate Key
    // -------------------------------------------------------------
    let dummy_digest = Digest::hash_bytes(b"INPUT_CONTENT");
    let comp = Computation::builder_with("compile", "rustc")
        .arg("src/main.rs")
        .arg("--crate-type=bin")
        .input("src/main.rs", dummy_digest.clone(), 1024)
        .env("RUSTFLAGS", "-C opt-level=3")
        .output("target/main.exe", true)
        .build()?;

    let key_iters = 5000;
    let t0 = Instant::now();
    let mut computed_key = CacheKey::new(dummy_digest.clone());
    for _ in 0..key_iters {
        computed_key = comp.compute_key()?;
    }
    let dur_gen_key = t0.elapsed();
    let avg_gen_key_us = dur_gen_key.as_micros() as f64 / key_iters as f64;

    // -------------------------------------------------------------
    // 5. Store Cache (Metadata + Output Artifacts)
    // -------------------------------------------------------------
    let artifact_data = vec![0x77u8; 64 * 1024]; // 64 KB artifact
    let (artifact_digest, artifact_size) = cache.storage().store_object_bytes(&artifact_data)?;

    let entry = CacheEntry::new(
        computed_key.clone(),
        comp.clone(),
        vec![OutputManifestItem {
            path: "target/main.exe".to_string(),
            digest: artifact_digest,
            size: artifact_size,
            is_executable: Some(true),
        }],
        ExecutionMetadata {
            exit_code: 0,
            execution_time_ms: 250,
            stdout_digest: None,
            stderr_digest: None,
            timings: TimingMetrics::default(),
        },
    );

    let store_iters = 200;
    let t0 = Instant::now();
    for _ in 0..store_iters {
        cache.store(&entry)?;
    }
    let dur_store = t0.elapsed();
    let avg_store_us = dur_store.as_micros() as f64 / store_iters as f64;

    // -------------------------------------------------------------
    // 6. Lookup Cache
    // -------------------------------------------------------------
    let lookup_iters = 1000;
    let t0 = Instant::now();
    for _ in 0..lookup_iters {
        let _ = cache.lookup(&computed_key)?;
    }
    let dur_lookup = t0.elapsed();
    let avg_lookup_us = dur_lookup.as_micros() as f64 / lookup_iters as f64;

    // -------------------------------------------------------------
    // 7. Restore Cache
    // -------------------------------------------------------------
    let restore_dest = ws_dir.join("restore_dest");
    fs::create_dir_all(&restore_dest)?;

    let restore_iters = 200;
    let t0 = Instant::now();
    for _ in 0..restore_iters {
        cache.restore(&entry, &restore_dest)?;
    }
    let dur_restore = t0.elapsed();
    let avg_restore_us = dur_restore.as_micros() as f64 / restore_iters as f64;

    // -------------------------------------------------------------
    // 8. Serialize Metadata (JSON)
    // -------------------------------------------------------------
    let serde_iters = 5000;
    let t0 = Instant::now();
    let mut serialized_json = String::new();
    for _ in 0..serde_iters {
        serialized_json = serde_json::to_string(&entry)?;
    }
    let dur_serialize = t0.elapsed();
    let avg_serialize_us = dur_serialize.as_micros() as f64 / serde_iters as f64;

    // -------------------------------------------------------------
    // 9. Deserialize Metadata (JSON)
    // -------------------------------------------------------------
    let t0 = Instant::now();
    for _ in 0..serde_iters {
        let _deserialized: CacheEntry = serde_json::from_str(&serialized_json)?;
    }
    let dur_deserialize = t0.elapsed();
    let avg_deserialize_us = dur_deserialize.as_micros() as f64 / serde_iters as f64;

    // -------------------------------------------------------------
    // 10. Concurrent Lookup (8 threads x 500 requests = 4000 total)
    // -------------------------------------------------------------
    let num_threads = 8;
    let thread_ops = 500;
    let cache_arc = Arc::new(cache);
    let key_arc = Arc::new(computed_key);

    let t0 = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..num_threads {
        let c = Arc::clone(&cache_arc);
        let k = Arc::clone(&key_arc);
        handles.push(thread::spawn(move || {
            for _ in 0..thread_ops {
                let res = c.lookup(&k).unwrap();
                assert!(res.is_some());
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
    let dur_concurrent = t0.elapsed();
    let total_concurrent_ops = num_threads * thread_ops;
    let concurrent_ops_sec = total_concurrent_ops as f64 / dur_concurrent.as_secs_f64();

    // -------------------------------------------------------------
    // Report Formatting
    // -------------------------------------------------------------
    println!("Benchmark Results (10 Required Dimensions):");
    println!("---------------------------------------------------------------");
    println!(
        "  1. Hash Small File (4 KB):        {:>8.2} \u{00b5}s / op",
        avg_hash_small_us
    );
    println!(
        "  2. Hash Large File (10 MB):       {:>8.2} ms / op ({:.2} MB/s)",
        avg_hash_large_ms, throughput_mb_s
    );
    println!(
        "  3. Hash Directory (100 files):    {:>8.2} ms / op",
        avg_hash_dir_ms
    );
    println!(
        "  4. Generate Key:                  {:>8.2} \u{00b5}s / op",
        avg_gen_key_us
    );
    println!(
        "  5. Lookup Cache:                  {:>8.2} \u{00b5}s / op",
        avg_lookup_us
    );
    println!(
        "  6. Store Cache:                   {:>8.2} \u{00b5}s / op",
        avg_store_us
    );
    println!(
        "  7. Restore Cache:                 {:>8.2} \u{00b5}s / op",
        avg_restore_us
    );
    println!(
        "  8. Serialize Metadata (JSON):     {:>8.2} \u{00b5}s / op",
        avg_serialize_us
    );
    println!(
        "  9. Deserialize Metadata (JSON):   {:>8.2} \u{00b5}s / op",
        avg_deserialize_us
    );
    println!(
        " 10. Concurrent Lookup (8 threads): {:>8.0} ops / sec",
        concurrent_ops_sec
    );
    println!("---------------------------------------------------------------\n");
    println!("Milestone 13.1 Benchmark suite completed successfully!");

    Ok(())
}
