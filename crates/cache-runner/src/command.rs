use dcc_core::{CachePolicy, InputFile, OutputFile, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Structured command specification for computation execution and caching.
///
/// Ensures direct process execution without insecure shell string concatenation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub executable: String,
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub inputs: Vec<InputFile>,
    pub outputs: Vec<OutputFile>,
    pub cache_policy: CachePolicy,
}

impl CommandSpec {
    pub fn builder(executable: impl Into<String>) -> CommandSpecBuilder {
        CommandSpecBuilder::new(executable)
    }

    /// Convert the CommandSpec into a Computation model for key generation and caching.
    pub fn to_computation(&self, operation: impl Into<String>) -> dcc_core::Computation {
        let mut comp = dcc_core::Computation::builder(operation, &self.executable)
            .args(self.arguments.clone())
            .policy(self.cache_policy);

        for input in &self.inputs {
            comp = comp.input(&input.path, input.digest.clone(), input.size);
        }

        for output in &self.outputs {
            comp = comp.output(&output.path, output.required);
        }

        for (k, v) in &self.environment {
            comp = comp.env(k, v);
        }

        comp.build().unwrap_or_else(|_| dcc_core::Computation {
            schema_version: dcc_core::Computation::CURRENT_SCHEMA_VERSION,
            operation: "command".to_string(),
            command: self.executable.clone(),
            args: self.arguments.clone(),
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            env: self.environment.clone(),
            platform: dcc_core::PlatformConstraints::default(),
            tool: None,
            policy: self.cache_policy,
            metadata: BTreeMap::new(),
            working_dir: Some(self.working_directory.to_string_lossy().to_string()),
        })
    }
}

#[derive(Debug, Clone)]
pub struct CommandSpecBuilder {
    executable: String,
    arguments: Vec<String>,
    working_directory: PathBuf,
    environment: BTreeMap<String, String>,
    inputs: Vec<InputFile>,
    outputs: Vec<OutputFile>,
    cache_policy: CachePolicy,
}

