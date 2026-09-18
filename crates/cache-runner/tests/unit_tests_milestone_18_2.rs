//! Integration Test Suite for Milestone 18.2
//! Tests complete end-to-end flows:
//! 1. Command Miss
//! 2. Command Hit
//! 3. Input Changed
//! 4. Output Missing
//! 5. Cache Corruption
//! 6. Concurrent Access
//! 7. Failed Command

use dcc_core::CacheError;
use dcc_runner::{EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_test_utils::TestEnv;
use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

// ============================================================================
// 1. FLOW 1: COMMAND MISS
// ============================================================================

#[test]
fn test_flow_1_command_miss_lifecycle() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("data.json", b"{\"version\": 1}")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('output.txt', 'GENERATED_DATA'); [Console]::Out.Write('STDOUT_LOG')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'GENERATED_DATA' > output.txt && echo -n 'STDOUT_LOG'".to_string(),
        ],
    );

    let spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("data.json")
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

    let res = engine.execute_command(&spec).unwrap();
    assert_eq!(res.status, ExecutionStatus::Miss);
    assert_eq!(res.exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&res.stdout), "STDOUT_LOG");
    assert_eq!(
        env.read_output_file("output.txt").unwrap(),
        b"GENERATED_DATA"
    );

    // Verify metadata was stored into CAS
    let stored = env.storage.get_entry(&res.key).unwrap();
    assert!(stored.is_some());
    assert_eq!(stored.unwrap().outputs[0].path, "output.txt");
}

// ============================================================================
// 2. FLOW 2: COMMAND HIT
// ============================================================================

#[test]
fn test_flow_2_command_hit_lifecycle() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("source.txt", b"INPUT_PAYLOAD_V1")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('result.bin', 'ARTIFACT_BYTES')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'ARTIFACT_BYTES' > result.bin".to_string(),
        ],
    );

    let spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("source.txt")
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

    // Cold run (Miss)
    let res1 = engine.execute_command(&spec).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);

    // Delete workspace output artifact
    fs::remove_file(env.workspace_dir.path().join("result.bin")).unwrap();
    assert!(!env.workspace_dir.path().join("result.bin").exists());

    // Warm run (Hit)
    let res2 = engine.execute_command(&spec).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Hit);
    assert_eq!(res2.key, res1.key);
    assert_eq!(res2.execution_time_ms, 0);
    assert!(env.workspace_dir.path().join("result.bin").exists());
    assert_eq!(
        env.read_output_file("result.bin").unwrap(),
        b"ARTIFACT_BYTES"
    );
}

// ============================================================================
// 3. FLOW 3: INPUT CHANGED
// ============================================================================

#[test]
fn test_flow_3_input_changed_invalidation() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("input.txt", b"CONTENT_STATE_A")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "$c = [System.IO.File]::ReadAllText('input.txt'); [System.IO.File]::WriteAllText('out.txt', \"OUT: $c\")".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n \"OUT: $(cat input.txt)\" > out.txt".to_string(),
        ],
    );

    let spec = dcc_runner::CommandSpec::builder(cmd)
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

    // First execution (State A)
    let res_a = engine.execute_command(&spec).unwrap();
    assert_eq!(res_a.status, ExecutionStatus::Miss);

    // Modify input file (State B)
    env.create_input_file("input.txt", b"CONTENT_STATE_B_MODIFIED")
        .unwrap();

    // Second execution (State B -> Must Miss with different key)
    let res_b = engine.execute_command(&spec).unwrap();
    assert_eq!(res_b.status, ExecutionStatus::Miss);
    assert_ne!(
        res_a.key, res_b.key,
        "Altering input bytes must change computation key"
    );
    assert_eq!(
        String::from_utf8_lossy(&env.read_output_file("out.txt").unwrap()).trim(),
        "OUT: CONTENT_STATE_B_MODIFIED"
    );
}

// ============================================================================
// 4. FLOW 4: OUTPUT MISSING
// ============================================================================

#[test]
fn test_flow_4_output_missing_fails_validation() {
    let env = TestEnv::new().unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Write-Output 'exiting without creating output file'".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = ("echo", vec!["noop".to_string()]);

    let spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .output_path("missing_target.bin") // required output
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
// 5. FLOW 5: CACHE CORRUPTION
// ============================================================================

#[test]
fn test_flow_5_cache_corruption_quarantine_and_fallback() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("src.dat", b"HEALTHY_SOURCE_DATA")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('dist.bin', 'CORRECT_OUTPUT')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'CORRECT_OUTPUT' > dist.bin".to_string(),
        ],
    );

    let spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("src.dat")
        .output_path("dist.bin")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // Initial run: Stores clean cache entry
    let res1 = engine.execute_command(&spec).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);

    // Corrupt the CAS object on disk
    let out_digest = &res1.outputs[0].digest;
    let cas_path = env.storage.object_path(out_digest);
    fs::write(&cas_path, b"CORRUPTED_TAMPERED_BYTES").unwrap();

    // Remove workspace output
    fs::remove_file(env.workspace_dir.path().join("dist.bin")).unwrap();

    // Second run: Detects corruption, quarantines object, falls back to re-computation
    let res2 = engine.execute_command(&spec).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Miss);
    assert_eq!(res2.exit_code, 0);
    assert_eq!(env.read_output_file("dist.bin").unwrap(), b"CORRECT_OUTPUT");
}

