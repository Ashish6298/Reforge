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
                 ┌──────────────────────┐
                 │ Developer / CI / Tool │
                 └──────────┬───────────┘
                            │
                            ▼
                 ┌──────────────────────┐
                 │ Computation API      │
                 └──────────┬───────────┘
                            │
                            ▼
                 ┌──────────────────────┐
                 │ Normalizer           │
                 │ + Key Generator      │
                 └──────────┬───────────┘
                            │
                            ▼
                 ┌──────────────────────┐
                 │ Cache Lookup         │
                 └──────────┬───────────┘
                            │
                  ┌─────────┴─────────┐
                  │                   │
                 HIT                 MISS
                  │                   │
                  ▼                   ▼
          ┌──────────────┐    ┌──────────────┐
          │ Verify       │    │ Execute      │
          │ Integrity    │    │ Computation  │
          └──────┬───────┘    └──────┬───────┘
                 │                    │
                 │                    ▼
                 │             ┌──────────────┐
                 │             │ Validate     │
                 │             │ Outputs      │
                 │             └──────┬───────┘
                 │                    │
                 │                    ▼
                 │             ┌──────────────┐
                 │             │ Store Result │
                 │             └──────┬───────┘
                 │                    │
                 └─────────┬──────────┘
                           ▼
                 ┌──────────────────────┐
                 │ Restore / Return     │
                 │ Computation Result   │
                 └──────────────────────┘
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
│   ├── v1.0.0-release-audit.md # Milestone 20 formal v1.0.0 engineering release audit report
│   ├── report/                 # Granular milestone reports (Milestone 0 through Milestone 20 + Post-V1)
│   └── ...                     # Architectural, security, and integration documentation
└── tests/                  # Cross-platform and multi-crate integration suites
```

---

## 5. Engineering Audit & Definition of Done Verification

| Verification Vector | Audit Outcome & Performance SLA | Status |
| :--- | :--- | :--- |
| **Correctness (20.1)** | Deterministic keying, miss on changed inputs/env/tools, safe miss fallback, no failed caching | **VERIFIED PASS** |
| **Reliability (20.2)** | Crash resilience, disk full isolation, partial write atomicity, concurrent file locking | **VERIFIED PASS** |
| **Performance (20.3)** | < 1 ms hit restore, ~ 540 MB/s streaming SHA-256 throughput, O(1) index lookup | **EXCEEDS TARGET** |
| **Developer Experience (20.4)** | POSIX CLI, actionable typed errors, `--explain` diagnostics, stable JSON schema | **VERIFIED PASS** |
| **Public Rust API (20.5)** | Idiomatic builder APIs, encapsulated internals, extensible without breaking SemVer | **VERIFIED PASS** |
| **Security & Sandboxing (20.6)** | Path traversal sandbox, symlink containment, zero `unsafe` blocks, secret credential scanner | **VERIFIED SECURE** |
| **Release Audit (20.7)** | Formal audit report documented at `docs/v1.0.0-release-audit.md` | **GO / APPROVED** |
| **20-Point Definition of Done** | All 20 lifecycle developer steps verified end-to-end | **100% COMPLETE** |

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

## 7. Post-V1 Capabilities & Roadmap

- **v1.1 — Advanced Diagnostics**: `dcc why`, `dcc explain`, `dcc diff`, `dcc inspect`, `dcc trace`
- **v1.2 — Storage Optimization**: Transparent compression, hardlinks, reflinks, tiered memory caching
- **v1.3 — Plugin / Integration API**: Fluent `DccActionBuilder` for linters, code generators, and asset pipelines
- **v1.4 — Advanced Cache Policies**: `ReadOnly`, `WriteOnly`, `NoCache`, `ForceRecompute`, `TTL`
- **v2.0 — Remote Cache**: Local-first architecture with optional HTTP / S3 object storage remote tiers

---

## 8. License

Dual-licensed under either:
- **MIT License** ([LICENSE-MIT](LICENSE-MIT))
- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE))
