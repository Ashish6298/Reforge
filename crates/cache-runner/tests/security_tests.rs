use dcc_core::{Computation, Digest};
use dcc_runner::{EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_test_utils::TestEnv;
use std::fs::File;
use std::io::Write;

#[test]
fn test_corrupted_cas_object_causes_safe_fallback() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("source.txt", b"important source code")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Copy-Item source.txt -Destination build_out.txt".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "cp",
        vec!["source.txt".to_string(), "build_out.txt".to_string()],
    );

    let computation = Computation::builder_with("build-integrity", cmd)
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
    assert_eq!(
        env.read_output_file("build_out.txt").unwrap(),
        b"important source code"
    );
}

#[test]
fn test_milestone_14_1_malicious_cas_object_quarantined_and_rejected() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cache_dir = temp_dir.path().join(".dcc_cache");
    let ws_dir = temp_dir.path().join("ws");
    std::fs::create_dir_all(&ws_dir).unwrap();

    let cache = dcc_storage::Cache::open(&cache_dir).unwrap();

    // 1. Store a legitimate entry and payload
    let legitimate_data = b"legitimate_compiled_binary_payload";
    let (legit_digest, legit_size) = cache.storage().store_object_bytes(legitimate_data).unwrap();

    let comp = Computation::builder_with("compile", "rustc")
        .input("main.rs", legit_digest.clone(), legit_size)
        .output("main.exe", true)
        .build()
        .unwrap();
    let key = comp.compute_key().unwrap();

    let entry = dcc_core::CacheEntry::new(
        key.clone(),
        comp,
        vec![dcc_core::OutputManifestItem {
            path: "main.exe".to_string(),
            digest: legit_digest.clone(),
            size: legit_size,
            is_executable: Some(true),
        }],
        dcc_core::ExecutionMetadata::default(),
    );
    cache.store(&entry).unwrap();

    // 2. Attack: Poison the CAS object on disk with malicious payload
    let cas_obj_path = cache.storage().object_path(&legit_digest);
    assert!(cas_obj_path.exists());
    let malicious_data = b"MALICIOUS_TROJAN_PAYLOAD_EXECUTABLE";
    std::fs::write(&cas_obj_path, malicious_data).unwrap();

    // 3. Verify that restoration detects poisoning and rejects extraction
    let restore_dest = ws_dir.join("dest");
    std::fs::create_dir_all(&restore_dest).unwrap();
    let restore_result = cache.restore(&entry, &restore_dest);

    assert!(
        restore_result.is_err(),
        "Restoration must fail when CAS object checksum is compromised"
    );

    // Destination target must NOT have been created or modified
    assert!(
        !restore_dest.join("main.exe").exists(),
        "Malicious payload must never be materialized in destination workspace"
    );

    // 4. Verify CAS object was quarantined to .corrupted
    let corrupted_path = cas_obj_path.with_extension("corrupted");
    assert!(
        corrupted_path.exists(),
        "Poisoned CAS object must be isolated to .corrupted"
    );
}

#[test]
fn test_milestone_14_1_malicious_metadata_key_spoofing_prevented() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cache_dir = temp_dir.path().join(".dcc_cache");
    let ws_dir = temp_dir.path().join("ws");
    std::fs::create_dir_all(&ws_dir).unwrap();

    let cache = dcc_storage::Cache::open(&cache_dir).unwrap();

    // 1. Create a legitimate computation
    let legit_digest = Digest::from_bytes(b"safe_code");
    let safe_comp = Computation::builder_with("safe_build", "compiler")
        .input("src.rs", legit_digest.clone(), 9)
        .output("out.bin", true)
        .build()
        .unwrap();
    let legit_key = safe_comp.compute_key().unwrap();

    let (blob_digest, blob_size) = cache
        .storage()
        .store_object_bytes(b"safe_output_blob")
        .unwrap();

    // 2. Attack: Forged metadata entry where key is legitimate safe_key,
    // but embedded computation is malicious ("rm -rf /" or "malicious_tool")
    let evil_comp = Computation::builder_with("evil_op", "malicious_tool")
        .arg("--exploit")
        .input("src.rs", legit_digest, 9)
        .output("out.bin", true)
        .build()
        .unwrap();

    let forged_entry = dcc_core::CacheEntry::new(
        legit_key.clone(), // Key spoofing
        evil_comp,
        vec![dcc_core::OutputManifestItem {
            path: "out.bin".to_string(),
            digest: blob_digest,
            size: blob_size,
            is_executable: None,
        }],
        dcc_core::ExecutionMetadata::default(),
    );

    // 3. Verify entry identity fails validation
    assert!(
        forged_entry.verify_identity().is_err(),
        "Forged metadata must fail verify_identity"
    );

    // 4. Verify restoration pre-check rejects forged entry before extracting files
    let dest_dir = ws_dir.join("dest");
    std::fs::create_dir_all(&dest_dir).unwrap();
    let restore_res = cache.restore(&forged_entry, &dest_dir);
    assert!(
        restore_res.is_err(),
        "Restore must reject forged metadata with mismatched key identity"
    );
    assert!(
        !dest_dir.join("out.bin").exists(),
        "No output must be written for forged metadata"
    );
}

#[test]
fn test_milestone_14_1_tampered_output_staging_rollback() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cache_dir = temp_dir.path().join(".dcc_cache");
    let ws_dir = temp_dir.path().join("ws");
    std::fs::create_dir_all(&ws_dir).unwrap();

    let cache = dcc_storage::Cache::open(&cache_dir).unwrap();

    // 1. Store valid CAS object
    let expected_data = b"trusted_application_binary";
    let (correct_digest, size) = cache.storage().store_object_bytes(expected_data).unwrap();

    // 2. Craft entry declaring a different digest than the actual blob
    let comp = Computation::builder_with("pack", "tool")
        .input("in.txt", correct_digest.clone(), size)
        .output("app.bin", true)
        .build()
        .unwrap();
    let key = comp.compute_key().unwrap();

    let fake_digest = Digest::from_bytes(b"mismatched_forged_digest");
    let entry_with_mismatch = dcc_core::CacheEntry::new(
        key,
        comp,
        vec![dcc_core::OutputManifestItem {
            path: "app.bin".to_string(),
            digest: fake_digest,
            size,
            is_executable: None,
        }],
        dcc_core::ExecutionMetadata::default(),
    );

    // 3. Attempt restoration
    let dest_dir = ws_dir.join("restore_dest");
    std::fs::create_dir_all(&dest_dir).unwrap();
    let res = cache.restore(&entry_with_mismatch, &dest_dir);

    assert!(
        res.is_err(),
        "Restoration must fail when output digest does not match storage"
    );
    assert!(
        !dest_dir.join("app.bin").exists(),
        "Destination file must never be materialized on integrity error"
    );
}