// ============================================================================
// 6. FLOW 6: CONCURRENT ACCESS
// ============================================================================

#[test]
fn test_flow_6_concurrent_access_deduplication() {
    let env = Arc::new(TestEnv::new().unwrap());
    env.create_input_file("shared_src.txt", b"PARALLEL_INPUT_DATA")
        .unwrap();

    let num_threads = 6;
    let barrier = Arc::new(Barrier::new(num_threads));
    let mut handles = Vec::new();

    for thread_id in 0..num_threads {
        let env_clone = Arc::clone(&env);
        let barrier_clone = Arc::clone(&barrier);

        handles.push(thread_id_runner(env_clone, barrier_clone, thread_id));
    }

    let mut hit_count = 0;
    let mut miss_count = 0;

    for handle in handles {
        let (status, _out_file) = handle.join().unwrap();
        match status {
            ExecutionStatus::Hit => hit_count += 1,
            ExecutionStatus::Miss => miss_count += 1,
            ExecutionStatus::Bypassed => {}
        }
    }

    // Under ComputationLock deduplication, exactly 1 thread executes cold (Miss) while 5 receive warm Hits
    assert_eq!(miss_count, 1, "Exactly 1 worker executes the computation");
    assert_eq!(
        hit_count,
        num_threads - 1,
        "Remaining workers receive instant cache hits"
    );
}

fn thread_id_runner(
    env: Arc<TestEnv>,
    barrier: Arc<Barrier>,
    thread_id: usize,
) -> thread::JoinHandle<(ExecutionStatus, String)> {
    thread::spawn(move || {
        let thread_workspace = env.workspace_dir.path().join(format!("ws_{}", thread_id));
        fs::create_dir_all(&thread_workspace).unwrap();
        fs::copy(
            env.workspace_dir.path().join("shared_src.txt"),
            thread_workspace.join("shared_src.txt"),
        )
        .unwrap();

        #[cfg(windows)]
        let (cmd, args) = (
            "powershell.exe",
            vec![
                "-Command".to_string(),
                "Start-Sleep -Milliseconds 100; [System.IO.File]::WriteAllText('shared_out.bin', 'COMPILED_RESULT')".to_string(),
            ],
        );
        #[cfg(not(windows))]
        let (cmd, args) = (
            "sh",
            vec![
                "-c".to_string(),
                "sleep 0.1 && echo -n 'COMPILED_RESULT' > shared_out.bin".to_string(),
            ],
        );

        let spec = dcc_runner::CommandSpec::builder(cmd)
            .args(args)
            .current_dir(&thread_workspace)
            .input_path("shared_src.txt")
            .output_path("shared_out.bin")
            .build()
            .unwrap();

        let engine = RunnerEngine::new(
            &env.storage,
            EngineOptions {
                working_dir: thread_workspace.clone(),
                ..Default::default()
            },
        );

        // Synchronize all threads before executing simultaneously
        barrier.wait();

        let res = engine.execute_command(&spec).unwrap();
        let out_content = fs::read(thread_workspace.join("shared_out.bin")).unwrap();
        assert_eq!(out_content, b"COMPILED_RESULT");

        (res.status, format!("ws_{}", thread_id))
    })
}

// ============================================================================
// 7. FLOW 7: FAILED COMMAND
// ============================================================================

#[test]
fn test_flow_7_failed_command_never_cached() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("faulty_input.txt", b"TRIGGER_ERROR")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[Console]::Error.Write('SYNTAX_ERROR_LINE_42'); exit 1".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'SYNTAX_ERROR_LINE_42' >&2; exit 1".to_string(),
        ],
    );

    let spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("faulty_input.txt")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // Run 1: Fails with exit code 1
    let res1 = engine.execute_command(&spec).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);
    assert_eq!(res1.exit_code, 1);
    assert_eq!(
        String::from_utf8_lossy(&res1.stderr),
        "SYNTAX_ERROR_LINE_42"
    );

    // Verify DO NOT CACHE: Metadata entry MUST NOT exist
    assert!(env.storage.get_entry(&res1.key).unwrap().is_none());

    // Run 2: Since nothing was stored, run 2 must execute again (Miss), not Hit
    let res2 = engine.execute_command(&spec).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Miss);
    assert_eq!(res2.exit_code, 1);
    assert_eq!(res2.key, res1.key);
}
