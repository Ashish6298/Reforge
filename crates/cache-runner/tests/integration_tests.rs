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

    let computation = Computation::builder_with("copy-test", cmd)
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

    let computation = Computation::builder_with("diff-test", cmd)
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
    let invalid_comp = Computation::builder_with("traversal", "test")
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

    let computation = Computation::builder_with("missing-out", cmd)
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

#[test]
fn test_milestone_5_1_input_changes_comprehensive() {
    let env = TestEnv::new().unwrap();

    // 1. Initial State: Input A has content X
    let hash_x_bytes = b"PAYLOAD_CONTENT_HASH_X";
    env.create_input_file("input_a.txt", hash_x_bytes).unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "$content = [System.IO.File]::ReadAllText('input_a.txt'); [System.IO.File]::WriteAllText('output.txt', \"PROCESSED: $content\")".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "content=$(cat input_a.txt); echo -n \"PROCESSED: $content\" > output.txt".to_string(),
        ],
    );

    let command_spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("input_a.txt")
        .output_path("output.txt")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // Initial Execution with Input A = Hash X
    let res_x = engine.execute_command(&command_spec).unwrap();
    assert_eq!(res_x.status, ExecutionStatus::Miss);
    let key_x = res_x.key;
    let expected_x_output = format!("PROCESSED: {}", String::from_utf8_lossy(hash_x_bytes));
    assert_eq!(
        String::from_utf8_lossy(&env.read_output_file("output.txt").unwrap()).trim(),
        expected_x_output.trim()
    );

    // Verify cache hit on unchanged input
    let res_x_hit = engine.execute_command(&command_spec).unwrap();
    assert_eq!(res_x_hit.status, ExecutionStatus::Hit);
    assert_eq!(res_x_hit.key, key_x);

    // 2. Modify Input A: from Hash X to Hash Y
    let hash_y_bytes = b"PAYLOAD_CONTENT_HASH_Y_DIFFERENT";
    env.create_input_file("input_a.txt", hash_y_bytes).unwrap();

    // Execute with Input A = Hash Y
    let res_y = engine.execute_command(&command_spec).unwrap();
    assert_eq!(res_y.status, ExecutionStatus::Miss);
    let key_y = res_y.key;

    // Invariant: Key X must differ from Key Y
    assert_ne!(
        key_x, key_y,
        "input A = hash X vs input A = hash Y MUST produce different computation keys"
    );

    // Verify output reflects new Input Y
    let expected_y_output = format!("PROCESSED: {}", String::from_utf8_lossy(hash_y_bytes));
    assert_eq!(
        String::from_utf8_lossy(&env.read_output_file("output.txt").unwrap()).trim(),
        expected_y_output.trim()
    );

    // Verify MissExplainer identifies the exact input change
    let comp_x = env.storage.get_entry(&key_x).unwrap().unwrap().computation;
    let comp_y = env.storage.get_entry(&key_y).unwrap().unwrap().computation;
    let miss_reason = dcc_runner::MissExplainer::explain(&comp_y, Some(&comp_x));
    match miss_reason {
        dcc_core::MissReason::InputChanged {
            path,
            old_digest,
            new_digest,
        } => {
            assert_eq!(path, "input_a.txt");
            assert_ne!(old_digest.unwrap(), new_digest);
        }
        other => panic!("Expected InputChanged miss reason, got {:?}", other),
    }
}

