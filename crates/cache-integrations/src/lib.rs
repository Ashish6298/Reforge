use dcc_core::{Computation, Digest, Result};
use dcc_runner::{EngineOptions, ExecutionResult, RunnerEngine};
use dcc_storage::CasStorage;
use std::path::Path;

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

    pub fn run_codegen(
        &self,
        schema_path: &str,
        output_path: &str,
        generator_cmd: &str,
        extra_args: &[String],
    ) -> Result<ExecutionResult> {
        let mut args = vec![schema_path.to_string(), "-o".to_string(), output_path.to_string()];
        args.extend_from_slice(extra_args);

        let comp = Computation::builder("codegen", generator_cmd)
            .args(args)
            .input(schema_path, Digest::from_bytes(b""), 0)
            .output(output_path, true)
            .build()?;

        self.engine.execute(comp)
    }
}
