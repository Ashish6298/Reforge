use std::sync::Arc;
use std::thread;
use dcc_core::{Computation, Digest};
use dcc_runner::{EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_test_utils::TestEnv;

#[test]
fn test_concurrent_identical_computations() {
    let env = Arc::new(TestEnv::new().unwrap());
    env.create_input_file("shared.txt", b"concurrent payload").unwrap();

    let num_threads = 8;
    let mut handles = Vec::new();

    for _ in 0..num_threads {
        let env_clone = Arc::clone(&env);
        let handle = thread::spawn(move || {
            #[cfg(windows)]
            let (cmd, args) = (
                "powershell.exe",
                vec![
                    "-Command".to_string(),
                    "Copy-Item shared.txt -Destination shared_out.txt".to_string(),
                ],
            );
            #[cfg(not(windows))]
            let (cmd, args) = ("cp", vec!["shared.txt".to_string(), "shared_out.txt".to_string()]);

            let computation = Computation::builder("concurrent-op", cmd)
                .args(args)
                .input("shared.txt", Digest::from_bytes(b""), 0)
                .output("shared_out.txt", true)
                .build()
                .unwrap();

            let engine = RunnerEngine::new(
                &env_clone.storage,
                EngineOptions {
                    working_dir: env_clone.workspace_dir.path().to_path_buf(),
                    ..Default::default()
                },
            );

            engine.execute(computation).unwrap()
        });
        handles.push(handle);
    }

    let mut hits = 0;
    let mut misses = 0;

    for handle in handles {
        let res = handle.join().unwrap();
        match res.status {
            ExecutionStatus::Hit => hits += 1,
            ExecutionStatus::Miss => misses += 1,
            _ => {}
        }
    }

    assert!(misses >= 1, "At least 1 thread should execute the computation");
    assert!(hits + misses == num_threads);
    assert_eq!(env.read_output_file("shared_out.txt").unwrap(), b"concurrent payload");
}