#[test]
fn test_milestone_5_2_command_and_argument_changes_comprehensive() {
    let env = TestEnv::new().unwrap();

    // Shared input file for both commands
    env.create_input_file("source.txt", b"INPUT_DATA_123")
        .unwrap();

    // Command 1: generator --fast (simulated via powershell / sh)
    #[cfg(windows)]
    let (cmd1, args1) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('mode.out', 'RESULT_FAST_MODE')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd1, args1) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'RESULT_FAST_MODE' > mode.out".to_string(),
        ],
    );

    // Command 2: generator --safe (simulated via powershell / sh)
    #[cfg(windows)]
    let (cmd2, args2) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('mode.out', 'RESULT_SAFE_MODE')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd2, args2) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'RESULT_SAFE_MODE' > mode.out".to_string(),
        ],
    );

    let spec_fast = dcc_runner::CommandSpec::builder(cmd1)
        .args(args1)
        .current_dir(env.workspace_dir.path())
        .input_path("source.txt")
        .output_path("mode.out")
        .build()
        .unwrap();

    let spec_safe = dcc_runner::CommandSpec::builder(cmd2)
        .args(args2)
        .current_dir(env.workspace_dir.path())
        .input_path("source.txt")
        .output_path("mode.out")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // 1. Run generator --fast (Cold run -> MISS)
    let res_fast_1 = engine.execute_command(&spec_fast).unwrap();
    assert_eq!(res_fast_1.status, ExecutionStatus::Miss);
    let key_fast = res_fast_1.key;
    assert_eq!(
        String::from_utf8_lossy(&env.read_output_file("mode.out").unwrap()).trim(),
        "RESULT_FAST_MODE"
    );

    // 2. Run generator --safe (Cold run -> MISS)
    let res_safe_1 = engine.execute_command(&spec_safe).unwrap();
    assert_eq!(res_safe_1.status, ExecutionStatus::Miss);
    let key_safe = res_safe_1.key;
    assert_eq!(
        String::from_utf8_lossy(&env.read_output_file("mode.out").unwrap()).trim(),
        "RESULT_SAFE_MODE"
    );

    // Invariant: generator --fast and generator --safe MUST create different keys
    assert_ne!(
        key_fast, key_safe,
        "generator --fast and generator --safe must produce different cache keys"
    );

    // 3. Delete output file and rerun generator --fast (Warm run -> HIT)
    fs::remove_file(env.workspace_dir.path().join("mode.out")).unwrap();
    let res_fast_2 = engine.execute_command(&spec_fast).unwrap();
    assert_eq!(res_fast_2.status, ExecutionStatus::Hit);
    assert_eq!(res_fast_2.key, key_fast);
    assert_eq!(
        String::from_utf8_lossy(&env.read_output_file("mode.out").unwrap()).trim(),
        "RESULT_FAST_MODE"
    );

    // 4. Delete output file and rerun generator --safe (Warm run -> HIT)
    fs::remove_file(env.workspace_dir.path().join("mode.out")).unwrap();
    let res_safe_2 = engine.execute_command(&spec_safe).unwrap();
    assert_eq!(res_safe_2.status, ExecutionStatus::Hit);
    assert_eq!(res_safe_2.key, key_safe);
    assert_eq!(
        String::from_utf8_lossy(&env.read_output_file("mode.out").unwrap()).trim(),
        "RESULT_SAFE_MODE"
    );

    // 5. Verify MissExplainer identifies argument change
    let comp_fast = env
        .storage
        .get_entry(&key_fast)
        .unwrap()
        .unwrap()
        .computation;
    let comp_safe = env
        .storage
        .get_entry(&key_safe)
        .unwrap()
        .unwrap()
        .computation;
    let miss_reason = dcc_runner::MissExplainer::explain(&comp_safe, Some(&comp_fast));
    match miss_reason {
        dcc_core::MissReason::ArgumentsChanged { old, new } => {
            assert_ne!(old, new);
        }
        other => panic!("Expected ArgumentsChanged miss reason, got {:?}", other),
    }
}

#[test]
fn test_milestone_5_3_tool_version_invalidation_comprehensive() {
    let env = TestEnv::new().unwrap();

    env.create_input_file("source.rs", b"fn main() {}").unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('binary.out', 'COMPILED_BINARY')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'COMPILED_BINARY' > binary.out".to_string(),
        ],
    );

    // Spec with compiler version 1.80.0
    let spec_v1_80 = dcc_runner::CommandSpec::builder(cmd)
        .args(args.clone())
        .current_dir(env.workspace_dir.path())
        .input_path("source.rs")
        .output_path("binary.out")
        .tool_version("rustc", "1.80.0")
        .build()
        .unwrap();

    // Spec with compiler version 1.81.0
    let spec_v1_81 = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("source.rs")
        .output_path("binary.out")
        .tool_version("rustc", "1.81.0")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // 1. Run with compiler 1.80.0 (Cold run -> MISS)
    let res_80 = engine.execute_command(&spec_v1_80).unwrap();
    assert_eq!(res_80.status, ExecutionStatus::Miss);
    let key_80 = res_80.key;

    // 2. Run with compiler 1.81.0 (Cold run -> MISS, MUST NOT reuse 1.80 cached result)
    let res_81 = engine.execute_command(&spec_v1_81).unwrap();
    assert_eq!(res_81.status, ExecutionStatus::Miss);
    let key_81 = res_81.key;

    // Invariant: compiler 1.80.0 and compiler 1.81.0 must derive different keys
    assert_ne!(
        key_80, key_81,
        "A computation using compiler 1.80 must not silently reuse a result from compiler 1.81"
    );

    // 3. Rerun compiler 1.80.0 -> HIT
    fs::remove_file(env.workspace_dir.path().join("binary.out")).unwrap();
    let res_80_hit = engine.execute_command(&spec_v1_80).unwrap();
    assert_eq!(res_80_hit.status, ExecutionStatus::Hit);
    assert_eq!(res_80_hit.key, key_80);

    // 4. Verify MissExplainer identifies tool identity change
    let comp_80 = env.storage.get_entry(&key_80).unwrap().unwrap().computation;
    let comp_81 = env.storage.get_entry(&key_81).unwrap().unwrap().computation;
    let miss_reason = dcc_runner::MissExplainer::explain(&comp_81, Some(&comp_80));
    match miss_reason {
        dcc_core::MissReason::ToolChanged { reason } => {
            assert!(reason.contains("1.80.0") || reason.contains("1.81.0"));
        }
        other => panic!("Expected ToolChanged miss reason, got {:?}", other),
    }
}

