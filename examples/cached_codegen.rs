use dcc_integrations::{Cache, Computation, ExecutionStatus, GenericIntegration};
use std::fs;
use tempfile::tempdir;

fn main() -> anyhow::Result<()> {
    println!("=== DCC Developer Integration Example: Code Generator ===");

    // 1. Initialize temporary workspace and cache directories
    let temp_cache = tempdir()?;
    let temp_work = tempdir()?;

    // 2. Open DCC cache using the first-class Cache public API
    let cache = Cache::open(temp_cache.path())?;
    let integration = GenericIntegration::from_cache(&cache, temp_work.path());

    // 3. Prepare realistic input source: protobuf/JSON schema definition
    let schema_file = temp_work.path().join("schema.json");
    fs::write(
        &schema_file,
        r#"{
  "service": "PaymentGateway",
  "version": "1.0",
  "models": [
    { "name": "Transaction", "fields": { "id": "u64", "amount": "f64", "currency": "String" } },
    { "name": "Account", "fields": { "account_id": "String", "balance": "f64" } }
  ]
}"#,
    )?;

    // 4. Setup mock generator CLI command (platform agnostic)
    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Set-Content -Path models.rs -Value '// Generated Rust Models for PaymentGateway\npub struct Transaction { pub id: u64, pub amount: f64, pub currency: String }\npub struct Account { pub account_id: String, pub balance: f64 }'".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "printf '// Generated Rust Models for PaymentGateway\npub struct Transaction { pub id: u64, pub amount: f64, pub currency: String }\npub struct Account { pub account_id: String, pub balance: f64 }\n' > models.rs".to_string(),
        ],
    );

    // 5. Construct Computation using fluent ergonomic Builder API
    let computation = Computation::builder()
        .operation("codegen")
        .command(cmd)
        .args(args)
        .input_path(&schema_file)?
        .output("models.rs", true)
        .env("TARGET_LANG", "rust")
        .meta("generator_version", "1.2.0")
        .build()?;

    println!("Initial Execution (Cold Cache -> Expect MISS):");
    let res1 = integration.execute(computation.clone())?;
    println!("  Execution Status: {:?}", res1.status);
    println!("  Computation Key:  {}", res1.key);
    println!("  Exit Code:        {}", res1.exit_code);
    println!("  Execution Time:   {} ms", res1.execution_time_ms);
    assert_eq!(res1.status, ExecutionStatus::Miss);

    let generated_file = temp_work.path().join("models.rs");
    assert!(generated_file.is_file());

    // 6. Delete generated output to demonstrate true cache restoration
    fs::remove_file(&generated_file)?;
    assert!(!generated_file.exists());

    println!("\nSecond Execution (Warm Cache -> Expect HIT):");
    let res2 = integration.execute(computation)?;
    println!("  Execution Status: {:?}", res2.status);
    println!("  Computation Key:  {}", res2.key);
    println!("  Outputs Restored: {}", res2.outputs.len());
    assert_eq!(res2.status, ExecutionStatus::Hit);
    assert!(generated_file.is_file());

    let generated_code = fs::read_to_string(&generated_file)?;
    println!("\nRestored Code Content:\n{}", generated_code);
    println!("\nCached Codegen Example completed successfully!");

    Ok(())
}
