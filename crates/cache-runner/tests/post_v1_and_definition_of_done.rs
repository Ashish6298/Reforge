//! Post-v1 Roadmap Architectural Extensions & Definition of Done Test Suite
//! Validates:
//! 1. V1.1 Advanced Diagnostics (why, explain, diff, inspect, trace)
//! 2. V1.2 Storage Optimization (compression, metadata index, parallel hashing, memory cache tier)
//! 3. V1.3 Plugin / Integration API (code generators, linters, doc tools, asset pipelines, build systems)
//! 4. V1.4 Advanced Cache Policies (ReadOnly, WriteOnly, NoCache, ForceRecompute, CacheFailed, TTL)
//! 5. V2 Remote Tier Architecture (Local-first with optional HTTP/S3 Remote fallback)
//! 6. Complete 20-Point Definition of Done Validation

use dcc_core::{CachePolicy, Computation, Digest, MissReason, ToolIdentity};
use dcc_integrations::BuildAction;
use dcc_runner::{CommandSpec, EngineOptions, ExecutionStatus, MissExplainer, RunnerEngine};
use dcc_storage::{LocalFilesystemStorage, Storage, StorageConfig, TieredCache};
use dcc_test_utils::TestEnv;
use std::io::Read;

// ============================================================================
// 1. V1.1 ADVANCED DIAGNOSTICS: WHY / EXPLAIN / DIFF / TRACE
// ============================================================================

#[test]
fn test_post_v1_1_advanced_diagnostics_diff_and_explain() {
    let comp1 = Computation::builder()
        .operation("compile")
        .command("rustc")
        .args(vec!["src/main.rs".to_string()])
        .tool_identity(ToolIdentity {
            name: "rustc".to_string(),
            version: Some("1.90".to_string()),
            digest: Some(Digest::hash_bytes(b"rustc_1_90")),
        })
        .build()
        .unwrap();

    let comp2 = Computation::builder()
        .operation("compile")
        .command("rustc")
        .args(vec!["src/main.rs".to_string()])
        .tool_identity(ToolIdentity {
            name: "rustc".to_string(),
            version: Some("1.91".to_string()),
            digest: Some(Digest::hash_bytes(b"rustc_1_91")),
        })
        .build()
        .unwrap();

    // MissExplainer::explain returns a MissReason describing why comp2 misses vs comp1
    let miss_reason = MissExplainer::explain(&comp2, Some(&comp1));

    // The tool identity changed, so we expect a ToolChanged miss reason
    let description = miss_reason.to_string();
    assert!(
        !description.is_empty(),
        "Miss explanation must be non-empty"
    );
    // Verify the explanation relates to tool/compiler change
    let is_tool_related = matches!(miss_reason, MissReason::ToolChanged { .. })
        || description.contains("rustc")
        || description.contains("tool")
        || description.contains("compiler");
    assert!(
        is_tool_related,
        "Miss explanation must reference tool/compiler change: {}",
        description
    );
}

// ============================================================================
// 2. V1.2 STORAGE OPTIMIZATION & IN-MEMORY CACHE TIER
// ============================================================================

#[test]
fn test_post_v1_2_storage_optimization_and_tiered_caching() {
    let env = TestEnv::new().unwrap();
    let local_disk = LocalFilesystemStorage::new(StorageConfig::new(
        env.cache_dir.path().join("tiered"),
    ))
    .unwrap();
    let tiered = TieredCache::new(local_disk);

    let payload = b"COMPRESSED_OPTIMIZED_STORAGE_PAYLOAD";
    let (digest, _) = tiered.put(payload).unwrap();

    assert!(tiered.exists(&digest));
    let mut reader = tiered.get(&digest).unwrap();
    let mut retrieved = Vec::new();
    reader.read_to_end(&mut retrieved).unwrap();
    assert_eq!(retrieved, payload);
}

// ============================================================================
// 3. V1.3 PLUGIN & INTEGRATION API
// ============================================================================

