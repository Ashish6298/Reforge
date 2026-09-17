use dcc_integrations::{Cache, Computation, ExecutionStatus, GenericIntegration};
use std::fs;
use tempfile::tempdir;

fn main() -> anyhow::Result<()> {
    println!("=== DCC Developer Integration Example: Static Code Analysis / Linter ===");

    // 1. Initialize temporary workspace and cache directories
    let temp_cache = tempdir()?;
    let temp_work = tempdir()?;

    // 2. Open DCC cache using Cache::open and create GenericIntegration runner
    let cache = Cache::open(temp_cache.path())?;
    let integration = GenericIntegration::from_cache(&cache, temp_work.path());

    // 3. Prepare realistic source files and analysis configuration rules
    let config_file = temp_work.path().join("rules.toml");
    fs::write(
        &config_file,
        r#"[rules]
max_complexity = 10
deny_unsafe = true
warn_unused = true
"#,
    )?;

    let source_file = temp_work.path().join("service.rs");
    fs::write(
        &source_file,
        r#"pub fn calculate_total(items: &[f64]) -> f64 {
    items.iter().sum()
}
"#,
    )?;

    // 4. Setup mock static analysis CLI command (producing a report artifact and stdout report)
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Set-Content -Path analysis_report.json -Value '{\"errors\": 0, \"warnings\": 0, \"complexity\": 2, \"passed\": true}'".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "printf '{\"errors\": 0, \"warnings\": 0, \"complexity\": 2, \"passed\": true}' > analysis_report.json".to_string(),
        ],
    );

    // 5. Construct Computation using fluent ergonomic Builder API
    let computation = Computation::builder()
        .operation("static-analysis")
        .command(cmd)
        .args(args)
        .input_path(&config_file)?
        .input_path(&source_file)?
        .output("analysis_report.json", true)
        .env("ANALYSIS_LEVEL", "strict")
        .meta("tool", "dcc-linter")
        .build()?;

    println!("Initial Analysis Run (Cold Cache -> Expect MISS):");
    let res1 = integration.execute(computation.clone())?;
    println!("  Execution Status: {:?}", res1.status);
    println!("  Computation Key:  {}", res1.key);
    println!("  Exit Code:        {}", res1.exit_code);
    assert_eq!(res1.status, ExecutionStatus::Miss);

    let report_path = temp_work.path().join("analysis_report.json");
    assert!(report_path.is_file());

    // 6. Delete report to verify instant cache restoration on unchanged files
    fs::remove_file(&report_path)?;
    assert!(!report_path.exists());

    println!("\nSecond Analysis Run (Warm Cache -> Expect HIT):");
    let res2 = integration.execute(computation)?;
    println!("  Execution Status: {:?}", res2.status);
    println!("  Computation Key:  {}", res2.key);
    println!("  Outputs Restored: {}", res2.outputs.len());
    assert_eq!(res2.status, ExecutionStatus::Hit);
    assert!(report_path.is_file());

    let report_content = fs::read_to_string(&report_path)?;
    println!("\nRestored Analysis Report:\n{}", report_content);
    println!("\nCached Static Analysis Example completed successfully!");

    Ok(())
}
