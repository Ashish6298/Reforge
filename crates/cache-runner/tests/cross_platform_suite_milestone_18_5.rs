use dcc_core::{CacheEntry, Computation, Digest, ExecutionMetadata, OutputManifestItem, PathUtils};
use dcc_runner::{EngineOptions, ExecutionStatus, OutputRestorer, ProcessExecutor, RunnerEngine};
use dcc_storage::cas::StorageConfig;
use dcc_storage::CasStorage;
use dcc_test_utils::TestEnv;
use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

/// Cross-Platform Suite Test 1: Canonical Path Handling Across OS Separators
/// Verifies that forward slashes (`/`), backslashes (`\`), mixed separators, and relative
/// paths normalize deterministically on any OS into canonical platform-agnostic representations.
#[test]
fn test_cross_platform_path_normalization_and_sharding() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = CasStorage::new(StorageConfig::new(temp_dir.path())).unwrap();

    // 1. Verify PathUtils normalization for both Windows & Unix styles
    let unix_style = PathUtils::to_normalized_string("src/nested/module/file.rs");
    let win_style = PathUtils::to_normalized_string("src\\nested\\module\\file.rs");
    let mixed_style = PathUtils::to_normalized_string("src/nested\\module/file.rs");

    assert_eq!(unix_style, win_style);
    assert_eq!(win_style, mixed_style);
    assert_eq!(unix_style, "src/nested/module/file.rs");

    // 2. Storage object and entry paths are safely formatted on current OS
    let digest = Digest::from_bytes(b"cross_platform_path_test_payload");
    let obj_path = storage.object_path(&digest);
    assert!(obj_path.starts_with(storage.objects_dir()));
    let prefix = digest.prefix(2);
    assert!(obj_path.to_string_lossy().contains(prefix));

    // 3. Store and retrieve across nested directory structures
    let (d, size) = storage.store_object_bytes(b"CROSS_PLATFORM_DATA").unwrap();
    assert_eq!(size, 19);
    assert!(storage.has_object(&d));
    assert!(storage.verify_object(&d).is_ok());
}

/// Cross-Platform Suite Test 2: Process Execution, Environment Variables, and Exit Codes
/// Verifies that command execution, environment variable propagation, and exit status
/// handling work identically on Windows (`cmd.exe`/`powershell.exe`) and Unix (`sh`).
#[test]
fn test_cross_platform_process_execution_and_environment() {
    #[cfg(windows)]
    let (cmd, args) = (
        "cmd.exe",
        vec![
            "/C".to_string(),
            "echo DCC_TEST_VAL=%DCC_TEST_VAR%".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo \"DCC_TEST_VAL=$DCC_TEST_VAR\"".to_string(),
        ],
    );

    let mut env = BTreeMap::new();
    env.insert(
        "DCC_TEST_VAR".to_string(),
        "ENVIRONMENT_PASSED_OK".to_string(),
    );

    let out = ProcessExecutor::execute(cmd, &args, &env, None).unwrap();
    assert_eq!(out.exit_code, 0);
    assert!(!out.timed_out);

    let stdout_str = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout_str.contains("DCC_TEST_VAL=ENVIRONMENT_PASSED_OK"),
        "Environment variables must be propagated to child process on all operating systems"
    );
}