#[test]
fn test_milestone_5_4_declared_environment_invalidation_comprehensive() {
    let env = TestEnv::new().unwrap();

    env.create_input_file("config.json", b"{\"app\": \"demo\"}")
        .unwrap();

    // Command outputs the value of the environment variable FEATURE_MODE to env_out.txt
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "$val = $env:FEATURE_MODE; [System.IO.File]::WriteAllText('env_out.txt', \"MODE: $val\")".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n \"MODE: $FEATURE_MODE\" > env_out.txt".to_string(),
        ],
    );

    // Spec 1 with declared env FEATURE_MODE = "legacy"
    let spec_legacy = dcc_runner::CommandSpec::builder(cmd)
        .args(args.clone())
        .current_dir(env.workspace_dir.path())
        .input_path("config.json")
        .output_path("env_out.txt")
        .env("FEATURE_MODE", "legacy")
        .build()
        .unwrap();

    // Spec 2 with declared env FEATURE_MODE = "modern"
    let spec_modern = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("config.json")
        .output_path("env_out.txt")
        .env("FEATURE_MODE", "modern")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // 1. Run spec_legacy (Cold run -> MISS)
    let res_legacy = engine.execute_command(&spec_legacy).unwrap();
    assert_eq!(res_legacy.status, ExecutionStatus::Miss);
    let key_legacy = res_legacy.key;
    assert_eq!(
        String::from_utf8_lossy(&env.read_output_file("env_out.txt").unwrap()).trim(),
        "MODE: legacy"
    );

    // 2. Run spec_modern (Cold run -> MISS, MUST NOT reuse legacy result)
    let res_modern = engine.execute_command(&spec_modern).unwrap();
    assert_eq!(res_modern.status, ExecutionStatus::Miss);
    let key_modern = res_modern.key;
    assert_eq!(
        String::from_utf8_lossy(&env.read_output_file("env_out.txt").unwrap()).trim(),
        "MODE: modern"
    );

    // Invariant: Changing declared environment must produce different cache keys
    assert_ne!(
        key_legacy, key_modern,
        "Declared environment differences must produce distinct cache keys"
    );

    // 3. Rerun spec_legacy -> HIT, restores "MODE: legacy"
    fs::remove_file(env.workspace_dir.path().join("env_out.txt")).unwrap();
    let res_legacy_hit = engine.execute_command(&spec_legacy).unwrap();
    assert_eq!(res_legacy_hit.status, ExecutionStatus::Hit);
    assert_eq!(res_legacy_hit.key, key_legacy);
    assert_eq!(
        String::from_utf8_lossy(&env.read_output_file("env_out.txt").unwrap()).trim(),
        "MODE: legacy"
    );

    // 4. Verify MissExplainer identifies environment variable change
    let comp_legacy = env
        .storage
        .get_entry(&key_legacy)
        .unwrap()
        .unwrap()
        .computation;
    let comp_modern = env
        .storage
        .get_entry(&key_modern)
        .unwrap()
        .unwrap()
        .computation;
    let miss_reason = dcc_runner::MissExplainer::explain(&comp_modern, Some(&comp_legacy));
    match miss_reason {
        dcc_core::MissReason::EnvironmentChanged { key, old, new } => {
            assert_eq!(key, "FEATURE_MODE");
            assert_eq!(old, Some("legacy".to_string()));
            assert_eq!(new, Some("modern".to_string()));
        }
        other => panic!("Expected EnvironmentChanged miss reason, got {:?}", other),
    }
}

#[test]
fn test_milestone_5_5_platform_invalidation_comprehensive() {
    let env = TestEnv::new().unwrap();

    env.create_input_file("target_app.rs", b"fn entry() {}")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('app.bin', 'COMPILED_FOR_TARGET')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'COMPILED_FOR_TARGET' > app.bin".to_string(),
        ],
    );

    // Spec 1 targeting x86_64-unknown-linux-gnu
    let platform_x86 = dcc_core::PlatformConstraints::new("linux", "x86_64")
        .with_target("x86_64-unknown-linux-gnu");
    let spec_x86 = dcc_runner::CommandSpec::builder(cmd)
        .args(args.clone())
        .current_dir(env.workspace_dir.path())
        .input_path("target_app.rs")
        .output_path("app.bin")
        .platform(platform_x86)
        .build()
        .unwrap();

    // Spec 2 targeting aarch64-unknown-linux-gnu
    let platform_arm = dcc_core::PlatformConstraints::new("linux", "aarch64")
        .with_target("aarch64-unknown-linux-gnu");
    let spec_arm = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("target_app.rs")
        .output_path("app.bin")
        .platform(platform_arm)
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // 1. Run x86 target (Cold run -> MISS)
    let res_x86 = engine.execute_command(&spec_x86).unwrap();
    assert_eq!(res_x86.status, ExecutionStatus::Miss);
    let key_x86 = res_x86.key;

    // 2. Run ARM target (Cold run -> MISS, MUST NOT reuse x86 cached result)
    let res_arm = engine.execute_command(&spec_arm).unwrap();
    assert_eq!(res_arm.status, ExecutionStatus::Miss);
    let key_arm = res_arm.key;

    // Invariant: Target architecture/triple differences must produce different keys
    assert_ne!(
        key_x86, key_arm,
        "Target platform differences (x86_64 vs aarch64) must derive distinct cache keys"
    );

    // 3. Rerun x86 target -> HIT
    fs::remove_file(env.workspace_dir.path().join("app.bin")).unwrap();
    let res_x86_hit = engine.execute_command(&spec_x86).unwrap();
    assert_eq!(res_x86_hit.status, ExecutionStatus::Hit);
    assert_eq!(res_x86_hit.key, key_x86);

    // 4. Verify MissExplainer identifies platform difference
    let comp_x86 = env
        .storage
        .get_entry(&key_x86)
        .unwrap()
        .unwrap()
        .computation;
    let comp_arm = env
        .storage
        .get_entry(&key_arm)
        .unwrap()
        .unwrap()
        .computation;
    let miss_reason = dcc_runner::MissExplainer::explain(&comp_arm, Some(&comp_x86));
    match miss_reason {
        dcc_core::MissReason::PlatformChanged { reason } => {
            assert!(reason.contains("aarch64") || reason.contains("x86_64"));
        }
        other => panic!("Expected PlatformChanged miss reason, got {:?}", other),
    }
}

