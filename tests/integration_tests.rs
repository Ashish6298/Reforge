use std::fs;
use dcc_core::{Computation, Digest};
use dcc_runner::{EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_test_utils::TestEnv;

#[test]
fn test_cold_miss_then_warm_hit() {
    let env = TestEnv::new().unwrap();

    let input_path = env.create_input_file("data.txt", b"input content A").unwrap();

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
    let (cmd, args) = ("cp", vec!["input.txt".to_string(), "output.txt".to_string()]);

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
    env.create_input_file("input.txt", b"version 2 (changed)").unwrap();

    let res2 = engine.execute(computation).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Miss);
    assert_ne!(res1.key, res2.key);
    assert_eq!(env.read_output_file("output.txt").unwrap(), b"version 2 (changed)");
}

#[test]
fn test_path_traversal_rejection() {
    let env = TestEnv::new().unwrap();

    let invalid_comp = Computation::builder_with("traversal", "test")
        .output("../../escaped.txt", true)
        .build();

    assert!(invalid_comp.is_err());
}

#[test]
fn test_missing_declared_output_verification() {
    let env = TestEnv::new().unwrap();

    // Command exits with 0 but fails to produce declared output file
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Write-Output 'completed without producing output'".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = ("echo", vec!["completed".to_string()]);

    let computation = Computation::builder_with("missing-out-test", cmd)
        .args(args)
        .output("expected_artifact.bin", true) // required output that is never generated
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
        "Execution must return an error if a declared required output is missing"
    );
    assert!(matches!(res.unwrap_err(), dcc_core::CacheError::MissingOutput(_)));
}
