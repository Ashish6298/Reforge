use anyhow::{Context, Result};
use clap::Parser;
use dcc_core::{CacheKey, CachePolicy, Computation, Digest};
use dcc_runner::{EngineOptions, ExecutionStatus, RunnerEngine};
use dcc_storage::{CasStorage, EvictionStrategy, Pruner, StorageConfig, StorageStats};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use walkdir::WalkDir;

mod cli;
use cli::{CacheCommands, Cli, Commands, RunArgs};

fn main() -> Result<()> {
    let cli = Cli::parse();

    let cache_dir = cli.cache_dir.unwrap_or_else(|| {
        if let Ok(dir) = std::env::var("DCC_CACHE_DIR") {
            PathBuf::from(dir)
        } else {
            let base = dirs_base();
            base.join(".dcc_cache")
        }
    });

    let config = StorageConfig {
        root_dir: cache_dir,
        max_size_bytes: Some(10 * 1024 * 1024 * 1024),
    };

    let storage = CasStorage::new(config)?;

    match cli.command {
        Commands::Init { max_size } => handle_init(&storage, max_size.as_deref(), cli.json)?,
        Commands::Run(args) => handle_run(&storage, args, cli.json)?,
        Commands::Inspect { key } => handle_inspect(&storage, &key, cli.json)?,
        Commands::Stats => handle_stats(&storage, cli.json)?,
        Commands::Verify => handle_verify(&storage, cli.json)?,
        Commands::Clean { key } => handle_clean(&storage, key.as_deref(), cli.json)?,
        Commands::Prune {
            max_size,
            strategy,
            dry_run,
        } => handle_prune(&storage, max_size.as_deref(), &strategy, dry_run, cli.json)?,
        Commands::Config { get } => handle_config(&storage, get.as_deref(), cli.json)?,
        Commands::Doctor => handle_doctor(&storage, cli.json)?,
        Commands::Cache { command } => match command {
            CacheCommands::Clean { key } => handle_clean(&storage, key.as_deref(), cli.json)?,
            CacheCommands::Prune {
                max_size,
                strategy,
                dry_run,
            } => handle_prune(&storage, max_size.as_deref(), &strategy, dry_run, cli.json)?,
            CacheCommands::Verify => handle_verify(&storage, cli.json)?,
            CacheCommands::Stats => handle_stats(&storage, cli.json)?,
        },
    }

    Ok(())
}

fn dirs_base() -> PathBuf {
    if let Some(user_dirs) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(user_dirs);
    }
    PathBuf::from(".")
}

fn handle_init(storage: &CasStorage, max_size_str: Option<&str>, json: bool) -> Result<()> {
    let max_size = if let Some(s) = max_size_str {
        Some(
            dcc_core::ByteSize::parse(s)
                .with_context(|| format!("Invalid max_size value '{}'", s))?,
        )
    } else {
        None
    };
    storage.init_dirs()?;
    let limit_bytes = max_size
        .map(|s| s.as_bytes())
        .unwrap_or(10 * 1024 * 1024 * 1024);
    if json {
        println!(
            "{}",
            serde_json::json!({
                "status": "initialized",
                "cache_dir": storage.root_dir(),
                "max_size_bytes": limit_bytes,
                "max_size_human": dcc_core::ByteSize::bytes(limit_bytes).to_human_readable()
            })
        );
    } else {
        println!("DCC cache initialized at: {}", storage.root_dir().display());
        println!(
            "Max Cache Size: {}",
            dcc_core::ByteSize::bytes(limit_bytes).to_human_readable()
        );
        println!("Objects: {}", storage.objects_dir().display());
        println!("Entries: {}", storage.entries_dir().display());
    }
    Ok(())
}