#[test]
fn test_milestone_5_6_explainable_cache_misses_comprehensive() {
    // 1. Initial State: No Entry Exists
    let comp_base = Computation::builder_with("build", "rustc")
        .arg("main.rs")
        .input(
            "src/parser.rs",
            Digest::from_bytes(b"fn parse() -> bool { true }"),
            30,
        )
        .env("OPTIMIZATION_LEVEL", "2")
        .tool("rustc", Some("1.80.0".into()), None)
        .platform(
            dcc_core::PlatformConstraints::new("linux", "x86_64")
                .with_target("x86_64-unknown-linux-gnu"),
        )
        .build()
        .unwrap();

    let miss_no_entry = dcc_runner::MissExplainer::explain(&comp_base, None);
    assert_eq!(miss_no_entry, dcc_core::MissReason::NoEntryFound);
    let msg = format!("{}", miss_no_entry);
    assert!(msg.contains("No previous cache entry"));

    // 2. Input Changed
    let comp_input_changed = Computation::builder_with("build", "rustc")
        .arg("main.rs")
        .input(
            "src/parser.rs",
            Digest::from_bytes(b"fn parse() -> bool { false }"),
            31,
        )
        .env("OPTIMIZATION_LEVEL", "2")
        .tool("rustc", Some("1.80.0".into()), None)
        .platform(
            dcc_core::PlatformConstraints::new("linux", "x86_64")
                .with_target("x86_64-unknown-linux-gnu"),
        )
        .build()
        .unwrap();

    let miss_input = dcc_runner::MissExplainer::explain(&comp_input_changed, Some(&comp_base));
    match &miss_input {
        dcc_core::MissReason::InputChanged { path, .. } => {
            assert_eq!(path, "src/parser.rs");
        }
        other => panic!("Expected InputChanged, got {:?}", other),
    }
    let msg_input = format!("{}", miss_input);
    assert!(msg_input.contains("src/parser.rs"));

    // 3. Command Executable Changed
    let comp_cmd_changed = Computation::builder_with("build", "clang")
        .arg("main.rs")
        .input(
            "src/parser.rs",
            Digest::from_bytes(b"fn parse() -> bool { true }"),
            30,
        )
        .env("OPTIMIZATION_LEVEL", "2")
        .tool("rustc", Some("1.80.0".into()), None)
        .platform(
            dcc_core::PlatformConstraints::new("linux", "x86_64")
                .with_target("x86_64-unknown-linux-gnu"),
        )
        .build()
        .unwrap();

    let miss_cmd = dcc_runner::MissExplainer::explain(&comp_cmd_changed, Some(&comp_base));
    match &miss_cmd {
        dcc_core::MissReason::CommandChanged { old, new } => {
            assert_eq!(old, "rustc");
            assert_eq!(new, "clang");
        }
        other => panic!("Expected CommandChanged, got {:?}", other),
    }

    // 4. Command Arguments Changed
    let comp_args_changed = Computation::builder_with("build", "rustc")
        .arg("main.rs")
        .arg("--release")
        .input(
            "src/parser.rs",
            Digest::from_bytes(b"fn parse() -> bool { true }"),
            30,
        )
        .env("OPTIMIZATION_LEVEL", "2")
        .tool("rustc", Some("1.80.0".into()), None)
        .platform(
            dcc_core::PlatformConstraints::new("linux", "x86_64")
                .with_target("x86_64-unknown-linux-gnu"),
        )
        .build()
        .unwrap();

    let miss_args = dcc_runner::MissExplainer::explain(&comp_args_changed, Some(&comp_base));
    match &miss_args {
        dcc_core::MissReason::ArgumentsChanged { old, new } => {
            assert_eq!(old, &vec!["main.rs".to_string()]);
            assert_eq!(new, &vec!["main.rs".to_string(), "--release".to_string()]);
        }
        other => panic!("Expected ArgumentsChanged, got {:?}", other),
    }

    // 5. Tool Identity Changed
    let comp_tool_changed = Computation::builder_with("build", "rustc")
        .arg("main.rs")
        .input(
            "src/parser.rs",
            Digest::from_bytes(b"fn parse() -> bool { true }"),
            30,
        )
        .env("OPTIMIZATION_LEVEL", "2")
        .tool("rustc", Some("1.81.0".into()), None)
        .platform(
            dcc_core::PlatformConstraints::new("linux", "x86_64")
                .with_target("x86_64-unknown-linux-gnu"),
        )
        .build()
        .unwrap();

    let miss_tool = dcc_runner::MissExplainer::explain(&comp_tool_changed, Some(&comp_base));
    match &miss_tool {
        dcc_core::MissReason::ToolChanged { reason } => {
            assert!(reason.contains("1.80.0") && reason.contains("1.81.0"));
        }
        other => panic!("Expected ToolChanged, got {:?}", other),
    }

    // 6. Relevant Environment Changed
    let comp_env_changed = Computation::builder_with("build", "rustc")
        .arg("main.rs")
        .input(
            "src/parser.rs",
            Digest::from_bytes(b"fn parse() -> bool { true }"),
            30,
        )
        .env("OPTIMIZATION_LEVEL", "3")
        .tool("rustc", Some("1.80.0".into()), None)
        .platform(
            dcc_core::PlatformConstraints::new("linux", "x86_64")
                .with_target("x86_64-unknown-linux-gnu"),
        )
        .build()
        .unwrap();

    let miss_env = dcc_runner::MissExplainer::explain(&comp_env_changed, Some(&comp_base));
    match &miss_env {
        dcc_core::MissReason::EnvironmentChanged { key, old, new } => {
            assert_eq!(key, "OPTIMIZATION_LEVEL");
            assert_eq!(old.as_deref(), Some("2"));
            assert_eq!(new.as_deref(), Some("3"));
        }
        other => panic!("Expected EnvironmentChanged, got {:?}", other),
    }

    // 7. Platform Changed
    let comp_plat_changed = Computation::builder_with("build", "rustc")
        .arg("main.rs")
        .input(
            "src/parser.rs",
            Digest::from_bytes(b"fn parse() -> bool { true }"),
            30,
        )
        .env("OPTIMIZATION_LEVEL", "2")
        .tool("rustc", Some("1.80.0".into()), None)
        .platform(
            dcc_core::PlatformConstraints::new("linux", "aarch64")
                .with_target("aarch64-unknown-linux-gnu"),
        )
        .build()
        .unwrap();

    let miss_plat = dcc_runner::MissExplainer::explain(&comp_plat_changed, Some(&comp_base));
    match &miss_plat {
        dcc_core::MissReason::PlatformChanged { reason } => {
            assert!(reason.contains("aarch64") && reason.contains("x86_64"));
        }
        other => panic!("Expected PlatformChanged, got {:?}", other),
    }

    // 8. Corrupted Cache / Failed Output Integrity Verification
    let miss_corrupted = dcc_core::MissReason::CorruptedCache {
        reason: "CAS object 4a2b missing or integrity hash failed".into(),
    };
    let msg_corrupted = format!("{}", miss_corrupted);
    assert!(msg_corrupted.contains("Corrupted cache entry"));
}

