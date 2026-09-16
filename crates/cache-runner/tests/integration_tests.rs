use dcc_core::{Computation, Digest};
use dcc_runner::{EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_test_utils::TestEnv;
use std::fs;

#[test]
fn test_cold_miss_then_warm_hit() {
    let env = TestEnv::new().unwrap();

    let _input_path = env
        .create_input_file("data.txt", b"input content A")
        .unwrap();

    // Use a python/cmd/powershell or simple command
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Copy-Item data.txt -Destination out.txt".to_string(),
        ],
    );

    #[cfg(not(windows))]
    let (cmd, args) = ("cp", vec!["data.txt".to_string(), "out.txt".to_string()]);

    let computation = Computation::builder("copy-test", cmd)
        .args(args)
        .input("data.txt", Digest::from_bytes(b""), 0)
        .output("out.txt", true)
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // 1st Execution: MISS
    let res1 = engine.execute(computation.clone()).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);
    assert_eq!(res1.exit_code, 0);
    assert_eq!(env.read_output_file("out.txt").unwrap(), b"input content A");

    // Remove output file to verify cache restores it
    fs::remove_file(env.workspace_dir.path().join("out.txt")).unwrap();
    assert!(!env.workspace_dir.path().join("out.txt").exists());

    // 2nd Execution: HIT
    let res2 = engine.execute(computation).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Hit);
    assert_eq!(res2.key, res1.key);
    assert!(env.workspace_dir.path().join("out.txt").exists());
    assert_eq!(env.read_output_file("out.txt").unwrap(), b"input content A");
}

#[test]
fn test_input_change_causes_cache_miss() {
    let env = TestEnv::new().unwrap();

    env.create_input_file("input.txt", b"version 1").unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Copy-Item input.txt -Destination output.txt".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "cp",
        vec!["input.txt".to_string(), "output.txt".to_string()],
    );

    let computation = Computation::builder("diff-test", cmd)
        .args(args)
        .input("input.txt", Digest::from_bytes(b""), 0)
        .output("output.txt", true)
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    let res1 = engine.execute(computation.clone()).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);

    // Change input content
    env.create_input_file("input.txt", b"version 2 (changed)")
        .unwrap();

    let res2 = engine.execute(computation).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Miss);
    assert_ne!(res1.key, res2.key);
    assert_eq!(
        env.read_output_file("output.txt").unwrap(),
        b"version 2 (changed)"
    );
}

#[test]
fn test_path_traversal_rejection() {
    let invalid_comp = Computation::builder("traversal", "test")
        .output("../../escaped.txt", true)
        .build();

    assert!(invalid_comp.is_err());
}

#[test]
fn test_missing_declared_output_verification() {
    let env = TestEnv::new().unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Write-Output 'no output generated'".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = ("echo", vec!["no output".to_string()]);

    let computation = Computation::builder("missing-out", cmd)
        .args(args)
        .output("never_created_output.txt", true)
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    let res = engine.execute(computation);
    assert!(
        res.is_err(),
        "Must error if declared required output was not created"
    );
    assert!(matches!(
        res.unwrap_err(),
        dcc_core::CacheError::MissingOutput(_)
    ));
}

#[test]
fn test_full_execution_lifecycle_state_machine() {
    let env = TestEnv::new().unwrap();

    // 1. Prepare inputs on disk
    let src_path = env
        .create_input_file("source.txt", b"function calculate() { return 42; }")
        .unwrap();
    assert!(src_path.is_file());

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Copy-Item source.txt -Destination compiled.bin; Write-Output 'Compilation Succeeded'"
                .to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "cp source.txt compiled.bin && echo 'Compilation Succeeded'".to_string(),
        ],
    );

    let command_spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("source.txt")
        .output_path("compiled.bin")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // Initial Execution: Lifecycle executes [Prepare -> Collect -> Calculate Key -> Lookup -> MISS -> Execute -> Validate -> Store]
    let res_miss = engine.execute_command(&command_spec).unwrap();
    assert_eq!(res_miss.status, ExecutionStatus::Miss);
    assert_eq!(res_miss.exit_code, 0);
    assert_eq!(res_miss.outputs.len(), 1);
    assert_eq!(res_miss.outputs[0].path, "compiled.bin");
    assert_eq!(
        env.read_output_file("compiled.bin").unwrap(),
        b"function calculate() { return 42; }"
    );

    // Verify metadata entry stored in CAS
    let stored_entry = env.storage.get_entry(&res_miss.key).unwrap();
    assert!(stored_entry.is_some());
    assert_eq!(stored_entry.unwrap().key, res_miss.key);

    // Wipe output file to simulate fresh run
    fs::remove_file(env.workspace_dir.path().join("compiled.bin")).unwrap();
    assert!(!env.workspace_dir.path().join("compiled.bin").exists());

    // Second Execution: Lifecycle executes [Prepare -> Collect -> Calculate Key -> Lookup -> HIT -> Restore]
    let res_hit = engine.execute_command(&command_spec).unwrap();
    assert_eq!(res_hit.status, ExecutionStatus::Hit);
    assert_eq!(res_hit.key, res_miss.key);
    assert_eq!(res_hit.execution_time_ms, 0);
    assert!(env.workspace_dir.path().join("compiled.bin").exists());
    assert_eq!(
        env.read_output_file("compiled.bin").unwrap(),
        b"function calculate() { return 42; }"
    );
}

