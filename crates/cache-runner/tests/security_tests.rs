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

#[test]
fn test_milestone_14_2_path_traversal_relative_parent_escape_rejection() {
    let temp_dir = tempfile::tempdir().unwrap();
    let workspace = temp_dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    // Critical test targets from specification: "../../important-file", "..\..\important-file"
    let malicious_escape_paths = vec![
        "../../important-file",
        r"..\..\important-file",
        "../important-file",
        r"..\important-file",
        "sub/../../important-file",
        "sub/dir/../../../important-file",
        "a/b/c/../../../../escaped.txt",
        "..",
        "../",
        r"..\.",
    ];

    for bad_path in malicious_escape_paths {
        let res = dcc_core::PathUtils::sanitize_relative_path(&workspace, bad_path);
        assert!(
            res.is_err(),
            "PathUtils::sanitize_relative_path must reject traversal: '{}'",
            bad_path
        );
    }
}

#[test]
fn test_milestone_14_2_path_traversal_absolute_paths_rejection() {
    let temp_dir = tempfile::tempdir().unwrap();
    let workspace = temp_dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let malicious_absolute_paths = vec![
        "/etc/passwd",
        "/var/run/secrets.key",
        r"C:\Windows\System32\cmd.exe",
        r"C:\important-file",
        "D:/secrets.txt",
        r"\\server\share\file.txt",
        r"\\?\C:\secret.txt",
    ];

    for abs_path in malicious_absolute_paths {
        let res = dcc_core::PathUtils::sanitize_relative_path(&workspace, abs_path);
        assert!(
            res.is_err(),
            "PathUtils::sanitize_relative_path must reject absolute or prefixed path: '{}'",
            abs_path
        );
    }
}

#[test]
fn test_milestone_14_2_path_traversal_runner_and_cache_restore_rejection() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cache_dir = temp_dir.path().join(".dcc_cache");
    let workspace_dir = temp_dir.path().join("workspace");
    let sensitive_victim_dir = temp_dir.path().join("sensitive");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    std::fs::create_dir_all(&sensitive_victim_dir).unwrap();

    let victim_file = sensitive_victim_dir.join("important-file.txt");
    std::fs::write(&victim_file, b"CRITICAL_SENSITIVE_SYSTEM_DATA").unwrap();

    let cache = dcc_storage::Cache::open(&cache_dir).unwrap();

    // 1. Store a CAS object payload
    let (blob_digest, blob_size) = cache
        .storage()
        .store_object_bytes(b"MALICIOUS_OVERWRITE_PAYLOAD")
        .unwrap();

    // Builder rejects traversal path at build time
    let builder_res = Computation::builder_with("exploit", "tool")
        .input("src.txt", blob_digest.clone(), blob_size)
        .output("../sensitive/important-file.txt", true)
        .build();
    assert!(
        builder_res.is_err(),
        "ComputationBuilder must reject escaping path declarations"
    );

    // If constructed manually, Cache::restore and OutputRestorer::restore_entry must reject
    let malicious_entry = dcc_core::CacheEntry::new(
        dcc_core::CacheKey::from_bytes(b"malicious_key"),
        Computation {
            schema_version: 1,
            operation: "exploit".to_string(),
            command: "tool".to_string(),
            args: vec![],
            inputs: vec![],
            outputs: vec![dcc_core::OutputFile {
                path: "../../sensitive/important-file.txt".to_string(),
                required: true,
            }],
            env: std::collections::BTreeMap::new(),
            platform: dcc_core::PlatformConstraints::default(),
            tool: None,
            policy: dcc_core::CachePolicy::ReadWrite,
            working_dir: None,
            metadata: std::collections::BTreeMap::new(),
        },
        vec![dcc_core::OutputManifestItem {
            path: "../../sensitive/important-file.txt".to_string(),
            digest: blob_digest,
            size: blob_size,
            is_executable: None,
        }],
        dcc_core::ExecutionMetadata::default(),
    );

    // 3. Attempt Cache::restore into workspace_dir
    let restore_res = cache.restore(&malicious_entry, &workspace_dir);
    assert!(
        restore_res.is_err(),
        "Cache::restore must reject output path escaping workspace"
    );

    // 4. Attempt OutputRestorer::restore_entry into workspace_dir
    let restorer_res = dcc_runner::OutputRestorer::restore_entry(
        cache.storage(),
        &malicious_entry,
        &workspace_dir,
    );
    assert!(
        restorer_res.is_err(),
        "OutputRestorer must reject output path escaping workspace"
    );

    // 5. Verify victim file was NEVER overwritten or modified
    assert_eq!(
        std::fs::read(&victim_file).unwrap(),
        b"CRITICAL_SENSITIVE_SYSTEM_DATA",
        "Sensitive file outside workspace must remain strictly intact and unpoisoned"
    );
}

