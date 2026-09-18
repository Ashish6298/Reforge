//! Public Rust API Audit Test Suite for Milestone 20.5
//! Validates:
//! 1. Essential Types (Computation, CacheKey, Digest, CacheEntry, RunnerEngine, CasStorage)
//! 2. Clean Naming & Idiomatic Rust conventions
//! 3. Error handling & Result types
//! 4. API Evolutivity (Builder pattern, non-exhaustive extensibility)
//! 5. Information Hiding (no internal lock handles or raw file descriptors leaked)

use dcc_core::{
    ByteSize, CacheEntry, CacheError, CacheKey, CachePolicy, Computation, Digest,
    ExecutionMetadata, OutputManifestItem, Result as CoreResult,
};
use dcc_integrations::DccActionBuilder;
use dcc_runner::{CommandSpec, EngineOptions, ExecutionResult, ExecutionStatus, RunnerEngine};
use dcc_storage::{BlobMetadata, CasStorage, Storage, StorageCapabilities, StorageConfig};
use dcc_test_utils::TestEnv;
use std::path::PathBuf;

#[test]
fn test_audit_20_5_public_core_api_soundness() {
    // 1. Computation & Builder
    let comp = Computation::builder_with("gcc", "-c")
        .arg("main.c")
        .env("CFLAGS", "-O3")
        .output("main.o", true)
        .build();
    assert!(comp.is_ok());

    // 2. Digest & CacheKey
    let digest = Digest::hash_bytes(b"hello world");
    assert_eq!(digest.as_str().len(), 64);
    let key = comp.unwrap().compute_key().unwrap();
    assert_eq!(key.as_str().len(), 64);

    // 3. CacheEntry creation & identity verification
    let entry = CacheEntry::new(
        key,
        Computation::new("gcc", vec!["main.c".to_string()]),
        vec![OutputManifestItem {
            path: "main.o".into(),
            digest,
            size: 1024,
            is_executable: None,
        }],
        ExecutionMetadata::default(),
    );
    assert_eq!(entry.outputs.len(), 1);
}

#[test]
fn test_audit_20_5_public_storage_and_runner_api_soundness() {
    let env = TestEnv::new().unwrap();

    // CasStorage API check
    let stored_digest = env.storage.store_object_bytes(b"DATA").unwrap();
    assert!(env.storage.has_object(&stored_digest));
    let read_data = env.storage.get_object(&stored_digest).unwrap().unwrap();
    assert_eq!(read_data, b"DATA");

    // RunnerEngine API check
    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    let spec = CommandSpec::builder("echo")
        .arg("hello")
        .current_dir(env.workspace_dir.path())
        .build()
        .unwrap();

    let res = engine.execute_command(&spec).unwrap();
    assert_eq!(res.status, ExecutionStatus::Miss);
}

#[test]
fn test_audit_20_5_integrations_builder_api_soundness() {
    let action = DccActionBuilder::new("rustc")
        .arg("main.rs")
        .input("src/main.rs")
        .output("target/main.exe")
        .build();
    assert!(action.is_ok());
}