#[test]
fn test_milestone_5_7_correctness_test_matrix_comprehensive() {
    let env = TestEnv::new().unwrap();

    #[cfg(windows)]
    let (cmd, base_args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Copy-Item input.txt -Destination output.txt; Write-Output 'RUN_SUCCESS'".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, base_args) = (
        "sh",
        vec![
            "-c".to_string(),
            "cp input.txt output.txt && echo 'RUN_SUCCESS'".to_string(),
        ],
    );

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // ==========================================
    // DIMENSION 1: SAME INPUTS -> CACHE HIT
    // ==========================================
    env.create_input_file("input.txt", b"BASELINE_PAYLOAD_V1")
        .unwrap();

    let spec_base = dcc_runner::CommandSpec::builder(cmd)
        .args(base_args.clone())
        .current_dir(env.workspace_dir.path())
        .input_path("input.txt")
        .output_path("output.txt")
        .tool_version("compiler", "1.0.0")
        .env("BUILD_MODE", "debug")
        .platform(dcc_core::PlatformConstraints::new("linux", "x86_64"))
        .build()
        .unwrap();

    let res_base = engine.execute_command(&spec_base).unwrap();
    assert_eq!(res_base.status, ExecutionStatus::Miss);
    let key_base = res_base.key;

    // Second run with identical inputs & spec -> HIT
    fs::remove_file(env.workspace_dir.path().join("output.txt")).unwrap();
    let res_hit = engine.execute_command(&spec_base).unwrap();
    assert_eq!(res_hit.status, ExecutionStatus::Hit);
    assert_eq!(res_hit.key, key_base);
    assert_eq!(
        env.read_output_file("output.txt").unwrap(),
        b"BASELINE_PAYLOAD_V1"
    );

    // ==========================================
    // DIMENSION 2: DIFFERENT INPUT CONTENTS -> MISS & DIFFERENT KEY
    // ==========================================
    env.create_input_file("input.txt", b"MODIFIED_PAYLOAD_V2")
        .unwrap();
    let res_diff_content = engine.execute_command(&spec_base).unwrap();
    assert_eq!(res_diff_content.status, ExecutionStatus::Miss);
    assert_ne!(res_diff_content.key, key_base);
    assert_eq!(
        env.read_output_file("output.txt").unwrap(),
        b"MODIFIED_PAYLOAD_V2"
    );

    // Reset input file to baseline for subsequent comparisons
    env.create_input_file("input.txt", b"BASELINE_PAYLOAD_V1")
        .unwrap();

    // ==========================================
    // DIMENSION 3: DIFFERENT PATHS -> MISS & DIFFERENT KEY
    // ==========================================
    env.create_input_file("other_input.txt", b"BASELINE_PAYLOAD_V1")
        .unwrap();
    let spec_diff_path = dcc_runner::CommandSpec::builder(cmd)
        .args(base_args.clone())
        .current_dir(env.workspace_dir.path())
        .input_path("other_input.txt")
        .output_path("output.txt")
        .tool_version("compiler", "1.0.0")
        .env("BUILD_MODE", "debug")
        .platform(dcc_core::PlatformConstraints::new("linux", "x86_64"))
        .build()
        .unwrap();

    let res_diff_path = engine.execute_command(&spec_diff_path).unwrap();
    assert_eq!(res_diff_path.status, ExecutionStatus::Miss);
    assert_ne!(res_diff_path.key, key_base);

    // ==========================================
    // DIMENSION 4: DIFFERENT ARGUMENTS -> MISS & DIFFERENT KEY
    // ==========================================
    #[cfg(windows)]
    let diff_args = vec![
        "-Command".to_string(),
        "Copy-Item input.txt -Destination output.txt; Write-Output 'RUN_ALT'".to_string(),
    ];
    #[cfg(not(windows))]
    let diff_args = vec![
        "-c".to_string(),
        "cp input.txt output.txt && echo 'RUN_ALT'".to_string(),
    ];

    let spec_diff_args = dcc_runner::CommandSpec::builder(cmd)
        .args(diff_args)
        .current_dir(env.workspace_dir.path())
        .input_path("input.txt")
        .output_path("output.txt")
        .tool_version("compiler", "1.0.0")
        .env("BUILD_MODE", "debug")
        .platform(dcc_core::PlatformConstraints::new("linux", "x86_64"))
        .build()
        .unwrap();

    let res_diff_args = engine.execute_command(&spec_diff_args).unwrap();
    assert_eq!(res_diff_args.status, ExecutionStatus::Miss);
    assert_ne!(res_diff_args.key, key_base);

    // ==========================================
    // DIMENSION 5: DIFFERENT ENVIRONMENT -> MISS & DIFFERENT KEY
    // ==========================================
    let spec_diff_env = dcc_runner::CommandSpec::builder(cmd)
        .args(base_args.clone())
        .current_dir(env.workspace_dir.path())
        .input_path("input.txt")
        .output_path("output.txt")
        .tool_version("compiler", "1.0.0")
        .env("BUILD_MODE", "release") // changed from debug
        .platform(dcc_core::PlatformConstraints::new("linux", "x86_64"))
        .build()
        .unwrap();

    let res_diff_env = engine.execute_command(&spec_diff_env).unwrap();
    assert_eq!(res_diff_env.status, ExecutionStatus::Miss);
    assert_ne!(res_diff_env.key, key_base);

    // ==========================================
    // DIMENSION 6: DIFFERENT TOOL VERSION -> MISS & DIFFERENT KEY
    // ==========================================
    let spec_diff_tool = dcc_runner::CommandSpec::builder(cmd)
        .args(base_args.clone())
        .current_dir(env.workspace_dir.path())
        .input_path("input.txt")
        .output_path("output.txt")
        .tool_version("compiler", "2.0.0") // changed version
        .env("BUILD_MODE", "debug")
        .platform(dcc_core::PlatformConstraints::new("linux", "x86_64"))
        .build()
        .unwrap();

    let res_diff_tool = engine.execute_command(&spec_diff_tool).unwrap();
    assert_eq!(res_diff_tool.status, ExecutionStatus::Miss);
    assert_ne!(res_diff_tool.key, key_base);

    // ==========================================
    // DIMENSION 7: DIFFERENT PLATFORM -> MISS & DIFFERENT KEY
    // ==========================================
    let spec_diff_platform = dcc_runner::CommandSpec::builder(cmd)
        .args(base_args)
        .current_dir(env.workspace_dir.path())
        .input_path("input.txt")
        .output_path("output.txt")
        .tool_version("compiler", "1.0.0")
        .env("BUILD_MODE", "debug")
        .platform(dcc_core::PlatformConstraints::new("windows", "x86_64")) // changed os
        .build()
        .unwrap();

    let res_diff_platform = engine.execute_command(&spec_diff_platform).unwrap();
    assert_eq!(res_diff_platform.status, ExecutionStatus::Miss);
    assert_ne!(res_diff_platform.key, key_base);

    // ==========================================
    // DIMENSION 8: MISSING OUTPUT ON EXECUTION -> STRICT VALIDATION ERROR
    // ==========================================
    #[cfg(windows)]
    let (no_out_cmd, no_out_args) = (
        "powershell.exe",
        vec!["-Command".to_string(), "Write-Output 'NOOP'".to_string()],
    );
    #[cfg(not(windows))]
    let (no_out_cmd, no_out_args) = ("echo", vec!["NOOP".to_string()]);

    let spec_missing_out = dcc_runner::CommandSpec::builder(no_out_cmd)
        .args(no_out_args)
        .current_dir(env.workspace_dir.path())
        .input_path("input.txt")
        .output_path("non_existent_output.txt")
        .build()
        .unwrap();

    let err_missing_out = engine.execute_command(&spec_missing_out);
    assert!(
        err_missing_out.is_err(),
        "Command execution omitting required output file must return ValidationError"
    );

    // ==========================================
    // DIMENSION 9: MODIFIED CACHED OUTPUT (INTEGRITY FAILURE) -> RECOMPUTE & HEAL
    // ==========================================
    // Restore base cache
    let entry = env.storage.get_entry(&key_base).unwrap().unwrap();
    let blob_digest = &entry.outputs[0].digest;
    let blob_path = env.storage.object_path(blob_digest);
    assert!(blob_path.exists());

    // Tamper with cached CAS object byte
    fs::write(&blob_path, b"CORRUPTED_CAS_DATA").unwrap();

    // Executing spec_base must detect the corruption, quarantine it, and safely fall back to recomputation
    fs::remove_file(env.workspace_dir.path().join("output.txt")).unwrap();
    let res_corrupted_blob = engine.execute_command(&spec_base).unwrap();
    assert_eq!(res_corrupted_blob.status, ExecutionStatus::Miss);
    assert!(
        matches!(
            res_corrupted_blob.miss_reason,
            Some(dcc_core::MissReason::CorruptedCache { .. })
        ),
        "Expected CorruptedCache miss reason"
    );
    assert_eq!(
        env.read_output_file("output.txt").unwrap(),
        b"BASELINE_PAYLOAD_V1"
    );

    // ==========================================
    // DIMENSION 10: CORRUPTED METADATA -> IDENTITY MISMATCH DETECTION & HEAL
    // ==========================================
    // Fetch newly stored entry after recomputation
    let entry_path = env.storage.entry_path(&key_base);
    assert!(entry_path.exists());

    // Corrupt entry metadata JSON (e.g. invalidate inner computation tool name)
    let entry_json = fs::read_to_string(&entry_path).unwrap();
    let corrupted_json = entry_json.replace("\"compiler\"", "\"tampered_tool\"");
    assert_ne!(entry_json, corrupted_json);
    fs::write(&entry_path, corrupted_json).unwrap();

    fs::remove_file(env.workspace_dir.path().join("output.txt")).unwrap();
    let res_corrupted_meta = engine.execute_command(&spec_base).unwrap();
    assert_eq!(res_corrupted_meta.status, ExecutionStatus::Miss);
    assert!(matches!(
        res_corrupted_meta.miss_reason,
        Some(dcc_core::MissReason::CorruptedCache { .. })
    ));

    // ==========================================
    // DIMENSION 11: PARTIAL CACHE (MISSING CAS OBJECT REFERENCED BY ENTRY)
    // ==========================================
    let entry_fresh = env.storage.get_entry(&key_base).unwrap().unwrap();
    let blob_fresh = &entry_fresh.outputs[0].digest;
    let blob_fresh_path = env.storage.object_path(blob_fresh);
    if blob_fresh_path.exists() {
        fs::remove_file(blob_fresh_path).unwrap();
    }

    fs::remove_file(env.workspace_dir.path().join("output.txt")).unwrap();
    let res_partial_cache = engine.execute_command(&spec_base).unwrap();
    assert_eq!(res_partial_cache.status, ExecutionStatus::Miss);
    assert!(matches!(
        res_partial_cache.miss_reason,
        Some(dcc_core::MissReason::CorruptedCache { .. })
    ));
    assert_eq!(
        env.read_output_file("output.txt").unwrap(),
        b"BASELINE_PAYLOAD_V1"
    );
}

