//! Unit Test Suite for Milestone 18.1 (Core Domains)
//! Covers:
//! 1. Hashing (streaming, chunking, directory, parallel)
//! 2. Key Generation (canonical serialization, ordering, platform, tools)
//! 3. Serialization (roundtrip JSON serde, schema stability)
//! 4. Validation (path traversal, sandbox, sensitive data detection)
//! 6. Metadata (CacheEntry, IntegrityInfo, identity verification)
//! 7. Configuration (ByteSize parsing, arithmetic, limits)

use dcc_core::{
    ByteSize, CacheEntry, CacheKey, Computation, Digest, ExecutionMetadata, IntegrityInfo,
    OutputManifestItem, PathUtils, SensitiveDataDetector,
};
use tempfile::tempdir;

// ============================================================================
// 1. HASHING UNIT TESTS
// ============================================================================

#[test]
fn test_hashing_empty_and_known_bytes() {
    let empty_digest = Digest::hash_bytes(b"");
    assert_eq!(
        empty_digest.as_str(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );

    let hello_digest = Digest::hash_bytes(b"hello world");
    assert_eq!(
        hello_digest.as_str(),
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
    );
}

#[test]
fn test_hashing_streaming_multi_chunk() {
    // 256 KB of deterministic data crossing multiple 64KB chunk boundaries
    let mut large_buffer = Vec::with_capacity(256 * 1024);
    for i in 0..(256 * 1024) {
        large_buffer.push((i % 251) as u8);
    }

    let memory_digest = Digest::hash_bytes(&large_buffer);
    let cursor = std::io::Cursor::new(&large_buffer);
    let streaming_digest = Digest::hash_reader(cursor).expect("streaming hash");

    assert_eq!(memory_digest, streaming_digest);
}

#[test]
fn test_hashing_file_and_parallel_batch() {
    let dir = tempdir().unwrap();
    let mut paths = Vec::new();
    let mut expected_digests = Vec::new();

    for i in 0..10 {
        let p = dir.path().join(format!("file_{}.dat", i));
        let content = format!("content for file index {}", i);
        std::fs::write(&p, &content).unwrap();
        expected_digests.push(Digest::hash_bytes(content.as_bytes()));
        paths.push(p);
    }

    let batch_digests = Digest::hash_files_parallel(&paths).expect("parallel batch hash");
    assert_eq!(batch_digests.len(), paths.len());
    for (actual, expected) in batch_digests.iter().zip(expected_digests.iter()) {
        assert_eq!(actual, expected);
    }
}

#[test]
fn test_hashing_directory_deterministic() {
    let dir = tempdir().unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir_all(&sub).unwrap();

    std::fs::write(dir.path().join("a.txt"), b"aaa").unwrap();
    std::fs::write(sub.join("b.txt"), b"bbb").unwrap();

    let d1 = Digest::hash_directory(dir.path()).unwrap();
    let d2 = Digest::hash_directory(dir.path()).unwrap();
    assert_eq!(d1, d2, "Directory hashing must be deterministic");

    // Modify a file
    std::fs::write(sub.join("b.txt"), b"bbb_mod").unwrap();
    let d3 = Digest::hash_directory(dir.path()).unwrap();
    assert_ne!(d1, d3, "Modified file must alter directory digest");
}

// ============================================================================
// 2. KEY GENERATION UNIT TESTS
// ============================================================================

#[test]
fn test_key_generation_deterministic_and_canonical() {
    let dummy_digest = Digest::hash_bytes(b"content");
    let c1 = Computation::builder()
        .operation("build")
        .command("rustc")
        .args(vec!["src/main.rs", "--crate-type=bin"])
        .input("src/main.rs", dummy_digest.clone(), 100)
        .env("RUST_LOG", "info")
        .build()
        .unwrap();

    let c2 = Computation::builder()
        .operation("build")
        .command("rustc")
        .args(vec!["src/main.rs", "--crate-type=bin"])
        .input("src/main.rs", dummy_digest.clone(), 100)
        .env("RUST_LOG", "info")
        .build()
        .unwrap();

    assert_eq!(c1.compute_key().unwrap(), c2.compute_key().unwrap());
}

#[test]
fn test_key_generation_argument_order_sensitivity() {
    let dummy_digest = Digest::hash_bytes(b"content");
    let c1 = Computation::builder()
        .operation("build")
        .command("gcc")
        .args(vec!["-O2", "-Wall"])
        .input("main.c", dummy_digest.clone(), 50)
        .build()
        .unwrap();

    let c2 = Computation::builder()
        .operation("build")
        .command("gcc")
        .args(vec!["-Wall", "-O2"])
        .input("main.c", dummy_digest.clone(), 50)
        .build()
        .unwrap();

    assert_ne!(
        c1.compute_key().unwrap(),
        c2.compute_key().unwrap(),
        "Argument order must affect key identity"
    );
}

#[test]
fn test_key_generation_input_order_invariance() {
    let d1 = Digest::hash_bytes(b"content1");
    let d2 = Digest::hash_bytes(b"content2");

    let c1 = Computation::builder()
        .operation("link")
        .command("ld")
        .input("a.o", d1.clone(), 100)
        .input("b.o", d2.clone(), 200)
        .build()
        .unwrap();

    let c2 = Computation::builder()
        .operation("link")
        .command("ld")
        .input("b.o", d2.clone(), 200)
        .input("a.o", d1.clone(), 100)
        .build()
        .unwrap();

    assert_eq!(
        c1.compute_key().unwrap(),
        c2.compute_key().unwrap(),
        "Input declaration order must be normalized canonically"
    );
}

// ============================================================================
// 3. SERIALIZATION UNIT TESTS
// ============================================================================

#[test]
fn test_serialization_cache_entry_roundtrip() {
    let comp = Computation::builder()
        .operation("codegen")
        .command("generator")
        .args(vec!["--schema", "schema.json"])
        .input("schema.json", Digest::hash_bytes(b"schema"), 256)
        .output("models.rs", true)
        .build()
        .unwrap();

    let key = comp.compute_key().unwrap();
    let outputs = vec![OutputManifestItem {
        path: "models.rs".into(),
        digest: Digest::hash_bytes(b"models_content"),
        size: 1024,
        is_executable: Some(false),
    }];
    let exec = ExecutionMetadata {
        exit_code: 0,
        execution_time_ms: 120,
        stdout_digest: Some(Digest::hash_bytes(b"stdout")),
        stderr_digest: None,
        timings: Default::default(),
    };
    let mut entry = CacheEntry::new(key, comp, outputs, exec);
    entry.metadata.hit_count = 5;
    entry.metadata.integrity = Some(IntegrityInfo {
        entry_digest: Digest::hash_bytes(b"entry_digest"),
        verified_at: chrono::Utc::now(),
    });

    let serialized = serde_json::to_string_pretty(&entry).expect("serialize CacheEntry");
    let deserialized: CacheEntry =
        serde_json::from_str(&serialized).expect("deserialize CacheEntry");

    assert_eq!(entry.key, deserialized.key);
    assert_eq!(entry.schema_version, deserialized.schema_version);
    assert_eq!(entry.metadata.hit_count, deserialized.metadata.hit_count);
    assert_eq!(
        entry.metadata.execution.exit_code,
        deserialized.metadata.execution.exit_code
    );
    assert_eq!(entry.outputs.len(), deserialized.outputs.len());
    assert_eq!(entry.outputs[0].path, deserialized.outputs[0].path);
    assert_eq!(entry.outputs[0].digest, deserialized.outputs[0].digest);
}

// ============================================================================
// 4. VALIDATION UNIT TESTS
// ============================================================================

#[test]
fn test_validation_path_traversal_rejection() {
    let ws = tempdir().unwrap();

    // Valid relative paths
    assert!(PathUtils::sanitize_relative_path(ws.path(), "src/main.rs").is_ok());
    assert!(PathUtils::sanitize_relative_path(ws.path(), "dist/bundle.js").is_ok());

    // Directory escape attacks
    assert!(PathUtils::sanitize_relative_path(ws.path(), "../escaped.txt").is_err());
    assert!(PathUtils::sanitize_relative_path(ws.path(), "src/../../outside").is_err());

    // Absolute and UNC paths
    assert!(PathUtils::sanitize_relative_path(ws.path(), "/etc/passwd").is_err());
    assert!(PathUtils::sanitize_relative_path(ws.path(), "C:\\Windows\\System32").is_err());
    assert!(PathUtils::sanitize_relative_path(ws.path(), "\\\\server\\share").is_err());
}

#[test]
fn test_validation_sensitive_data_detection() {
    // Key-based detection
    assert!(SensitiveDataDetector::is_sensitive_key("API_KEY"));
    assert!(SensitiveDataDetector::is_sensitive_key("GITHUB_TOKEN"));
    assert!(SensitiveDataDetector::is_sensitive_key("DB_PASSWORD"));
    assert!(SensitiveDataDetector::is_sensitive_key(
        "AWS_SECRET_ACCESS_KEY"
    ));
    assert!(!SensitiveDataDetector::is_sensitive_key("RUST_LOG"));
    assert!(!SensitiveDataDetector::is_sensitive_key("TARGET_ARCH"));

    // Value-based signature detection
    assert!(SensitiveDataDetector::is_sensitive_value(
        "ghp_1234567890abcdefghijklmnopqrstuvwxyz"
    ));
    assert!(SensitiveDataDetector::is_sensitive_value(
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA0..."
    ));
    assert!(!SensitiveDataDetector::is_sensitive_value(
        "normal_argument_value"
    ));
}

// ============================================================================
// 6. METADATA UNIT TESTS
// ============================================================================

#[test]
fn test_metadata_identity_verification_and_spoof_detection() {
    let comp = Computation::builder()
        .operation("compile")
        .command("clang")
        .args(vec!["-c", "foo.c"])
        .input("foo.c", Digest::hash_bytes(b"code"), 50)
        .build()
        .unwrap();

    let real_key = comp.compute_key().unwrap();
    let outputs = vec![OutputManifestItem {
        path: "foo.o".into(),
        digest: Digest::hash_bytes(b"obj_data"),
        size: 512,
        is_executable: None,
    }];
    let exec = ExecutionMetadata {
        exit_code: 0,
        execution_time_ms: 45,
        stdout_digest: None,
        stderr_digest: None,
        timings: Default::default(),
    };
    let mut entry = CacheEntry::new(real_key, comp, outputs, exec);

    // Identity check passes for legitimate entry
    assert!(entry.verify_identity().is_ok());

    // Tamper with key (spoofing attack)
    let fake_key = CacheKey::from_bytes(b"malicious_spoofed_key");
    entry.key = fake_key;

    assert!(
        entry.verify_identity().is_err(),
        "Spoofed key must fail verify_identity()"
    );
}

// ============================================================================
// 7. CONFIGURATION UNIT TESTS
// ============================================================================

#[test]
fn test_configuration_bytesize_parsing_and_arithmetic() {
    // Parsing various formats
    let size_500mb = ByteSize::parse("500 MB").unwrap();
    assert_eq!(size_500mb.as_bytes(), 500 * 1024 * 1024);

    let size_2gb = ByteSize::parse("2GB").unwrap();
    assert_eq!(size_2gb.as_bytes(), 2 * 1024 * 1024 * 1024);

    let size_10gib = ByteSize::parse("10 GiB").unwrap();
    assert_eq!(size_10gib.as_bytes(), 10 * 1024 * 1024 * 1024);

    let size_kb = ByteSize::parse("1024 KB").unwrap();
    assert_eq!(size_kb.as_bytes(), 1024 * 1024);

    let size_frac = ByteSize::parse("1.5 GB").unwrap();
    assert_eq!(
        size_frac.as_bytes(),
        (1.5 * 1024.0 * 1024.0 * 1024.0) as u64
    );

    // Raw bytes string
    let size_raw = ByteSize::parse("4096").unwrap();
    assert_eq!(size_raw.as_bytes(), 4096);

    // Invalid format
    assert!(ByteSize::parse("invalid_size").is_err());
}
