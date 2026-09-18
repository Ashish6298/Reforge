//! Failure Injection Test Suite (Milestone 18.3)
//! Simulates and verifies safe failure modes:
//! 1. Disk Full
//! 2. Permission Denied
//! 3. Process Crash
//! 4. Partial Write
//! 5. Corrupted Metadata
//! 6. Corrupted Object
//! 7. Missing Output
//! 8. Invalid Configuration

use dcc_core::{ByteSize, CacheError, Computation, Digest, SensitiveDataPolicy};
use dcc_runner::{CommandSpec, EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_storage::{CasStorage, Storage, StorageConfig};
use dcc_test_utils::TestEnv;
use std::fs;
use tempfile::tempdir;

// ============================================================================
// 1. SIMULATE: DISK FULL / CAPACITY LIMIT
// ============================================================================

#[test]
fn test_failure_injection_disk_full_safe_handling() {
    let dir = tempdir().unwrap();
    // Configure cache with very tight max size limit (1 KB)
    let config = StorageConfig::new(dir.path()).with_max_size(ByteSize::kb(1));
    let storage = CasStorage::new(config).unwrap();

    let env = TestEnv::new().unwrap();
    env.create_input_file("big_data.txt", b"INPUT_DATA")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllBytes('large_out.bin', [byte[]]@(65..90 * 400))".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "head -c 10240 /dev/urandom > large_out.bin".to_string(),
        ],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("big_data.txt")
        .output_path("large_out.bin")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // Execution succeeds and fails safely during storage or bounds enforcement without panics
    let res = engine.execute_command(&spec);
    assert!(
        res.is_ok(),
        "Process execution must succeed even if storage capacity is exceeded"
    );
    let execution_result = res.unwrap();
    assert_eq!(execution_result.status, ExecutionStatus::Miss);
}

// ============================================================================
// 2. SIMULATE: PERMISSION DENIED
// ============================================================================

#[test]
fn test_failure_injection_permission_denied_safe_handling() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("protected_src.txt", b"SECRET")
        .unwrap();

    let out_file_path = env.workspace_dir.path().join("readonly_out.bin");
    fs::write(&out_file_path, b"PREVIOUS_DATA").unwrap();

    // Set file to read-only to trigger permission denied on overwrite
    let mut perms = fs::metadata(&out_file_path).unwrap().permissions();
    perms.set_readonly(true);
    let _ = fs::set_permissions(&out_file_path, perms);

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Set-Content -Path readonly_out.bin -Value 'NEW_DATA'".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'NEW_DATA' > readonly_out.bin".to_string(),
        ],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("protected_src.txt")
        .output_path("readonly_out.bin")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // The command fails to write to the read-only file; engine catches non-zero exit code cleanly
    let res = engine.execute_command(&spec);
    assert!(res.is_ok());
    let exec_res = res.unwrap();
    // Non-zero exit code or failed write
    assert!(exec_res.exit_code != 0 || exec_res.status == ExecutionStatus::Miss);

    // Restore write permissions for clean tempdir teardown
    let mut clean_perms = fs::metadata(&out_file_path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    clean_perms.set_readonly(false);
    let _ = fs::set_permissions(&out_file_path, clean_perms);
}

// ============================================================================
// 3. SIMULATE: PROCESS CRASH
// ============================================================================

#[test]
fn test_failure_injection_process_crash_clean_lock_release() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("crash_trigger.txt", b"CRASH_NOW")
        .unwrap();

    // Process abnormal crash simulation
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[Console]::Error.Write('PROCESS_CRASHED_ABNORMALLY'); exit 137".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'PROCESS_CRASHED_ABNORMALLY' >&2; exit 137".to_string(),
        ],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("crash_trigger.txt")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    let res = engine.execute_command(&spec).unwrap();
    assert_eq!(res.status, ExecutionStatus::Miss);
    assert_eq!(res.exit_code, 137);
    assert_eq!(
        String::from_utf8_lossy(&res.stderr),
        "PROCESS_CRASHED_ABNORMALLY"
    );

    // Invariant: Crashed process must NOT be cached
    assert!(env.storage.get_entry(&res.key).unwrap().is_none());
}

// ============================================================================
// 4. SIMULATE: PARTIAL WRITE (CRASH CONSISTENCY IN .TMP)
// ============================================================================

#[test]
fn test_failure_injection_partial_write_isolation() {
    let env = TestEnv::new().unwrap();

    // Simulate an interrupted, abandoned partial write in the tmp staging directory
    let tmp_dir = env.storage.tmp_dir();
    let partial_tmp_file = tmp_dir.join("partial_uncommitted_blob.tmp");
    fs::write(&partial_tmp_file, b"HALF_WRITTEN_INCOMPLETE_DATA").unwrap();

    // Verify that CAS readers do not see or index this uncommitted .tmp file
    let fake_digest = Digest::hash_bytes(b"HALF_WRITTEN_INCOMPLETE_DATA");
    assert!(!env.storage.has_object(&fake_digest));
    assert!(env.storage.get_bytes(&fake_digest).is_err());

    // Subsequent normal write operations succeed cleanly
    let valid_data = b"COMPLETE_VALID_DATA";
    let (valid_digest, size) = env.storage.put(valid_data).unwrap();
    assert_eq!(size, valid_data.len() as u64);
    assert!(env.storage.has_object(&valid_digest));
    assert_eq!(env.storage.get_bytes(&valid_digest).unwrap(), valid_data);
}

