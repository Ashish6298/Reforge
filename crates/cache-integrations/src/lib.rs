//! # DCC Generic Developer Integration API & Contract
//!
//! This crate defines the generic developer integration contract for embedding
//! content-addressed computation caching into custom build systems, code generators,
//! linters, and data transformation tools.
//!
//! ## Integration Contract
//!
//! ### 1. How to Declare Inputs
//! Inputs represent all filesystem dependencies required to produce the computation outputs.
//! Inputs must be declared with relative paths (or normalized workspace paths) and their content digests:
//! ```rust,ignore
//! let comp = Computation::builder()
//!     .operation("codegen")
//!     .command("generator")
//!     .input("schema.json", Digest::hash_file(Path::new("schema.json"))?, file_size)
//!     // Or automatically hash and resolve file metadata:
//!     .input_path(Path::new("schema.json"))?
//!     .build()?;
//! ```
//!
//! ### 2. How to Declare Outputs
//! Outputs represent files or artifacts that the computation creates or modifies in the workspace:
//! ```rust,ignore
//! let comp = Computation::builder()
//!     .operation("codegen")
//!     .command("generator")
//!     .output("models.rs", true) // required output
//!     .build()?;
//! ```
//! When a cache HIT occurs, all declared outputs are atomically restored from the CAS store.
//!
//! ### 3. How to Declare Environment
//! Environment variables that influence the computation must be explicitly declared in the computation model:
//! ```rust,ignore
//! let comp = Computation::builder()
//!     .operation("build")
//!     .command("compiler")
//!     .env("TARGET", "x86_64-unknown-linux-gnu")
//!     // Or capture from the ambient process environment if present:
//!     .declared_env("RUST_LOG")
//!     .build()?;
//! ```
//! Undeclared environment variables in the host OS do NOT participate in cache key generation,
//! preventing accidental cache fragmentation.
//!
//! ### 4. How Cache Identity Works
//! Cache keys (`CacheKey`) are computed deterministically from a canonical representation of the computation:
//! - Canonical JSON serialization with strict alphabetical key ordering.
//! - SHA-256 cryptographic digest over all declared inputs, command, arguments, environment, platform, and tool identity.
//! - Path normalization: slashes are converted to forward slashes (`/`), and timestamp-invariance is preserved.
//!
//! ### 5. How Errors Work
//! - **Configuration & Validation Errors**: Detected before execution (e.g. empty command, path traversals).
//! - **Execution Failures**: Non-zero exit codes fail the execution. By default (`FailurePolicy::DoNotCache`),
//!   failed computations are NOT cached.
//! - **Corruption Errors**: If a CAS object fails integrity verification, DCC safely falls back to re-executing
//!   the computation and quarantines the bad object.
//!
//! ### 6. How Cache Misses Work
//! When a computation key is not found in the cache (or its entry has been evicted), a cold MISS occurs:
//! 1. The engine executes the process in the workspace.
//! 2. Declared outputs and stdout/stderr are captured, hashed, and stored into CAS.
//! 3. A new `CacheEntry` record is atomically written to the `entries/` directory.
//!
//! When running with `--explain` or inspecting via `MissExplainer`, the engine details exactly *why* a miss occurred
//! (e.g., specific input changed, command argument modified, or platform mismatch).
//!
//! ### 7. How to Disable or Bypass Caching
//! Caching can be controlled at runtime via `CachePolicy`:
//! - `CachePolicy::ReadWrite` (Default): Normal caching (lookup on start, store on miss).
//! - `CachePolicy::ReadOnly`: Only read from cache; do not store new results.
//! - `CachePolicy::WriteOnly`: Always execute; overwrite/store new results in cache.
//! - `CachePolicy::Bypass`: Completely bypass cache lookup and storage.
//! - `CachePolicy::ForceRecompute`: Force process re-execution but update the cache entry with new results.

pub use dcc_core::{
    BuildAction, BuildActionBuilder, ByteSize, CacheEntry, CacheError, CacheKey, CacheMetadata,
    CachePolicy, CacheResult, Computation, ComputationBuilder, Digest, EventKind, EventSubscriber,
    ExecutionMetadata, InputFile, OutputFile, OutputManifest, OutputManifestItem,
    PlatformConstraints, Result, StructuredEvent, TimingMetrics, ToolIdentity,
};
pub use dcc_runner::{
    CommandSpec, CommandSpecBuilder, EngineOptions, ExecutionResult, ExecutionStatus,
    FailurePolicy, MissExplainer, ProcessExecutor, ProcessOutput, RunnerEngine,
};
pub use dcc_storage::{
    BlobMetadata, Cache, CasStorage, EvictionPolicy, EvictionResult, EvictionStrategy, Pruner,
    Storage, StorageConfig, StorageStats, VerifyResult,
};

