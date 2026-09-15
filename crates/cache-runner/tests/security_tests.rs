use std::fs::{self, File};
use std::io::Write;
use dcc_core::{Computation, Digest};
use dcc_runner::{EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_test_utils::TestEnv;

#[test]
fn test_corrupted_cas_object_causes_safe_fallback() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("source.txt", b"important source code").unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Copy-Item source.txt -Destination build_out.txt".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = ("cp", vec!["source.txt".to_string(), "build_out.txt".to_string()]);

    let computation = Computation::builder("build-integrity", cmd)
        .args(args)
        .input("source.txt", Digest::from_bytes(b""), 0)
        .output("build_out.txt", true)
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // 1st run: store in cache
    let res1 = engine.execute(computation.clone()).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);

    // Corrupt the CAS object in storage
    let out_digest = &res1.outputs[0].digest;
    let cas_obj_path = env.storage.object_path(out_digest);
    assert!(cas_obj_path.exists());

    // Tamper with bytes
    let mut file = File::create(&cas_obj_path).unwrap();
    file.write_all(b"corrupted tampered data").unwrap();

    // 2nd run: Detection of corruption & safe fallback execution
    let res2 = engine.execute(computation).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Miss);
    assert!(res2.miss_reason.is_some());
    // Ensure final output matches legitimate source data, not corrupted CAS
    assert_eq!(env.read_output_file("build_out.txt").unwrap(), b"important source code");
}
