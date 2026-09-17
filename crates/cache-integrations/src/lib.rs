pub use dcc_core::{
    ByteSize, CacheEntry, CacheError, CacheKey, CacheMetadata, CachePolicy, CacheResult,
    Computation, ComputationBuilder, Digest, EventKind, EventSubscriber, ExecutionMetadata,
    InputFile, OutputFile, OutputManifest, OutputManifestItem, PlatformConstraints, Result,
    StructuredEvent, TimingMetrics, ToolIdentity,
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
}
