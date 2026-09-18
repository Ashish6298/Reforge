use dcc_core::{CacheEntry, Computation, ExecutionMetadata, OutputManifestItem};
use dcc_runner::{EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_storage::lock::ObjectLock;
use dcc_storage::Pruner;
use dcc_test_utils::TestEnv;
use std::io::Read;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

/// 18.4 Concurrency Test 1: Many Readers
/// Verifies that multiple concurrent reader threads can simultaneously access and stream
/// CAS objects and query cache entry metadata with zero contention or corrupted reads.
#[test]
fn test_milestone_18_4_many_readers() {
    let env = Arc::new(TestEnv::new().unwrap());

    // Setup payload and store CAS object and entry
    let payload = vec![0x42; 128 * 1024]; // 128 KB
    let (digest, size) = env.storage.store_object_bytes(&payload).unwrap();
    assert_eq!(size, payload.len() as u64);

    let comp = Computation::builder_with("many_readers_op", "compiler")
        .arg("--fast")
        .build()
        .unwrap();
    let key = comp.compute_key().unwrap();
    let entry = CacheEntry::new(
        key.clone(),
        comp,
        vec![OutputManifestItem {
            path: "artifact.dat".into(),
            digest: digest.clone(),
            size: payload.len() as u64,
            is_executable: Some(false),
        }],
        ExecutionMetadata::default(),
    );
    env.storage.store_entry(&entry).unwrap();

    let num_readers = 32;
    let barrier = Arc::new(Barrier::new(num_readers));
    let mut handles = Vec::new();

    for reader_id in 0..num_readers {
        let env_clone = Arc::clone(&env);
        let barrier_clone = Arc::clone(&barrier);
        let digest_clone = digest.clone();
        let key_clone = key.clone();
        let expected_payload = payload.clone();

        let handle = thread::spawn(move || {
            barrier_clone.wait();

            for iter in 0..15 {
                // Read CAS object stream
                let mut reader = env_clone
                    .storage
                    .get_object_reader(&digest_clone)
                    .unwrap_or_else(|e| {
                        panic!(
                            "Reader {} iter {} failed to get object reader: {}",
                            reader_id, iter, e
                        )
                    });
                let mut buf = Vec::new();
                reader.read_to_end(&mut buf).unwrap();
                assert_eq!(
                    buf, expected_payload,
                    "Reader {} iter {} payload mismatch",
                    reader_id, iter
                );

                // Read cache entry
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
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }
}

/// 18.4 Concurrency Test 2: Many Writers
/// Verifies that multiple concurrent writers storing distinct and identical CAS objects
/// do not corrupt disk state, cleanly deduplicate, and keep tmp/ clean.
#[test]
fn test_milestone_18_4_many_writers() {
    let env = Arc::new(TestEnv::new().unwrap());
    let num_writers = 32;
    let barrier = Arc::new(Barrier::new(num_writers));
    let mut handles = Vec::new();

    for writer_id in 0..num_writers {
        let env_clone = Arc::clone(&env);
        let barrier_clone = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            barrier_clone.wait();

            // Store distinct object
            let distinct_payload = format!(
                "DISTINCT_PAYLOAD_WRITER_{}_{}",
                writer_id,
                "W".repeat(8 * 1024)
            );
            let (dist_digest, dist_size) = env_clone
                .storage
                .store_object_bytes(distinct_payload.as_bytes())
                .unwrap();
            assert_eq!(dist_size, distinct_payload.len() as u64);
            assert!(env_clone.storage.verify_object(&dist_digest).is_ok());

            // Race on storing shared identical object
            let shared_payload = b"SHARED_IDENTICAL_CONCURRENT_BLOB_FOR_ALL_WRITERS";
            let (shared_digest, shared_size) = env_clone
                .storage
                .store_object_bytes(shared_payload)
                .unwrap();
            assert_eq!(shared_size, shared_payload.len() as u64);
            assert!(env_clone.storage.verify_object(&shared_digest).is_ok());

            dist_digest
        });
        handles.push(handle);
    }

    let mut distinct_digests = Vec::new();
    for handle in handles {
        distinct_digests.push(handle.join().unwrap());
    }

    // 32 distinct objects + 1 shared deduplicated object = 33 total objects
    assert_eq!(env.storage.count_objects().unwrap(), num_writers + 1);

    for d in &distinct_digests {
        assert!(env.storage.verify_object(d).is_ok());
    }

    let mut tmp_entries = std::fs::read_dir(env.storage.tmp_dir()).unwrap();
    assert!(tmp_entries.next().is_none(), "Tmp directory must be clean");
}

/// 18.4 Concurrency Test 3: Same-Key Writers
/// Verifies that when multiple concurrent runners execute the exact same computation
/// simultaneously, duplicate computation is avoided via ComputationLock and secondary cache re-check,
/// yielding exactly 1 execution (Miss) and (N-1) cache hits.
#[test]
fn test_milestone_18_4_same_key_writers() {
    let env = Arc::new(TestEnv::new().unwrap());

    let shared_counter_dir = tempfile::tempdir().unwrap();
    let counter_file = shared_counter_dir.path().join("execution.log");
    let counter_file_str = counter_file.to_str().unwrap().replace('\\', "/");

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            format!(
                "Start-Sleep -Milliseconds 150; Add-Content -Path '{}' -Value 'RAN'; [System.IO.File]::WriteAllText('out.txt', 'SAME_KEY_RESULT')",
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
                "sleep 0.15 && echo 'RAN' >> '{}' && echo -n 'SAME_KEY_RESULT' > out.txt",
                counter_file_str
            ),
        ],
    );

    let num_threads = 8;
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
                .output_path("out.txt")
                .build()
                .unwrap();

            let engine = RunnerEngine::new(
                &env_clone.storage,
                EngineOptions {
                    working_dir: worker_dir.clone(),
                    lock_timeout: Duration::from_secs(30),
                    ..Default::default()
                },
            );

            barrier_clone.wait();

            let res = engine
                .execute_command(&spec)
                .unwrap_or_else(|e| panic!("Thread {} failed: {}", thread_idx, e));

            let out_data = std::fs::read(worker_dir.join("out.txt")).unwrap();
            assert_eq!(String::from_utf8_lossy(&out_data).trim(), "SAME_KEY_RESULT");

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

    assert_eq!(misses, 1, "Exactly 1 worker must execute computation");
    assert_eq!(
        hits,
        num_threads - 1,
        "Remaining workers must receive cache Hit"
    );

    let counter_content = std::fs::read_to_string(&counter_file).unwrap();
    let exec_lines = counter_content.lines().filter(|l| !l.is_empty()).count();
    assert_eq!(exec_lines, 1, "Underlying process executed only once");
}