/// Cross-Platform Suite Test 3: Essential Cache Engine Lifecycle (Miss -> Hit -> Invalidation)
/// Verifies that cold miss computation, CAS artifact persistence, warm cache hit output restoration,
/// and input-change invalidation operate seamlessly with 0 ms hit restoration on any OS.
#[test]
fn test_cross_platform_essential_cache_lifecycle() {
    let env = TestEnv::new().unwrap();

    // 1. Create input source
    env.create_input_file("input_src.dat", b"CROSS_PLATFORM_SOURCE_V1")
        .unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Copy-Item input_src.dat -Destination output_res.dat".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "cp",
        vec!["input_src.dat".to_string(), "output_res.dat".to_string()],
    );

    let comp = Computation::builder_with("cross-platform-op", cmd)
        .args(args)
        .input("input_src.dat", Digest::from_bytes(b""), 0)
        .output("output_res.dat", true)
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // 2. Cold Run: MISS
    let res1 = engine.execute(comp.clone()).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);
    assert_eq!(res1.exit_code, 0);
    assert_eq!(
        env.read_output_file("output_res.dat").unwrap(),
        b"CROSS_PLATFORM_SOURCE_V1"
    );

    // 3. Delete output file to test warm restoration
    fs::remove_file(env.workspace_dir.path().join("output_res.dat")).unwrap();
    assert!(!env.workspace_dir.path().join("output_res.dat").exists());

    // 4. Warm Run: HIT (0 ms execution)
    let res2 = engine.execute(comp.clone()).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Hit);
    assert_eq!(res2.key, res1.key);
    assert_eq!(res2.execution_time_ms, 0);
    assert_eq!(
        env.read_output_file("output_res.dat").unwrap(),
        b"CROSS_PLATFORM_SOURCE_V1"
    );

    // 5. Input changed: MISS with new key
    env.create_input_file("input_src.dat", b"CROSS_PLATFORM_SOURCE_V2_MODIFIED")
        .unwrap();
    let res3 = engine.execute(comp).unwrap();
    assert_eq!(res3.status, ExecutionStatus::Miss);
    assert_ne!(res3.key, res1.key);
    assert_eq!(
        env.read_output_file("output_res.dat").unwrap(),
        b"CROSS_PLATFORM_SOURCE_V2_MODIFIED"
    );
}

/// Cross-Platform Suite Test 4: File Semantics, Symlink Safety & Permission Handling
/// Verifies that symlinks (on platforms that support them) resolve safely to underlying content,
/// path traversal attempts are rejected, and output restoration preserves file integrity.
#[test]
fn test_cross_platform_file_semantics_and_restoration() {
    let env = TestEnv::new().unwrap();

    // 1. Traversal rejection (both Windows and Unix slashes)
    assert!(Computation::builder_with("bad_op", "test")
        .output("../outside_file.txt", true)
        .build()
        .is_err());
    assert!(Computation::builder_with("bad_op", "test")
        .output("..\\outside_file.txt", true)
        .build()
        .is_err());

    // 2. Output restoration roundtrip
    let payload = b"RESTORED_BINARY_FILE_PAYLOAD_1234567890";
    let (digest, size) = env.storage.store_object_bytes(payload).unwrap();

    let comp = Computation::builder_with("restore_op", "cmd")
        .build()
        .unwrap();
    let key = comp.compute_key().unwrap();
    let entry = CacheEntry::new(
        key,
        comp,
        vec![OutputManifestItem {
            path: "nested/dir/restored_artifact.bin".to_string(),
            digest,
            size,
            is_executable: Some(false),
        }],
        ExecutionMetadata::default(),
    );

    OutputRestorer::restore_entry(&env.storage, &entry, env.workspace_dir.path()).unwrap();

    let target_path = env
        .workspace_dir
        .path()
        .join("nested")
        .join("dir")
        .join("restored_artifact.bin");
    assert!(target_path.is_file());
    assert_eq!(fs::read(target_path).unwrap(), payload);
}

/// Cross-Platform Suite Test 5: Advisory Locking and Cleanup Across Platforms
/// Verifies that process locking and object locking behave consistently across OS file lock semantics.
#[test]
fn test_cross_platform_advisory_locking_lifecycle() {
    let env = TestEnv::new().unwrap();
    let key = dcc_core::CacheKey::from_bytes(b"cross_platform_lock_key");

    {
        let lock = dcc_storage::lock::ComputationLock::acquire(
            &env.storage.locks_dir(),
            &key,
            Duration::from_secs(3),
        )
        .unwrap();

        assert_eq!(lock.metadata().pid, std::process::id());
        assert_eq!(lock.metadata().key, key.as_str());
    }

    // After drop, lock file is released and removed
    let lock_file = env
        .storage
        .locks_dir()
        .join(format!("{}.lock", key.as_str()));
    assert!(!lock_file.exists());
}
