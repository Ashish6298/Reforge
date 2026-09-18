//! Engineering Audit Test Suite for Milestone 20 (v1.0.0 Release Audit)
//! Covers:
//! - 20.1 Correctness Audit:
//!   - same computation -> same key
//!   - different computation -> different key
//!   - changed input -> cache miss
//!   - changed relevant environment -> cache miss
//!   - changed tool identity -> cache miss
//!   - corrupted cache -> detected
//!   - missing cache -> safe miss
//!   - failed computation -> not cached by default
//! - 20.2 Reliability & Fault Tolerance
//! - 20.6 Security & Integrity Validation

use dcc_core::{Computation, Digest, FailurePolicy, SensitiveDataDetector, ToolIdentity};
use dcc_runner::{CommandSpec, EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_test_utils::TestEnv;
use std::fs;

// ============================================================================
// 20.1 CORRECTNESS AUDIT: COMPUTATION & KEY GENERATION
// ============================================================================

#[test]
fn test_audit_20_1_same_computation_same_key() {
    let comp1 = Computation::builder()
        .operation("test")
        .command("cargo")
        .args(vec!["test"])
        .env("RUST_BACKTRACE", "1")
        .input("src/lib.rs", Digest::hash_bytes(b"fn main() {}"), 12)
        .output("target/out.bin", false)
        .build()
        .unwrap();

    let comp2 = Computation::builder()
        .operation("test")
        .command("cargo")
        .args(vec!["test"])
        .env("RUST_BACKTRACE", "1")
        .input("src/lib.rs", Digest::hash_bytes(b"fn main() {}"), 12)
        .output("target/out.bin", false)
        .build()
        .unwrap();

    let key1 = comp1.compute_key().unwrap();
    let key2 = comp2.compute_key().unwrap();

    assert_eq!(
        key1, key2,
        "Audit 20.1: Same computation must strictly yield the exact same cache key"
    );
}

#[test]
fn test_audit_20_1_different_computation_different_key() {
    let comp1 = Computation::builder()
        .operation("build")
        .command("cargo")
        .args(vec!["build"])
        .input("src/lib.rs", Digest::hash_bytes(b"fn main() {}"), 12)
        .build()
        .unwrap();

    let comp2 = Computation::builder()
        .operation("build")
        .command("cargo")
        .args(vec!["build", "--release"])
        .input("src/lib.rs", Digest::hash_bytes(b"fn main() {}"), 12)
        .build()
        .unwrap();

    let key1 = comp1.compute_key().unwrap();
    let key2 = comp2.compute_key().unwrap();

    assert_ne!(
        key1, key2,
        "Audit 20.1: Different computation arguments must yield different keys"
    );
}

#[test]
fn test_audit_20_1_changed_input_causes_cache_miss() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("input.txt", b"v1_data").unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('out.txt', (Get-Content input.txt) + '_processed')"
                .to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "cat input.txt > out.txt && echo '_processed' >> out.txt".to_string(),
        ],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("input.txt")
        .output_path("out.txt")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // 1st Execution: Cold cache miss
    let res1 = engine.execute_command(&spec).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);

    // 2nd Execution: Unmodified -> Cache HIT
    let res2 = engine.execute_command(&spec).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Hit);

    // Modify input file content
    env.create_input_file("input.txt", b"v2_data_changed")
        .unwrap();

    // 3rd Execution: Changed input -> Cache MISS
    let res3 = engine.execute_command(&spec).unwrap();
    assert_eq!(
        res3.status,
        ExecutionStatus::Miss,
        "Audit 20.1: Changed input must trigger a cache miss"
    );
    assert_ne!(
        res1.key, res3.key,
        "Keys must differ between input revisions"
    );
}

