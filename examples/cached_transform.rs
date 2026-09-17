use dcc_integrations::{Cache, Computation, ExecutionStatus, GenericIntegration};
use std::fs;
use tempfile::tempdir;

fn main() -> anyhow::Result<()> {
    println!("=== DCC Developer Integration Example: Data Transformation / Asset Pipeline ===");

    // 1. Initialize temporary workspace and cache directories
    let temp_cache = tempdir()?;
    let temp_work = tempdir()?;

    // 2. Open DCC cache using Cache::open and create GenericIntegration runner
    let cache = Cache::open(temp_cache.path())?;
    let integration = GenericIntegration::from_cache(&cache, temp_work.path());

    // 3. Prepare raw data/asset file (e.g. uncompressed dataset or stylesheet/asset)
    let raw_input = temp_work.path().join("dataset.csv");
    fs::write(
        &raw_input,
        "id,user,score\n101,alice,95.5\n102,bob,88.0\n103,charlie,92.3\n",
    )?;

    // 4. Setup mock transformer CLI command (optimizing/converting CSV to minified JSON)
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Set-Content -Path transformed.json -Value '[{\"id\":101,\"user\":\"alice\",\"score\":95.5},{\"id\":102,\"user\":\"bob\",\"score\":88.0},{\"id\":103,\"user\":\"charlie\",\"score\":92.3}]'".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "printf '[{\"id\":101,\"user\":\"alice\",\"score\":95.5},{\"id\":102,\"user\":\"bob\",\"score\":88.0},{\"id\":103,\"user\":\"charlie\",\"score\":92.3}]' > transformed.json".to_string(),
        ],
    );

    // 5. Construct Computation using fluent ergonomic Builder API
    let computation = Computation::builder()
        .operation("data-transform")
        .command(cmd)
        .args(args)
        .input_path(&raw_input)?
        .output("transformed.json", true)
        .env("TRANSFORM_FORMAT", "json_min")
        .meta("pipeline", "etl_v1")
        .build()?;

    println!("Initial Transform Pipeline (Cold Cache -> Expect MISS):");
    let res1 = integration.execute(computation.clone())?;
    println!("  Execution Status: {:?}", res1.status);
    println!("  Computation Key:  {}", res1.key);
    println!("  Exit Code:        {}", res1.exit_code);
    assert_eq!(res1.status, ExecutionStatus::Miss);

    let transformed_path = temp_work.path().join("transformed.json");
    assert!(transformed_path.is_file());

    // 6. Delete transformed output to verify instant cache restoration
    fs::remove_file(&transformed_path)?;
    assert!(!transformed_path.exists());

    println!("\nSecond Transform Pipeline (Warm Cache -> Expect HIT):");
    let res2 = integration.execute(computation)?;
    println!("  Execution Status: {:?}", res2.status);
    println!("  Computation Key:  {}", res2.key);
    println!("  Outputs Restored: {}", res2.outputs.len());
    assert_eq!(res2.status, ExecutionStatus::Hit);
    assert!(transformed_path.is_file());

    let transformed_content = fs::read_to_string(&transformed_path)?;
    println!("\nRestored Transformed JSON:\n{}", transformed_content);
    println!("\nCached Asset Transformation Example completed successfully!");

    Ok(())
}
