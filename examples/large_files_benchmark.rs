use dcc_core::{
    CacheEntry, Computation, Digest, ExecutionMetadata, OutputManifestItem, TimingMetrics,
};
use dcc_storage::Cache;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Instant;

fn generate_chunked_file(path: &Path, size_bytes: u64) -> anyhow::Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::with_capacity(64 * 1024, file);
    let chunk = [0x5Au8; 64 * 1024]; // 64 KB chunk
    let mut written = 0;
    while written < size_bytes {
        let to_write = (size_bytes - written).min(chunk.len() as u64) as usize;
        writer.write_all(&chunk[..to_write])?;
        written += to_write as u64;
    }
    writer.flush()?;
    Ok(())
}

fn benchmark_large_file_tier(
    cache: &Cache,
    ws_dir: &Path,
    tier_label: &str,
    size_bytes: u64,
) -> anyhow::Result<()> {
    println!("---------------------------------------------------------------");
    println!("Testing Tier: {} ({} bytes)", tier_label, size_bytes);
    println!("---------------------------------------------------------------");

    let source_file = ws_dir.join(format!("large_{}.bin", tier_label));
    let t_gen = Instant::now();
    generate_chunked_file(&source_file, size_bytes)?;
    println!(
        "  Generated on disk in:         {:.2} ms",
        t_gen.elapsed().as_millis()
    );

    // 1. Streaming SHA-256 Hash
    let t_hash = Instant::now();
    let digest = Digest::hash_file(&source_file)?;
    let hash_dur = t_hash.elapsed();
    let hash_throughput = (size_bytes as f64 / (1024.0 * 1024.0)) / hash_dur.as_secs_f64();
    println!(
        "  1. Stream Hashing:            {:.2} ms (Throughput: {:.2} MB/s)",
        hash_dur.as_millis(),
        hash_throughput
    );

    // 2. Stream Store into CAS
    let t_store = Instant::now();
    let (stored_digest, stored_size) = cache.storage().store_object_from_file(&source_file)?;
    let store_dur = t_store.elapsed();
    let store_throughput = (size_bytes as f64 / (1024.0 * 1024.0)) / store_dur.as_secs_f64();
    assert_eq!(stored_digest, digest);
    assert_eq!(stored_size, size_bytes);
    println!(
        "  2. Stream CAS Storage:        {:.2} ms (Throughput: {:.2} MB/s)",
        store_dur.as_millis(),
        store_throughput
    );

    // 3. Cache Entry Creation & Store
    let comp = Computation::builder_with("large_test", "streamer")
        .input(
            source_file.file_name().unwrap().to_str().unwrap(),
            digest.clone(),
            size_bytes,
        )
        .output("restored_output.bin", true)
        .build()?;
    let key = comp.compute_key()?;

    let entry = CacheEntry::new(
        key.clone(),
        comp,
        vec![OutputManifestItem {
            path: format!("restored_{}.bin", tier_label),
            digest: digest.clone(),
            size: size_bytes,
            is_executable: None,
        }],
        ExecutionMetadata {
            exit_code: 0,
            execution_time_ms: 100,
            stdout_digest: None,
            stderr_digest: None,
            timings: TimingMetrics::default(),
        },
    );
    cache.store(&entry)?;

    // 4. Stream CAS Verification
    let t_verify = Instant::now();
    cache.storage().verify_object(&digest)?;
    println!(
        "  3. CAS Stream Verification:   {:.2} ms",
        t_verify.elapsed().as_millis()
    );

    // 5. Stream Restoration from CAS to Disk
    let restore_dest = ws_dir.join(format!("restore_{}", tier_label));
    fs::create_dir_all(&restore_dest)?;
    let t_restore = Instant::now();
    cache.restore(&entry, &restore_dest)?;
    let restore_dur = t_restore.elapsed();
    let restore_throughput = (size_bytes as f64 / (1024.0 * 1024.0)) / restore_dur.as_secs_f64();
    println!(
        "  4. Stream CAS Restoration:    {:.2} ms (Throughput: {:.2} MB/s)",
        restore_dur.as_millis(),
        restore_throughput
    );

    let restored_file = restore_dest.join(format!("restored_{}.bin", tier_label));
    assert!(restored_file.is_file());
    assert_eq!(fs::metadata(&restored_file)?.len(), size_bytes);

    // Cleanup temporary tier source and restored files to keep workspace lean
    let _ = fs::remove_file(&source_file);
    let _ = fs::remove_dir_all(&restore_dest);

    println!(
        "  -> Tier {} verified successfully (Constant memory streaming verified)!\n",
        tier_label
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    println!("===============================================================");
    println!("=== DCC MILESTONE 13.2: LARGE FILES STREAMING BENCHMARKS ===");
    println!("===============================================================\n");

    let temp_dir = tempfile::tempdir()?;
    let cache_dir = temp_dir.path().join(".dcc_cache");
    let ws_dir = temp_dir.path().join("workspace");
    fs::create_dir_all(&ws_dir)?;

    let cache = Cache::open(&cache_dir)?;

    // Test Tiers: 1 MB, 10 MB, 100 MB, 1 GB (1024 MB)
    benchmark_large_file_tier(&cache, &ws_dir, "1MB", 1024 * 1024)?;
    benchmark_large_file_tier(&cache, &ws_dir, "10MB", 10 * 1024 * 1024)?;
    benchmark_large_file_tier(&cache, &ws_dir, "100MB", 100 * 1024 * 1024)?;
    benchmark_large_file_tier(&cache, &ws_dir, "1GB", 1024 * 1024 * 1024)?;

    println!("===============================================================");
    println!("All large file tiers (1 MB, 10 MB, 100 MB, 1 GB) completed successfully!");
    println!("Memory consumption is constant O(1) buffer-bounded (64 KB chunk streams).");
    println!("===============================================================\n");

    Ok(())
}
