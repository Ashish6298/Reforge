use dcc_core::{
    CacheEntry, CacheError, CacheKey, CachePolicy, CanonicalComputation, Computation, Digest,
    ExecutionMetadata, MissReason, OutputManifestItem, Result,
};
use dcc_storage::{CasStorage, ComputationLock};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Duration;

use crate::process::ProcessExecutor;
use crate::restore::OutputRestorer;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionStatus {
    Hit,
    Miss,
    Bypassed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub key: CacheKey,
    pub status: ExecutionStatus,
    pub exit_code: i32,
    pub execution_time_ms: u64,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub outputs: Vec<OutputManifestItem>,
    pub miss_reason: Option<MissReason>,
    #[serde(default)]
    pub timings: dcc_core::TimingMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    #[default]
    DoNotCache,
    CacheIfExplicit,
}

#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub policy: CachePolicy,
    pub failure_policy: FailurePolicy,
    pub working_dir: PathBuf,
    pub lock_timeout: Duration,
    pub verbose: bool,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            policy: CachePolicy::ReadWrite,
            failure_policy: FailurePolicy::DoNotCache,
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            lock_timeout: Duration::from_secs(30),
            verbose: false,
        }
    }
}

pub struct RunnerEngine<'a> {
    storage: &'a CasStorage,
    options: EngineOptions,
}

impl<'a> RunnerEngine<'a> {
    pub fn new(storage: &'a CasStorage, options: EngineOptions) -> Self {
        Self { storage, options }
    }

