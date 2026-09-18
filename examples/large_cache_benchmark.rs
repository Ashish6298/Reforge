use dcc_core::{
    CacheEntry, Computation, Digest, ExecutionMetadata, OutputManifestItem, TimingMetrics,
};
use dcc_storage::stats::StorageStats;
use dcc_storage::Cache;
use std::time::Instant;

fn populate_and_benchmark_tier(num_entries: usize) -> anyhow::Result<()> {
    println!("---------------------------------------------------------------");
    println!(
        "Evaluating Synthetic Cache Tier: {:>7} entries",
        num_entries
    );
    println!("---------------------------------------------------------------");

    let temp_dir = tempfile::tempdir()?;
    let cache_dir = temp_dir.path().join(".dcc_cache");
    let cache = Cache::open(&cache_dir)?;

    // 1. Synthetic Population using multi-threaded batch workers
    let dummy_blob = b"synthetic_large_cache_blob_payload";
    let (blob_digest, blob_size) = cache.storage().store_object_bytes(dummy_blob)?;
    let cache_arc = std::sync::Arc::new(cache);

    let t_pop = Instant::now();
    let num_threads = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(8)
        .min(16);
    let chunk_size = num_entries.div_ceil(num_threads);

    let sample_keys = std::sync::Mutex::new(Vec::new());

    std::thread::scope(|s| {
        for t_idx in 0..num_threads {
            let start = t_idx * chunk_size;
            let end = ((t_idx + 1) * chunk_size).min(num_entries);
            if start >= end {
                continue;
            }
            let c = std::sync::Arc::clone(&cache_arc);
            let b_dig = blob_digest.clone();
            let sample_ref = &sample_keys;

            s.spawn(move || {
                let mut local_samples = Vec::new();
                for i in start..end {
                    let digest_i = Digest::hash_bytes(
                        format!("input_synthetic_{}_{}", num_entries, i).as_bytes(),
                    );
                    let comp = Computation::builder_with(format!("op_{}", i % 50), "compiler")
                        .arg(format!("arg_{}", i))
                        .input(format!("src/input_{}.rs", i), digest_i, 100)
                        .env("OPT_LEVEL", "3")
                        .output(format!("target/out_{}.rlib", i), true)
                        .build()
                        .unwrap();
                    let key = comp.compute_key().unwrap();

                    let entry = CacheEntry::new(
                        key.clone(),
                        comp,
                        vec![OutputManifestItem {
                            path: format!("target/out_{}.rlib", i),
                            digest: b_dig.clone(),
                            size: blob_size,
                            is_executable: None,
                        }],
                        ExecutionMetadata {
                            exit_code: 0,
                            execution_time_ms: 50,
                            stdout_digest: None,
                            stderr_digest: None,
                            timings: TimingMetrics::default(),
                        },
                    );

                    let _ = c.store(&entry);

                    if local_samples.len() < 100 {
                        local_samples.push(key);
                    }
                }

                let mut g = sample_ref.lock().unwrap();
                g.extend(local_samples);
            });
        }
    });

    let pop_dur = t_pop.elapsed();
    let insert_rate = num_entries as f64 / pop_dur.as_secs_f64();
    println!(
        "  1. Insertion Time:            {:>8.2} ms ({:.0} stores/sec)",
        pop_dur.as_millis(),
        insert_rate
    );

    let generated_keys = sample_keys.into_inner().unwrap();
    let cache = cache_arc;

    // 2. Measure Point Lookup Performance (Random access across 1,000 sampled keys)
    let lookups_to_run = generated_keys.len().min(1000);
    let t_lookup = Instant::now();
    for key in &generated_keys[..lookups_to_run] {
        let entry = cache.lookup(key)?;
        assert!(entry.is_some());
    }
    let lookup_dur = t_lookup.elapsed();
    let avg_lookup_us = lookup_dur.as_micros() as f64 / lookups_to_run as f64;
    let lookup_rate = lookups_to_run as f64 / lookup_dur.as_secs_f64();
    println!(
        "  2. Point Lookup Latency:      {:>8.2} \u{00b5}s / lookup ({:.0} lookups/sec)",
        avg_lookup_us, lookup_rate
    );

    // 3. Measure Cache Miss Lookup Performance (Non-existent keys)
    let non_existent_key = dcc_core::CacheKey::from_bytes(b"NON_EXISTENT_KEY_12345");
    let t_miss = Instant::now();
    for _ in 0..500 {
        let entry = cache.lookup(&non_existent_key)?;
        assert!(entry.is_none());
    }
    let miss_dur = t_miss.elapsed();
    let avg_miss_us = miss_dur.as_micros() as f64 / 500.0;
    println!(
        "  3. Negative Lookup Latency:   {:>8.2} \u{00b5}s / lookup",
        avg_miss_us
    );

    // 4. Measure Maintenance Performance (StorageStats inspection & GC pruning)
    let t_maint = Instant::now();
    let stats = StorageStats::collect(cache.storage())?;
    let maint_dur = t_maint.elapsed();
    assert_eq!(stats.total_entries, num_entries);
    println!(
        "  4. Full Index Scan/Inspection: {:>8.2} ms (scanned {} entries)",
        maint_dur.as_millis(),
        stats.total_entries
    );

    // 5. Measure Safe Prune Maintenance
    let t_prune = Instant::now();
    let prune_res = cache.prune()?;
    let prune_dur = t_prune.elapsed();
    println!(
        "  5. Prune / GC Execution:      {:>8.2} ms (unreferenced deleted: {})",
        prune_dur.as_millis(),
        prune_res.deleted_objects
    );

    println!(
        "  -> Tier {:>7} entries verified successfully!\n",
        num_entries
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    println!("===============================================================");
    println!("=== DCC MILESTONE 13.3: LARGE CACHE PERFORMANCE BENCHMARKS ===");
    println!("===============================================================\n");

    // Benchmark Tiers: 1,000 | 10,000 | 100,000 entries
    populate_and_benchmark_tier(1_000)?;
    populate_and_benchmark_tier(10_000)?;
    populate_and_benchmark_tier(100_000)?;

    println!("===============================================================");
    println!("All large cache tiers (1k, 10k, 100k) completed successfully!");
    println!("256-shard CAS & entries directory layout ensures scalable lookup latency.");
    println!("===============================================================\n");

    Ok(())
}
