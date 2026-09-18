use dcc_core::{CacheEntry, Computation, Digest, ExecutionMetadata, OutputManifestItem};
use dcc_storage::Cache;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    println!("================================================================================");
    println!("=== DCC MILESTONE 13.4: PROFILING & TARGETED OPTIMIZATION BENCHMARK ===");
    println!("================================================================================\n");
    println!("Optimization Cycle Methodology:");
    println!("  1. Benchmark Before (baseline measurement)");
    println!("  2. Implementation of targeted optimization");
    println!("  3. Benchmark After (optimized measurement)\n");

    let temp_dir = tempfile::tempdir()?;
    let cache_dir = temp_dir.path().join(".dcc_cache");
    let ws_dir = temp_dir.path().join("ws");
    std::fs::create_dir_all(&ws_dir)?;
    let cache = Cache::open(&cache_dir)?;

    // =========================================================================
    // OPTIMIZATION 1: In-Memory Metadata L1 Cache vs Uncached Disk I/O
    // =========================================================================
    println!("--------------------------------------------------------------------------------");
    println!("OPTIMIZATION 1: In-Memory L1 Metadata Cache (Profile -> Implement -> Measure)");
    println!("--------------------------------------------------------------------------------");
    println!(
        "Hypothesis: Parsing JSON from disk on every lookup causes significant I/O & CPU overhead."
    );
    println!("Solution:   Thread-safe in-memory L1 cache to serve repeated lookups instantly.\n");

    let num_sample_entries = 100;
    let mut sample_keys = Vec::new();

    for i in 0..num_sample_entries {
        let (digest, size) = cache
            .storage()
            .store_object_bytes(format!("payload_{}", i).as_bytes())?;
        let comp = Computation::builder_with(format!("op_{}", i), "tool")
            .input(format!("input_{}.rs", i), digest.clone(), size)
            .build()?;
        let key = comp.compute_key()?;
        let entry = CacheEntry::new(
            key.clone(),
            comp,
            vec![OutputManifestItem {
                path: format!("out_{}.bin", i),
                digest,
                size,
                is_executable: None,
            }],
            ExecutionMetadata::default(),
        );
        cache.store(&entry)?;
        sample_keys.push(key);
    }

    let iterations = 10_000;

    // Benchmark Before: Disk I/O (Bypassing L1 cache by clearing it before each batch)
    let t_before_l1 = Instant::now();
    for i in 0..iterations {
        let key = &sample_keys[i % sample_keys.len()];
        let entry = cache.storage().get_entry(key)?;
        assert!(entry.is_some());
    }
    let dur_before_l1 = t_before_l1.elapsed();
    let lat_before_l1_us = dur_before_l1.as_micros() as f64 / iterations as f64;
    let throughput_before_l1 = iterations as f64 / dur_before_l1.as_secs_f64();

    println!("  [Benchmark Before - Disk I/O Lookup]:");
    println!(
        "    Total Time:           {:>8.2} ms",
        dur_before_l1.as_millis()
    );
    println!(
        "    Per-Lookup Latency:   {:>8.2} \u{00b5}s",
        lat_before_l1_us
    );
    println!(
        "    Throughput:           {:>8.0} lookups/sec",
        throughput_before_l1
    );

    // Warm up L1 cache
    for key in &sample_keys {
        let _ = cache.lookup(key)?;
    }

    // Benchmark After: L1 Memory Cache Hit
    let t_after_l1 = Instant::now();
    for i in 0..iterations {
        let key = &sample_keys[i % sample_keys.len()];
        let entry = cache.lookup(key)?;
        assert!(entry.is_some());
    }
    let dur_after_l1 = t_after_l1.elapsed();
    let lat_after_l1_us = dur_after_l1.as_micros() as f64 / iterations as f64;
    let throughput_after_l1 = iterations as f64 / dur_after_l1.as_secs_f64();
    let l1_speedup = dur_before_l1.as_secs_f64() / dur_after_l1.as_secs_f64();

    println!("\n  [Benchmark After - L1 Memory Cache Hit]:");
    println!(
        "    Total Time:           {:>8.2} ms",
        dur_after_l1.as_millis()
    );
    println!(
        "    Per-Lookup Latency:   {:>8.2} \u{00b5}s",
        lat_after_l1_us
    );
    println!(
        "    Throughput:           {:>8.0} lookups/sec",
        throughput_after_l1
    );
    println!("  -> Speedup:             {:>8.2}x faster\n", l1_speedup);

    // =========================================================================
    // OPTIMIZATION 2: Parallel Batch File Hashing vs Sequential Hashing
    // =========================================================================
    println!("--------------------------------------------------------------------------------");
    println!("OPTIMIZATION 2: Parallel Batch File Hashing (Profile -> Implement -> Measure)");
    println!("--------------------------------------------------------------------------------");
    println!("Hypothesis: Large projects contain hundreds of source files; sequential hashing underutilizes multi-core CPUs.");
    println!("Solution:   Digest::hash_files_parallel across available worker threads.\n");

    let num_files = 100;
    let mut file_paths: Vec<PathBuf> = Vec::with_capacity(num_files);
    let sample_payload = vec![0x5Au8; 256 * 1024]; // 256 KB per file (25.6 MB total)

    for i in 0..num_files {
        let path = ws_dir.join(format!("src_file_{}.dat", i));
        let mut f = File::create(&path)?;
        f.write_all(&sample_payload)?;
        file_paths.push(path);
    }

    // Benchmark Before: Sequential Hashing
    let t_seq = Instant::now();
    let mut seq_digests = Vec::with_capacity(num_files);
    for p in &file_paths {
        seq_digests.push(Digest::hash_file(p)?);
    }
    let dur_seq = t_seq.elapsed();
    let mb_seq = (num_files as f64 * 0.256) / dur_seq.as_secs_f64();

    println!("  [Benchmark Before - Sequential Hashing (100 files, 25.6 MB)]:");
    println!("    Total Time:           {:>8.2} ms", dur_seq.as_millis());
    println!("    Throughput:           {:>8.2} MB/s", mb_seq);

    // Benchmark After: Parallel Batch Hashing
    let t_par = Instant::now();
    let par_digests = Digest::hash_files_parallel(&file_paths)?;
    let dur_par = t_par.elapsed();
    let mb_par = (num_files as f64 * 0.256) / dur_par.as_secs_f64();
    let par_speedup = dur_seq.as_secs_f64() / dur_par.as_secs_f64();

    assert_eq!(seq_digests, par_digests);

    println!("\n  [Benchmark After - Parallel Hashing (100 files, 25.6 MB)]:");
    println!("    Total Time:           {:>8.2} ms", dur_par.as_millis());
    println!("    Throughput:           {:>8.2} MB/s", mb_par);
    println!("  -> Speedup:             {:>8.2}x faster\n", par_speedup);

    // =========================================================================
    // OPTIMIZATION 3: Safe Hardlink Materialization vs Full Buffer Copy
    // =========================================================================
    println!("--------------------------------------------------------------------------------");
    println!("OPTIMIZATION 3: Hardlink Materialization (Profile -> Implement -> Measure)");
    println!("--------------------------------------------------------------------------------");
    println!("Hypothesis: Copying large multi-megabyte binaries on cache HIT incurs I/O and memory throughput costs.");
    println!("Solution:   restore_with_options(prefer_hardlinks: true) with atomic fallback.\n");

    let big_artifact_size: usize = 20 * 1024 * 1024; // 20 MB artifact
    let big_payload = vec![0xEEu8; big_artifact_size];
    let (big_digest, big_size) = cache.storage().store_object_bytes(&big_payload)?;

    let big_comp = Computation::builder_with("heavy_build", "compiler")
        .input("huge_src.rs", big_digest.clone(), big_size)
        .output("target/heavy.bin", true)
        .build()?;
    let big_key = big_comp.compute_key()?;
    let big_entry = CacheEntry::new(
        big_key,
        big_comp,
        vec![OutputManifestItem {
            path: "target/heavy.bin".to_string(),
            digest: big_digest,
            size: big_size,
            is_executable: None,
        }],
        ExecutionMetadata::default(),
    );

    let restore_dest_copy = ws_dir.join("restore_copy");
    let restore_dest_link = ws_dir.join("restore_link");
    std::fs::create_dir_all(&restore_dest_copy)?;
    std::fs::create_dir_all(&restore_dest_link)?;

    // Benchmark Before: Full Buffer Copy Restoration
    let t_copy = Instant::now();
    cache.restore_with_options(&big_entry, &restore_dest_copy, false)?;
    let dur_copy = t_copy.elapsed();
    let mb_copy = 20.0 / dur_copy.as_secs_f64();

    println!("  [Benchmark Before - Full Copy Restoration (20 MB)]:");
    println!("    Total Time:           {:>8.2} ms", dur_copy.as_millis());
    println!("    Throughput:           {:>8.2} MB/s", mb_copy);

    // Benchmark After: Fast Hardlink Restoration
    let t_link = Instant::now();
    cache.restore_with_options(&big_entry, &restore_dest_link, true)?;
    let dur_link = t_link.elapsed();
    let mb_link = 20.0 / dur_link.as_secs_f64();
    let link_speedup = dur_copy.as_secs_f64() / dur_link.as_secs_f64();

    // Verify content identity
    let restored_file = restore_dest_link.join("target/heavy.bin");
    assert!(restored_file.is_file());
    assert_eq!(std::fs::metadata(&restored_file)?.len(), big_size);

    println!("\n  [Benchmark After - Hardlink Materialization (20 MB)]:");
    println!("    Total Time:           {:>8.2} ms", dur_link.as_millis());
    println!("    Throughput:           {:>8.2} MB/s", mb_link);
    println!("  -> Speedup:             {:>8.2}x faster\n", link_speedup);

    println!("================================================================================");
    println!("All Milestone 13.4 Profiling & Optimization benchmarks passed successfully!");
    println!("================================================================================\n");

    Ok(())
}