/// 18.4 Concurrency Test 4: Different-Key Writers
/// Verifies that multiple concurrent writers storing completely different computation keys
/// execute in parallel with zero deadlock or mutual interference.
#[test]
fn test_milestone_18_4_different_key_writers() {
    let env = Arc::new(TestEnv::new().unwrap());
    let num_writers = 12;
    let barrier = Arc::new(Barrier::new(num_writers));
    let mut handles = Vec::new();

    for writer_id in 0..num_writers {
        let env_clone = Arc::clone(&env);
        let barrier_clone = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            let worker_temp = tempfile::tempdir().unwrap();
            let worker_dir = worker_temp.path().to_path_buf();
            let payload = format!("DIFFERENT_KEY_PAYLOAD_{}", writer_id);
            std::fs::write(worker_dir.join("in.txt"), payload.as_bytes()).unwrap();

            #[cfg(windows)]
            let (cmd, args) = (
                "powershell.exe",
                vec![
                    "-Command".to_string(),
                    format!("Copy-Item in.txt -Destination out_{}.txt", writer_id),
                ],
            );
            #[cfg(not(windows))]
            let (cmd, args) = (
                "cp",
                vec!["in.txt".to_string(), format!("out_{}.txt", writer_id)],
            );

            let spec = dcc_runner::CommandSpec::builder(cmd)
                .args(args)
                .current_dir(&worker_dir)
                .input_path("in.txt")
                .output_path(format!("out_{}.txt", writer_id))
                .build()
                .unwrap();

            let engine = RunnerEngine::new(
                &env_clone.storage,
                EngineOptions {
                    working_dir: worker_dir.clone(),
                    lock_timeout: Duration::from_secs(30),
                    ..Default::default()
                },
            );

            barrier_clone.wait();

            let res = engine
                .execute_command(&spec)
                .unwrap_or_else(|e| panic!("Writer {} failed: {}", writer_id, e));

            assert_eq!(res.status, ExecutionStatus::Miss);
            let out_data =
                std::fs::read(worker_dir.join(format!("out_{}.txt", writer_id))).unwrap();
            assert_eq!(out_data, payload.as_bytes());

            res.key
        });
        handles.push(handle);
    }

    let mut keys = Vec::new();
    for handle in handles {
        keys.push(handle.join().unwrap());
    }

    assert_eq!(keys.len(), num_writers);
    for key in &keys {
        let entry = env.storage.get_entry(key).unwrap().expect("Entry exists");
        assert_eq!(&entry.key, key);
        assert!(entry.verify_identity().is_ok());
    }

    assert_eq!(env.storage.count_entries().unwrap(), num_writers);
}

