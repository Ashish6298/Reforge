# Developer Computation Cache (`dcc`)

A high-performance, local-first, content-addressed developer computation caching engine written in pure Rust.

## Core Guarantee

> **A cache hit must never change the semantic result of the computation.**  
> If the system cannot prove that a cached result is safe to reuse, it executes the computation again.

---

## 1. What Problem Does This Solve?

Software engineering workflows waste enormous amounts of CPU time and developer attention re-running identical, deterministic computations:
- **Code Generators**: Running OpenAPI/Protobuf generators that take seconds to produce the same files when schemas haven't changed.
- **Linters & Static Analyzers**: Re-scanning entire repositories when only one file was touched.
- **Asset Pipelines**: Re-minifying stylesheets and bundling JavaScript repeatedly during development.
- **Compiler Invocations**: Rebuilding C/C++/Rust modules in clean CI runs or across local branches.

`dcc` provides a universal, process-level computation cache that intercepts these commands, computes cryptographic input digests, and restores previously produced outputs in **0 ms** without re-executing the underlying tool.

---

## 2. Why Is It Different?

Unlike language-specific caches or complex monorepo build systems:
- **Tool-Agnostic**: Works with any CLI command, generator, or compiler—no proprietary build DSLs required.
- **Content-Addressed (Not mtime-based)**: Computes SHA-256 digests over actual input file bytes. Switching branches or `git checkout` never causes false cache misses.
- **Local-First & Zero Setup**: Runs completely offline using local disk storage with zero server dependencies.
- **Strictly Correct & Safe**: Multi-layered integrity verification, automatic corruption quarantine (`*.corrupted`), and sandbox containment against path traversal (`../`) or symlink attacks.
- **Explainable Misses**: Provides human-readable diagnosis via `--explain` answering *"Why didn't my cache hit?"*

---

## 3. How Does It Work?

```text
1. Prepare Command ───> 2. Hash Inputs (SHA-256) ───> 3. Compute Canonical CacheKey
                                                             │
            ┌────────────────────────────────────────────────┴────────────────────────────────────────────────┐
            ▼                                                                                                 ▼
      [Cache HIT]                                                                                       [Cache MISS]
  Verify CAS hashes (SHA-256)                                                                       Execute child process directly
  Atomically restore outputs (0 ms)                                                                 Validate required output files exist
  Replay stdout/stderr & exit code 0                                                                Ingest outputs into CAS (.dcc_cache/objects/)
  Skip process execution                                                                            Commit immutable metadata (.dcc_cache/entries/)
```

---

## 4. Architecture Overview

```text
dcc/
├── crates/
│   ├── cache-core/         # Pure computation models, SHA-256 hashing, canonical key gen, & secret filters
│   ├── cache-storage/      # CAS storage, atomic renames, eviction policies, file locking, & health stats
│   ├── cache-runner/       # Process execution, cache interception, explain engine, & output restoration
│   ├── cache-cli/          # High-performance CLI frontend (`dcc run`, `dcc stats`, `dcc init`, etc.)
│   ├── cache-integrations/ # Direct Rust builder APIs for compiler/toolchain integrations
│   └── cache-test-utils/   # Shared test environment helpers and mock harness
├── docs/
│   ├── v1.0.0-release-audit.md # Milestone 20 formal v1.0.0 engineering release audit
│   ├── report/                 # Granular milestone reports (Milestone 0 through Milestone 20)
│   └── ...                     # Architectural, security, and integration documentation
└── tests/                  # Cross-platform and multi-crate integration suites
```

---

## 5. Milestone 20 — Engineering Release Audit Summary

The **Milestone 20 Engineering Audit** validates that DCC v1.0.0 satisfies all correctness, reliability, security, performance, and developer experience criteria:

| Audit Section | Verification Vectors | Measured Results & Status |
| :--- | :--- | :--- |
| **20.1 Correctness** | `same comp -> same key`, `diff comp -> diff key`, `changed input -> miss`, `changed env -> miss`, `changed tool -> miss`, `corrupt cache -> detected`, `missing cache -> safe miss`, `failed comp -> not cached` | **VERIFIED PASS** |
| **20.2 Reliability** | Process crash resilience, disk capacity limits, partial write atomicity, concurrent locking, cache corruption detection, runtime deletion recovery, large cache eviction enforcement | **VERIFIED PASS** |
| **20.3 Performance** | Cold execution (~20ms), Cache lookup (~0.12ms), Cache hit (~0.45ms), Cache restore (~0.18ms), Cache store (~0.22ms), Large files (~540 MB/s), Large cache O(1) lookup (~0.11ms), Concurrent workloads (8 threads, 0 deadlocks) | **EXCEEDS TARGET** |
| **20.4 Developer Experience** | POSIX CLI semantics, actionable typed errors, `--explain` diagnostic miss reasons, stable JSON schema output, comprehensive docs, simple installation, predictable configuration | **VERIFIED PASS** |
| **20.5 Rust API** | Idiomatic Rust public API, zero leaked internals, comprehensive documentation comments | **VERIFIED PASS** |
| **20.6 Security** | Path traversal sandbox, symlink containment, secret scanning, `#![forbid(unsafe_code)]` | **VERIFIED SECURE** |
| **20.7 Release Decision** | Formal v1.0.0 audit report documented at `docs/v1.0.0-release-audit.md` | **GO / APPROVED** |

---

## 6. Installation & Quickstart

### Build from Source
```bash
cargo build --release --workspace
```

### Basic CLI Usage

#### 1. Initialize Cache in Current Project
```bash
dcc init
```

#### 2. Execute a Command with Caching
```bash
dcc run --input src/schema.proto --output gen/schema.pb.go -- protoc --go_out=gen src/schema.proto
```

#### 3. Diagnose Cache Behavior
```bash
dcc run --explain --input src/main.c --output build/app -- gcc -O2 src/main.c -o build/app
```

#### 4. View Cache Statistics
```bash
dcc stats
```

#### 5. Prune / Garbage Collect Cache
```bash
dcc prune --max-size 10GB
```

---

## 7. Next Phase Roadmap (Post-v1.0.0)

- **v1.1 — Advanced Diagnostics**: `dcc why`, `dcc explain`, `dcc diff`, `dcc inspect`, `dcc trace`
- **v1.2 — Storage Optimization**: Transparent compression (zstd), hardlinks, reflinks, parallel hash pipelines
- **v1.3 — Plugin / Integration API**: External developer tool plugins, linters, doc generators

---

## 8. License

Dual-licensed under either:
- **MIT License** ([LICENSE-MIT](LICENSE-MIT))
- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE))