    pub fn storage(&self) -> &'a CasStorage {
        self.storage
    }

    pub fn options(&self) -> &EngineOptions {
        &self.options
    }

    /// Create a runner engine instance directly wrapping a Cache library instance.
    pub fn from_cache(cache: &'a dcc_storage::Cache, options: Option<EngineOptions>) -> Self {
        Self {
            storage: cache.storage(),
            options: options.unwrap_or_default(),
        }
    }

    /// Execute a command specified via structured CommandSpec.
    pub fn execute_command(&self, spec: &crate::command::CommandSpec) -> Result<ExecutionResult> {
        let computation = spec.to_computation("run");
        self.execute(computation)
    }

    pub fn execute(&self, mut computation: Computation) -> Result<ExecutionResult> {
        computation.validate()?;

        if self.options.verbose {
            eprintln!("[INPUT] hashing files");
        }

        // 1. Collect and hash all declared input files
        let mut computed_inputs = Vec::new();
        for input in &computation.inputs {
            let full_input_path = self.options.working_dir.join(&input.path);
            if !full_input_path.exists() {
                return Err(CacheError::ConfigurationError(format!(
                    "Declared input file does not exist: {}",
                    full_input_path.display()
                )));
            }
            let file = File::open(&full_input_path)?;
            let digest = Digest::from_reader(BufReader::new(file))?;
            let size = fs::metadata(&full_input_path)?.len();
            computed_inputs.push(dcc_core::InputFile {
                path: input.path.clone(),
                digest,
                size,
                is_executable: input.is_executable,
            });
        }
        computed_inputs.sort_by(|a, b| a.path.cmp(&b.path));
        computation.inputs = computed_inputs;

        // 2. Canonical serialization and CacheKey generation
        if self.options.verbose {
            eprintln!("[KEY] generating computation key");
        }
        let canonical = CanonicalComputation::from_computation(&computation);
        let key = canonical.compute_key()?;

        // Check cache policy
        if self.options.policy == CachePolicy::Bypass {
            return self.run_and_store(
                key,
                computation,
                false,
                Some(MissReason::ForcedRecompute),
                0,
            );
        }

        if self.options.policy == CachePolicy::ForceRecompute {
            return self.run_and_store(
                key,
                computation,
                true,
                Some(MissReason::ForcedRecompute),
                0,
            );
        }

        // 3. Cache lookup
        if self.options.verbose {
            eprintln!("[LOOKUP] checking cache");
        }
        let lookup_start = std::time::Instant::now();
        if self.options.policy != CachePolicy::WriteOnly {
            match self.storage.get_entry(&key) {
                Ok(Some(entry)) => {
                    let lookup_time_ms = lookup_start.elapsed().as_millis() as u64;

                    // Step 2: Verify cache metadata identity and integrity
                    if let Err(e) = entry.verify_identity() {
                        let _ = self.storage.delete_entry(&key);
                        return self.run_and_store(
                            key,
                            computation,
                            true,
                            Some(MissReason::CorruptedCache {
                                reason: e.to_string(),
                            }),
                            lookup_time_ms,
                        );
                    }

                    // Step 3: Verify all output CAS objects exist and restore outputs safely
                    let restore_start = std::time::Instant::now();
                    match OutputRestorer::restore_entry(
                        self.storage,
                        &entry,
                        &self.options.working_dir,
                    ) {
                        Ok(()) => {
                            let restore_time_ms = restore_start.elapsed().as_millis() as u64;

                            // Step 4: Restore metadata (stdout/stderr streams and execution metadata)
                            let stdout =
                                if let Some(out_digest) = &entry.metadata.execution.stdout_digest {
                                    let mut buf = Vec::new();
                                    if let Ok(mut r) = self.storage.get_object_reader(out_digest) {
                                        let _ = std::io::Read::read_to_end(&mut r, &mut buf);
                                    }
                                    buf
                                } else {
                                    Vec::new()
                                };

                            let stderr =
                                if let Some(err_digest) = &entry.metadata.execution.stderr_digest {
                                    let mut buf = Vec::new();
                                    if let Ok(mut r) = self.storage.get_object_reader(err_digest) {
                                        let _ = std::io::Read::read_to_end(&mut r, &mut buf);
                                    }
                                    buf
                                } else {
                                    Vec::new()
                                };

                            // Step 5 & 6: Report HIT with timing breakdown
                            let timings = dcc_core::TimingMetrics {
                                execution_time_ms: entry.metadata.execution.execution_time_ms,
                                lookup_time_ms,
                                restore_time_ms,
                                store_time_ms: entry.metadata.execution.timings.store_time_ms,
                            };

                            return Ok(ExecutionResult {
                                key,
                                status: ExecutionStatus::Hit,
                                exit_code: entry.metadata.execution.exit_code,
                                execution_time_ms: 0,
                                stdout,
                                stderr,
                                outputs: entry.outputs,
                                miss_reason: None,
                                timings,
                            });
                        }
                        Err(e) => {
                            // Integrity failure: fall through to recompute
                            let _ = self.storage.delete_entry(&key);
                            return self.run_and_store(
                                key,
                                computation,
                                true,
                                Some(MissReason::CorruptedCache {
                                    reason: e.to_string(),
                                }),
                                lookup_time_ms,
                            );
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    let lookup_time_ms = lookup_start.elapsed().as_millis() as u64;
                    let _ = self.storage.delete_entry(&key);
                    return self.run_and_store(
                        key,
                        computation,
                        true,
                        Some(MissReason::CorruptedCache {
                            reason: e.to_string(),
                        }),
                        lookup_time_ms,
                    );
                }
            }
        }

        let lookup_time_ms = lookup_start.elapsed().as_millis() as u64;

        // Cache MISS -> Acquire lock and execute
        self.run_and_store(
            key,
            computation,
            true,
            Some(MissReason::NoEntryFound),
            lookup_time_ms,
        )
    }

    fn run_and_store(
        &self,
        key: CacheKey,
        computation: Computation,
        should_store: bool,
        miss_reason: Option<MissReason>,
        lookup_time_ms: u64,
    ) -> Result<ExecutionResult> {
        // Concurrency Lock
        let _lock =
            ComputationLock::acquire(&self.storage.locks_dir(), &key, self.options.lock_timeout)?;

        // Re-check cache in case another process completed it while waiting for lock
        if self.options.policy != CachePolicy::WriteOnly
            && self.options.policy != CachePolicy::ForceRecompute
        {
            if let Some(entry) = self.storage.get_entry(&key)? {
                if entry.verify_identity().is_ok()
                    && OutputRestorer::restore_entry(
                        self.storage,
                        &entry,
                        &self.options.working_dir,
                    )
                    .is_ok()
                {
                    let stdout = if let Some(out_digest) = &entry.metadata.execution.stdout_digest {
                        let mut buf = Vec::new();
                        if let Ok(mut r) = self.storage.get_object_reader(out_digest) {
                            let _ = std::io::Read::read_to_end(&mut r, &mut buf);
                        }
                        buf
                    } else {
                        Vec::new()
                    };

                    let stderr = if let Some(err_digest) = &entry.metadata.execution.stderr_digest {
                        let mut buf = Vec::new();
                        if let Ok(mut r) = self.storage.get_object_reader(err_digest) {
                            let _ = std::io::Read::read_to_end(&mut r, &mut buf);
                        }
                        buf
                    } else {
                        Vec::new()
                    };

                    let timings = dcc_core::TimingMetrics {
                        execution_time_ms: entry.metadata.execution.execution_time_ms,
                        lookup_time_ms,
                        restore_time_ms: 0,
                        store_time_ms: entry.metadata.execution.timings.store_time_ms,
                    };

                    return Ok(ExecutionResult {
                        key,
                        status: ExecutionStatus::Hit,
                        exit_code: entry.metadata.execution.exit_code,
                        execution_time_ms: 0,
                        stdout,
                        stderr,
                        outputs: entry.outputs,
                        miss_reason: None,
                        timings,
                    });
                }
            }
        }

        // Cache MISS -> Execute process
        if self.options.verbose {
            eprintln!("[MISS] no entry");
            eprintln!("[EXEC] running command");
        }

        // Run process
        let proc_output = ProcessExecutor::execute(
            &computation.command,
            &computation.args,
            &computation.env,
            Some(&self.options.working_dir),
        )?;

        if proc_output.exit_code != 0 {
            // By default, do not cache failed computations
            let timings = dcc_core::TimingMetrics {
                execution_time_ms: proc_output.execution_time_ms,
                lookup_time_ms,
                restore_time_ms: 0,
                store_time_ms: 0,
            };

            return Ok(ExecutionResult {
                key,
                status: ExecutionStatus::Miss,
                exit_code: proc_output.exit_code,
                execution_time_ms: proc_output.execution_time_ms,
                stdout: proc_output.stdout,
                stderr: proc_output.stderr,
                outputs: Vec::new(),
                miss_reason,
                timings,
            });
        }

        // Validate and store outputs
        if self.options.verbose {
            eprintln!("[OUTPUT] validating outputs");
        }
        let mut manifest_items = Vec::new();
        for output in &computation.outputs {
            let full_out_path = self.options.working_dir.join(&output.path);
            if !full_out_path.exists() {
                if output.required {
                    return Err(CacheError::MissingOutput(format!(
                        "Declared required output file not found: {}",
                        full_out_path.display()
                    )));
                }
                continue;
            }

            let (digest, size) = self.storage.store_object_from_file(&full_out_path)?;
            manifest_items.push(OutputManifestItem {
                path: output.path.clone(),
                digest,
                size,
                is_executable: None,
            });
        }

        // Store stdout and stderr as CAS objects
        let stdout_digest = if !proc_output.stdout.is_empty() {
            Some(self.storage.store_object_bytes(&proc_output.stdout)?.0)
        } else {
            None
        };

        let stderr_digest = if !proc_output.stderr.is_empty() {
            Some(self.storage.store_object_bytes(&proc_output.stderr)?.0)
        } else {
            None
        };

        let store_start = std::time::Instant::now();
        if should_store && self.options.policy != CachePolicy::ReadOnly {
            if self.options.verbose {
                eprintln!("[STORE] writing objects");
            }
            let entry = CacheEntry::new(
                key.clone(),
                computation,
                manifest_items.clone(),
                ExecutionMetadata {
                    exit_code: proc_output.exit_code,
                    execution_time_ms: proc_output.execution_time_ms,
                    stdout_digest,
                    stderr_digest,
                    timings: dcc_core::TimingMetrics {
                        execution_time_ms: proc_output.execution_time_ms,
                        lookup_time_ms,
                        restore_time_ms: 0,
                        store_time_ms: 0,
                    },
                },
            );
            self.storage.store_entry(&entry)?;
            if self.options.verbose {
                eprintln!("[DONE] stored result");
            }
        }
        let store_time_ms = store_start.elapsed().as_millis() as u64;

        let timings = dcc_core::TimingMetrics {
            execution_time_ms: proc_output.execution_time_ms,
            lookup_time_ms,
            restore_time_ms: 0,
            store_time_ms,
        };

        Ok(ExecutionResult {
            key,
            status: ExecutionStatus::Miss,
            exit_code: proc_output.exit_code,
            execution_time_ms: proc_output.execution_time_ms,
            stdout: proc_output.stdout,
            stderr: proc_output.stderr,
            outputs: manifest_items,
            miss_reason,
            timings,
        })
    }
}