#[test]
fn test_cache_hit_lifecycle_and_guarantees() {
    let env = TestEnv::new().unwrap();

    // 1. Prepare input
    env.create_input_file("input.json", b"{\"name\": \"dcc\"}")
        .unwrap();

    // The command writes "artifact.out" and also writes to stdout and stderr
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllBytes('artifact.out', [System.Text.Encoding]::UTF8.GetBytes('BUILD_RESULT_123')); [Console]::Out.Write('STDOUT_PAYLOAD'); [Console]::Error.Write('STDERR_PAYLOAD')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'BUILD_RESULT_123' > artifact.out && echo -n 'STDOUT_PAYLOAD' && echo -n 'STDERR_PAYLOAD' >&2".to_string(),
        ],
    );

    let command_spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("input.json")
        .output_path("artifact.out")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // Initial run (MISS): Populates cache
    let miss_result = engine.execute_command(&command_spec).unwrap();
    assert_eq!(miss_result.status, ExecutionStatus::Miss);
    assert_eq!(miss_result.exit_code, 0);
    assert_eq!(
        String::from_utf8_lossy(&miss_result.stdout),
        "STDOUT_PAYLOAD"
    );
    assert_eq!(
        String::from_utf8_lossy(&miss_result.stderr),
        "STDERR_PAYLOAD"
    );
    assert_eq!(
        env.read_output_file("artifact.out").unwrap(),
        b"BUILD_RESULT_123"
    );

    // Verify metadata was stored
    let entry_opt = env.storage.get_entry(&miss_result.key).unwrap();
    assert!(entry_opt.is_some());
    let entry = entry_opt.unwrap();
    assert_eq!(entry.metadata.execution.exit_code, 0);
    assert!(entry.metadata.execution.stdout_digest.is_some());
    assert!(entry.metadata.execution.stderr_digest.is_some());

    // Delete output file and overwrite with garbage to test restoration
    fs::remove_file(env.workspace_dir.path().join("artifact.out")).unwrap();

    // Now execute again: Must produce a CACHE HIT
    // Verification:
    // 1. Metadata retrieved
    // 2. Cache integrity verified
    // 3. Output restored (artifact.out recreated with original bytes)
    // 4. Metadata restored (stdout, stderr, exit code)
    // 5. Reported as HIT (status = Hit, miss_reason = None)
    // 6. Command was not executed (proven because execution_time_ms = 0 and CAS cache hit is reported)
    let hit_result = engine.execute_command(&command_spec).unwrap();
    assert_eq!(hit_result.status, ExecutionStatus::Hit);
    assert_eq!(hit_result.key, miss_result.key);
    assert_eq!(hit_result.exit_code, 0);
    assert_eq!(hit_result.execution_time_ms, 0);
    assert_eq!(hit_result.miss_reason, None);
    assert_eq!(
        String::from_utf8_lossy(&hit_result.stdout),
        "STDOUT_PAYLOAD"
    );
    assert_eq!(
        String::from_utf8_lossy(&hit_result.stderr),
        "STDERR_PAYLOAD"
    );
    assert_eq!(
        env.read_output_file("artifact.out").unwrap(),
        b"BUILD_RESULT_123"
    );
}

