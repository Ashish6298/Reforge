use std::fs;
use dcc_core::{Computation, Digest};
use dcc_runner::{EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_storage::{CasStorage, StorageConfig};
use tempfile::tempdir;

fn main() -> anyhow::Result<()> {
    println!("=== DCC Developer Integration Example: Code Generator ===");

    let temp_cache = tempdir()?;
    let temp_work = tempdir()?;

    let storage = CasStorage::new(StorageConfig {
        root_dir: temp_cache.path().to_path_buf(),
        max_size_bytes: None,
    })?;

    let schema_file = temp_work.path().join("schema.json");
    fs::write(&schema_file, r#"{"models": ["User", "Account"]}"#)?;

    #[cfg(windows)]
    let (cmd, args) = (
        "powershell.exe",
        vec![
            "-Command".to_string(),
            "Set-Content -Path models.rs -Value '// Generated Rust Models'".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (cmd, args) = (
        "sh",
        vec![
            "-c".to_string(),
            "echo '// Generated Rust Models' > models.rs".to_string(),
        ],
    );

    let computation = Computation::builder_with("codegen", cmd)
        .args(args)
        .input("schema.json", Digest::from_bytes(b""), 0)
        .output("models.rs", true)
        .build()?;

    let engine = RunnerEngine::new(
        &storage,
        EngineOptions {
            working_dir: temp_work.path().to_path_buf(),
            ..Default::default()
        },
    );

    println!("Run 1 (Cold Execution):");
    let res1 = engine.execute(computation.clone())?;
    println!("Status: {:?}", res1.status);
    println!("Key:    {}", res1.key);

    println!("\nRun 2 (Cached Hit):");
    let res2 = engine.execute(computation)?;
    println!("Status: {:?}", res2.status);
    println!("Key:    {}", res2.key);

    let generated_code = fs::read_to_string(temp_work.path().join("models.rs"))?;
    println!("\nGenerated Output Content:\n{}", generated_code);

    Ok(())
}
