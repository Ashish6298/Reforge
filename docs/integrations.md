# Developer Integrations Guide

This guide explains how to integrate DCC into your custom build systems, code generators, asset pipelines, linters, and compiler frontends.

---

## 1. Integration Contract

When integrating DCC into developer tools, adhere to these contract rules:

1. **Declare Complete Inputs**: All files whose content directly affects the output must be declared. DCC hashes input contents using SHA-256 (independent of file timestamps).
2. **Declare Explicit Outputs**: All files or directories produced by the command must be declared. Required outputs must physically exist after execution.
3. **Declare Only Relevant Environment Variables**: Only declare environment variables that alter output bytes (e.g. `DEBUG`, `TARGET_ARCH`, `OPTIMIZATION`). Do not include ambient system variables (`USER`, `PATH`, `PWD`), which cause unnecessary cache fragmentation.
4. **Normalize Paths**: Use relative workspace-bound paths. DCC automatically normalizes path separators to `/` across all platforms.
5. **Handle Failures Safely**: By default, DCC never caches failed executions (`exit_code != 0`).

---

## 2. Common Integration Patterns

### Pattern A: Code Generators (Schema ➔ Source)

```rust
use dcc_integrations::{Cache, ComputationBuilder, GenericIntegration};
use std::path::Path;

let cache = Cache::open(Path::new("./.dcc_cache"))?;
let integration = GenericIntegration::from_cache(&cache, Default::default());

let spec = ComputationBuilder::new("openapi-codegen")
    .with_arguments(vec!["generate".into(), "-i".into(), "api/schema.yaml".into()])
    .with_inputs(vec![Path::new("api/schema.yaml").to_path_buf()])
    .with_outputs(vec![
        Path::new("src/generated/api.rs").to_path_buf(),
        Path::new("src/generated/models.rs").to_path_buf(),
    ])
    .build();

let result = integration.execute(&spec, Path::new("."))?;
```

---

### Pattern B: Static Analysis & Linters (Source ➔ Report)

```rust
let spec = ComputationBuilder::new("linter-check")
    .with_arguments(vec!["--config".into(), "lint.toml".into(), "src/".into()])
    .with_inputs(vec![
        Path::new("lint.toml").to_path_buf(),
        Path::new("src/lib.rs").to_path_buf(),
        Path::new("src/main.rs").to_path_buf(),
    ])
    .with_outputs(vec![Path::new("target/lint-report.json").to_path_buf()])
    .build();

let result = integration.execute(&spec, Path::new("."))?;
```

---

### Pattern C: Compiler Build Actions (`BuildAction`)

```rust
use dcc_integrations::BuildAction;

let action = BuildAction::builder()
    .compiler("clang++")
    .arguments(vec!["-c".into(), "src/engine.cpp".into(), "-O3".into(), "-o".into(), "build/engine.o".into()])
    .source_input("src/engine.cpp", engine_digest, engine_size)
    .dependency_input("include/engine.h", header_digest, header_size)
    .compiler_version("17.0.6")
    .target("x86_64-pc-linux-gnu")
    .output("build/engine.o", true)
    .build()?;

let result = integration.execute_build_action(action)?;
```

---

## 3. Working Examples in Repository

Explore the ready-to-run examples in the `examples/` directory:

```bash
# Code generator caching
cargo run --example cached_codegen

# Static analysis and linter caching
cargo run --example cached_analysis

# Asset minification & transformation
cargo run --example cached_transform

# Rust compiler build caching
cargo run --example cached_rust_build

# Full reproducible build lifecycle demo
cargo run --example build_demonstration
```