#[test]
fn test_cache_miss_lifecycle_and_guarantees() {
    let env = TestEnv::new().unwrap();

    // Setup input file
    env.create_input_file("source.in", b"RAW_SOURCE_DATA_V1")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllBytes('result.bin', [System.Text.Encoding]::UTF8.GetBytes('TRANSFORMED_DATA')); [Console]::Out.Write('MISS_STDOUT_LOG'); [Console]::Error.Write('MISS_STDERR_LOG')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'TRANSFORMED_DATA' > result.bin && echo -n 'MISS_STDOUT_LOG' && echo -n 'MISS_STDERR_LOG' >&2".to_string(),
        ],
    );

    let command_spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("source.in")
        .output_path("result.bin")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // Step 1 - 9 Verification on Cache Miss:
    // 1. Report reason: NoEntryFound
    // 2. Execute command
    // 3. Capture exit code (0)
    // 4. Capture stdout/stderr ('MISS_STDOUT_LOG' / 'MISS_STDERR_LOG')
    // 5. Verify outputs ('result.bin' exists on disk)
    // 6. Hash outputs (computes valid SHA-256 digest)
    // 7. Store outputs (CAS blobs stored)
    // 8. Store metadata (CacheEntry committed)
    // 9. Return ExecutionResult
    let result = engine.execute_command(&command_spec).unwrap();

    // 1. Report Reason
    assert_eq!(result.status, ExecutionStatus::Miss);
    assert_eq!(result.miss_reason, Some(dcc_core::MissReason::NoEntryFound));

    // 3. Exit Code
    assert_eq!(result.exit_code, 0);

    // 4. Capture stdout / stderr
    assert_eq!(String::from_utf8_lossy(&result.stdout), "MISS_STDOUT_LOG");
    assert_eq!(String::from_utf8_lossy(&result.stderr), "MISS_STDERR_LOG");

    // 5. Verify Outputs exist on disk
    assert!(env.workspace_dir.path().join("result.bin").exists());

    // 6. Hash Outputs & Manifest
    assert_eq!(result.outputs.len(), 1);
    assert_eq!(result.outputs[0].path, "result.bin");
    assert_eq!(result.outputs[0].size, 16); // "TRANSFORMED_DATA".len() == 16
    let expected_output_digest = Digest::from_bytes(b"TRANSFORMED_DATA");
    assert_eq!(result.outputs[0].digest, expected_output_digest);

    // 7. Store Outputs in CAS
    assert!(env.storage.has_object(&expected_output_digest));

    // 8. Store Metadata Entry
    let stored_entry = env.storage.get_entry(&result.key).unwrap();
    assert!(stored_entry.is_some());
    let entry = stored_entry.unwrap();
    assert_eq!(entry.key, result.key);
    assert_eq!(entry.outputs[0].digest, expected_output_digest);
    assert!(entry.metadata.execution.stdout_digest.is_some());
    assert!(entry.metadata.execution.stderr_digest.is_some());

    // 9. Return Execution Result
    assert_eq!(result.outputs[0].digest, expected_output_digest);
}

#[test]
fn test_failed_computations_are_not_cached_by_default() {
    let env = TestEnv::new().unwrap();

    env.create_input_file("error_input.txt", b"trigger failure")
        .unwrap();

    // Command exits with code 1 (failure) and writes error output
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[Console]::Error.Write('FATAL COMPILATION ERROR'); exit 42".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'FATAL COMPILATION ERROR' >&2; exit 42".to_string(),
        ],
    );

    let command_spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("error_input.txt")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // First execution: Fails with exit code 42
    let res1 = engine.execute_command(&command_spec).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);
    assert_eq!(res1.exit_code, 42);
    assert_eq!(
        String::from_utf8_lossy(&res1.stderr),
        "FATAL COMPILATION ERROR"
    );

    // Verify DO NOT STORE invariant: Entry must NOT be written to cache storage
    let stored_entry = env.storage.get_entry(&res1.key).unwrap();
    assert!(
        stored_entry.is_none(),
        "Failed computations (exit code != 0) must NOT be cached by default"
    );

    // Second execution: Since nothing was stored, it must run again (MISS), not HIT
    let res2 = engine.execute_command(&command_spec).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Miss);
    assert_eq!(res2.exit_code, 42);
    assert_eq!(res2.key, res1.key);

    let stored_entry2 = env.storage.get_entry(&res2.key).unwrap();
    assert!(
        stored_entry2.is_none(),
        "Second execution of failed computation must still NOT be cached"
    );
}
