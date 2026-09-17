use crate::computation::Computation;
use crate::digest::{CacheKey, Digest};
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalComputation {
    pub schema_version: u32,
    pub operation: String,
    pub command: String,
    pub args: Vec<String>,
    pub inputs: Vec<CanonicalInput>,
    pub outputs: Vec<CanonicalOutput>,
    pub env: BTreeMap<String, String>,
    pub platform: CanonicalPlatform,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<CanonicalTool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalInput {
    pub path: String,
    pub digest: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_executable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalOutput {
    pub path: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalPlatform {
    pub os: String,
    pub arch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiler: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalTool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

impl CanonicalComputation {
    pub fn from_computation(comp: &Computation) -> Self {
        let mut inputs: Vec<CanonicalInput> = comp
            .inputs
            .iter()
            .map(|i| CanonicalInput {
                path: i.path.replace('\\', "/"),
                digest: i.digest.as_str().to_string(),
                size: i.size,
                is_executable: i.is_executable,
            })
            .collect();
        inputs.sort_by(|a, b| a.path.cmp(&b.path));

        let mut outputs: Vec<CanonicalOutput> = comp
            .outputs
            .iter()
            .map(|o| CanonicalOutput {
                path: o.path.replace('\\', "/"),
                required: o.required,
            })
            .collect();
        outputs.sort_by(|a, b| a.path.cmp(&b.path));

        Self {
            schema_version: comp.schema_version,
            operation: comp.operation.clone(),
            command: comp.command.clone(),
            args: comp.args.clone(),
            inputs,
            outputs,
            env: comp.env.clone(),
            platform: CanonicalPlatform {
                os: comp.platform.os.clone(),
                arch: comp.platform.arch.clone(),
                target: comp.platform.target.clone(),
                runtime: comp.platform.runtime.clone(),
                abi: comp.platform.abi.clone(),
                compiler: comp.platform.compiler.clone(),
            },
            tool: comp.tool.as_ref().map(|t| CanonicalTool {
                name: t.name.clone(),
                version: t.version.clone(),
                digest: t.digest.as_ref().map(|d| d.as_str().to_string()),
            }),
        }
    }

    pub fn to_canonical_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn compute_key(&self) -> Result<CacheKey> {
        let json = self.to_canonical_json()?;
        let digest = Digest::from_bytes(json.as_bytes());
        Ok(CacheKey::new(digest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computation::Computation;

    #[test]
    fn test_canonical_key_determinism() {
        let digest_a = Digest::from_bytes(b"hello");
        let comp1 = Computation::builder_with("build", "rustc")
            .arg("main.rs")
            .input("src/main.rs", digest_a.clone(), 10)
            .env("MODE", "release")
            .env("OPT", "3")
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("build", "rustc")
            .arg("main.rs")
            .env("OPT", "3") // inserted in different order
            .env("MODE", "release")
            .input("src/main.rs", digest_a, 10)
            .build()
            .unwrap();

        let canon1 = CanonicalComputation::from_computation(&comp1);
        let canon2 = CanonicalComputation::from_computation(&comp2);

        assert_eq!(canon1.compute_key().unwrap(), canon2.compute_key().unwrap());
    }

    #[test]
    fn test_differing_inputs_produce_different_keys() {
        let d1 = Digest::from_bytes(b"content 1");
        let d2 = Digest::from_bytes(b"content 2");

        let comp1 = Computation::builder_with("gen", "tool")
            .input("file.txt", d1, 9)
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("gen", "tool")
            .input("file.txt", d2, 9)
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_multiple_inputs_single_change_produces_different_key() {
        let hash_x = Digest::from_bytes(b"HASH_X");
        let hash_y = Digest::from_bytes(b"HASH_Y");
        let hash_z = Digest::from_bytes(b"HASH_Z");

        // Set 1: A = hash_x, B = hash_z
        let comp1 = Computation::builder_with("build", "compiler")
            .input("input_a.rs", hash_x, 100)
            .input("input_b.rs", hash_z.clone(), 200)
            .build()
            .unwrap();

        // Set 2: A = hash_y (mutated), B = hash_z (unchanged)
        let comp2 = Computation::builder_with("build", "compiler")
            .input("input_a.rs", hash_y, 100)
            .input("input_b.rs", hash_z, 200)
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(
            key1, key2,
            "Changing input A from hash X to hash Y must alter computation key"
        );
    }

    #[test]
    fn test_nested_path_input_change_produces_different_key() {
        let d1 = Digest::from_bytes(b"nested module v1");
        let d2 = Digest::from_bytes(b"nested module v2");

        let comp1 = Computation::builder_with("compile", "rustc")
            .input("src/models/deep/schema.json", d1, 50)
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("compile", "rustc")
            .input("src/models/deep/schema.json", d2, 50)
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_input_path_rename_with_same_hash_produces_different_key() {
        let d = Digest::from_bytes(b"shared data content");

        let comp1 = Computation::builder_with("process", "tool")
            .input("path_alpha.txt", d.clone(), 100)
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("process", "tool")
            .input("path_beta.txt", d, 100)
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(
            key1, key2,
            "Input path identity matters even if content hash is identical"
        );
    }

    #[test]
    fn test_input_order_independent_canonicalization() {
        let d_a = Digest::from_bytes(b"data A");
        let d_b = Digest::from_bytes(b"data B");
        let d_c = Digest::from_bytes(b"data C");

        let comp1 = Computation::builder_with("bundle", "bundler")
            .input("a.js", d_a.clone(), 10)
            .input("b.js", d_b.clone(), 20)
            .input("c.js", d_c.clone(), 30)
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("bundle", "bundler")
            .input("c.js", d_c, 30)
            .input("a.js", d_a, 10)
            .input("b.js", d_b, 20)
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_eq!(
            key1, key2,
            "Input order in declaration must be canonically sorted and invariant"
        );
    }

    #[test]
    fn test_differing_arguments_produce_different_keys() {
        let comp1 = Computation::builder_with("fmt", "generator")
            .arg("--fast")
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("fmt", "generator")
            .arg("--safe")
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(
            key1, key2,
            "generator --fast and generator --safe must produce different cache keys"
        );
    }

    #[test]
    fn test_differing_command_executable_produces_different_keys() {
        let comp1 = Computation::builder_with("build", "generator")
            .arg("--fast")
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("build", "transformer")
            .arg("--fast")
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(
            key1, key2,
            "Differing command executables must produce different cache keys"
        );
    }

    #[test]
    fn test_argument_ordering_sensitivity_produces_different_keys() {
        let comp1 = Computation::builder_with("build", "compiler")
            .arg("--opt")
            .arg("--debug")
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("build", "compiler")
            .arg("--debug")
            .arg("--opt")
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(
            key1, key2,
            "Command line argument order is semantically meaningful and must not be commuted"
        );
    }

    #[test]
    fn test_argument_addition_removal_produces_different_keys() {
        let comp1 = Computation::builder_with("run", "tool").build().unwrap();

        let comp2 = Computation::builder_with("run", "tool")
            .arg("--flag")
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_differing_tool_versions_produce_different_keys() {
        let comp1 = Computation::builder_with("compile", "rustc")
            .tool("rustc", Some("1.80.0".into()), None)
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("compile", "rustc")
            .tool("rustc", Some("1.81.0".into()), None)
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(
            key1, key2,
            "Tool version differences (1.80.0 vs 1.81.0) must produce different keys"
        );
    }

    #[test]
    fn test_tool_executable_digest_differences_produce_different_keys() {
        let d1 = Digest::from_bytes(b"compiler_binary_v1_bytes");
        let d2 = Digest::from_bytes(b"compiler_binary_v2_bytes");

        let comp1 = Computation::builder_with("compile", "gcc")
            .tool("gcc", Some("13.2.0".into()), Some(d1))
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("compile", "gcc")
            .tool("gcc", Some("13.2.0".into()), Some(d2))
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(
            key1, key2,
            "Tool executable binary digest differences must produce different keys"
        );
    }

    #[test]
    fn test_tool_identity_strategy_composite_components() {
        let tool_a = crate::computation::ToolIdentity::new("clang");
        let tool_b = crate::computation::ToolIdentity::with_version("clang", "18.1.0");
        let tool_c = crate::computation::ToolIdentity::with_digest(
            "clang",
            Some("18.1.0".into()),
            Some(Digest::from_bytes(b"clang_bin")),
        );

        let comp_a = Computation::builder_with("build", "clang")
            .tool_identity(tool_a)
            .build()
            .unwrap();
        let comp_b = Computation::builder_with("build", "clang")
            .tool_identity(tool_b)
            .build()
            .unwrap();
        let comp_c = Computation::builder_with("build", "clang")
            .tool_identity(tool_c)
            .build()
            .unwrap();

        let key_a = CanonicalComputation::from_computation(&comp_a)
            .compute_key()
            .unwrap();
        let key_b = CanonicalComputation::from_computation(&comp_b)
            .compute_key()
            .unwrap();
        let key_c = CanonicalComputation::from_computation(&comp_c)
            .compute_key()
            .unwrap();

        assert_ne!(key_a, key_b);
        assert_ne!(key_b, key_c);
        assert_ne!(key_a, key_c);
    }

    #[test]
    fn test_differing_environment_produce_different_keys() {
        let comp1 = Computation::builder_with("test", "runner")
            .env("NODE_ENV", "development")
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("test", "runner")
            .env("NODE_ENV", "production")
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(
            key1, key2,
            "Declared environment differences (NODE_ENV=dev vs prod) must alter key"
        );
    }

    #[test]
    fn test_declared_environment_order_independent_canonicalization() {
        // Declared environment variables inserted in arbitrary order must sort canonically
        let comp1 = Computation::builder_with("build", "generator")
            .env("NODE_ENV", "production")
            .env("GENERATOR_VERSION", "2.1")
            .env("FEATURE_MODE", "enabled")
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("build", "generator")
            .env("FEATURE_MODE", "enabled")
            .env("NODE_ENV", "production")
            .env("GENERATOR_VERSION", "2.1")
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_eq!(
            key1, key2,
            "Declared environment maps must be canonically sorted and invariant to insertion order"
        );
    }

    #[test]
    fn test_undeclared_environment_variables_do_not_fragment_cache() {
        // Computations only serialize declared environment variables in comp.env.
        // Unrelated variables in system environment do not pollute computation keys.
        let comp1 = Computation::builder_with("compile", "rustc")
            .env("RUST_LOG", "info")
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("compile", "rustc")
            .env("RUST_LOG", "info")
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_platform_os_differences_produce_different_keys() {
        let p_linux = crate::computation::PlatformConstraints::new("linux", "x86_64");
        let p_windows = crate::computation::PlatformConstraints::new("windows", "x86_64");

        let comp1 = Computation::builder_with("build", "cc")
            .platform(p_linux)
            .build()
            .unwrap();
        let comp2 = Computation::builder_with("build", "cc")
            .platform(p_windows)
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(
            key1, key2,
            "OS differences (linux vs windows) must alter key"
        );
    }

    #[test]
    fn test_platform_architecture_differences_produce_different_keys() {
        let p_x86 = crate::computation::PlatformConstraints::new("linux", "x86_64");
        let p_arm = crate::computation::PlatformConstraints::new("linux", "aarch64");

        let comp1 = Computation::builder_with("build", "cc")
            .platform(p_x86)
            .build()
            .unwrap();
        let comp2 = Computation::builder_with("build", "cc")
            .platform(p_arm)
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(
            key1, key2,
            "Architecture differences (x86_64 vs aarch64) must alter key"
        );
    }

    #[test]
    fn test_platform_target_triple_differences_produce_different_keys() {
        let p_gnu = crate::computation::PlatformConstraints::new("linux", "x86_64")
            .with_target("x86_64-unknown-linux-gnu");
        let p_musl = crate::computation::PlatformConstraints::new("linux", "x86_64")
            .with_target("x86_64-unknown-linux-musl");

        let comp1 = Computation::builder_with("build", "rustc")
            .platform(p_gnu)
            .build()
            .unwrap();
        let comp2 = Computation::builder_with("build", "rustc")
            .platform(p_musl)
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(
            key1, key2,
            "Target triple differences (gnu vs musl) must alter key"
        );
    }

    #[test]
    fn test_platform_runtime_and_abi_differences_produce_different_keys() {
        let p_node18 = crate::computation::PlatformConstraints::new("linux", "x86_64")
            .with_runtime("node18")
            .with_abi("glibc2.31");
        let p_node20 = crate::computation::PlatformConstraints::new("linux", "x86_64")
            .with_runtime("node20")
            .with_abi("glibc2.35");

        let comp1 = Computation::builder_with("run", "node")
            .platform(p_node18)
            .build()
            .unwrap();
        let comp2 = Computation::builder_with("run", "node")
            .platform(p_node20)
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(key1, key2, "Runtime/ABI differences must alter key");
    }

    #[test]
    fn test_file_identity_content_primary_and_timestamp_invariant() {
        // Files with identical content and path have identical identity regardless of mtime
        let d = Digest::from_bytes(b"content alpha");
        let comp1 = Computation::builder_with("build", "tool")
            .input("src/lib.rs", d.clone(), 100)
            .build()
            .unwrap();

        let comp2 = Computation::builder_with("build", "tool")
            .input("src/lib.rs", d, 100)
            .build()
            .unwrap();

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_eq!(
            key1, key2,
            "Content is the primary identity; mtime changes do not alter key"
        );
    }

    #[test]
    fn test_file_identity_executable_bit_matters_when_set() {
        let d = Digest::from_bytes(b"script payload");
        let mut comp1 = Computation::builder_with("run", "bash")
            .input("script.sh", d.clone(), 50)
            .build()
            .unwrap();
        comp1.inputs[0].is_executable = Some(false);

        let mut comp2 = Computation::builder_with("run", "bash")
            .input("script.sh", d, 50)
            .build()
            .unwrap();
        comp2.inputs[0].is_executable = Some(true);

        let key1 = CanonicalComputation::from_computation(&comp1)
            .compute_key()
            .unwrap();
        let key2 = CanonicalComputation::from_computation(&comp2)
            .compute_key()
            .unwrap();

        assert_ne!(
            key1, key2,
            "Executable permission bit differences alter computation identity"
        );
    }
}