#[test]
fn test_milestone_14_3_symlink_overwrite_attack_prevention() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cache_dir = temp_dir.path().join(".dcc_cache");
    let workspace_dir = temp_dir.path().join("workspace");
    let victim_dir = temp_dir.path().join("victim_dir");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    std::fs::create_dir_all(&victim_dir).unwrap();

    let victim_file = victim_dir.join("critical_target.key");
    std::fs::write(&victim_file, b"ORIGINAL_CRITICAL_SECRET").unwrap();

    let cache = dcc_storage::Cache::open(&cache_dir).unwrap();

    // 1. Store a cache payload
    let (blob_digest, blob_size) = cache
        .storage()
        .store_object_bytes(b"MALICIOUS_OVERWRITE_PAYLOAD")
        .unwrap();

    let comp = Computation::builder_with("build", "tool")
        .input("src.txt", blob_digest.clone(), blob_size)
        .output("output.txt", true)
        .build()
        .unwrap();
    let key = comp.compute_key().unwrap();

    let entry = dcc_core::CacheEntry::new(
        key,
        comp,
        vec![dcc_core::OutputManifestItem {
            path: "output.txt".to_string(),
            digest: blob_digest,
            size: blob_size,
            is_executable: None,
        }],
        dcc_core::ExecutionMetadata::default(),
    );

    // 2. Plant a symlink inside the workspace pointing to victim_file
    let symlink_path = workspace_dir.join("output.txt");

    let mut symlink_created = false;
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        if symlink(&victim_file, &symlink_path).is_ok() {
            symlink_created = true;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_file;
        if symlink_file(&victim_file, &symlink_path).is_ok() {
            symlink_created = true;
        }
    }

    if symlink_created {
        // 3. Restore entry into workspace
        let res = cache.restore(&entry, &workspace_dir);
        assert!(res.is_ok(), "Restore should succeed cleanly");

        // 4. Verify victim file was NOT overwritten through the symlink
        let victim_content = std::fs::read(&victim_file).unwrap();
        assert_eq!(
            victim_content, b"ORIGINAL_CRITICAL_SECRET",
            "Victim file must NOT be overwritten through a malicious symlink"
        );

        // 5. Verify the restored file in workspace is a normal regular file containing payload
        let restored_content = std::fs::read(&symlink_path).unwrap();
        assert_eq!(
            restored_content, b"MALICIOUS_OVERWRITE_PAYLOAD",
            "Workspace file must contain the restored payload"
        );
        let meta = std::fs::symlink_metadata(&symlink_path).unwrap();
        assert!(
            !meta.file_type().is_symlink(),
            "Workspace target must have replaced the malicious symlink, not followed it"
        );
    }
}

#[test]
fn test_milestone_14_3_symlink_directory_escape_prevention() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cache_dir = temp_dir.path().join(".dcc_cache");
    let workspace_dir = temp_dir.path().join("workspace");
    let external_dir = temp_dir.path().join("external_system");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    std::fs::create_dir_all(&external_dir).unwrap();

    let cache = dcc_storage::Cache::open(&cache_dir).unwrap();

    // Store a payload
    let (blob_digest, blob_size) = cache.storage().store_object_bytes(b"PAYLOAD_DATA").unwrap();

    // Create an entry declaring output inside "symlink_folder/target.txt"
    let comp = Computation::builder_with("build", "tool")
        .input("src.txt", blob_digest.clone(), blob_size)
        .output("symlink_folder/target.txt", true)
        .build()
        .unwrap();
    let key = comp.compute_key().unwrap();

    let entry = dcc_core::CacheEntry::new(
        key,
        comp,
        vec![dcc_core::OutputManifestItem {
            path: "symlink_folder/target.txt".to_string(),
            digest: blob_digest,
            size: blob_size,
            is_executable: None,
        }],
        dcc_core::ExecutionMetadata::default(),
    );

    // Plant a symlink directory in workspace pointing outside workspace
    let symlink_dir_path = workspace_dir.join("symlink_folder");
    let mut symlink_dir_created = false;
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        if symlink(&external_dir, &symlink_dir_path).is_ok() {
            symlink_dir_created = true;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_dir;
        if symlink_dir(&external_dir, &symlink_dir_path).is_ok() {
            symlink_dir_created = true;
        }
    }

    if symlink_dir_created {
        // Attempt restoration
        let res = cache.restore(&entry, &workspace_dir);
        assert!(
            res.is_err(),
            "Restoring through a symlinked directory pointing outside workspace must be rejected"
        );

        // Verify external dir was untouched
        assert!(
            !external_dir.join("target.txt").exists(),
            "External directory must never receive restored files through symlink folder"
        );
    }
}
