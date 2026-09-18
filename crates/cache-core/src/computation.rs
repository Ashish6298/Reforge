use crate::digest::Digest;
use crate::entry::CachePolicy;
use crate::error::{CacheError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputFile {
    pub path: String,
    pub digest: Digest,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_executable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputFile {
    pub path: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolIdentity {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<Digest>,
}

impl ToolIdentity {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: None,
            digest: None,
        }
    }

    pub fn with_version(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: Some(version.into()),
            digest: None,
        }
    }

    pub fn with_digest(
        name: impl Into<String>,
        version: Option<String>,
        digest: Option<Digest>,
    ) -> Self {
        Self {
            name: name.into(),
            version,
            digest,
        }
    }

    /// Construct tool identity by reading and hashing the executable binary at `path`.
    pub fn from_executable(
        name: impl Into<String>,
        executable_path: &std::path::Path,
        version: Option<String>,
    ) -> Result<Self> {
        let digest = if executable_path.is_file() {
            let file = std::fs::File::open(executable_path)?;
            Some(Digest::from_reader(std::io::BufReader::new(file))?)
        } else {
            None
        };

        Ok(Self {
            name: name.into(),
            version,
            digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformConstraints {
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiler: Option<String>,
}

impl Default for PlatformConstraints {
    fn default() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            target: None,
            runtime: None,
            abi: None,
            compiler: None,
        }
    }
}

impl PlatformConstraints {
    pub fn new(os: impl Into<String>, arch: impl Into<String>) -> Self {
        Self {
            os: os.into(),
            arch: arch.into(),
            target: None,
            runtime: None,
            abi: None,
            compiler: None,
        }
    }

    pub fn host() -> Self {
        Self::default()
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn with_runtime(mut self, runtime: impl Into<String>) -> Self {
        self.runtime = Some(runtime.into());
        self
    }

    pub fn with_abi(mut self, abi: impl Into<String>) -> Self {
        self.abi = Some(abi.into());
        self
    }

    pub fn with_compiler(mut self, compiler: impl Into<String>) -> Self {
        self.compiler = Some(compiler.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Computation {
    pub schema_version: u32,
    pub operation: String,
    pub command: String,
    pub args: Vec<String>,
    pub inputs: Vec<InputFile>,
    pub outputs: Vec<OutputFile>,
    pub env: BTreeMap<String, String>,
    pub platform: PlatformConstraints,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolIdentity>,
    #[serde(default)]
    pub policy: CachePolicy,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}

impl Computation {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    /// Ergonomic 0-argument builder entry point:
    /// ```rust,ignore
    /// Computation::builder()
    ///     .operation("codegen")
    ///     .command("generator")
    ///     .args(...)
    ///     .input(...)
    ///     .output(...)
    ///     .env(...)
    ///     .build()
    /// ```
    pub fn builder() -> ComputationBuilder {
        ComputationBuilder::default()
    }

    /// Builder entry point with explicit operation and command identifiers:
    pub fn builder_with(
        operation: impl Into<String>,
        command: impl Into<String>,
    ) -> ComputationBuilder {
        ComputationBuilder::new(operation, command)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_with_policy(crate::sensitive::SensitiveDataPolicy::Allow)
    }

    /// Validates the computation model against a specified sensitive data policy (Milestone 14.4).
    pub fn validate_with_policy(
        &self,
        sensitive_policy: crate::sensitive::SensitiveDataPolicy,
    ) -> Result<()> {
        if self.operation.trim().is_empty() {
            return Err(CacheError::ValidationError(
                "Operation identifier cannot be empty".into(),
            ));
        }
        if self.command.trim().is_empty() {
            return Err(CacheError::ValidationError(
                "Command executable cannot be empty".into(),
            ));
        }
        for out in &self.outputs {
            crate::paths::PathUtils::validate_computation_path(&out.path)?;
        }
        for inp in &self.inputs {
            crate::paths::PathUtils::validate_computation_path(&inp.path)?;
        }

        // Sensitive Data Inspection (Milestone 14.4)
        match sensitive_policy {
            crate::sensitive::SensitiveDataPolicy::Deny => {
                for (k, v) in &self.env {
                    if let Some(reason) =
                        crate::sensitive::SensitiveDataDetector::scan_env_var(k, v)
                    {
                        return Err(CacheError::SensitiveDataError(reason));
                    }
                }
                for arg in &self.args {
                    if let Some(reason) =
                        crate::sensitive::SensitiveDataDetector::scan_argument(arg)
                    {
                        return Err(CacheError::SensitiveDataError(reason));
                    }
                }
            }
            crate::sensitive::SensitiveDataPolicy::Warn => {
                for (k, v) in &self.env {
                    if let Some(reason) =
                        crate::sensitive::SensitiveDataDetector::scan_env_var(k, v)
                    {
                        eprintln!("[DCC WARNING - SENSITIVE DATA]: {}", reason);
                    }
                }
                for arg in &self.args {
                    if let Some(reason) =
                        crate::sensitive::SensitiveDataDetector::scan_argument(arg)
                    {
                        eprintln!("[DCC WARNING - SENSITIVE DATA]: {}", reason);
                    }
                }
            }
            crate::sensitive::SensitiveDataPolicy::Allow
            | crate::sensitive::SensitiveDataPolicy::Mask => {}
        }

        Ok(())
    }

    /// Compute the deterministic canonical CacheKey for this computation.
    pub fn compute_key(&self) -> Result<crate::digest::CacheKey> {
        let canonical = crate::canonical::CanonicalComputation::from_computation(self);
        canonical.compute_key()
    }
}

pub struct ComputationBuilder {
    schema_version: u32,
    operation: String,
    command: String,
    args: Vec<String>,
    inputs: Vec<InputFile>,
    outputs: Vec<OutputFile>,
    env: BTreeMap<String, String>,
    platform: PlatformConstraints,
    tool: Option<ToolIdentity>,
    policy: CachePolicy,
    metadata: BTreeMap<String, String>,
    working_dir: Option<String>,
    sensitive_policy: crate::sensitive::SensitiveDataPolicy,
}

impl Default for ComputationBuilder {
    fn default() -> Self {
        Self {
            schema_version: Computation::CURRENT_SCHEMA_VERSION,
            operation: "default_op".to_string(),
            command: "default_cmd".to_string(),
            args: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            env: BTreeMap::new(),
            platform: PlatformConstraints::default(),
            tool: None,
            policy: CachePolicy::ReadWrite,
            metadata: BTreeMap::new(),
            working_dir: None,
            sensitive_policy: crate::sensitive::SensitiveDataPolicy::Allow,
        }
    }
}

impl ComputationBuilder {
    pub fn new(operation: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            schema_version: Computation::CURRENT_SCHEMA_VERSION,
            operation: operation.into(),
            command: command.into(),
            args: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            env: BTreeMap::new(),
            platform: PlatformConstraints::default(),
            tool: None,
            policy: CachePolicy::ReadWrite,
            metadata: BTreeMap::new(),
            working_dir: None,
            sensitive_policy: crate::sensitive::SensitiveDataPolicy::Allow,
        }
    }

    pub fn operation(mut self, op: impl Into<String>) -> Self {
        self.operation = op.into();
        self
    }

    pub fn command(mut self, cmd: impl Into<String>) -> Self {
        self.command = cmd.into();
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn input(mut self, path: impl Into<String>, digest: Digest, size: u64) -> Self {
        let norm_path = path.into().replace('\\', "/");
        self.inputs.push(InputFile {
            path: norm_path,
            digest,
            size,
            is_executable: None,
        });
        self
    }

    pub fn input_file(mut self, input: InputFile) -> Self {
        let mut inp = input;
        inp.path = inp.path.replace('\\', "/");
        self.inputs.push(inp);
        self
    }

    pub fn inputs<I>(mut self, inputs: I) -> Self
    where
        I: IntoIterator<Item = InputFile>,
    {
        for inp in inputs {
            self = self.input_file(inp);
        }
        self
    }

    pub fn input_path(mut self, path: impl AsRef<std::path::Path>) -> Result<Self> {
        let p = path.as_ref();
        let digest = Digest::hash_file(p)?;
        let meta = std::fs::metadata(p)?;
        let size = meta.len();
        let norm_path = p.to_string_lossy().replace('\\', "/");
        self.inputs.push(InputFile {
            path: norm_path,
            digest,
            size,
            is_executable: None,
        });
        Ok(self)
    }

    pub fn output(mut self, path: impl Into<String>, required: bool) -> Self {
        let norm_path = path.into().replace('\\', "/");
        self.outputs.push(OutputFile {
            path: norm_path,
            required,
        });
        self
    }

    pub fn output_file(mut self, output: OutputFile) -> Self {
        let mut out = output;
        out.path = out.path.replace('\\', "/");
        self.outputs.push(out);
        self
    }

    pub fn outputs<I>(mut self, outputs: I) -> Self
    where
        I: IntoIterator<Item = OutputFile>,
    {
        for out in outputs {
            self = self.output_file(out);
        }
        self
    }

    pub fn output_path(mut self, path: impl AsRef<std::path::Path>) -> Self {
        let norm_path = path.as_ref().to_string_lossy().replace('\\', "/");
        self.outputs.push(OutputFile {
            path: norm_path,
            required: true,
        });
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn envs<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (k, v) in vars {
            self.env.insert(k.into(), v.into());
        }
        self
    }

    /// Explicitly declare an environment variable to capture from the current process environment.
    /// If the variable is set, it will be included in the computation's declared environment map.
    pub fn declared_env(mut self, key: impl Into<String>) -> Self {
        let k = key.into();
        if let Ok(val) = std::env::var(&k) {
            self.env.insert(k, val);
        }
        self
    }

    /// Explicitly declare multiple environment variables to capture from current environment.
    pub fn declared_envs<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for key in keys {
            self = self.declared_env(key);
        }
        self
    }

    pub fn tool(
        mut self,
        name: impl Into<String>,
        version: Option<String>,
        digest: Option<Digest>,
    ) -> Self {
        self.tool = Some(ToolIdentity {
            name: name.into(),
            version,
            digest,
        });
        self
    }

    pub fn tool_identity(mut self, tool: ToolIdentity) -> Self {
        self.tool = Some(tool);
        self
    }

    pub fn platform(mut self, platform: PlatformConstraints) -> Self {
        self.platform = platform;
        self
    }

    pub fn policy(mut self, policy: CachePolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    pub fn working_dir(mut self, wd: impl Into<String>) -> Self {
        self.working_dir = Some(wd.into());
        self
    }

    pub fn sensitive_policy(mut self, policy: crate::sensitive::SensitiveDataPolicy) -> Self {
        self.sensitive_policy = policy;
        self
    }

    pub fn build(mut self) -> Result<Computation> {
        // Sort inputs by normalized path for canonical deterministic ordering
        self.inputs.sort_by(|a, b| a.path.cmp(&b.path));
        // Sort outputs by path
        self.outputs.sort_by(|a, b| a.path.cmp(&b.path));

        // If policy is Mask, redact sensitive values in env
        if self.sensitive_policy == crate::sensitive::SensitiveDataPolicy::Mask {
            for (k, v) in self.env.iter_mut() {
                if crate::sensitive::SensitiveDataDetector::scan_env_var(k, v).is_some() {
                    *v = crate::sensitive::SensitiveDataDetector::redact_value(v);
                }
            }
        }

        let comp = Computation {
            schema_version: self.schema_version,
            operation: self.operation,
            command: self.command,
            args: self.args,
            inputs: self.inputs,
            outputs: self.outputs,
            env: self.env,
            platform: self.platform,
            tool: self.tool,
            policy: self.policy,
            metadata: self.metadata,
            working_dir: self.working_dir,
        };
        comp.validate_with_policy(self.sensitive_policy)?;
        Ok(comp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_milestone_10_2_fluent_builder_api() {
        let comp = Computation::builder()
            .operation("codegen")
            .command("generator")
            .args(vec!["--schema", "schema.json", "--opt"])
            .arg("--fast")
            .input("schema.json", Digest::from_bytes(b"{}"), 2)
            .output("models.rs", true)
            .env("TARGET_LANG", "rust")
            .meta("author", "dcc-dev")
            .build()
            .expect("Builder with valid fields must succeed");

        assert_eq!(comp.operation, "codegen");
        assert_eq!(comp.command, "generator");
        assert_eq!(
            comp.args,
            vec!["--schema", "schema.json", "--opt", "--fast"]
        );
        assert_eq!(comp.inputs.len(), 1);
        assert_eq!(comp.outputs.len(), 1);
        assert_eq!(comp.env.get("TARGET_LANG").unwrap(), "rust");
        assert_eq!(comp.metadata.get("author").unwrap(), "dcc-dev");
    }

    #[test]
    fn test_milestone_10_2_builder_validation_empty_operation() {
        let err = Computation::builder()
            .operation("")
            .command("generator")
            .output("out.txt", true)
            .build()
            .expect_err("Empty operation must fail validation");

        match err {
            CacheError::ValidationError(msg) => {
                assert!(msg.contains("Operation identifier cannot be empty"));
            }
            other => panic!("Expected ValidationError, got {:?}", other),
        }
    }

    #[test]
    fn test_milestone_10_2_builder_validation_empty_command() {
        let err = Computation::builder()
            .operation("codegen")
            .command("   ")
            .output("out.txt", true)
            .build()
            .expect_err("Empty command must fail validation");

        match err {
            CacheError::ValidationError(msg) => {
                assert!(msg.contains("Command executable cannot be empty"));
            }
            other => panic!("Expected ValidationError, got {:?}", other),
        }
    }

    #[test]
    fn test_milestone_10_2_builder_validation_path_traversal() {
        let err_out = Computation::builder()
            .operation("codegen")
            .command("generator")
            .output("../../../etc/passwd", true)
            .build()
            .expect_err("Path traversal in output must fail validation");

        assert!(matches!(err_out, CacheError::PathTraversal(_)));

        let err_inp = Computation::builder()
            .operation("codegen")
            .command("generator")
            .input("../secret.key", Digest::from_bytes(b""), 0)
            .build()
            .expect_err("Path traversal in input must fail validation");

        assert!(matches!(err_inp, CacheError::PathTraversal(_)));
    }

    #[test]
    fn test_milestone_14_4_sensitive_data_policy_deny_and_mask() {
        use crate::sensitive::SensitiveDataPolicy;

        // Deny policy rejects sensitive env vars
        let err_deny = Computation::builder()
            .operation("build")
            .command("cargo")
            .env("AWS_SECRET_ACCESS_KEY", "AKIAIOSFODNN7EXAMPLE")
            .sensitive_policy(SensitiveDataPolicy::Deny)
            .build();
        assert!(
            err_deny.is_err(),
            "SensitiveDataPolicy::Deny must reject sensitive env vars"
        );

        // Deny policy rejects sensitive arguments
        let err_arg = Computation::builder()
            .operation("build")
            .command("cargo")
            .arg("--token=ghp_1234567890abcdef")
            .sensitive_policy(SensitiveDataPolicy::Deny)
            .build();
        assert!(
            err_arg.is_err(),
            "SensitiveDataPolicy::Deny must reject sensitive arguments"
        );

        // Mask policy redacts sensitive value
        let comp_masked = Computation::builder()
            .operation("build")
            .command("cargo")
            .env("GITHUB_TOKEN", "ghp_1234567890abcdef")
            .sensitive_policy(SensitiveDataPolicy::Mask)
            .build()
            .unwrap();
        assert_eq!(
            comp_masked.env.get("GITHUB_TOKEN").unwrap(),
            "[REDACTED]",
            "SensitiveDataPolicy::Mask must redact sensitive value"
        );
    }
}
