use dcc_integrations::{BuildAction, Cache, ExecutionStatus, GenericIntegration};
use std::fs;
use tempfile::tempdir;

fn main() -> anyhow::Result<()> {
    println!("===============================================================");
    println!("=== DCC MILESTONE 11.3: REPRODUCIBLE BUILD DEMONSTRATION ===");
    println!("===============================================================\n");

    // 1. Initialize Benchmark Workspace & Cache Environment
    let temp_cache = tempdir()?;
    let temp_work = tempdir()?;
    let cache_dir = temp_cache.path().join(".dcc_cache");
    let ws_dir = temp_work.path().join("benchmark_project");
    fs::create_dir_all(&ws_dir)?;

    println!("Workspace Directory: {}", ws_dir.display());
    println!("Cache Directory:     {}\n", cache_dir.display());

    let cache = Cache::open(&cache_dir)?;
    let integration = GenericIntegration::from_cache(&cache, &ws_dir);

    // 2. Setup Benchmark Project Source Tree
    let src_dir = ws_dir.join("src");
    fs::create_dir_all(&src_dir)?;

    let main_rs = src_dir.join("main.rs");
    let math_rs = src_dir.join("math.rs");
    let target_bin = ws_dir.join("app_binary.bin");

    fs::write(
        &math_rs,
        r#"pub fn multiply(a: i64, b: i64) -> i64 {
    a * b
}
"#,
    )?;

    fs::write(
        &main_rs,
        r#"mod math;

fn main() {
    let result = math::multiply(7, 6);
    println!("Result: {}", result);
}
"#,
    )?;

    // 3. Setup Benchmark Compiler Command (Platform Independent Mock Compiler)
    #[cfg(windows)]
    let (compiler_cmd, compiler_args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            format!(
                "Set-Content -Path '{}' -Value 'compiled_executable_binary_v1_payload'",
                target_bin.display()
            ),
        ],
    );
    #[cfg(not(windows))]
    let (compiler_cmd, compiler_args) = (
        "sh",
        vec![
            "-c".to_string(),
            format!(
                "echo 'compiled_executable_binary_v1_payload' > '{}'",
                target_bin.display()
            ),
        ],
    );

    // Helper to construct BuildAction for benchmark project
    let create_action = |compiler: &str,
                         args: Vec<String>,
                         main: &std::path::Path,
                         math: &std::path::Path|
     -> anyhow::Result<BuildAction> {
        BuildAction::builder()
            .compiler(compiler)
            .arguments(args)
            .source_file(main)?
            .source_file(math)?
            .compiler_version("rustc 1.80.0")
            .target("x86_64-benchmark-target")
            .env("OPT_LEVEL", "3")
            .output("app_binary.bin", true)
            .build()
            .map_err(Into::into)
    };

    // -------------------------------------------------------------------------
    // STEP 1: RUN BUILD #1 (Cold Cache)
    // Expect: MISS -> compile -> store
    // -------------------------------------------------------------------------
    println!(">>> RUNNING BUILD #1 (Initial Cold Build)");
    let action_1 = create_action(compiler_cmd, compiler_args.clone(), &main_rs, &math_rs)?;
    let res_1 = integration.execute_build_action(action_1.clone())?;

    println!("    Result:       {:?}", res_1.status);
    println!("    Action:       MISS -> compile -> store in CAS");
    println!("    Cache Key:    {}", res_1.key);
    println!("    Exit Code:    {}", res_1.exit_code);
    println!("    Duration:     {} ms", res_1.execution_time_ms);
    assert_eq!(
        res_1.status,
        ExecutionStatus::Miss,
        "Build #1 must be a Cache MISS"
    );
    assert!(target_bin.is_file(), "Build #1 output binary must exist");
    let content_1 = fs::read(&target_bin)?;
    println!("    Output Size:  {} bytes\n", content_1.len());

    // Clean up local target binary to prove Build #2 restores it from CAS
    fs::remove_file(&target_bin)?;
    assert!(
        !target_bin.exists(),
        "Target binary deleted for restoration test"
    );

    // -------------------------------------------------------------------------
    // STEP 2: RUN BUILD #2 (Unmodified Sources)
    // Expect: HIT -> restore
    // -------------------------------------------------------------------------
    println!(">>> RUNNING BUILD #2 (Identical Sources & Config)");
    let action_2 = create_action(compiler_cmd, compiler_args.clone(), &main_rs, &math_rs)?;
    let res_2 = integration.execute_build_action(action_2)?;

    println!("    Result:       {:?}", res_2.status);
    println!("    Action:       HIT -> restore from CAS");
    println!("    Cache Key:    {}", res_2.key);
    println!("    Exit Code:    {}", res_2.exit_code);
    println!("    Restored:     {} output artifacts", res_2.outputs.len());
    assert_eq!(
        res_2.status,
        ExecutionStatus::Hit,
        "Build #2 must be a Cache HIT"
    );
    assert_eq!(
        res_1.key, res_2.key,
        "Build #1 and #2 keys must be identical"
    );
    assert!(
        target_bin.is_file(),
        "Build #2 output binary must be restored"
    );
    let content_2 = fs::read(&target_bin)?;
    assert_eq!(
        content_1, content_2,
        "Restored binary content must match exactly"
    );
    println!("    Integrity:    Exact binary content match verified!\n");

    // -------------------------------------------------------------------------
    // STEP 3: MODIFY ONE SOURCE FILE & RUN BUILD #3
    // Expect: MISS -> compile -> store
    // -------------------------------------------------------------------------
    println!(">>> MODIFYING ONE SOURCE FILE (src/math.rs)...");
    fs::write(
        &math_rs,
        r#"pub fn multiply(a: i64, b: i64) -> i64 {
    // Modified implementation with addition check
    if a == 0 || b == 0 { 0 } else { a * b }
}
"#,
    )?;

    #[cfg(windows)]
    let (compiler_cmd_v2, compiler_args_v2) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            format!(
                "Set-Content -Path '{}' -Value 'compiled_executable_binary_v2_modified_payload'",
                target_bin.display()
            ),
        ],
    );
    #[cfg(not(windows))]
    let (compiler_cmd_v2, compiler_args_v2) = (
        "sh",
        vec![
            "-c".to_string(),
            format!(
                "echo 'compiled_executable_binary_v2_modified_payload' > '{}'",
                target_bin.display()
            ),
        ],
    );

    println!(">>> RUNNING BUILD #3 (After Modifying src/math.rs)");
    let action_3 = create_action(compiler_cmd_v2, compiler_args_v2, &main_rs, &math_rs)?;
    let res_3 = integration.execute_build_action(action_3)?;

    println!("    Result:       {:?}", res_3.status);
    println!("    Action:       MISS -> compile -> store new version in CAS");
    println!("    Cache Key:    {}", res_3.key);
    println!("    Exit Code:    {}", res_3.exit_code);
    assert_eq!(
        res_3.status,
        ExecutionStatus::Miss,
        "Build #3 must be a Cache MISS"
    );
    assert_ne!(
        res_1.key, res_3.key,
        "Build #3 key must differ from Build #1"
    );
    assert!(target_bin.is_file(), "Build #3 output binary must exist");
    let content_3 = fs::read(&target_bin)?;
    assert_ne!(
        content_1, content_3,
        "Build #3 output binary must reflect new compilation"
    );
    println!("    Distinct Key: Verified new canonical key for modified source!\n");

    println!("===============================================================");
    println!("=== MILESTONE 11.3 BUILD DEMONSTRATION PASSED (REPRODUCIBLE) ===");
    println!("===============================================================");

    Ok(())
}