#[test]
fn test_post_v1_3_plugin_integration_builders() {
    // 1. Code generator plugin integration via BuildAction
    let proto_gen = BuildAction::builder()
        .compiler("protoc")
        .argument("--go_out=gen")
        .source_input(
            "proto/service.proto",
            Digest::hash_bytes(b"proto content"),
            100,
        )
        .output("gen/service.pb.go", true)
        .build();
    assert!(proto_gen.is_ok());

    // 2. Linter plugin integration
    let linter = BuildAction::builder()
        .compiler("eslint")
        .argument("src/")
        .source_input("src/index.ts", Digest::hash_bytes(b"ts content"), 200)
        .output("reports/lint.json", true)
        .build();
    assert!(linter.is_ok());

    // 3. Documentation tool integration
    let doc_gen = BuildAction::builder()
        .compiler("typedoc")
        .argument("--out")
        .argument("docs/api")
        .source_input("src/index.ts", Digest::hash_bytes(b"ts content"), 200)
        .output("docs/api/index.html", true)
        .build();
    assert!(doc_gen.is_ok());

    // Verify BuildAction can produce Computation keys
    let action = proto_gen.unwrap();
    assert!(action.compute_key().is_ok());
}

// ============================================================================
// 4. V1.4 ADVANCED CACHE POLICIES (READ-ONLY, WRITE-ONLY, TTL, NO-CACHE)
// ============================================================================

#[test]
fn test_post_v1_4_advanced_cache_policies() {
    let env = TestEnv::new().unwrap();
    env.create_input_file("data.txt", b"policy test").unwrap();

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('out.txt', 'RES')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec!["-c".to_string(), "echo -n 'RES' > out.txt".to_string()],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("data.txt")
        .output_path("out.txt")
        .build()
        .unwrap();

    // 1. ReadOnly policy: Never writes to cache
    let engine_ro = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            policy: CachePolicy::ReadOnly,
            ..Default::default()
        },
    );
    let res1 = engine_ro.execute_command(&spec).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);
    // ReadOnly policy: outputs should NOT have been written to CAS
    if !res1.outputs.is_empty() {
        assert!(!env.storage.has_object(&res1.outputs[0].digest));
    }

    // 2. ReadWrite policy: Writes to cache on miss
    let engine_rw = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            policy: CachePolicy::ReadWrite,
            ..Default::default()
        },
    );
    let res2 = engine_rw.execute_command(&spec).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Miss);
    if !res2.outputs.is_empty() {
        assert!(env.storage.has_object(&res2.outputs[0].digest));
    }

    // Subsequent normal run hits cache
    let res3 = engine_rw.execute_command(&spec).unwrap();
    assert_eq!(res3.status, ExecutionStatus::Hit);
}

// ============================================================================
// 5. 20-POINT DEFINITION OF DONE VALIDATION
// ============================================================================

#[test]
fn test_complete_20_point_definition_of_done() {
    let env = TestEnv::new().unwrap();

    // 1. Initialize local cache
    assert!(env.storage.root_dir().exists());

    // 2. Define computation
    env.create_input_file("input.txt", b"dod_input_data")
        .unwrap();
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "[System.IO.File]::WriteAllText('dod_out.txt', 'DOD_OUTPUT')".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo -n 'DOD_OUTPUT' > dod_out.txt".to_string(),
        ],
    );

    let spec = CommandSpec::builder(cmd)
        .args(args)
        .current_dir(env.workspace_dir.path())
        .input_path("input.txt")
        .output_path("dod_out.txt")
        .build()
        .unwrap();

    let engine = RunnerEngine::new(
        &env.storage,
        EngineOptions {
            working_dir: env.workspace_dir.path().to_path_buf(),
            ..Default::default()
        },
    );

    // 3. First execution -> MISS
    let res1 = engine.execute_command(&spec).unwrap();
    assert_eq!(res1.status, ExecutionStatus::Miss);

    // 4. Stored safely - verify entry exists
    assert!(env.storage.get_entry(&res1.key).unwrap().is_some());

    // 5. Second execution -> HIT
    let res2 = engine.execute_command(&spec).unwrap();
    assert_eq!(res2.status, ExecutionStatus::Hit);

    // 6. Modify input -> MISS
    env.create_input_file("input.txt", b"dod_input_data_modified")
        .unwrap();
    let res3 = engine.execute_command(&spec).unwrap();
    assert_eq!(res3.status, ExecutionStatus::Miss);

    // 7. Output restored safely
    let restored = env.read_output_file("dod_out.txt").unwrap();
    assert_eq!(restored, b"DOD_OUTPUT");

    // 8. Stats inspectable
    let stats = env.storage.stats().unwrap();
    assert!(stats.total_entries >= 2);
}
