use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use crate::digest::Digest;
use crate::entry::CachePolicy;
use crate::error::{CacheError, Result};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformConstraints {
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

impl Default for PlatformConstraints {
    fn default() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            target: None,
        }
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

    pub fn builder(operation: impl Into<String>, command: impl Into<String>) -> ComputationBuilder {
        ComputationBuilder::new(operation, command)
    }

    pub fn validate(&self) -> Result<()> {
        if self.operation.trim().is_empty() {
            return Err(CacheError::ValidationError("Operation identifier cannot be empty".into()));
        }
        if self.command.trim().is_empty() {
            return Err(CacheError::ValidationError("Command executable cannot be empty".into()));
        }
        for out in &self.outputs {
            let p = out.path.replace('\\', "/");
            if p.starts_with('/') || p.starts_with("../") || p.contains("/../") || p == ".." {
                return Err(CacheError::PathTraversal(format!("Output path contains invalid traversal or absolute path: {}", out.path)));
            }
        }
        for inp in &self.inputs {
            let p = inp.path.replace('\\', "/");
            if p.starts_with('/') || p.starts_with("../") || p.contains("/../") || p == ".." {
                return Err(CacheError::PathTraversal(format!("Input path contains invalid traversal or absolute path: {}", inp.path)));
            }
        }
        Ok(())
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
        }
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

    pub fn output(mut self, path: impl Into<String>, required: bool) -> Self {
        let norm_path = path.into().replace('\\', "/");
        self.outputs.push(OutputFile {
            path: norm_path,
            required,
        });
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn tool(mut self, name: impl Into<String>, version: Option<String>, digest: Option<Digest>) -> Self {
        self.tool = Some(ToolIdentity {
            name: name.into(),
            version,
            digest,
        });
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

    pub fn build(mut self) -> Result<Computation> {
        // Sort inputs by normalized path for canonical deterministic ordering
        self.inputs.sort_by(|a, b| a.path.cmp(&b.path));
        // Sort outputs by path
        self.outputs.sort_by(|a, b| a.path.cmp(&b.path));

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
        comp.validate()?;
        Ok(comp)
    }
}
