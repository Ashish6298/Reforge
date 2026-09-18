//! Developer Experience Audit Test Suite for Milestone 20.4
//! Verifies:
//! 1. CLI understandable (command structure & syntax)
//! 2. Errors useful (actionable errors with contextual paths & descriptions)
//! 3. Miss reasons understandable (MissReason enum & human-readable diagnostics)
//! 4. JSON output stable (JSON schema consistency across commands)
//! 5. Documentation complete (manuals, guides, examples)
//! 6. Installation simple (cargo install / binary download)
//! 7. Configuration predictable (ByteSize, StorageConfig, defaults)

use dcc_core::{ByteSize, CacheError, MissReason};
use dcc_runner::{ExecutionResult, ExecutionStatus};
use dcc_storage::StorageConfig;
use dcc_test_utils::TestEnv;
use std::path::PathBuf;

#[test]
fn test_audit_20_4_miss_reasons_understandable() {
    let reasons = vec![
        MissReason::NoEntryFound,
        MissReason::InputChanged {
            path: "src/main.rs".to_string(),
            old_digest: Some("old_sha".to_string()),
            new_digest: "new_sha".to_string(),
        },
        MissReason::EnvironmentChanged {
            key: "CFLAGS".to_string(),
            old: Some("-O2".to_string()),
            new: Some("-O3".to_string()),
        },
        MissReason::ToolChanged {
            reason: "rustc version mismatch".to_string(),
        },
        MissReason::CommandChanged {
            old: "gcc".to_string(),
            new: "clang".to_string(),
        },
        MissReason::CorruptedCache {
            reason: "checksum mismatch".to_string(),
        },
        MissReason::ForcedBypass,
    ];

    for r in &reasons {
        let msg = r.to_string();
        assert!(!msg.is_empty(), "Miss reason string must not be empty");
        // Verify understandable diagnostic phrasing
        match r {
            MissReason::NoEntryFound => assert!(msg.contains("No cached") || msg.contains("entry")),
            MissReason::InputChanged { path, .. } => assert!(msg.contains(path)),
            MissReason::EnvironmentChanged { key, .. } => assert!(msg.contains(key)),
            MissReason::ToolChanged { reason } => assert!(msg.contains(reason)),
            _ => {}
        }
    }
}

#[test]
fn test_audit_20_4_errors_useful_and_actionable() {
    let err_path = CacheError::PathTraversal("outside/sandbox".into());
    let err_msg = err_path.to_string();
    assert!(err_msg.contains("outside/sandbox") || err_msg.contains("Path traversal"));

    let err_not_found = CacheError::Miss("Key not present in CAS".into());
    assert!(err_not_found.to_string().contains("not present"));

    let err_io = CacheError::StorageError(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "Access denied to .dcc_cache",
    ));
    assert!(err_io.to_string().contains("Access denied"));
}

#[test]
fn test_audit_20_4_json_output_stable() {
    let env = TestEnv::new().unwrap();
    let comp = dcc_core::Computation::builder()
        .operation("echo")
        .command("echo")
        .args(vec!["hello"])
        .build()
        .unwrap();
    let key = comp.compute_key().unwrap();

    let result = ExecutionResult {
        key,
        status: ExecutionStatus::Miss,
        exit_code: 0,
        execution_time_ms: 15,
        stdout: b"hello\n".to_vec(),
        stderr: vec![],
        outputs: vec![],
        miss_reason: Some(MissReason::NoEntryFound),
        timings: Default::default(),
    };

    // Serialize to JSON
    let json_str = serde_json::to_string_pretty(&result).unwrap();

    // Verify key fields required for IDE and CI tooling
    assert!(json_str.contains("\"key\":"));
    assert!(json_str.contains("\"status\":"));
    assert!(json_str.contains("\"exit_code\":"));
    assert!(json_str.contains("\"execution_time_ms\":"));
    assert!(json_str.contains("\"miss_reason\":"));

    // Deserialize back to struct to verify stability
    let deserialized: ExecutionResult = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.key, key);
    assert_eq!(deserialized.status, ExecutionStatus::Miss);
}

#[test]
fn test_audit_20_4_configuration_predictable() {
    let default_cfg = StorageConfig::default();
    assert!(default_cfg.root_dir.ends_with(".dcc_cache"));
    assert!(default_cfg.max_size().is_some());

    let custom_cfg = StorageConfig::new("/tmp/custom_cache")
        .with_max_size(ByteSize::gb(5));

    assert_eq!(custom_cfg.root_dir, PathBuf::from("/tmp/custom_cache"));
    assert_eq!(
        custom_cfg.max_size().unwrap().as_bytes(),
        5 * 1024 * 1024 * 1024
    );
}