fn handle_run(storage: &CasStorage, args: RunArgs, json: bool) -> Result<()> {
    if args.command.is_empty() {
        eprintln!("Error: No command specified to run.");
        std::process::exit(5);
    }

    let cmd_exe = &args.command[0];
    let cmd_args = &args.command[1..];

    let policy = match args.policy.to_lowercase().as_str() {
        "read-only" => CachePolicy::ReadOnly,
        "write-only" => CachePolicy::WriteOnly,
        "bypass" => CachePolicy::Bypass,
        "force-recompute" => CachePolicy::ForceRecompute,
        _ => CachePolicy::ReadWrite,
    };

    let mut comp_builder = Computation::builder(&args.operation, cmd_exe).args(cmd_args.to_vec());

    for input in &args.inputs {
        let dummy_digest = Digest::from_bytes(b"");
        comp_builder = comp_builder.input(input, dummy_digest, 0);
    }

    for output in &args.outputs {
        comp_builder = comp_builder.output(output, true);
    }

    for env_decl in &args.env {
        if let Some((k, v)) = env_decl.split_once('=') {
            comp_builder = comp_builder.env(k, v);
        } else if let Ok(val) = std::env::var(env_decl) {
            comp_builder = comp_builder.env(env_decl, val);
        }
    }

    let computation = comp_builder
        .build()
        .context("Invalid computation specification")?;

    let engine = RunnerEngine::new(
        storage,
        EngineOptions {
            policy,
            failure_policy: dcc_runner::FailurePolicy::default(),
            working_dir: std::env::current_dir()?,
            lock_timeout: Duration::from_secs(30),
        },
    );

    let result = engine.execute(computation)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        match result.status {
            ExecutionStatus::Hit => {
                println!(
                    "[DCC HIT] Restored outputs from cache (key: {})",
                    result.key
                );
                if !result.stdout.is_empty() {
                    print!("{}", String::from_utf8_lossy(&result.stdout));
                }
                if !result.stderr.is_empty() {
                    eprint!("{}", String::from_utf8_lossy(&result.stderr));
                }
            }
            ExecutionStatus::Miss | ExecutionStatus::Bypassed => {
                println!(
                    "[DCC MISS] Executed in {}ms (key: {})",
                    result.execution_time_ms, result.key
                );
                if args.explain {
                    if let Some(reason) = &result.miss_reason {
                        println!("Reason: {}", reason);
                    }
                }
                if !result.stdout.is_empty() {
                    print!("{}", String::from_utf8_lossy(&result.stdout));
                }
                if !result.stderr.is_empty() {
                    eprint!("{}", String::from_utf8_lossy(&result.stderr));
                }
            }
        }
    }

    if result.exit_code != 0 {
        std::process::exit(result.exit_code);
    }

    Ok(())
}

fn handle_inspect(storage: &CasStorage, key_str: &str, json: bool) -> Result<()> {
    let digest = Digest::new(key_str)?;
    let key = CacheKey::new(digest);

    match storage.get_entry(&key)? {
        Some(entry) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&entry)?);
            } else {
                println!("--- Computation Entry ---");
                println!("Key:           {}", entry.key);
                println!("Operation:     {}", entry.computation.operation);
                println!("Command:       {}", entry.computation.command);
                println!("Arguments:     {:?}", entry.computation.args);
                println!("Inputs ({}):", entry.computation.inputs.len());
                for inp in &entry.computation.inputs {
                    println!(
                        "  - {} ({} bytes, sha256:{})",
                        inp.path, inp.size, inp.digest
                    );
                }
                println!("Outputs ({}):", entry.outputs.len());
                for out in &entry.outputs {
                    println!(
                        "  - {} ({} bytes, sha256:{})",
                        out.path, out.size, out.digest
                    );
                }
                println!("Created At:    {}", entry.metadata.created_at);
                println!("Last Accessed: {}", entry.metadata.last_accessed_at);
                println!("Hit Count:     {}", entry.metadata.hit_count);
                println!(
                    "Execution Time:{} ms",
                    entry.metadata.execution.execution_time_ms
                );
            }
        }
        None => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "error": "not_found", "key": key_str })
                );
            } else {
                println!("No cache entry found for key: {}", key_str);
            }
            std::process::exit(1);
        }
    }
    Ok(())
}

fn handle_stats(storage: &CasStorage, json: bool) -> Result<()> {
    let stats = StorageStats::collect(storage)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("=== DCC Cache Storage Statistics ===");
        println!("Total Entries:        {}", stats.total_entries);
        println!("Total CAS Objects:    {}", stats.total_objects);
        println!(
            "Objects Disk Usage:   {:.2} MB",
            stats.total_object_size_bytes as f64 / (1024.0 * 1024.0)
        );
        println!(
            "Entries Disk Usage:   {:.2} KB",
            stats.total_entry_size_bytes as f64 / 1024.0
        );
        println!(
            "Total Disk Usage:     {:.2} MB",
            stats.total_size_bytes as f64 / (1024.0 * 1024.0)
        );
        println!(
            "Largest Object:       {} bytes",
            stats.largest_object_size_bytes
        );
    }
    Ok(())
}