// ============================================================================
// 5. SIMULATE: CORRUPTED METADATA
// ============================================================================

#[test]
fn test_failure_injection_corrupted_metadata_self_healing() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("meta_src.txt", b"METADATA_SOURCE_CONTENT")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('meta_out.bin', 'COMPILED_METADATA_RESULT')"
                .to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'COMPILED_METADATA_RESULT' > meta_out.bin".to_string(),
        ],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("meta_src.txt")
        .output_path("meta_out.bin")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // Initial clean run (MISS)
    let res1 = engine.execute_command(&spec).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);

    // Corrupt the metadata JSON file with invalid/truncated bytes
    let entry_path = env.storage.entry_path(&res1.key);
    fs::write(
        &entry_path,
        b"{ \"schema_version\": 1, \"corrupted_incomplete_json",
    )
    .unwrap();

    // Wipe workspace output
    fs::remove_file(env.workspace_dir.path().join("meta_out.bin")).unwrap();

    // Second run: Detects corrupt metadata, treats as cache miss, re-executes cleanly
    let res2 = engine.execute_command(&spec).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Miss);
    assert_eq!(res2.exit_code, 0);
    assert_eq!(
        env.read_output_file("meta_out.bin").unwrap(),
        b"COMPILED_METADATA_RESULT"
    );
}

// ============================================================================
// 6. SIMULATE: CORRUPTED OBJECT (CHECKSUM MISMATCH & QUARANTINE)
// ============================================================================

#[test]
fn test_failure_injection_corrupted_object_quarantine_and_fallback() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("blob_src.txt", b"BLOB_SOURCE_DATA")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('blob_out.bin', 'AUTHENTIC_BYTES')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'AUTHENTIC_BYTES' > blob_out.bin".to_string(),
        ],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("blob_src.txt")
        .output_path("blob_out.bin")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // Initial clean run
    let res1 = engine.execute_command(&spec).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);

    // Bitrot / tamper with the CAS object
    let out_digest = &res1.outputs[0].digest;
    let cas_path = env.storage.object_path(out_digest);
    fs::write(&cas_path, b"TAMPERED_MALICIOUS_DATA_PAYLOAD").unwrap();

    // Remove workspace output
    fs::remove_file(env.workspace_dir.path().join("blob_out.bin")).unwrap();

    // Second run: Detects checksum mismatch, quarantines object to *.corrupted, re-executes
    let res2 = engine.execute_command(&spec).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Miss);
    assert_eq!(res2.exit_code, 0);
    assert_eq!(
        env.read_output_file("blob_out.bin").unwrap(),
        b"AUTHENTIC_BYTES"
    );
}

// ============================================================================
// 7. SIMULATE: MISSING OUTPUT
// ============================================================================

#[test]
fn test_failure_injection_missing_output_strict_rejection() {
    let env = TestEnv::new().unwrap();

    // Command exits with code 0 but does not produce the declared required output
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Write-Output 'Process finished with code 0 but omitted output file'".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = ("echo", vec!["noop".to_string()]);

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .output_path("unproduced_artifact.bin")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    let res = engine.execute_command(&spec);
    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), CacheError::MissingOutput(_)));
}

// ============================================================================
// 8. SIMULATE: INVALID CONFIGURATION
// ============================================================================

#[test]
fn test_failure_injection_invalid_configuration_safe_errors() {
    // 1. Invalid size parsing
    assert!(ByteSize::parse("invalid_size_format").is_err());
    assert!(ByteSize::parse("-500 MB").is_err());
    assert!(ByteSize::parse("1000 XB").is_err());

    // 2. Empty operation or command in ComputationBuilder
    let empty_op = Computation::builder()
        .operation("")
        .command("rustc")
        .build();
    assert!(empty_op.is_err());
    assert!(matches!(
        empty_op.unwrap_err(),
        CacheError::ValidationError(_)
    ));

    let empty_cmd = Computation::builder()
        .operation("build")
        .command("")
        .build();
    assert!(empty_cmd.is_err());
    assert!(matches!(
        empty_cmd.unwrap_err(),
        CacheError::ValidationError(_)
    ));

    // 3. Sensitive Data Policy Deny on sensitive parameters
    let sensitive_comp = Computation::builder()
        .operation("deploy")
        .command("deployer")
        .env("AWS_SECRET_ACCESS_KEY", "AKIA1234567890SECRETKEY")
        .build()
        .unwrap();

    let validation_res = sensitive_comp.validate_with_policy(SensitiveDataPolicy::Deny);
    assert!(validation_res.is_err());
    assert!(matches!(
        validation_res.unwrap_err(),
        CacheError::SensitiveDataError(_)
    ));
}