use std::path::Path;

/// High-level generic integration helper for embedding DCC caching in external tools, linters, and generators.
pub struct GenericIntegration<'a> {
    engine: RunnerEngine<'a>,
}

impl<'a> GenericIntegration<'a> {
    pub fn new(storage: &'a CasStorage, working_dir: &Path) -> Self {
        Self {
            engine: RunnerEngine::new(
                storage,
                EngineOptions {
                    working_dir: working_dir.to_path_buf(),
                    ..Default::default()
                },
            ),
        }
    }

    pub fn from_cache(cache: &'a Cache, working_dir: &Path) -> Self {
        Self::new(cache.storage(), working_dir)
    }

    pub fn execute(&self, computation: Computation) -> Result<ExecutionResult> {
        self.engine.execute(computation)
    }

    /// Execute a compiler `BuildAction` model using the caching runner engine.
    pub fn execute_build_action(&self, action: BuildAction) -> Result<ExecutionResult> {
        let comp = action.to_computation()?;
        self.engine.execute(comp)
    }

    pub fn run_codegen(
        &self,
        schema_path: &str,
        output_path: &str,
        generator_cmd: &str,
        extra_args: &[String],
    ) -> Result<ExecutionResult> {
        let mut args = vec![
            schema_path.to_string(),
            "-o".to_string(),
            output_path.to_string(),
        ];
        args.extend_from_slice(extra_args);

        let comp = Computation::builder()
            .operation("codegen")
            .command(generator_cmd)
            .args(args)
            .input(schema_path, Digest::from_bytes(b""), 0)
            .output(output_path, true)
            .build()?;

        self.engine.execute(comp)
    }

    /// Compile a Rust source artifact using `rustc` caching with explicit tool identity,
    /// source files, and compiler arguments (Milestone 11.2).
    pub fn run_rust_build(
        &self,
        source_path: &str,
        output_path: &str,
        opt_level: Option<&str>,
        extra_flags: &[String],
        rustc_path: Option<&str>,
    ) -> Result<ExecutionResult> {
        let compiler = rustc_path.unwrap_or("rustc");
        let mut args = vec![
            source_path.to_string(),
            "-o".to_string(),
            output_path.to_string(),
        ];
        if let Some(opt) = opt_level {
            args.push(format!("-Copt-level={}", opt));
        }
        args.extend_from_slice(extra_flags);

        let action = BuildAction::builder()
            .compiler(compiler)
            .arguments(args)
            .source_input(source_path, Digest::from_bytes(b""), 0)
            .output(output_path, true)
            .build()?;

        self.execute_build_action(action)
    }

    /// Execute a comprehensive build benchmark measuring cold build, warm build without cache,
    /// and warm build with cache, reporting execution time, lookup time, restore time, storage size, and speedup (Milestone 11.4).
    pub fn run_benchmark(&self, action: BuildAction) -> Result<BenchmarkMetrics> {
        let comp = action.to_computation()?;

        // 1. Cold Build (With Cache -> Expect MISS)
        let cold_res = self.engine.execute(comp.clone())?;

        // 2. Warm Build WITHOUT Cache (Bypass Cache -> Force recompilation)
        let bypass_engine = RunnerEngine::new(
            self.engine.storage(),
            EngineOptions {
                working_dir: self.engine.options().working_dir.clone(),
                policy: CachePolicy::Bypass,
                ..Default::default()
            },
        );
        let warm_nocache_res = bypass_engine.execute(comp.clone())?;

        // 3. Warm Build WITH Cache (Normal ReadWrite -> Expect HIT)
        let warm_cached_res = self.engine.execute(comp)?;

        let storage_stats = StorageStats::collect(self.engine.storage())?;
        let storage_size_bytes = storage_stats.total_size_bytes;

        let cold_time_ms = cold_res
            .timings
            .execution_time_ms
            .max(cold_res.execution_time_ms);
        let warm_nocache_time_ms = warm_nocache_res
            .timings
            .execution_time_ms
            .max(warm_nocache_res.execution_time_ms);
        let warm_cached_time_ms =
            warm_cached_res.timings.lookup_time_ms + warm_cached_res.timings.restore_time_ms;

        let speedup = if warm_cached_time_ms > 0 {
            warm_nocache_time_ms as f64 / warm_cached_time_ms as f64
        } else {
            warm_nocache_time_ms.max(1) as f64
        };

        Ok(BenchmarkMetrics {
            cold_build_time_ms: cold_time_ms,
            warm_build_without_cache_time_ms: warm_nocache_time_ms,
            warm_build_with_cache_time_ms: warm_cached_time_ms,
            cache_lookup_time_ms: warm_cached_res.timings.lookup_time_ms,
            restore_time_ms: warm_cached_res.timings.restore_time_ms,
            storage_size_bytes,
            speedup,
        })
    }
}