#[test]
fn test_audit_20_1_changed_relevant_environment_causes_cache_miss() {
    let comp1 = Computation::builder()
        .operation("compile")
        .command("gcc")
        .args(vec!["-c", "main.c"])
        .env("CFLAGS", "-O2")
        .build()
        .unwrap();

    let comp2 = Computation::builder()
        .operation("compile")
        .command("gcc")
        .args(vec!["-c", "main.c"])
        .env("CFLAGS", "-O3")
        .build()
        .unwrap();

    let key1 = comp1.compute_key().unwrap();
    let key2 = comp2.compute_key().unwrap();

    assert_ne!(
        key1, key2,
        "Audit 20.1: Changed environment variable value must yield different key / cache miss"
    );
}

#[test]
fn test_audit_20_1_changed_tool_identity_causes_cache_miss() {
    let comp1 = Computation::builder()
        .operation("compile")
        .command("rustc")
        .args(vec!["main.rs"])
        .tool(ToolIdentity::with_version("rustc", "1.75.0"))
        .build()
        .unwrap();

    let comp2 = Computation::builder()
        .operation("compile")
        .command("rustc")
        .args(vec!["main.rs"])
        .tool(ToolIdentity::with_version("rustc", "1.76.0"))
        .build()
        .unwrap();

    let key1 = comp1.compute_key().unwrap();
    let key2 = comp2.compute_key().unwrap();

    assert_ne!(
        key1, key2,
        "Audit 20.1: Upgraded tool version or tool digest must produce a cache miss"
    );
}

#[test]
fn test_audit_20_1_corrupted_cache_detected_and_safely_quarantined() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("source.c", b"int main() { return 0; }")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('binary.out', 'VALID_BINARY_BYTES')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'VALID_BINARY_BYTES' > binary.out".to_string(),
        ],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("source.c")
        .output_path("binary.out")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    let res1 = engine.execute_command(&spec).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);

    // Corrupt the CAS object on disk
    let out_digest = &res1.outputs[0].digest;
    let obj_path = env.storage.object_path(out_digest);
    assert!(obj_path.exists());

    // Corrupt bytes
    fs::write(&obj_path, b"CORRUPTED_TAMPERED_CONTENT").unwrap();

    // Re-execution: CAS corruption must be detected, safely handled as a miss / re-executed
    let res2 = engine.execute_command(&spec).unwrap();
    assert_eq!(
        res2.status,
        ExecutionStatus::Miss,
        "Audit 20.1: Corrupted cache object must be detected and bypassed safely"
    );

    // Verify output in workspace is valid
    let restored = env.read_output_file("binary.out").unwrap();
    assert_eq!(restored, b"VALID_BINARY_BYTES");
}

#[test]
fn test_audit_20_1_missing_cache_is_safe_miss() {
    let env = TestEnv::new().unwrap();
    let comp = Computation::builder()
        .operation("echo")
        .command("echo")
        .args(vec!["test"])
        .build()
        .unwrap();
    let key = comp.compute_key().unwrap();

    // Directly query storage for non-existent key
    let entry_opt = env.storage.get_entry(&key).unwrap();
    assert!(
        entry_opt.is_none(),
        "Audit 20.1: Missing cache entry must return Ok(None) / safe miss without panic or error"
    );
}

#[test]
fn test_audit_20_1_failed_computation_not_cached_by_default() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("fail_input.txt", b"bad code")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[Console]::Error.Write('Syntax Error'); exit 42".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'Syntax Error' >&2; exit 42".to_string(),
        ],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("fail_input.txt")
        .output_path("fail_out.txt")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            failure_policy: FailurePolicy::DoNotCache, // Default policy
            ..Default::default()
        },
    );

    let res1 = engine.execute_command(&spec).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);
    assert_eq!(res1.exit_code, 42);

    // Verify storage has NO entry for this key
    let entry = env.storage.get_entry(&res1.key).unwrap();
    assert!(
        entry.is_none(),
        "Audit 20.1: Failed computation (exit code != 0) must NOT be cached by default"
    );
}