/// 18.4 Concurrency Test 5: Reader + Writer
/// Verifies that concurrent readers continuously reading existing entries and CAS objects
/// do not experience errors, torn reads, or crashes while concurrent writers are creating
/// new entries and CAS objects in the same storage root.
#[test]
fn test_milestone_18_4_reader_plus_writer() {
    let env = Arc::new(TestEnv::new().unwrap());

    // 1. Pre-populate initial object and entry for readers
    let base_payload = vec![0x33; 64 * 1024];
    let (base_digest, base_size) = env.storage.store_object_bytes(&base_payload).unwrap();
    let base_comp = Computation::builder_with("base_op", "base_cmd")
        .build()
        .unwrap();
    let base_key = base_comp.compute_key().unwrap();
    let base_entry = CacheEntry::new(
        base_key.clone(),
        base_comp,
        vec![OutputManifestItem {
            path: "base.bin".into(),
            digest: base_digest.clone(),
            size: base_size,
            is_executable: Some(false),
        }],
        ExecutionMetadata::default(),
    );
    env.storage.store_entry(&base_entry).unwrap();

    let num_readers = 16;
    let num_writers = 8;
    let total_threads = num_readers + num_writers;
    let barrier = Arc::new(Barrier::new(total_threads));
    let mut handles = Vec::new();

    // Spawn readers
    for reader_id in 0..num_readers {
        let env_clone = Arc::clone(&env);
        let barrier_clone = Arc::clone(&barrier);
        let base_digest_clone = base_digest.clone();
        let base_key_clone = base_key.clone();
        let expected_payload = base_payload.clone();

        let handle = thread::spawn(move || {
            barrier_clone.wait();

            for iter in 0..25 {
                let mut reader = env_clone
                    .storage
                    .get_object_reader(&base_digest_clone)
                    .unwrap();
                let mut buf = Vec::new();
                reader.read_to_end(&mut buf).unwrap();
                assert_eq!(
                    buf, expected_payload,
                    "Reader {} iter {} corrupted payload",
                    reader_id, iter
                );

                let entry = env_clone
                    .storage
                    .get_entry(&base_key_clone)
                    .unwrap()
                    .expect("Entry exists");
                assert_eq!(entry.key, base_key_clone);
                thread::sleep(Duration::from_millis(5));
            }
        });
        handles.push(handle);
    }

    // Spawn writers
    for writer_id in 0..num_writers {
        let env_clone = Arc::clone(&env);
        let barrier_clone = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            barrier_clone.wait();

            for iter in 0..10 {
                let payload = format!("NEW_PAYLOAD_WRITER_{}_ITER_{}", writer_id, iter);
                let (digest, size) = env_clone
                    .storage
                    .store_object_bytes(payload.as_bytes())
                    .unwrap();
                assert_eq!(size, payload.len() as u64);

                let comp = Computation::builder_with(format!("op_{}_{}", writer_id, iter), "cmd")
                    .build()
                    .unwrap();
                let key = comp.compute_key().unwrap();
                let entry = CacheEntry::new(
                    key.clone(),
                    comp,
                    vec![OutputManifestItem {
                        path: "out.txt".into(),
                        digest,
                        size,
                        is_executable: Some(false),
                    }],
                    ExecutionMetadata::default(),
                );
                env_clone.storage.store_entry(&entry).unwrap();
                thread::sleep(Duration::from_millis(10));
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Storage state check
    assert!(env.storage.verify_object(&base_digest).is_ok());
    assert!(env.storage.get_entry(&base_key).unwrap().is_some());
    let mut tmp_entries = std::fs::read_dir(env.storage.tmp_dir()).unwrap();
    assert!(tmp_entries.next().is_none(), "Tmp directory must be clean");
}

/// 18.4 Concurrency Test 6: Pruner + Reader
/// Verifies that while an active reader holds an open stream / shared ObjectLock on a CAS object,
/// the garbage collection Pruner safely skips or coordinates so the active reader's object
/// is NEVER deleted mid-stream.
#[test]
fn test_milestone_18_4_pruner_plus_reader() {
    let env = Arc::new(TestEnv::new().unwrap());

    // 1. Store an object and don't create any cache entry for it (simulating unreferenced object)
    let payload = vec![0x88; 100 * 1024]; // 100 KB
    let (digest, size) = env.storage.store_object_bytes(&payload).unwrap();
    assert_eq!(size, payload.len() as u64);

    // 2. Reader thread acquires a shared ObjectLock (simulating active streaming/reading of an unreferenced object)
    let env_clone = Arc::clone(&env);
    let digest_clone = digest.clone();
    let (tx_ready, rx_ready) = std::sync::mpsc::channel();
    let (tx_done, rx_done) = std::sync::mpsc::channel();

    let reader_handle = thread::spawn(move || {
        let _shared_lock = ObjectLock::acquire_shared(
            &env_clone.storage.locks_dir(),
            &digest_clone,
            Duration::from_secs(5),
        )
        .expect("Reader acquires shared lock");

        // Verify object can be read cleanly
        let mut reader = env_clone.storage.get_object_reader(&digest_clone).unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf.len(), 100 * 1024);

        // Notify pruner that reader is holding shared lock
        tx_ready.send(()).unwrap();

        // Wait for pruner to run
        rx_done.recv().unwrap();

        // Read again to verify object was NOT deleted from under reader
        let mut reader2 = env_clone.storage.get_object_reader(&digest_clone).unwrap();
        let mut buf2 = Vec::new();
        reader2.read_to_end(&mut buf2).unwrap();
        assert_eq!(buf2.len(), 100 * 1024);

        drop(_shared_lock);
    });

    // Wait for reader to lock and begin
    rx_ready.recv().unwrap();

    // 3. Pruner runs garbage collection while reader is actively holding shared lock
    let pruner = Pruner::new(&env.storage);
    let prune_res = pruner.prune_unreferenced_objects().unwrap();

    // Pruner must have skipped the actively read object (0 deleted)
    assert_eq!(
        prune_res.deleted_objects, 0,
        "Pruner must skip objects with active shared read locks"
    );
    assert!(
        env.storage.has_object(&digest),
        "Object must remain on disk while read lock is held"
    );

    // Release reader thread
    tx_done.send(()).unwrap();
    reader_handle.join().unwrap();

    // 4. Now that reader is completely finished and dropped shared lock, pruner runs again
    let second_prune = pruner.prune_unreferenced_objects().unwrap();
    assert_eq!(
        second_prune.deleted_objects, 1,
        "Pruner must successfully clean unreferenced object once read lock is released"
    );
    assert!(
        !env.storage.has_object(&digest),
        "Object must be deleted after pruner runs"
    );
}
