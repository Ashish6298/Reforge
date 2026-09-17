use crate::computation::{Computation, InputFile, OutputFile, PlatformConstraints, ToolIdentity};
use crate::digest::Digest;
use crate::error::{CacheError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Represents a compiler build action model (Milestone 11.1).
///
/// Encapsulates:
/// - `compiler`: The compiler executable (e.g., "rustc", "gcc", "clang")
/// - `arguments`: Compiler flags, optimization levels, and compilation targets
/// - `source_inputs`: Primary source files (.rs, .c, .cpp)
/// - `dependency_inputs`: Compiled libraries, crates, headers, and metadata (.rlib, .a, .so, .h)
/// - `compiler_version`: Specific version string of the compiler (e.g., "1.80.0")
/// - `target`: Target architecture/triple (e.g., "x86_64-unknown-linux-gnu")
/// - `environment`: Relevant declared compilation environment variables
/// - `outputs`: Produced compiler artifacts (.rlib, .o, .exe, .so, .pdb)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildAction {
    pub compiler: String,
    pub arguments: Vec<String>,
    pub source_inputs: Vec<InputFile>,
    pub dependency_inputs: Vec<InputFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiler_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiler_digest: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    pub outputs: Vec<OutputFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}

impl BuildAction {
    pub fn builder() -> BuildActionBuilder {
        BuildActionBuilder::default()
    }

    /// Convert this `BuildAction` into a canonical `Computation` for key generation and execution.
    pub fn to_computation(&self) -> Result<Computation> {
        let mut all_inputs =
            Vec::with_capacity(self.source_inputs.len() + self.dependency_inputs.len());
        all_inputs.extend_from_slice(&self.source_inputs);
        all_inputs.extend_from_slice(&self.dependency_inputs);

        // Sort all inputs canonically
        all_inputs.sort_by(|a, b| a.path.cmp(&b.path));

        let mut platform = PlatformConstraints::host();
        if let Some(target) = &self.target {
            platform = platform.with_target(target.clone());
        }
        platform = platform.with_compiler(&self.compiler);

        let tool = Some(ToolIdentity {
            name: self.compiler.clone(),
            version: self.compiler_version.clone(),
            digest: self.compiler_digest.clone(),
        });

        let mut outputs = self.outputs.clone();
        outputs.sort_by(|a, b| a.path.cmp(&b.path));

        let comp = Computation {
            schema_version: Computation::CURRENT_SCHEMA_VERSION,
            operation: "build".to_string(),
            command: self.compiler.clone(),
            args: self.arguments.clone(),
            inputs: all_inputs,
            outputs,
            env: self.environment.clone(),
            platform,
            tool,
            policy: crate::entry::CachePolicy::ReadWrite,
            metadata: {
                let mut m = BTreeMap::new();
                m.insert("action_type".to_string(), "build_action".to_string());
                m.insert("compiler".to_string(), self.compiler.clone());
                if let Some(target) = &self.target {
                    m.insert("target".to_string(), target.clone());
                }
                m
            },
            working_dir: self.working_dir.clone(),
        };

        comp.validate()?;
        Ok(comp)
    }

    /// Compute the deterministic cache key for this build action.
    pub fn compute_key(&self) -> Result<crate::digest::CacheKey> {
        self.to_computation()?.compute_key()
    }
}

/// Fluent builder for constructing a `BuildAction`.
#[derive(Debug, Clone, Default)]
pub struct BuildActionBuilder {
    compiler: String,
    arguments: Vec<String>,
    source_inputs: Vec<InputFile>,
    dependency_inputs: Vec<InputFile>,
    compiler_version: Option<String>,
    compiler_digest: Option<Digest>,
    target: Option<String>,
    environment: BTreeMap<String, String>,
    outputs: Vec<OutputFile>,
    working_dir: Option<String>,
}

