use dcc_core::{Computation, Digest};
use dcc_runner::{EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_test_utils::TestEnv;
use std::sync::Arc;
use std::thread;

#[test]
fn test_concurrent_identical_computations() {
    let env = Arc::new(TestEnv::new().unwrap());
    env.create_input_file("shared.txt", b"concurrent payload")
        .unwrap();

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
            let (cmd, args) = (
                "cp",
                vec!["shared.txt".to_string(), "shared_out.txt".to_string()],
            );

            let computation = Computation::builder_with("concurrent-op", cmd)
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

    assert!(
        misses >= 1,
        "At least 1 thread should execute the computation"
    );
    assert!(hits + misses == num_threads);
    assert_eq!(
        env.read_output_file("shared_out.txt").unwrap(),
        b"concurrent payload"
    );
}

#[test]
fn test_milestone_6_1_concurrent_reads_cas_objects_and_entries() {
    use std::io::Read;
    let env = Arc::new(TestEnv::new().unwrap());

    // 1. Create and store a distinct CAS object (large binary payload)
    let payload = vec![0xAB; 256 * 1024]; // 256 KB
    let (digest, size) = env.storage.store_object_bytes(&payload).unwrap();
    assert_eq!(size, payload.len() as u64);

    // 2. Create and store a valid CacheEntry
    let comp = Computation::builder_with("op_read", "test_cmd")
        .arg("--parallel")
        .build()
        .unwrap();
    let key = comp.compute_key().unwrap();
    let entry = dcc_core::CacheEntry::new(
        key.clone(),
        comp,
        vec![dcc_core::OutputManifestItem {
            path: "artifact.bin".into(),
            digest: digest.clone(),
            size: payload.len() as u64,
            is_executable: None,
        }],
        dcc_core::ExecutionMetadata {
            exit_code: 0,
            execution_time_ms: 42,
            stdout_digest: Some(digest.clone()),
            stderr_digest: None,
            timings: Default::default(),
        },
    );
    env.storage.store_entry(&entry).unwrap();

    // 3. Launch 32 concurrent reader threads accessing the same CAS object and entry simultaneously
    let num_readers = 32;
    let mut handles = Vec::new();

    for reader_id in 0..num_readers {
        let env_clone = Arc::clone(&env);
        let digest_clone = digest.clone();
        let key_clone = key.clone();
        let expected_payload = payload.clone();

        let handle = thread::spawn(move || {
            for iter in 0..20 {
                // Concurrent CAS object stream read and verify
                let mut reader = env_clone
                    .storage
                    .get_object_reader(&digest_clone)
                    .unwrap_or_else(|e| {
                        panic!(
                            "Reader {} iter {} failed to get object reader: {}",
                            reader_id, iter, e
                        )
                    });
                let mut read_buf = Vec::new();
                reader.read_to_end(&mut read_buf).unwrap();
                assert_eq!(
                    read_buf, expected_payload,
                    "Reader {} iter {} payload mismatch",
                    reader_id, iter
                );

                // Concurrent Entry metadata lookup
                let fetched_entry = env_clone
                    .storage
                    .get_entry(&key_clone)
                    .unwrap_or_else(|e| {
                        panic!(
                            "Reader {} iter {} failed to get entry: {}",
                            reader_id, iter, e
                        )
                    })
                    .expect("Entry must exist");
                assert_eq!(fetched_entry.key, key_clone);
                assert_eq!(fetched_entry.outputs[0].digest, digest_clone);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn test_milestone_6_1_concurrent_reads_runner_hit_storm() {
    let env = Arc::new(TestEnv::new().unwrap());

    // 1. Setup computation source
    env.create_input_file("source.dat", b"IMMUTABLE_CONCURRENT_DATA")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Copy-Item source.dat -Destination output.dat; [System.IO.File]::WriteAllText('extra.txt', 'EXTRA_DATA'); Write-Output 'STDOUT_RECORDED'".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "cp source.dat output.dat && echo 'EXTRA_DATA' > extra.txt && echo 'STDOUT_RECORDED'"
                .to_string(),
        ],
    );

    let spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("source.dat")
        .output_path("output.dat")
        .output_path("extra.txt")
        .build()
        .unwrap();

    // 2. Initial warm-up run (Cold run -> MISS -> Populates cache)
    let engine_init = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );
    let initial_res = engine_init.execute_command(&spec).unwrap();
    assert_eq!(initial_res.status, ExecutionStatus::Miss);
    let cached_key = initial_res.key;

    // 3. Launch 16 concurrent reader threads, each with its own isolated working directory
    let num_readers = 16;
    let mut handles = Vec::new();

    for reader_idx in 0..num_readers {
        let env_clone = Arc::clone(&env);
        let spec_clone = spec.clone();
        let key_expected = cached_key.clone();

        let handle = thread::spawn(move || {
            // Create dedicated worker workspace
            let worker_temp = tempfile::tempdir().unwrap();
            let worker_dir = worker_temp.path().to_path_buf();

            // Copy input into worker workspace so input validation succeeds
            std::fs::write(worker_dir.join("source.dat"), b"IMMUTABLE_CONCURRENT_DATA").unwrap();

            let worker_engine = RunnerEngine::new(
                &env_clone.storage,
                EngineOptions {
                    working_dir: worker_dir.clone(),
                    ..Default::default()
                },
            );

            // Execute concurrent read
            let res = worker_engine
                .execute_command(&spec_clone)
                .unwrap_or_else(|e| {
                    panic!("Worker {} failed to execute command: {}", reader_idx, e)
                });

            // Invariant: All concurrent readers must experience an immediate HIT
            assert_eq!(
                res.status,
                ExecutionStatus::Hit,
                "Worker {} should get a HIT",
                reader_idx
            );
            assert_eq!(res.key, key_expected);
            assert_eq!(res.execution_time_ms, 0);

            // Verify stdout was properly replayed
            let stdout_str = String::from_utf8_lossy(&res.stdout);
            assert!(stdout_str.contains("STDOUT_RECORDED"));

            // Verify outputs were restored correctly in worker workspace
            let restored_out = std::fs::read(worker_dir.join("output.dat")).unwrap();
            assert_eq!(restored_out, b"IMMUTABLE_CONCURRENT_DATA");

            let restored_extra = std::fs::read(worker_dir.join("extra.txt")).unwrap();
            assert_eq!(
                String::from_utf8_lossy(&restored_extra).trim(),
                "EXTRA_DATA"
            );
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn test_milestone_6_2_concurrent_writes_identical_objects_dedup_race() {
    let env = Arc::new(TestEnv::new().unwrap());
    let payload = vec![0xEE; 512 * 1024]; // 512 KB payload
    let expected_digest = Digest::from_bytes(&payload);

    let num_threads = 32;
    let mut handles = Vec::new();

    for thread_id in 0..num_threads {
        let env_clone = Arc::clone(&env);
        let payload_clone = payload.clone();
        let expected_digest_clone = expected_digest.clone();

        let handle = thread::spawn(move || {
            for iter in 0..5 {
                let (digest, size) = env_clone
                    .storage
                    .store_object_bytes(&payload_clone)
                    .unwrap_or_else(|e| {
                        panic!(
                            "Thread {} iter {} failed to store object: {}",
                            thread_id, iter, e
                        )
                    });
                assert_eq!(digest, expected_digest_clone);
                assert_eq!(size, payload_clone.len() as u64);
                assert!(env_clone.storage.verify_object(&digest).is_ok());
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Invariants:
    // 1. Storage has exactly 1 physical CAS object (deduplicated)
    assert_eq!(env.storage.count_objects().unwrap(), 1);
    // 2. The object is 100% intact and passes cryptographic verification
    assert!(env.storage.verify_object(&expected_digest).is_ok());
    // 3. Tmp directory is completely empty
    let mut tmp_entries = std::fs::read_dir(env.storage.tmp_dir()).unwrap();
    assert!(
        tmp_entries.next().is_none(),
        "tmp directory must be completely clean after concurrent racing writes"
    );
}

#[test]
fn test_milestone_6_2_concurrent_writes_distinct_objects_throughput() {
    let env = Arc::new(TestEnv::new().unwrap());
    let num_threads = 32;
    let mut handles = Vec::new();

    for thread_id in 0..num_threads {
        let env_clone = Arc::clone(&env);

        let handle = thread::spawn(move || {
            let unique_payload = format!(
                "DISTINCT_CONCURRENT_WRITE_PAYLOAD_FOR_THREAD_{}_{}",
                thread_id,
                "X".repeat(16 * 1024)
            )
            .into_bytes();
            let (digest, size) = env_clone
                .storage
                .store_object_bytes(&unique_payload)
                .unwrap();
            assert_eq!(size, unique_payload.len() as u64);
            assert!(env_clone.storage.verify_object(&digest).is_ok());
            (digest, size)
        });
        handles.push(handle);
    }

    let mut stored_digests = Vec::new();
    for handle in handles {
        let (digest, _) = handle.join().unwrap();
        stored_digests.push(digest);
    }

    // Invariants:
    // 1. All 32 distinct objects are physically present
    assert_eq!(env.storage.count_objects().unwrap(), num_threads);
    // 2. All 32 objects pass cryptographic verification
    for d in &stored_digests {
        assert!(env.storage.verify_object(d).is_ok());
    }
    // 3. Tmp directory is clean
    let mut tmp_entries = std::fs::read_dir(env.storage.tmp_dir()).unwrap();
    assert!(tmp_entries.next().is_none());
}

#[test]
fn test_milestone_6_2_concurrent_entry_writes_atomic_safety() {
    let env = Arc::new(TestEnv::new().unwrap());

    let comp = Computation::builder_with("atomic_concurrent_op", "compiler")
        .arg("--opt")
        .build()
        .unwrap();
    let key = comp.compute_key().unwrap();

    let num_writers = 24;
    let mut handles = Vec::new();

    for writer_id in 0..num_writers {
        let env_clone = Arc::clone(&env);
        let comp_clone = comp.clone();
        let key_clone = key.clone();

        let handle = thread::spawn(move || {
            let entry = dcc_core::CacheEntry::new(
                key_clone,
                comp_clone,
                Vec::new(),
                dcc_core::ExecutionMetadata {
                    exit_code: 0,
                    execution_time_ms: 10 + writer_id as u64,
                    stdout_digest: None,
                    stderr_digest: None,
                    timings: Default::default(),
                },
            );
            env_clone.storage.store_entry(&entry).unwrap();
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Invariants:
    // 1. CacheEntry on disk is perfectly valid JSON and satisfies verify_identity
    let retrieved_entry = env
        .storage
        .get_entry(&key)
        .unwrap()
        .expect("Entry must exist");
    assert_eq!(retrieved_entry.key, key);
    assert!(retrieved_entry.verify_identity().is_ok());
    // 2. Tmp directory is clean
    let mut tmp_entries = std::fs::read_dir(env.storage.tmp_dir()).unwrap();
    assert!(tmp_entries.next().is_none());
}

#[test]
fn test_milestone_6_3_duplicate_computation_avoidance_comprehensive() {
    use std::sync::Barrier;

    let env = Arc::new(TestEnv::new().unwrap());

    // Create shared execution tracking file in a common directory
    let counter_dir = tempfile::tempdir().unwrap();
    let counter_file = counter_dir.path().join("execution_counter.log");
    let counter_file_str = counter_file.to_str().unwrap().replace('\\', "/");

    // Command sleeps for 200ms and appends a line to execution_counter.log
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            format!(
                "Start-Sleep -Milliseconds 200; Add-Content -Path '{}' -Value 'EXEC_ENTRY'; [System.IO.File]::WriteAllText('output.txt', 'EXPENSIVE_COMPUTATION_OUTPUT')",
                counter_file_str
            ),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            format!(
                "sleep 0.2 && echo 'EXEC_ENTRY' >> '{}' && printf '%s' 'EXPENSIVE_COMPUTATION_OUTPUT' > output.txt",
                counter_file_str
            ),
        ],
    );

    let num_threads = 6;
    let barrier = Arc::new(Barrier::new(num_threads));
    let mut handles = Vec::new();

    for thread_idx in 0..num_threads {
        let env_clone = Arc::clone(&env);
        let barrier_clone = Arc::clone(&barrier);
        let cmd_clone = cmd.to_string();
        let args_clone = args.clone();

        let handle = thread::spawn(move || {
            let worker_temp = tempfile::tempdir().unwrap();
            let worker_dir = worker_temp.path().to_path_buf();

            let spec = dcc_runner::CommandSpec::builder(cmd_clone)
                .args(args_clone)
                .current_dir(&worker_dir)
                .output_path("output.txt")
                .build()
                .unwrap();

            let engine = RunnerEngine::new(
                &env_clone.storage,
                EngineOptions {
                    working_dir: worker_dir.clone(),
                    lock_timeout: std::time::Duration::from_secs(30),
                    ..Default::default()
                },
            );

            // Synchronize all threads to start simultaneously on an empty cache
            barrier_clone.wait();

            let res = engine
                .execute_command(&spec)
                .unwrap_or_else(|e| panic!("Thread {} failed: {}", thread_idx, e));

            // Verify output was correctly created or restored
            let out_data = std::fs::read(worker_dir.join("output.txt")).unwrap();
            assert_eq!(
                String::from_utf8_lossy(&out_data).trim(),
                "EXPENSIVE_COMPUTATION_OUTPUT"
            );

            res.status
        });
        handles.push(handle);
    }

    let mut miss_count = 0;
    let mut hit_count = 0;

    for handle in handles {
        match handle.join().unwrap() {
            ExecutionStatus::Miss => miss_count += 1,
            ExecutionStatus::Hit => hit_count += 1,
            _ => {}
        }
    }

    // Invariants:
    // 1. Exactly 1 thread executes the expensive computation (MISS)
    // 2. All remaining threads wait on lock, re-check cache, and obtain a HIT
    assert_eq!(
        miss_count, 1,
        "Exactly 1 thread should execute the computation"
    );
    assert_eq!(
        hit_count,
        num_threads - 1,
        "All other threads must receive a cache HIT after waiting on lock"
    );

    // 3. The underlying expensive command ran exactly 1 time
    let counter_content = std::fs::read_to_string(&counter_file).unwrap();
    let execution_runs = counter_content.lines().filter(|l| !l.is_empty()).count();
    assert_eq!(
        execution_runs, 1,
        "Underlying expensive command must run exactly once across all competing threads"
    );
}

#[test]
fn test_milestone_6_4_crashed_process_lock_release_and_recovery() {
    let env = Arc::new(TestEnv::new().unwrap());

    env.create_input_file("input.txt", b"RECOVERY_DATA")
        .unwrap();

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

    let spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args.clone())
        .current_dir(env.workspace_dir.path())
        .input_path("input.txt")
        .output_path("output.txt")
        .build()
        .unwrap();

    let key = {
        let comp = dcc_core::Computation::builder_with("op", cmd)
            .args(args)
            .build()
            .unwrap();
        comp.compute_key().unwrap()
    };

    // 1. Simulate a crashed process: create an orphaned lock file on disk with dead PID
    let lock_path = env
        .storage
        .locks_dir()
        .join(format!("{}.lock", key.as_str()));
    std::fs::create_dir_all(env.storage.locks_dir()).unwrap();
    let stale_meta = dcc_storage::lock::LockMetadata {
        pid: 88888,
        created_at: chrono::Utc::now() - chrono::Duration::hours(5),
        key: key.as_str().to_string(),
    };
    std::fs::write(&lock_path, serde_json::to_vec(&stale_meta).unwrap()).unwrap();
    assert!(lock_path.is_file());

    // 2. Runner engine should recover seamlessly and execute without error
    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    let res = engine
        .execute_command(&spec)
        .expect("Should recover from stale lock and execute");
    assert_eq!(res.status, ExecutionStatus::Miss);
    assert_eq!(
        env.read_output_file("output.txt").unwrap(),
        b"RECOVERY_DATA"
    );
}

#[test]
fn test_milestone_6_4_corrupted_lock_metadata_recovery_in_runner() {
    let env = Arc::new(TestEnv::new().unwrap());

    env.create_input_file("input.txt", b"CORRUPTED_LOCK_DATA")
        .unwrap();

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

    let spec = dcc_runner::CommandSpec::builder(cmd)
        .args(args.clone())
        .current_dir(env.workspace_dir.path())
        .input_path("input.txt")
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

    // Compute key in advance to plant corrupted lockfile
    let key = {
        let comp = dcc_core::Computation::builder_with("op", cmd)
            .args(args)
            .build()
            .unwrap();
        comp.compute_key().unwrap()
    };

    let lock_path = env
        .storage
        .locks_dir()
        .join(format!("{}.lock", key.as_str()));
    std::fs::create_dir_all(env.storage.locks_dir()).unwrap();
    std::fs::write(
        &lock_path,
        b"CORRUPTED_LOCK_METADATA_NON_JSON_BYTES_!@#$%^&*()",
    )
    .unwrap();

    // Runner must execute without failing or hanging
    let res = engine
        .execute_command(&spec)
        .expect("Should recover from corrupted lock metadata");
    assert_eq!(res.status, ExecutionStatus::Miss);
    assert_eq!(
        env.read_output_file("output.txt").unwrap(),
        b"CORRUPTED_LOCK_DATA"
    );
}

#[test]
fn test_milestone_6_5_stress_10_concurrent_processes() {
    use std::sync::Barrier;

    let env = Arc::new(TestEnv::new().unwrap());
    let num_processes = 10;
    let barrier = Arc::new(Barrier::new(num_processes));
    let mut handles = Vec::new();

    for proc_id in 0..num_processes {
        let env_clone = Arc::clone(&env);
        let barrier_clone = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            let worker_temp = tempfile::tempdir().unwrap();
            let worker_dir = worker_temp.path().to_path_buf();

            // 5 processes share computation "shared_cluster", 5 have unique computation keys
            let is_shared = proc_id < 5;
            let payload = if is_shared {
                "SHARED_CLUSTER_PAYLOAD".to_string()
            } else {
                format!("UNIQUE_PAYLOAD_PROC_{}", proc_id)
            };

            std::fs::write(worker_dir.join("input.dat"), payload.as_bytes()).unwrap();

            #[cfg(windows)]
            let (cmd, args) = (
                "powershell.exe",
                vec![
                    "-Command".to_string(),
                    "Copy-Item input.dat -Destination output.dat; Write-Output 'STRESS_10_OK'"
                        .to_string(),
                ],
            );
            #[cfg(not(windows))]
            let (cmd, args) = (
                "sh",
                vec![
                    "-c".to_string(),
                    "cp input.dat output.dat && echo 'STRESS_10_OK'".to_string(),
                ],
            );

            let spec = dcc_runner::CommandSpec::builder(cmd)
                .args(args)
                .current_dir(&worker_dir)
                .input_path("input.dat")
                .output_path("output.dat")
                .build()
                .unwrap();

            let engine = RunnerEngine::new(
                &env_clone.storage,
                EngineOptions {
                    working_dir: worker_dir.clone(),
                    lock_timeout: std::time::Duration::from_secs(30),
                    ..Default::default()
                },
            );

            // Synchronize launch
            barrier_clone.wait();

            let res = engine
                .execute_command(&spec)
                .unwrap_or_else(|e| panic!("Process {} failed: {}", proc_id, e));

            // Verify output integrity
            let out_data = std::fs::read(worker_dir.join("output.dat")).unwrap();
            assert_eq!(out_data, payload.as_bytes());

            res.status
        });
        handles.push(handle);
    }

    let mut misses = 0;
    let mut hits = 0;
    for handle in handles {
        match handle.join().unwrap() {
            ExecutionStatus::Miss => misses += 1,
            ExecutionStatus::Hit => hits += 1,
            _ => {}
        }
    }

    // Invariant: 5 unique processes must MISS; 5 shared processes yield 1 MISS and 4 HITS
    assert_eq!(misses, 6);
    assert_eq!(hits, 4);

    // Verify storage integrity: tmp directory clean
    let mut tmp_entries = std::fs::read_dir(env.storage.tmp_dir()).unwrap();
    assert!(tmp_entries.next().is_none());
}

#[test]
fn test_milestone_6_5_stress_50_concurrent_processes() {
    use std::sync::Barrier;

    let env = Arc::new(TestEnv::new().unwrap());
    let num_processes = 50;
    let barrier = Arc::new(Barrier::new(num_processes));
    let mut handles = Vec::new();

    for proc_id in 0..num_processes {
        let env_clone = Arc::clone(&env);
        let barrier_clone = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            let worker_temp = tempfile::tempdir().unwrap();
            let worker_dir = worker_temp.path().to_path_buf();

            // 5 clusters of 5 shared workers (25 shared) + 25 unique workers
            let cluster_id = if proc_id < 25 {
                proc_id / 5 // clusters 0..5
            } else {
                proc_id // unique keys
            };

            let payload = format!("CLUSTER_PAYLOAD_DATA_{}", cluster_id);
            std::fs::write(worker_dir.join("input.dat"), payload.as_bytes()).unwrap();

            #[cfg(windows)]
            let (cmd, args) = (
                "powershell.exe",
                vec![
                    "-Command".to_string(),
                    "Copy-Item input.dat -Destination output.dat; [System.IO.File]::WriteAllText('side.txt', 'SIDE_DATA')".to_string(),
                ],
            );
            #[cfg(not(windows))]
            let (cmd, args) = (
                "sh",
                vec![
                    "-c".to_string(),
                    "cp input.dat output.dat && printf '%s' 'SIDE_DATA' > side.txt".to_string(),
                ],
            );

            let spec = dcc_runner::CommandSpec::builder(cmd)
                .args(args)
                .current_dir(&worker_dir)
                .input_path("input.dat")
                .output_path("output.dat")
                .output_path("side.txt")
                .build()
                .unwrap();

            let engine = RunnerEngine::new(
                &env_clone.storage,
                EngineOptions {
                    working_dir: worker_dir.clone(),
                    lock_timeout: std::time::Duration::from_secs(45),
                    ..Default::default()
                },
            );

            barrier_clone.wait();

            let res = engine
                .execute_command(&spec)
                .unwrap_or_else(|e| panic!("Process {} failed: {}", proc_id, e));

            // Verify both outputs
            let out_data = std::fs::read(worker_dir.join("output.dat")).unwrap();
            assert_eq!(out_data, payload.as_bytes());

            let side_data = std::fs::read(worker_dir.join("side.txt")).unwrap();
            assert_eq!(String::from_utf8_lossy(&side_data).trim(), "SIDE_DATA");

            res.status
        });
        handles.push(handle);
    }

    let mut total_results = 0;
    for handle in handles {
        let status = handle.join().unwrap();
        assert!(matches!(
            status,
            ExecutionStatus::Hit | ExecutionStatus::Miss
        ));
        total_results += 1;
    }

    assert_eq!(total_results, num_processes);

    // Verify storage integrity: tmp directory clean
    let mut tmp_entries = std::fs::read_dir(env.storage.tmp_dir()).unwrap();
    assert!(tmp_entries.next().is_none());
}

#[test]
fn test_milestone_6_5_stress_100_concurrent_operations() {
    use std::sync::Barrier;

    let env = Arc::new(TestEnv::new().unwrap());
    let num_ops = 100;
    let barrier = Arc::new(Barrier::new(num_ops));
    let mut handles = Vec::new();

    for op_id in 0..num_ops {
        let env_clone = Arc::clone(&env);
        let barrier_clone = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            let worker_temp = tempfile::tempdir().unwrap();
            let worker_dir = worker_temp.path().to_path_buf();

            // 4 clusters of 10 workers (40 shared operations) + 60 distinct operations
            let group_id = if op_id < 40 {
                op_id / 10 // groups 0, 1, 2, 3
            } else {
                op_id // distinct
            };

            let payload = format!("STRESS_100_PAYLOAD_GROUP_{}_{}", group_id, "Z".repeat(1024));
            std::fs::write(worker_dir.join("source.dat"), payload.as_bytes()).unwrap();

            #[cfg(windows)]
            let (cmd, args) = (
                "powershell.exe",
                vec![
                    "-Command".to_string(),
                    "Copy-Item source.dat -Destination result.dat; Write-Output 'OP_SUCCESS'"
                        .to_string(),
                ],
            );
            #[cfg(not(windows))]
            let (cmd, args) = (
                "sh",
                vec![
                    "-c".to_string(),
                    "cp source.dat result.dat && echo 'OP_SUCCESS'".to_string(),
                ],
            );

            let spec = dcc_runner::CommandSpec::builder(cmd)
                .args(args)
                .current_dir(&worker_dir)
                .input_path("source.dat")
                .output_path("result.dat")
                .build()
                .unwrap();

            let engine = RunnerEngine::new(
                &env_clone.storage,
                EngineOptions {
                    working_dir: worker_dir.clone(),
                    lock_timeout: std::time::Duration::from_secs(60),
                    ..Default::default()
                },
            );

            barrier_clone.wait();

            let res = engine
                .execute_command(&spec)
                .unwrap_or_else(|e| panic!("Operation {} failed: {}", op_id, e));

            // Verify output integrity
            let out_data = std::fs::read(worker_dir.join("result.dat")).unwrap();
            assert_eq!(out_data, payload.as_bytes());

            res.key
        });
        handles.push(handle);
    }

    let mut generated_keys = Vec::new();
    for handle in handles {
        let key = handle.join().unwrap();
        generated_keys.push(key);
    }

    assert_eq!(generated_keys.len(), num_ops);

    // Full Storage & Metadata Consistency Audit
    // 1. All generated entries on disk must exist and pass verify_identity
    for key in &generated_keys {
        let entry = env
            .storage
            .get_entry(key)
            .unwrap()
            .expect("Entry must exist in storage");
        assert_eq!(&entry.key, key);
        assert!(entry.verify_identity().is_ok());

        // Verify all output CAS objects referenced by the entry
        for output in &entry.outputs {
            assert!(env.storage.verify_object(&output.digest).is_ok());
        }
    }

    // 2. tmp directory must be completely clean (no leaked temporary files)
    let mut tmp_entries = std::fs::read_dir(env.storage.tmp_dir()).unwrap();
    assert!(tmp_entries.next().is_none());

    // 3. Storage stats sanity check
    let stats = env.storage.stats().unwrap();
    assert!(stats.total_objects > 0);
    assert!(stats.total_entries > 0);
    assert!(stats.total_size_bytes > 0);
}
