use dcc_integrations::{BuildAction, Cache, ExecutionStatus, GenericIntegration};
use std::fs;
use tempfile::tempdir;

fn main() -> anyhow::Result<()> {
    println!("=== DCC Developer Integration: Rust Build Caching (Milestone 11.2) ===");

    // 1. Initialize temporary workspace and cache directories
    let temp_cache = tempdir()?;
    let temp_work = tempdir()?;

    // 2. Open DCC cache using first-class Cache public API
    let cache = Cache::open(temp_cache.path())?;
    let integration = GenericIntegration::from_cache(&cache, temp_work.path());

    // 3. Prepare Rust source code file
    let src_file = temp_work.path().join("main.rs");
    fs::write(
        &src_file,
        r#"fn main() {
    println!("Hello from cached Rust binary!");
}
"#,
    )?;

    // 4. Setup mock rustc compiler command
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Set-Content -Path main.exe -Value 'rustc_mock_executable_binary_payload'".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo 'rustc_mock_executable_binary_payload' > main.exe".to_string(),
        ],
    );

    // 5. Construct BuildAction representing Rust compiler invocation:
    // source files + compiler configuration + tool identity -> computation key -> cached artifact
    let build_action = BuildAction::builder()
        .compiler(cmd)
        .arguments(args)
        .source_file(&src_file)?
        .compiler_version("rustc 1.80.0")
        .target("x86_64-pc-windows-msvc")
        .env("RUSTFLAGS", "-C opt-level=3")
        .output("main.exe", true)
        .build()?;

    println!("Build #1 (Cold Execution):");
    let res1 = integration.execute_build_action(build_action.clone())?;
    println!("  Status: {:?}", res1.status);
    println!("  Key:    {}", res1.key);
    assert_eq!(res1.status, ExecutionStatus::Miss);

    let output_exe = temp_work.path().join("main.exe");
    assert!(output_exe.is_file());

    // 6. Delete output binary to demonstrate true cache restoration
    fs::remove_file(&output_exe)?;
    assert!(!output_exe.exists());

    println!("\nBuild #2 (Warm Execution - Expect Cache Hit):");
    let res2 = integration.execute_build_action(build_action.clone())?;
    println!("  Status: {:?}", res2.status);
    println!("  Key:    {}", res2.key);
    assert_eq!(res2.status, ExecutionStatus::Hit);
    assert!(output_exe.is_file());

    // 7. Modify source file -> Expect Cache Miss
    fs::write(
        &src_file,
        r#"fn main() {
    println!("Modified Rust binary payload!");
}
"#,
    )?;

    // Rebuild build action with modified source
    let modified_action = BuildAction::builder()
        .compiler(build_action.compiler)
        .arguments(build_action.arguments)
        .source_file(&src_file)?
        .compiler_version(build_action.compiler_version.unwrap())
        .target(build_action.target.unwrap())
        .envs(build_action.environment)
        .output("main.exe", true)
        .build()?;

    println!("\nBuild #3 (Source Modified -> Expect Cache Miss):");
    let res3 = integration.execute_build_action(modified_action)?;
    println!("  Status: {:?}", res3.status);
    println!("  Key:    {}", res3.key);
    assert_eq!(res3.status, ExecutionStatus::Miss);
    assert_ne!(res1.key, res3.key);

    println!("\nRust Build Integration demonstration completed successfully!");
    Ok(())
}
