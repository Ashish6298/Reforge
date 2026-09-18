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

#[test]
fn test_milestone_16_1_ci_behavior_matrix() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("ci_source.txt", b"CI compilation source content")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Copy-Item ci_source.txt -Destination ci_out.txt".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "cp",
        vec!["ci_source.txt".to_string(), "ci_out.txt".to_string()],
    );

    let computation = Computation::builder_with("ci-job", cmd)
        .args(args)
        .input("ci_source.txt", Digest::from_bytes(b""), 0)
        .output("ci_out.txt", true)
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // 1. Cold Run / Cache Unavailable -> Executes cleanly
    let res_cold = engine.execute(computation.clone()).unwrap();
    assert_eq!(res_cold.status, ExecutionStatus::Miss);
    assert_eq!(res_cold.exit_code, 0);

    // 2. Warm Run / Cache Available -> Hits cleanly
    fs::remove_file(env.workspace_dir.path().join("ci_out.txt")).unwrap();
    let res_warm = engine.execute(computation.clone()).unwrap();
    assert_eq!(res_warm.status, ExecutionStatus::Hit);

    // 3. Corrupted Cache -> Detects and safely falls back
    let out_digest = &res_warm.outputs[0].digest;
    let cas_path = env.storage.object_path(out_digest);
    fs::write(&cas_path, b"CORRUPTED_DATA").unwrap();
    fs::remove_file(env.workspace_dir.path().join("ci_out.txt")).unwrap();

    let res_corrupted = engine.execute(computation.clone()).unwrap();
    assert_eq!(res_corrupted.status, ExecutionStatus::Miss);
    assert_eq!(res_corrupted.exit_code, 0);
    assert_eq!(
        env.read_output_file("ci_out.txt").unwrap(),
        b"CI compilation source content"
    );

    // 4. Partially Available Cache -> Detects missing blob and falls back
    let valid_out_digest = &res_corrupted.outputs[0].digest;
    let valid_cas_path = env.storage.object_path(valid_out_digest);
    if valid_cas_path.exists() {
        fs::remove_file(&valid_cas_path).unwrap();
    }
    fs::remove_file(env.workspace_dir.path().join("ci_out.txt")).unwrap();

    let res_partial = engine.execute(computation).unwrap();
    assert_eq!(res_partial.status, ExecutionStatus::Miss);
    assert_eq!(res_partial.exit_code, 0);
}

#[test]
fn test_milestone_16_2_graceful_degradation_when_cache_fails() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("app_code.txt", b"production application logic")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Copy-Item app_code.txt -Destination app_build.bin".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "cp",
        vec!["app_code.txt".to_string(), "app_build.bin".to_string()],
    );

    let computation = Computation::builder_with("app-build", cmd)
        .args(args)
        .input("app_code.txt", Digest::from_bytes(b""), 0)
        .output("app_build.bin", true)
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

    // Corrupt JSON metadata
    let entry_file = env.storage.entry_path(&res1.key);
    fs::write(&entry_file, b"{ CORRUPTED_NON_JSON_METADATA").unwrap();
    fs::remove_file(env.workspace_dir.path().join("app_build.bin")).unwrap();

    let res_fallback_json = engine.execute(computation.clone()).unwrap();
    assert_eq!(res_fallback_json.status, ExecutionStatus::Miss);
    assert_eq!(res_fallback_json.exit_code, 0);
    assert_eq!(
        env.read_output_file("app_build.bin").unwrap(),
        b"production application logic"
    );

    // Corrupt CAS blob
    let out_digest = &res_fallback_json.outputs[0].digest;
    let cas_path = env.storage.object_path(out_digest);
    fs::write(&cas_path, b"TAMPERED_CAS_PAYLOAD").unwrap();
    fs::remove_file(env.workspace_dir.path().join("app_build.bin")).unwrap();

    let res_fallback_cas = engine.execute(computation).unwrap();
    assert_eq!(res_fallback_cas.status, ExecutionStatus::Miss);
    assert_eq!(res_fallback_cas.exit_code, 0);
    assert_eq!(
        env.read_output_file("app_build.bin").unwrap(),
        b"production application logic"
    );
}