fn handle_verify(storage: &CasStorage, json: bool) -> Result<()> {
    let mut verified = 0;
    let mut corrupted = 0;
    let objects_dir = storage.objects_dir();

    if objects_dir.exists() {
        for entry in WalkDir::new(objects_dir).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                let name = entry.file_name().to_string_lossy();
                if let Ok(digest) = Digest::new(name.as_ref()) {
                    match storage.verify_object(&digest) {
                        Ok(()) => verified += 1,
                        Err(e) => {
                            corrupted += 1;
                            eprintln!(
                                "Corrupted object found at {}: {}",
                                entry.path().display(),
                                e
                            );
                        }
                    }
                }
            }
        }
    }

    if json {
        println!(
            "{}",
            serde_json::json!({
                "verified_objects": verified,
                "corrupted_objects": corrupted,
                "status": if corrupted == 0 { "healthy" } else { "corruption_detected" }
            })
        );
    } else {
        println!(
            "Integrity check complete: {} verified, {} corrupted.",
            verified, corrupted
        );
    }

    if corrupted > 0 {
        std::process::exit(4);
    }

    Ok(())
}

fn handle_clean(storage: &CasStorage, key_opt: Option<&str>, json: bool) -> Result<()> {
    if let Some(key_str) = key_opt {
        let digest = Digest::new(key_str)?;
        let key = CacheKey::new(digest);
        let deleted = storage.delete_entry(&key)?;
        if json {
            println!(
                "{}",
                serde_json::json!({ "key": key_str, "deleted": deleted })
            );
        } else {
            println!("Deleted key {}: {}", key_str, deleted);
        }
    } else {
        let _ = fs::remove_dir_all(storage.objects_dir());
        let _ = fs::remove_dir_all(storage.entries_dir());
        storage.init_dirs()?;
        if json {
            println!("{}", serde_json::json!({ "status": "cache_cleaned" }));
        } else {
            println!("Cleared all cached objects and entries.");
        }
    }
    Ok(())
}

fn handle_prune(
    storage: &CasStorage,
    max_size_str: Option<&str>,
    strategy_str: &str,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    let pruner = Pruner::new(storage);
    let strategy = match strategy_str.to_lowercase().as_str() {
        "lru" => EvictionStrategy::Lru,
        "fifo" => EvictionStrategy::Fifo,
        "lfu" => EvictionStrategy::Lfu,
        other => anyhow::bail!(
            "Unknown eviction strategy '{}'. Supported: lru, fifo, lfu",
            other
        ),
    };

    let result = if let Some(s) = max_size_str {
        let size = dcc_core::ByteSize::parse(s)
            .with_context(|| format!("Invalid max_size value '{}'", s))?;
        pruner.evict_with_strategy(strategy, size.as_bytes())?
    } else {
        pruner.prune_with_options(dry_run)?
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        let prefix = if dry_run {
            "Dry-run prune complete"
        } else {
            "Prune complete"
        };
        let strat_label = result
            .strategy
            .map(|s| format!(" (strategy: {:?})", s))
            .unwrap_or_default();
        println!(
            "{}{}: deleted {} entries, {} unreferenced objects, freed {:.2} MB.",
            prefix,
            strat_label,
            result.deleted_entries,
            result.deleted_objects,
            result.freed_bytes as f64 / (1024.0 * 1024.0)
        );
    }
    Ok(())
}

fn handle_doctor(storage: &CasStorage, json: bool) -> Result<()> {
    let write_ok = storage.init_dirs().is_ok();
    let stats = StorageStats::collect(storage)?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "healthy": write_ok,
                "cache_dir": storage.root_dir(),
                "writable": write_ok,
                "objects_count": stats.total_objects,
                "entries_count": stats.total_entries,
                "total_bytes": stats.total_size_bytes,
            })
        );
    } else {
        println!("=== DCC Doctor Diagnostic ===");
        println!("Cache Directory:    {}", storage.root_dir().display());
        println!(
            "Directory Writable: {}",
            if write_ok { "YES" } else { "NO" }
        );
        println!(
            "Health Status:      {}",
            if write_ok { "OK" } else { "DEGRADED" }
        );
        println!("Total Objects:      {}", stats.total_objects);
        println!("Total Entries:      {}", stats.total_entries);
    }
    Ok(())
}

