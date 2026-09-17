use dcc_integrations::{BuildAction, Cache, GenericIntegration};
use std::fs;
use tempfile::tempdir;

fn main() -> anyhow::Result<()> {
    println!("===============================================================");
    println!("=== DCC MILESTONE 11.4: BUILD CACHE BENCHMARK SUITE ===");
    println!("===============================================================\n");

    // 1. Setup Benchmark Environment
    let temp_cache = tempdir()?;
    let temp_work = tempdir()?;
    let cache_dir = temp_cache.path().join(".dcc_cache");
    let ws_dir = temp_work.path().join("benchmark_suite");
    fs::create_dir_all(&ws_dir)?;

    let cache = Cache::open(&cache_dir)?;
    let integration = GenericIntegration::from_cache(&cache, &ws_dir);

    // 2. Setup Realistic Multi-File Project
    let src_dir = ws_dir.join("src");
    fs::create_dir_all(&src_dir)?;

    let lib_file = src_dir.join("lib.rs");
    let main_file = src_dir.join("main.rs");
    let out_bin = ws_dir.join("engine_bench.exe");

    fs::write(
        &lib_file,
        r#"pub struct ComputationEngine {
    pub threads: usize,
    pub active: bool,
}

impl ComputationEngine {
    pub fn new(threads: usize) -> Self {
        Self { threads, active: true }
    }
    pub fn process(&self, data: &[u8]) -> usize {
        data.iter().map(|b| *b as usize).sum()
    }
}
"#,
    )?;

    fs::write(
        &main_file,
        r#"mod lib;
use lib::ComputationEngine;

fn main() {
    let engine = ComputationEngine::new(8);
    let payload = vec![1, 2, 3, 4, 5];
    let sum = engine.process(&payload);
    println!("Computed payload sum: {}", sum);
}
"#,
    )?;

    // 3. Setup Mock Compiler with simulated work
    #[cfg(windows)]
    let (compiler_cmd, compiler_args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            format!(
                "Start-Sleep -Milliseconds 50; Set-Content -Path '{}' -Value 'benchmark_compiled_executable_binary_payload'",
                out_bin.display()
            ),
        ],
    );
    #[cfg(not(windows))]
    let (compiler_cmd, compiler_args) = (
        "sh",
        vec![
            "-c".to_string(),
            format!(
                "sleep 0.05; echo 'benchmark_compiled_executable_binary_payload' > '{}'",
                out_bin.display()
            ),
        ],
    );

    let action = BuildAction::builder()
        .compiler(compiler_cmd)
        .arguments(compiler_args)
        .source_file(&lib_file)?
        .source_file(&main_file)?
        .compiler_version("rustc 1.80.0-nightly")
        .target("x86_64-benchmark-platform")
        .env("RUSTFLAGS", "-C opt-level=3 -C target-cpu=native")
        .output("engine_bench.exe", true)
        .build()?;

    println!("Executing Benchmark Measurements:\n");
    println!("  [1] Cold Build Execution (Cache MISS -> Process Compile -> CAS Storage)");
    println!("  [2] Warm Build WITHOUT Cache (Bypass Cache -> Force Re-compilation)");
    println!("  [3] Warm Build WITH Cache (Cache HIT -> Instant Metadata Lookup & CAS Restore)\n");

    let metrics = integration.run_benchmark(action)?;

    println!("===============================================================");
    println!("=== MILESTONE 11.4 BENCHMARK REPORT ===");
    println!("===============================================================");
    println!("Measurements:");
    println!(
        "  Cold Build Time:                 {} ms",
        metrics.cold_build_time_ms
    );
    println!(
        "  Warm Build (Without Cache) Time: {} ms",
        metrics.warm_build_without_cache_time_ms
    );
    println!(
        "  Warm Build (With Cache) Time:    {} ms",
        metrics.warm_build_with_cache_time_ms
    );
    println!(
        "  Cache Lookup Time:               {} ms",
        metrics.cache_lookup_time_ms
    );
    println!(
        "  Artifact Restore Time:           {} ms",
        metrics.restore_time_ms
    );
    println!(
        "  Storage Usage:                   {} bytes ({:.2} KB)",
        metrics.storage_size_bytes,
        metrics.storage_size_bytes as f64 / 1024.0
    );
    println!("  Effective Build Speedup:         {:.2}x", metrics.speedup);
    println!("===============================================================\n");

    println!("Benchmark completed successfully!");
    Ok(())
}
