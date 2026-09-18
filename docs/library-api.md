# Rust Library API Reference

DCC provides clean, idiomatic Rust public crate APIs enabling developers to embed caching directly into custom build tools, compilers, code generators, and linters without spawning CLI subprocesses.

---

## 1. Crate Hierarchy

- **`dcc_core`**: Core domain types (`Digest`, `CacheKey`, `Computation`, `CacheEntry`, `ByteSize`, `PathUtils`, `SensitiveDataDetector`).
- **`dcc_storage`**: Content-Addressed Storage (`Storage` trait, `CasStorage`, `Cache`, `TieredCache`, `Pruner`).
- **`dcc_runner`**: Process execution, output restoration, and explainable cache misses (`RunnerEngine`, `CommandSpec`, `MissExplainer`).
- **`dcc_integrations`**: High-level integration runners and build models (`GenericIntegration`, `BuildAction`, `ComputationBuilder`).

---

## 2. Low-Level Storage API (`dcc_storage::Cache`)

```rust
use dcc_storage::{Cache, CasStorage, StorageConfig};
use dcc_core::{CacheKey, CacheEntry, Digest};
use std::path::Path;

// Initialize local CAS storage
let config = StorageConfig::new(Path::new("./.dcc_cache"))
    .with_max_size_str("5 GB")?;
let storage = CasStorage::new(config)?;
let cache = Cache::new(storage);

// Query cache entry
if let Some(entry) = cache.lookup(&key)? {
    println!("Cache hit! Created at: {}", entry.created_at);
    // Atomically restore declared outputs
    cache.restore(&entry, Path::new("./workspace"))?;
} else {
    println!("Cache miss!");
    // Store new entry after execution
    cache.store(&new_entry)?;
}
```

---

## 3. Computation Builder API (`dcc_core::Computation`)

Construct computations fluently with automatic validation:

```rust
use dcc_core::{Computation, Digest};

let computation = Computation::builder()
    .operation("codegen")
    .command("generator")
    .arg("--schema")
    .arg("schema.json")
    .arg("--opt-level=2")
    .input("schema.json", schema_digest, schema_size)
    .output("generated/models.rs", true) // required output
    .output("generated/models.pdb", false) // optional output
    .env("CODEGEN_ENV", "production")
    .tool_name("codegen-cli")
    .tool_version("1.4.0")
    .build()?;

// Derive canonical SHA-256 key
let key = computation.canonical_key();
```

---

## 4. High-Level Integration Runner (`dcc_integrations::GenericIntegration`)

The easiest way to execute and cache computations in Rust applications:

```rust
use dcc_integrations::{Cache, ComputationBuilder, GenericIntegration};
use std::path::Path;

// 1. Open the cache directory
let cache = Cache::open(Path::new("./.dcc_cache"))?;

// 2. Define computation specification
let spec = ComputationBuilder::new("minify-assets")
    .with_arguments(vec!["bundle.js".into(), "-o".into(), "bundle.min.js".into()])
    .with_inputs(vec![Path::new("bundle.js").to_path_buf()])
    .with_outputs(vec![Path::new("bundle.min.js").to_path_buf()])
    .build();

// 3. Execute with caching
let integration = GenericIntegration::from_cache(&cache, Default::default());
let result = integration.execute(&spec, Path::new("."))?;

if result.was_hit {
    println!("Restored in {} ms (0ms execution)", result.duration_ms);
} else {
    println!("Executed and cached in {} ms", result.duration_ms);
}
```

---

## 5. Build Action Model (`dcc_integrations::BuildAction`)

Specialized model for compiler actions:

```rust
use dcc_integrations::{BuildAction, GenericIntegration};

let action = BuildAction::builder()
    .compiler("rustc")
    .arguments(vec!["src/main.rs", "--crate-type=bin", "-O"])
    .source_input("src/main.rs", src_digest, src_size)
    .compiler_version("1.80.0")
    .target("x86_64-unknown-linux-gnu")
    .env("RUSTFLAGS", "-C opt-level=3")
    .output("target/main", true)
    .build()?;

let key = action.compute_key()?;
let result = integration.execute_build_action(action)?;
```