impl BuildActionBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn compiler(mut self, compiler: impl Into<String>) -> Self {
        self.compiler = compiler.into();
        self
    }

    pub fn argument(mut self, arg: impl Into<String>) -> Self {
        self.arguments.push(arg.into());
        self
    }

    pub fn arguments<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for arg in args {
            self.arguments.push(arg.into());
        }
        self
    }

    pub fn source_input(mut self, path: impl Into<String>, digest: Digest, size: u64) -> Self {
        let norm_path = path.into().replace('\\', "/");
        self.source_inputs.push(InputFile {
            path: norm_path,
            digest,
            size,
            is_executable: None,
        });
        self
    }

    pub fn source_file(mut self, path: impl AsRef<Path>) -> Result<Self> {
        let p = path.as_ref();
        let digest = Digest::hash_file(p)?;
        let meta = std::fs::metadata(p)?;
        let size = meta.len();
        let norm_path = p.to_string_lossy().replace('\\', "/");
        self.source_inputs.push(InputFile {
            path: norm_path,
            digest,
            size,
            is_executable: None,
        });
        Ok(self)
    }

    pub fn dependency_input(mut self, path: impl Into<String>, digest: Digest, size: u64) -> Self {
        let norm_path = path.into().replace('\\', "/");
        self.dependency_inputs.push(InputFile {
            path: norm_path,
            digest,
            size,
            is_executable: None,
        });
        self
    }

    pub fn dependency_file(mut self, path: impl AsRef<Path>) -> Result<Self> {
        let p = path.as_ref();
        let digest = Digest::hash_file(p)?;
        let meta = std::fs::metadata(p)?;
        let size = meta.len();
        let norm_path = p.to_string_lossy().replace('\\', "/");
        self.dependency_inputs.push(InputFile {
            path: norm_path,
            digest,
            size,
            is_executable: None,
        });
        Ok(self)
    }

    pub fn compiler_version(mut self, version: impl Into<String>) -> Self {
        self.compiler_version = Some(version.into());
        self
    }

    pub fn compiler_digest(mut self, digest: Digest) -> Self {
        self.compiler_digest = Some(digest);
        self
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.insert(key.into(), value.into());
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

    pub fn output(mut self, path: impl Into<String>, required: bool) -> Self {
        let norm_path = path.into().replace('\\', "/");
        self.outputs.push(OutputFile {
            path: norm_path,
            required,
        });
        self
    }

    pub fn outputs<I>(mut self, outputs: I) -> Self
    where
        I: IntoIterator<Item = OutputFile>,
    {
        for out in outputs {
            let mut o = out;
            o.path = o.path.replace('\\', "/");
            self.outputs.push(o);
        }
        self
    }

    pub fn working_dir(mut self, wd: impl Into<String>) -> Self {
        self.working_dir = Some(wd.into());
        self
    }

    pub fn build(mut self) -> Result<BuildAction> {
        if self.compiler.trim().is_empty() {
            return Err(CacheError::ValidationError(
                "Compiler executable cannot be empty".into(),
            ));
        }

        self.source_inputs.sort_by(|a, b| a.path.cmp(&b.path));
        self.dependency_inputs.sort_by(|a, b| a.path.cmp(&b.path));
        self.outputs.sort_by(|a, b| a.path.cmp(&b.path));

        for out in &self.outputs {
            let p = out.path.replace('\\', "/");
            if p.starts_with('/') || p.starts_with("../") || p.contains("/../") || p == ".." {
                return Err(CacheError::PathTraversal(format!(
                    "Output path contains invalid traversal: {}",
                    out.path
                )));
            }
        }

        for inp in self
            .source_inputs
            .iter()
            .chain(self.dependency_inputs.iter())
        {
            let p = inp.path.replace('\\', "/");
            if p.starts_with('/') || p.starts_with("../") || p.contains("/../") || p == ".." {
                return Err(CacheError::PathTraversal(format!(
                    "Input path contains invalid traversal: {}",
                    inp.path
                )));
            }
        }

        Ok(BuildAction {
            compiler: self.compiler,
            arguments: self.arguments,
            source_inputs: self.source_inputs,
            dependency_inputs: self.dependency_inputs,
            compiler_version: self.compiler_version,
            compiler_digest: self.compiler_digest,
            target: self.target,
            environment: self.environment,
            outputs: self.outputs,
            working_dir: self.working_dir,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_milestone_11_1_build_action_model_and_key_generation() {
        let action = BuildAction::builder()
            .compiler("rustc")
            .arguments(vec!["src/main.rs", "--crate-type", "bin", "-O"])
            .source_input("src/main.rs", Digest::from_bytes(b"fn main() {}"), 14)
            .source_input("src/lib.rs", Digest::from_bytes(b"pub fn run() {}"), 16)
            .dependency_input(
                "target/deps/libserde.rlib",
                Digest::from_bytes(b"serde_blob"),
                1024,
            )
            .compiler_version("1.80.0")
            .target("x86_64-pc-windows-msvc")
            .env("RUSTFLAGS", "-C opt-level=3")
            .output("target/main.exe", true)
            .build()
            .expect("Valid BuildAction must build successfully");

        assert_eq!(action.compiler, "rustc");
        assert_eq!(action.source_inputs.len(), 2);
        assert_eq!(action.dependency_inputs.len(), 1);
        assert_eq!(action.outputs.len(), 1);

        let computation = action.to_computation().unwrap();
        assert_eq!(computation.operation, "build");
        assert_eq!(computation.command, "rustc");
        assert_eq!(computation.inputs.len(), 3); // 2 sources + 1 dependency
        assert_eq!(computation.platform.compiler.as_deref(), Some("rustc"));
        assert_eq!(
            computation.platform.target.as_deref(),
            Some("x86_64-pc-windows-msvc")
        );
        assert_eq!(computation.tool.as_ref().unwrap().name, "rustc");
        assert_eq!(
            computation.tool.as_ref().unwrap().version.as_deref(),
            Some("1.80.0")
        );

        let key = action.compute_key().unwrap();
        assert_eq!(key, computation.compute_key().unwrap());
    }

    #[test]
    fn test_milestone_11_1_build_action_source_change_changes_key() {
        let action1 = BuildAction::builder()
            .compiler("rustc")
            .source_input("src/main.rs", Digest::from_bytes(b"v1"), 2)
            .output("main.exe", true)
            .build()
            .unwrap();

        let action2 = BuildAction::builder()
            .compiler("rustc")
            .source_input("src/main.rs", Digest::from_bytes(b"v2"), 2)
            .output("main.exe", true)
            .build()
            .unwrap();

        assert_ne!(
            action1.compute_key().unwrap(),
            action2.compute_key().unwrap()
        );
    }

    #[test]
    fn test_milestone_11_1_build_action_dependency_change_changes_key() {
        let action1 = BuildAction::builder()
            .compiler("rustc")
            .source_input("src/main.rs", Digest::from_bytes(b"source"), 6)
            .dependency_input("libdep.rlib", Digest::from_bytes(b"dep_v1"), 6)
            .output("main.exe", true)
            .build()
            .unwrap();

        let action2 = BuildAction::builder()
            .compiler("rustc")
            .source_input("src/main.rs", Digest::from_bytes(b"source"), 6)
            .dependency_input("libdep.rlib", Digest::from_bytes(b"dep_v2"), 6)
            .output("main.exe", true)
            .build()
            .unwrap();

        assert_ne!(
            action1.compute_key().unwrap(),
            action2.compute_key().unwrap()
        );
    }

    #[test]
    fn test_milestone_11_1_build_action_compiler_version_change_changes_key() {
        let action1 = BuildAction::builder()
            .compiler("rustc")
            .compiler_version("1.79.0")
            .source_input("src/main.rs", Digest::from_bytes(b"source"), 6)
            .output("main.exe", true)
            .build()
            .unwrap();

        let action2 = BuildAction::builder()
            .compiler("rustc")
            .compiler_version("1.80.0")
            .source_input("src/main.rs", Digest::from_bytes(b"source"), 6)
            .output("main.exe", true)
            .build()
            .unwrap();

        assert_ne!(
            action1.compute_key().unwrap(),
            action2.compute_key().unwrap()
        );
    }

    #[test]
    fn test_milestone_11_1_build_action_target_change_changes_key() {
        let action1 = BuildAction::builder()
            .compiler("rustc")
            .target("x86_64-unknown-linux-gnu")
            .source_input("src/main.rs", Digest::from_bytes(b"source"), 6)
            .output("main.exe", true)
            .build()
            .unwrap();

        let action2 = BuildAction::builder()
            .compiler("rustc")
            .target("aarch64-unknown-linux-gnu")
            .source_input("src/main.rs", Digest::from_bytes(b"source"), 6)
            .output("main.exe", true)
            .build()
            .unwrap();

        assert_ne!(
            action1.compute_key().unwrap(),
            action2.compute_key().unwrap()
        );
    }

    #[test]
    fn test_milestone_11_1_build_action_validation() {
        let err_compiler = BuildAction::builder()
            .source_input("src/main.rs", Digest::from_bytes(b"source"), 6)
            .output("main.exe", true)
            .build()
            .expect_err("Empty compiler must fail");
        assert!(matches!(err_compiler, CacheError::ValidationError(_)));

        let err_traversal = BuildAction::builder()
            .compiler("rustc")
            .output("../escaped.exe", true)
            .build()
            .expect_err("Path traversal must fail");
        assert!(matches!(err_traversal, CacheError::PathTraversal(_)));
    }
}