#[test]
fn test_milestone_12_2_cross_platform_process_handling() {
    use dcc_runner::ProcessExecutor;
    use std::collections::BTreeMap;
    use std::time::Duration;

    // 1. Executable discovery
    #[cfg(windows)]
    {
        let cmd_exe = ProcessExecutor::discover_executable("cmd");
        assert!(cmd_exe.is_some());
        let powershell = ProcessExecutor::discover_executable("powershell");
        assert!(powershell.is_some());
    }

    #[cfg(not(windows))]
    {
        let sh = ProcessExecutor::discover_executable("sh");
        assert!(sh.is_some());
    }

    // 2. Cross-platform stdout/stderr and exit code capture
    #[cfg(windows)]
    let (cmd, args) = (
        "cmd.exe",
        vec!["/C".to_string(), "echo cross_platform_stdout".to_string()],
    );

    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec!["-c".to_string(), "echo cross_platform_stdout".to_string()],
    );

    let mut env = BTreeMap::new();
    env.insert("DCC_TEST_VAR".to_string(), "TEST_ENV_OK".to_string());

    let out = ProcessExecutor::execute(cmd, &args, &env, None).unwrap();
    assert_eq!(out.exit_code, 0);
    assert!(!out.timed_out);
    let stdout_str = String::from_utf8_lossy(&out.stdout);
    assert!(stdout_str.contains("cross_platform_stdout"));

    // 3. Process timeout termination handling
    #[cfg(windows)]
    let (sleep_cmd, sleep_args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Start-Sleep -Milliseconds 1200".to_string(),
        ],
    );

    #[cfg(not(windows))]
    let (sleep_cmd, sleep_args) = ("sleep", vec!["1.2".to_string()]);

    let timeout_res = ProcessExecutor::execute_with_timeout(
        sleep_cmd,
        &sleep_args,
        &BTreeMap::new(),
        None,
        Some(Duration::from_millis(100)),
    )
    .unwrap();

    assert!(timeout_res.timed_out);
    assert_eq!(timeout_res.exit_code, -1);
}