impl CommandSpecBuilder {
    pub fn new(executable: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
            arguments: Vec::new(),
            working_directory: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            environment: BTreeMap::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            cache_policy: CachePolicy::ReadWrite,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.arguments.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for arg in args {
            self.arguments.push(arg.into());
        }
        self
    }

    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_directory = dir.into();
        self
    }

    pub fn env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.environment.insert(key.into(), val.into());
        self
    }

    pub fn envs<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (k, v) in vars {
            self.environment.insert(k.into(), v.into());
        }
        self
    }

    pub fn input(mut self, path: impl Into<String>, digest: dcc_core::Digest, size: u64) -> Self {
        self.inputs.push(InputFile {
            path: path.into(),
            digest,
            size,
            is_executable: None,
        });
        self
    }

    /// Declare an input file by relative path. Its cryptographic digest and size will be computed automatically prior to lookup.
    pub fn input_path(mut self, path: impl Into<String>) -> Self {
        let dummy = dcc_core::Digest::from_bytes(b"");
        self.inputs.push(InputFile {
            path: path.into(),
            digest: dummy,
            size: 0,
            is_executable: None,
        });
        self
    }

    /// Declare multiple input files by relative paths.
    pub fn input_paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for p in paths {
            self = self.input_path(p);
        }
        self
    }

    pub fn output(mut self, path: impl Into<String>, required: bool) -> Self {
        self.outputs.push(OutputFile {
            path: path.into(),
            required,
        });
        self
    }

    /// Declare a required output file by relative path.
    pub fn output_path(mut self, path: impl Into<String>) -> Self {
        self.outputs.push(OutputFile {
            path: path.into(),
            required: true,
        });
        self
    }

    /// Declare multiple required output files by relative paths.
    pub fn output_paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for p in paths {
            self = self.output_path(p);
        }
        self
    }

    /// Declare an optional output file by relative path.
    pub fn output_optional(mut self, path: impl Into<String>) -> Self {
        self.outputs.push(OutputFile {
            path: path.into(),
            required: false,
        });
        self
    }

    pub fn policy(mut self, policy: CachePolicy) -> Self {
        self.cache_policy = policy;
        self
    }

    pub fn build(self) -> Result<CommandSpec> {
        if self.executable.trim().is_empty() {
            return Err(dcc_core::CacheError::ValidationError(
                "Executable path cannot be empty".into(),
            ));
        }

        Ok(CommandSpec {
            executable: self.executable,
            arguments: self.arguments,
            working_directory: self.working_directory,
            environment: self.environment,
            inputs: self.inputs,
            outputs: self.outputs,
            cache_policy: self.cache_policy,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcc_core::Digest;

    #[test]
    fn test_command_spec_builder() {
        let dummy_digest = Digest::from_bytes(b"content");
        let spec = CommandSpec::builder("rustc")
            .arg("main.rs")
            .arg("--crate-type=bin")
            .current_dir("./workspace")
            .env("RUST_LOG", "debug")
            .input("src/main.rs", dummy_digest, 120)
            .output("target/main.exe", true)
            .policy(CachePolicy::ReadOnly)
            .build()
            .unwrap();

        assert_eq!(spec.executable, "rustc");
        assert_eq!(spec.arguments, vec!["main.rs", "--crate-type=bin"]);
        assert_eq!(spec.working_directory, PathBuf::from("./workspace"));
        assert_eq!(spec.environment.get("RUST_LOG").unwrap(), "debug");
        assert_eq!(spec.inputs.len(), 1);
        assert_eq!(spec.outputs.len(), 1);
        assert_eq!(spec.cache_policy, CachePolicy::ReadOnly);

        let comp = spec.to_computation("compile");
        assert_eq!(comp.command, "rustc");
        assert_eq!(comp.operation, "compile");
        assert_eq!(comp.args, vec!["main.rs", "--crate-type=bin"]);
    }

    #[test]
    fn test_command_spec_empty_executable_fails() {
        let res = CommandSpec::builder("").build();
        assert!(res.is_err());
    }

    #[test]
    fn test_input_declaration_multiple_paths() {
        let spec = CommandSpec::builder("cargo")
            .arg("build")
            .input_path("src/main.rs")
            .input_path("src/lib.rs")
            .input_path("Cargo.toml")
            .output("target/debug/app.exe", true)
            .build()
            .unwrap();

        assert_eq!(spec.inputs.len(), 3);
        assert_eq!(spec.inputs[0].path, "src/main.rs");
        assert_eq!(spec.inputs[1].path, "src/lib.rs");
        assert_eq!(spec.inputs[2].path, "Cargo.toml");

        // Array / iterator convenience helper
        let spec2 = CommandSpec::builder("cargo")
            .arg("build")
            .input_paths(vec!["src/main.rs", "src/lib.rs", "Cargo.toml"])
            .build()
            .unwrap();

        assert_eq!(spec2.inputs.len(), 3);
        assert_eq!(spec.inputs, spec2.inputs);
    }

    #[test]
    fn test_output_declaration_helpers() {
        let spec = CommandSpec::builder("node")
            .arg("build.js")
            .output_path("dist/app.js")
            .output_path("dist/app.js.map")
            .output_optional("dist/stats.json")
            .build()
            .unwrap();

        assert_eq!(spec.outputs.len(), 3);
        assert_eq!(spec.outputs[0].path, "dist/app.js");
        assert!(spec.outputs[0].required);
        assert_eq!(spec.outputs[1].path, "dist/app.js.map");
        assert!(spec.outputs[1].required);
        assert_eq!(spec.outputs[2].path, "dist/stats.json");
        assert!(!spec.outputs[2].required);

        let spec2 = CommandSpec::builder("node")
            .arg("build.js")
            .output_paths(vec!["dist/app.js", "dist/app.js.map"])
            .build()
            .unwrap();

        assert_eq!(spec2.outputs.len(), 2);
        assert!(spec2.outputs.iter().all(|o| o.required));
    }
}