fn handle_config(storage: &CasStorage, get_key: Option<&str>, json: bool) -> Result<()> {
    let max_size = storage
        .max_size()
        .map(|bs| bs.as_bytes())
        .unwrap_or(10 * 1024 * 1024 * 1024);
    let cache_dir = storage.root_dir();

    if let Some(key) = get_key {
        match key.to_lowercase().as_str() {
            "max_size" | "max-size" => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "key": "max_size",
                            "value_bytes": max_size,
                            "value_human": dcc_core::ByteSize::bytes(max_size).to_human_readable()
                        })
                    );
                } else {
                    println!(
                        "{}",
                        dcc_core::ByteSize::bytes(max_size).to_human_readable()
                    );
                }
            }
            "cache_dir" | "cache-dir" | "dir" => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "key": "cache_dir",
                            "value": cache_dir.display().to_string()
                        })
                    );
                } else {
                    println!("{}", cache_dir.display());
                }
            }
            other => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "error": "unknown_config_key",
                            "key": other
                        })
                    );
                } else {
                    eprintln!(
                        "Unknown configuration key: '{}'. Supported: max_size, cache_dir",
                        other
                    );
                }
                std::process::exit(5);
            }
        }
    } else if json {
        println!(
            "{}",
            serde_json::json!({
                "cache_dir": cache_dir.display().to_string(),
                "max_size_bytes": max_size,
                "max_size_human": dcc_core::ByteSize::bytes(max_size).to_human_readable(),
                "objects_dir": storage.objects_dir().display().to_string(),
                "entries_dir": storage.entries_dir().display().to_string(),
                "locks_dir": storage.locks_dir().display().to_string(),
                "tmp_dir": storage.tmp_dir().display().to_string()
            })
        );
    } else {
        println!("=== DCC Configuration ===");
        println!("Cache Directory: {}", cache_dir.display());
        println!(
            "Max Cache Size:  {}",
            dcc_core::ByteSize::bytes(max_size).to_human_readable()
        );
        println!("Objects Dir:     {}", storage.objects_dir().display());
        println!("Entries Dir:     {}", storage.entries_dir().display());
        println!("Locks Dir:       {}", storage.locks_dir().display());
        println!("Tmp Dir:         {}", storage.tmp_dir().display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcc_core::entry::{CacheEntry, ExecutionMetadata, OutputManifestItem};

    #[test]
    fn test_cli_init_and_config_handlers() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().to_path_buf(),
            max_size_bytes: Some(2 * 1024 * 1024 * 1024),
        };
        let storage = CasStorage::new(config).unwrap();

        // 1. handle_init
        assert!(handle_init(&storage, Some("500 MB"), false).is_ok());
        assert!(handle_init(&storage, None, true).is_ok());

        // 2. handle_config
        assert!(handle_config(&storage, None, false).is_ok());
        assert!(handle_config(&storage, None, true).is_ok());
        assert!(handle_config(&storage, Some("max_size"), false).is_ok());
        assert!(handle_config(&storage, Some("cache_dir"), true).is_ok());
    }

    #[test]
    fn test_cli_stats_doctor_verify_clean_handlers() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().to_path_buf(),
            max_size_bytes: Some(10 * 1024 * 1024 * 1024),
        };
        let storage = CasStorage::new(config).unwrap();
        storage.init_dirs().unwrap();

        // 1. handle_doctor
        assert!(handle_doctor(&storage, false).is_ok());
        assert!(handle_doctor(&storage, true).is_ok());

        // 2. handle_stats on empty cache
        assert!(handle_stats(&storage, false).is_ok());
        assert!(handle_stats(&storage, true).is_ok());

        // 3. Populate an entry and verify
        let (d, s) = storage.store_object_bytes(b"cli test artifact").unwrap();
        let comp = Computation::builder("cli_test", "echo").build().unwrap();
        let key = comp.compute_key().unwrap();
        let entry = CacheEntry::new(
            key.clone(),
            comp,
            vec![OutputManifestItem {
                path: "out.txt".into(),
                digest: d,
                size: s,
                is_executable: None,
            }],
            ExecutionMetadata::default(),
        );
        storage.store_entry(&entry).unwrap();

        // 4. handle_inspect
        assert!(handle_inspect(&storage, key.as_str(), false).is_ok());
        assert!(handle_inspect(&storage, key.as_str(), true).is_ok());

        // 5. handle_verify
        assert!(handle_verify(&storage, false).is_ok());
        assert!(handle_verify(&storage, true).is_ok());

        // 6. handle_prune
        assert!(handle_prune(&storage, None, "lru", true, false).is_ok());
        assert!(handle_prune(&storage, Some("1 GB"), "fifo", false, true).is_ok());

        // 7. handle_clean specific key and all
        assert!(handle_clean(&storage, Some(key.as_str()), false).is_ok());
        assert!(handle_clean(&storage, None, true).is_ok());
    }
}