#[test]
fn test_milestone_12_3_file_semantics_cross_platform() {
    use dcc_core::{Digest, OutputManifestItem, PathUtils};
    use dcc_runner::OutputRestorer;
    use std::fs;

    let env = TestEnv::new().unwrap();

    // ==========================================
    // 1. SYMLINKS & RESOLUTION
    // ==========================================
    let target_file = env.workspace_dir.path().join("target_file.txt");
    fs::write(&target_file, b"SYMLINK_TARGET_PAYLOAD_12_3").unwrap();
    let _symlink_path = env.workspace_dir.path().join("symlink_link.txt");

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        if symlink(&target_file, &_symlink_path).is_ok() {
            let digest_target = Digest::hash_file(&target_file).unwrap();
            let digest_link = Digest::hash_file(&_symlink_path).unwrap();
            assert_eq!(
                digest_target, digest_link,
                "Hashing through symlink resolves to underlying content"
            );
        }
    }

    // ==========================================
    // 2. PERMISSIONS & EXECUTABLE BITS
    // ==========================================
    let script_name = "test_script.sh";
    let script_path = env.workspace_dir.path().join(script_name);
    fs::write(&script_path, b"#!/bin/sh\necho hello\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();

        let meta = fs::metadata(&script_path).unwrap();
        let is_exec = meta.permissions().mode() & 0o111 != 0;
        assert!(
            is_exec,
            "Executable permission bit must be detected on Unix"
        );
    }

    // ==========================================
    // 3. CASE-SENSITIVE VS CASE-INSENSITIVE PATHS
    // ==========================================
    // Verify PathUtils canonicalization distinguishes case-sensitive names
    let lower_path = "src/models/item.rs";
    let upper_path = "src/models/ITEM.rs";
    assert_ne!(
        PathUtils::canonicalize_for_key(lower_path),
        PathUtils::canonicalize_for_key(upper_path),
        "Case-sensitivity is strictly preserved in canonical key identity"
    );

    let d_content = Digest::from_bytes(b"DATA");
    let comp_lower = dcc_core::Computation::builder_with("check", "tool")
        .input(lower_path, d_content.clone(), 4)
        .build()
        .unwrap();

    let comp_upper = dcc_core::Computation::builder_with("check", "tool")
        .input(upper_path, d_content, 4)
        .build()
        .unwrap();

    let key_lower = dcc_core::CanonicalComputation::from_computation(&comp_lower)
        .compute_key()
        .unwrap();
    let key_upper = dcc_core::CanonicalComputation::from_computation(&comp_upper)
        .compute_key()
        .unwrap();

    assert_ne!(
        key_lower, key_upper,
        "Distinct path casing produces distinct computation keys across platforms"
    );

    // ==========================================
    // 4. PATH SEPARATORS ('/' VS '\')
    // ==========================================
    let win_sep = r"src\components\layout\grid.rs";
    let unix_sep = "src/components/layout/grid.rs";
    assert_eq!(
        PathUtils::to_normalized_string(win_sep),
        PathUtils::to_normalized_string(unix_sep),
        "Path normalization unifies Windows and Unix separators"
    );
    assert_eq!(
        PathUtils::canonicalize_for_key(win_sep),
        PathUtils::canonicalize_for_key(unix_sep),
        "Key canonicalization is invariant to separator differences"
    );

    // ==========================================
    // 5. RESTORATION PERMISSIONS & METADATA ROUNDTRIP
    // ==========================================
    let payload = b"RESTORE_PAYLOAD_WITH_EXEC_BIT";
    let (blob_digest, blob_size) = env.storage.store_object_bytes(payload).unwrap();

    let outputs = vec![OutputManifestItem {
        path: "bin/tool_artifact".to_string(),
        digest: blob_digest,
        size: blob_size,
        is_executable: Some(true),
    }];

    let execution = dcc_core::ExecutionMetadata {
        exit_code: 0,
        execution_time_ms: 10,
        stdout_digest: None,
        stderr_digest: None,
        timings: dcc_core::TimingMetrics::default(),
    };

    let entry = dcc_core::CacheEntry::new(key_lower, comp_lower, outputs, execution);

    OutputRestorer::restore_entry(&env.storage, &entry, env.workspace_dir.path()).unwrap();
    let restored_file = env.workspace_dir.path().join("bin/tool_artifact");
    assert!(restored_file.is_file());
    assert_eq!(fs::read(&restored_file).unwrap(), payload);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::metadata(&restored_file).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o111,
            0o111,
            "Restored executable output must preserve 0o755 executable permission bit"
        );
    }
}
