use dcc_core::{CacheError, Result};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub execution_time_ms: u64,
}

pub struct ProcessExecutor;

impl ProcessExecutor {
    pub fn execute(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        working_dir: Option<&Path>,
    ) -> Result<ProcessOutput> {
        let mut cmd = Command::new(command);
        cmd.args(args);

        for (k, v) in env {
            cmd.env(k, v);
        }

        if let Some(wd) = working_dir {
            cmd.current_dir(wd);
        }

        let start = Instant::now();
        let output = cmd.output().map_err(|e| {
            CacheError::ConfigurationError(format!(
                "Failed to execute process '{}': {}",
                command, e
            ))
        })?;
        let elapsed = start.elapsed().as_millis() as u64;

        let exit_code = output.status.code().unwrap_or(-1);

        Ok(ProcessOutput {
            exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
            execution_time_ms: elapsed,
        })
    }
}