/// Comprehensive benchmark metrics report (Milestone 11.4).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkMetrics {
    pub cold_build_time_ms: u64,
    pub warm_build_without_cache_time_ms: u64,
    pub warm_build_with_cache_time_ms: u64,
    pub cache_lookup_time_ms: u64,
    pub restore_time_ms: u64,
    pub storage_size_bytes: u64,
    pub speedup: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_milestone_10_1_generic_integration_api() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().join(".cache");
        let ws_dir = temp_dir.path().join("ws");
        std::fs::create_dir_all(&ws_dir).unwrap();

        // 1. Cache::open
        let cache = Cache::open(&cache_dir).unwrap();

        // 2. Prepare mock input file
        let src_file = ws_dir.join("input.txt");
        let out_file = ws_dir.join("output.txt");
        std::fs::write(&src_file, b"sample data for codegen").unwrap();

        // 3. GenericIntegration::from_cache
        let integration = GenericIntegration::from_cache(&cache, &ws_dir);

        #[cfg(windows)]
        let (cmd, args) = (
            "powershell.exe",
            vec![
                "-Command".to_string(),
                format!(
                    "Copy-Item '{}' -Destination '{}'",
                    src_file.display(),
                    out_file.display()
                ),
            ],
        );
        #[cfg(not(windows))]
        let (cmd, args) = (
            "cp",
            vec![
                src_file.to_str().unwrap().to_string(),
                out_file.to_str().unwrap().to_string(),
            ],
        );

        // 4. ComputationBuilder fluent API
        let comp = Computation::builder()
            .operation("codegen")
            .command(cmd)
            .args(args)
            .input(
                "input.txt",
                Digest::from_bytes(b"sample data for codegen"),
                23,
            )
            .output("output.txt", true)
            .build()
            .unwrap();

        // 5. Execute computation (Cold MISS)
        let res1 = integration.execute(comp.clone()).unwrap();
        assert_eq!(res1.status, ExecutionStatus::Miss);
        assert!(out_file.is_file());

        // Delete generated output file
        std::fs::remove_file(&out_file).unwrap();
        assert!(!out_file.exists());

        // 6. Execute computation (Warm HIT)
        let res2 = integration.execute(comp).unwrap();
        assert_eq!(res2.status, ExecutionStatus::Hit);
        assert!(out_file.is_file());
        assert_eq!(
            std::fs::read(&out_file).unwrap(),
            b"sample data for codegen"
        );
    }

    #[test]
    fn test_milestone_11_1_build_action_integration() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().join(".cache");
        let ws_dir = temp_dir.path().join("ws");
        std::fs::create_dir_all(&ws_dir).unwrap();

        let cache = Cache::open(&cache_dir).unwrap();
        let integration = GenericIntegration::from_cache(&cache, &ws_dir);

        let src_file = ws_dir.join("main.rs");
        let dep_file = ws_dir.join("libcore.rlib");
        let out_file = ws_dir.join("app.exe");

        std::fs::write(&src_file, b"fn main() { println!(\"hello\"); }").unwrap();
        std::fs::write(&dep_file, b"libcore_binary_data").unwrap();

        #[cfg(windows)]
        let (cmd, args) = (
            "powershell.exe",
            vec![
                "-Command".to_string(),
                format!(
                    "Set-Content -Path '{}' -Value 'binary_executable_output'",
                    out_file.display()
                ),
            ],
        );
        #[cfg(not(windows))]
        let (cmd, args) = (
            "sh",
            vec![
                "-c".to_string(),
                format!("echo 'binary_executable_output' > '{}'", out_file.display()),
            ],
        );

        let action = BuildAction::builder()
            .compiler(cmd)
            .arguments(args)
            .source_input("main.rs", Digest::hash_file(&src_file).unwrap(), 34)
            .dependency_input("libcore.rlib", Digest::hash_file(&dep_file).unwrap(), 19)
            .compiler_version("1.80.0")
            .target("x86_64-pc-windows-msvc")
            .env("OPT_LEVEL", "3")
            .output("app.exe", true)
            .build()
            .unwrap();

        // 1. Cold Build Action MISS
        let res1 = integration.execute_build_action(action.clone()).unwrap();
        assert_eq!(res1.status, ExecutionStatus::Miss);
        assert!(out_file.is_file());

        // Delete compiled binary
        std::fs::remove_file(&out_file).unwrap();
        assert!(!out_file.exists());

        // 2. Warm Build Action HIT
        let res2 = integration.execute_build_action(action).unwrap();
        assert_eq!(res2.status, ExecutionStatus::Hit);
        assert!(out_file.is_file());
    }

    #[test]
    fn test_milestone_11_2_rust_build_integration() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().join(".cache");
        let ws_dir = temp_dir.path().join("ws");
        std::fs::create_dir_all(&ws_dir).unwrap();

        let cache = Cache::open(&cache_dir).unwrap();
        let integration = GenericIntegration::from_cache(&cache, &ws_dir);

        let src_file = ws_dir.join("lib.rs");
        let out_file = ws_dir.join("libcalc.rlib");
        std::fs::write(&src_file, b"pub fn add(a: i32, b: i32) -> i32 { a + b }").unwrap();

        #[cfg(windows)]
        let (cmd, args) = (
            "powershell.exe",
            vec![
                "-Command".to_string(),
                format!(
                    "Set-Content -Path '{}' -Value 'rlib_compiled_binary_payload'",
                    out_file.display()
                ),
            ],
        );
        #[cfg(not(windows))]
        let (cmd, args) = (
            "sh",
            vec![
                "-c".to_string(),
                format!(
                    "echo 'rlib_compiled_binary_payload' > '{}'",
                    out_file.display()
                ),
            ],
        );

        let action = BuildAction::builder()
            .compiler(cmd)
            .arguments(args)
            .source_input("lib.rs", Digest::hash_file(&src_file).unwrap(), 42)
            .compiler_version("rustc 1.80.0")
            .target("x86_64-pc-windows-msvc")
            .env("RUSTFLAGS", "-C opt-level=2")
            .output("libcalc.rlib", true)
            .build()
            .unwrap();

        // 1. Initial Build Execution -> Expect MISS
        let res1 = integration.execute_build_action(action.clone()).unwrap();
        assert_eq!(res1.status, ExecutionStatus::Miss);
        assert!(out_file.is_file());
        assert_eq!(
            std::fs::read(&out_file).unwrap().trim_ascii(),
            b"rlib_compiled_binary_payload"
        );

        // Delete artifact
        std::fs::remove_file(&out_file).unwrap();
        assert!(!out_file.exists());

        // 2. Second Build Execution -> Expect HIT
        let res2 = integration.execute_build_action(action).unwrap();
        assert_eq!(res2.status, ExecutionStatus::Hit);
        assert!(out_file.is_file());
        assert_eq!(
            std::fs::read(&out_file).unwrap().trim_ascii(),
            b"rlib_compiled_binary_payload"
        );
    }

    #[test]
    fn test_milestone_11_3_build_demonstration_reproducible() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().join(".cache");
        let ws_dir = temp_dir.path().join("ws");
        std::fs::create_dir_all(&ws_dir).unwrap();

        let cache = Cache::open(&cache_dir).unwrap();
        let integration = GenericIntegration::from_cache(&cache, &ws_dir);

        let src_file = ws_dir.join("calc.rs");
        let out_file = ws_dir.join("calc.bin");
        std::fs::write(&src_file, b"pub fn calculate() -> i32 { 42 }").unwrap();

        #[cfg(windows)]
        let (cmd, args) = (
            "powershell.exe",
            vec![
                "-Command".to_string(),
                format!(
                    "Set-Content -Path '{}' -Value 'calc_v1_payload'",
                    out_file.display()
                ),
            ],
        );
        #[cfg(not(windows))]
        let (cmd, args) = (
            "sh",
            vec![
                "-c".to_string(),
                format!("echo 'calc_v1_payload' > '{}'", out_file.display()),
            ],
        );

        let action_1 = BuildAction::builder()
            .compiler(cmd)
            .arguments(args.clone())
            .source_file(&src_file)
            .unwrap()
            .compiler_version("rustc 1.80.0")
            .target("x86_64-pc-windows-msvc")
            .output("calc.bin", true)
            .build()
            .unwrap();

        // 1. Build #1: MISS -> compile -> store
        let res1 = integration.execute_build_action(action_1.clone()).unwrap();
        assert_eq!(res1.status, ExecutionStatus::Miss);
        assert!(out_file.is_file());
        assert_eq!(
            std::fs::read(&out_file).unwrap().trim_ascii(),
            b"calc_v1_payload"
        );

        // Delete output
        std::fs::remove_file(&out_file).unwrap();
        assert!(!out_file.exists());

        // 2. Build #2: HIT -> restore
        let res2 = integration.execute_build_action(action_1.clone()).unwrap();
        assert_eq!(res2.status, ExecutionStatus::Hit);
        assert_eq!(res1.key, res2.key);
        assert!(out_file.is_file());
        assert_eq!(
            std::fs::read(&out_file).unwrap().trim_ascii(),
            b"calc_v1_payload"
        );

        // 3. Modify source file -> Build #3: MISS -> compile -> store
        std::fs::write(&src_file, b"pub fn calculate() -> i32 { 100 }").unwrap();
        #[cfg(windows)]
        let (cmd_v2, args_v2) = (
            "powershell.exe",
            vec![
                "-Command".to_string(),
                format!(
                    "Set-Content -Path '{}' -Value 'calc_v2_payload'",
                    out_file.display()
                ),
            ],
        );
        #[cfg(not(windows))]
        let (cmd_v2, args_v2) = (
            "sh",
            vec![
                "-c".to_string(),
                format!("echo 'calc_v2_payload' > '{}'", out_file.display()),
            ],
        );

        let action_3 = BuildAction::builder()
            .compiler(cmd_v2)
            .arguments(args_v2)
            .source_file(&src_file)
            .unwrap()
            .compiler_version("rustc 1.80.0")
            .target("x86_64-pc-windows-msvc")
            .output("calc.bin", true)
            .build()
            .unwrap();

        let res3 = integration.execute_build_action(action_3).unwrap();
        assert_eq!(res3.status, ExecutionStatus::Miss);
        assert_ne!(res1.key, res3.key);
        assert!(out_file.is_file());
        assert_eq!(
            std::fs::read(&out_file).unwrap().trim_ascii(),
            b"calc_v2_payload"
        );
    }

    #[test]
    fn test_milestone_11_4_benchmark_measurements() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().join(".cache");
        let ws_dir = temp_dir.path().join("ws");
        std::fs::create_dir_all(&ws_dir).unwrap();

        let cache = Cache::open(&cache_dir).unwrap();
        let integration = GenericIntegration::from_cache(&cache, &ws_dir);

        let src_file = ws_dir.join("main.rs");
        let out_file = ws_dir.join("main.exe");
        std::fs::write(&src_file, b"fn main() { println!(\"benchmark\"); }").unwrap();

        #[cfg(windows)]
        let (cmd, args) = (
            "powershell.exe",
            vec![
                "-Command".to_string(),
                format!(
                    "Set-Content -Path '{}' -Value 'benchmark_bin_payload'",
                    out_file.display()
                ),
            ],
        );
        #[cfg(not(windows))]
        let (cmd, args) = (
            "sh",
            vec![
                "-c".to_string(),
                format!("echo 'benchmark_bin_payload' > '{}'", out_file.display()),
            ],
        );

        let action = BuildAction::builder()
            .compiler(cmd)
            .arguments(args)
            .source_file(&src_file)
            .unwrap()
            .compiler_version("rustc 1.80.0")
            .target("x86_64-pc-windows-msvc")
            .output("main.exe", true)
            .build()
            .unwrap();

        let metrics = integration.run_benchmark(action).unwrap();

        assert!(metrics.storage_size_bytes > 0);
        assert!(metrics.speedup >= 1.0 || metrics.warm_build_with_cache_time_ms == 0);
    }
}
