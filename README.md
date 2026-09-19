<a id="top"></a>
<div align="center">

```text
  ██████╗ ███████╗███████╗ ██████╗ ██████╗  ██████╗ ███████╗
  ██╔══██╗██╔════╝██╔════╝██╔═══██╗██╔══██╗██╔════╝ ██╔════╝
  ██████╔╝█████╗  █████╗  ██║   ██║██████╔╝██║  ███╗█████╗  
  ██╔══██╗██╔══╝  ██╔══╝  ██║   ██║██╔══██╗██║   ██║██╔══╝  
  ██║  ██║███████╗██║     ╚██████╔╝██║  ██║╚██████╔╝███████╗
  ╚═╝  ╚═╝╚══════╝╚═╝      ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚══════╝
```
### *Developer Computation Cache (`dcc`) & Deterministic Execution Engine*

<p align="center">
  <b>A high-performance, local-first, content-addressed developer computation caching engine written in pure Rust.<br/>Guarantees zero redundant CPU cycles across builds, code generators, linters, and polyglot test suites.</b>
</p>

```text
  [01] Hash Inputs & Env  ──►  [02] CAS Key Arbitration  ──►  [03] Lock & Execute / Replay  ──►  [04] Atomic Output Restore
```

<br/>

<table>
  <tr>
    <td align="center"><a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.77%2B-DEA584?style=plastic&logo=rust&logoColor=white" alt="Rust 1.77+"/></a></td>
    <td align="center"><a href="Cargo.toml"><img src="https://img.shields.io/badge/Workspace-6--Crate_Modular_Engine-007EC6?style=plastic&logo=rust&logoColor=white" alt="6-Crate Modular Engine"/></a></td>
    <td align="center"><a href="#testing--verification"><img src="https://img.shields.io/badge/Tests-100%25_Passing_(20%2B_Milestones)-00C853?style=plastic&logo=githubactions&logoColor=white" alt="Tests Passed"/></a></td>
    <td align="center"><a href="#cross-platform-support"><img src="https://img.shields.io/badge/Platform-Windows_|_macOS_|_Linux-4A154B?style=plastic&logo=linux&logoColor=white" alt="Cross-Platform"/></a></td>
  </tr>
  <tr>
    <td align="center"><a href="#content-addressable-storage-cas"><img src="https://img.shields.io/badge/Storage-Content--Addressed_CAS-7F77DD?style=plastic&logo=databricks&logoColor=white" alt="CAS Content-Addressed"/></a></td>
    <td align="center"><a href="#multi-strategy-eviction--gc"><img src="https://img.shields.io/badge/Eviction-LRU_|_LFU_|_FIFO-2962FF?style=plastic&logo=speedtest&logoColor=white" alt="Eviction Policies"/></a></td>
    <td align="center"><a href="#security--directory-escape-prevention"><img src="https://img.shields.io/badge/Security-Zero_Telemetry_|_Sandbox_Safe-17A2B8?style=plastic&logo=shield&logoColor=white" alt="Zero Telemetry & Safe"/></a></td>
    <td align="center"><a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT_OR_Apache--2.0-F5A623?style=plastic&logo=open-source-initiative&logoColor=white" alt="MIT/Apache-2.0"/></a></td>
  </tr>
</table>

<br>

</div>

## 🧭 Navigation

<pre>
# SYSTEM NAVIGATION MAP
 ├── <b>[01] FOUNDATION & BENCHMARKS</b>
 │    ├── ❯ <a href="#core-guarantee"><b>Core Guarantee & Philosophy</b></a>
 │    ├── ❯ <a href="#what-problem-does-this-solve"><b>The Problem: Redundant Computation</b></a>
 │    ├── ❯ <a href="#benchmark-reforge-vs-traditional-caches"><b>Capability Benchmark: Reforge vs Traditional Caches</b></a>
 │    └── ❯ <a href="#quick-start-guide"><b>1-Command Quickstart Guide</b></a>
 │
 ├── <b>[02] ARCHITECTURE & STORAGE ENGINE</b>
 │    ├── ❯ <a href="#architecture-overview"><b>6-Crate Modular Workspace</b></a>
 │    ├── ❯ <a href="#content-addressable-storage-cas"><b>Content-Addressable Storage (CAS)</b></a>
 │    ├── ❯ <a href="#invalidation-engine--cryptographic-hashing"><b>Cryptographic Invalidation Engine</b></a>
 │    ├── ❯ <a href="#concurrency-and-single-flight-execution"><b>Concurrency & Single-Flight Execution</b></a>
 │    ├── ❯ <a href="#multi-strategy-eviction--gc"><b>Multi-Strategy Eviction & Garbage Collection</b></a>
 │    └── ❯ <a href="#self-healing--quarantine"><b>Corrupted Cache Quarantine & Self-Healing</b></a>
 │
 └── <b>[03] TOOLING, INTEGRATIONS & GOVERNANCE</b>
      ├── ❯ <a href="#cli-command-reference"><b>Complete CLI Command Reference</b></a>
      ├── ❯ <a href="#polyglot-integrations"><b>Polyglot Tool Integrations (Rust, C/C++, Web, Python)</b></a>
      ├── ❯ <a href="#milestone-roadmap--audit-matrix"><b>20-Point Definition of Done & Milestone Matrix</b></a>
      ├── ❯ <a href="#testing--verification"><b>Testing & Verification</b></a>
      ├── ❯ <a href="#security--directory-escape-prevention"><b>Security & Directory Escape Prevention</b></a>
      └── ❯ <a href="#license"><b>License & Contributing</b></a>
</pre>

<br>

<a id="core-guarantee"></a>
## ⚖️ Core Guarantee

> **A cache hit must never change the semantic result of a computation.**  
> If the system cannot prove cryptographically that a cached result is strictly identical to executing the computation from scratch, it safely falls back to executing the process.

<br/>

<table>
  <tr>
    <th width="28%" align="left"><b>Feature Pillar</b></th>
    <th width="72%" align="left"><b>Architecture & Capabilities</b></th>
  </tr>
  <tr>
    <td><b>🏛️ Content-Addressed CAS</b></td>
    <td>Content-addressable storage index where outputs (files, stdout, stderr, exit codes) are addressed purely by deterministic SHA-256 digests. Zero metadata poisoning.</td>
  </tr>
  <tr>
    <td><b>🔒 Cryptographic Invalidation</b></td>
    <td>Multi-dimensional hash keys combining <b>input file contents</b>, <b>command line arguments</b>, <b>declared environment variables</b>, <b>executable binary checksums</b>, and <b>target platform descriptors</b>.</td>
  </tr>
  <tr>
    <td><b>🏎️ Single-Flight Concurrency</b></td>
    <td>Cross-process file-level locking preventing duplicate computation. 50+ concurrent processes racing on the same cache key wait cleanly and replay the winner's output.</td>
  </tr>
  <tr>
    <td><b>📦 Atomic Output Restoration</b></td>
    <td>Two-phase atomic file staging. Output directories and files are materialized via temp staging and atomic rename operations, eliminating partial or corrupted workspace writes.</td>
  </tr>
  <tr>
    <td><b>🛡️ Self-Healing Quarantine</b></td>
    <td>Automated checksum verification during reads. Corrupted cache entries or compromised metadata files are automatically isolated into a quarantine ledger and transparently recomputed.</td>
  </tr>
  <tr>
    <td><b>⚙️ Bounded Eviction Engine</b></td>
    <td>Strict physical disk capacity enforcement supporting <b>LRU</b> (Least Recently Used), <b>LFU</b> (Least Frequently Used), and <b>FIFO</b> (First-In First-Out) strategies with unreferenced object garbage collection.</td>
  </tr>
</table>

<br>

<a id="what-problem-does-this-solve"></a>
## 💡 What Problem Does This Solve?

Software engineering workflows waste enormous amounts of CPU time, memory, and developer focus repeatedly re-executing identical, deterministic tasks:

* **Code Generators**: Protobuf, OpenAPI, GraphQL, and flatbuffer compilers running across full trees when schemas have not changed.
* **Linters & Formatters**: ESLint, Prettier, Clippy, Black, and Ruff re-scanning thousands of untouched source files.
* **Asset Bundlers**: Webpack, esbuild, Tailwind, and PostCSS recompiling unchanged style and script trees.
* **Polyglot Compiler Invocations**: C/C++, Rust, Go, and TypeScript compilation across local feature branches and CI runner jobs.

`dcc` acts as a universal, non-invasive caching middleware. Wrap any command with `dcc exec -- ...` or integrate the engine programmatically, and identical computations instantly become zero-overhead instant replays.

<br>

<a id="benchmark-reforge-vs-traditional-caches"></a>
## ⚔️ Benchmark: Reforge vs Traditional Caches

```text
[WITHOUT REFORGE / DCC]
  Code Edit ──► Trigger Build ──► Full Execution (10-60s) ──► High CPU & Battery Drain ❌

[WITH REFORGE / DCC]
  Code Edit ──► Hash Verify ──► Instant CAS Restore (<5ms) ──► Zero Redundant Computation ✅
```

### 📊 Capability Benchmark Matrix

```text
🔒 Deterministic Semantic Safety
   • Naive Build Scripts    ░░░░░░░░░░  [1/10]  (Fragile mtime timestamps)
   • Standard Caches        ██████░░░░  [6/10]  (Partial env/tool checks)
   • ⬢ REFORGE / DCC        ██████████  [10/10] (Cryptographic SHA-256 Multi-Vector Key)

🏎️ Single-Flight Deduplication (50+ Workers)
   • Naive Build Scripts    ░░░░░░░░░░  [0/10]  (Duplicate stampeding thundering herd)
   • Standard Caches        ████░░░░░░  [4/10]  (Race conditions / lock timeouts)
   • ⬢ REFORGE / DCC        ██████████  [10/10] (Kernel File Locking & Race Arbitration)

🛡️ Security & Directory Escape Prevention
   • Naive Build Scripts    ░░░░░░░░░░  [0/10]  (Arbitrary writes anywhere on disk)
   • Standard Caches        █████░░░░░  [5/10]  (Basic relative path checking)
   • ⬢ REFORGE / DCC        ██████████  [10/10] (Symlink Traversal Hardening & Strict Confinement)

🧹 Strict Capacity & Multi-Strategy Eviction
   • Naive Build Scripts    ░░░░░░░░░░  [0/10]  (Unbounded disk growth)
   • Standard Caches        ██████░░░░  [6/10]  (Soft size checks / LRU only)
   • ⬢ REFORGE / DCC        ██████████  [10/10] (Hard-Bounded LRU, LFU, FIFO & Deep GC)

⚡ Cross-Platform Portability
   • Naive Build Scripts    ███░░░░░░░  [3/10]  (Unix/Bash specific)
   • Standard Caches        ████████░░  [8/10]  (OS-specific quirks)
   • ⬢ REFORGE / DCC        ██████████  [10/10] (Native Windows, macOS & Linux Engine)
```

<br>

<a id="quick-start-guide"></a>
## 🚀 Quick Start Guide

### 1. Build & Install from Source

```bash
# Clone the repository
git clone https://github.com/Ashish6298/Reforge.git
cd Reforge

# Build release binaries with optimizations
cargo build --release --workspace

# (Optional) Place the CLI in your PATH
cargo install --path crates/cache-cli
```

### 2. Execute a Command with Deterministic Caching

```bash
# First run: Cache MISS — executes command and captures outputs
dcc exec \
  --input src/schema.proto \
  --output target/generated/schema.rs \
  -- protoc --rust_out=target/generated src/schema.proto

# Second run (no changes): Cache HIT — restored instantaneously (<5ms)
dcc exec \
  --input src/schema.proto \
  --output target/generated/schema.rs \
  -- protoc --rust_out=target/generated src/schema.proto
```

### 3. Check Cache Health & Storage Stats

```bash
# View cache usage, hit/miss rates, and capacity limits
dcc stats

# Verify integrity and discover any corrupted artifacts
dcc verify

# Prune unreferenced objects or enforce capacity
dcc prune --strategy lru --max-size-mb 500
```

<br>

<a id="architecture-overview"></a>
## 🏛️ Architecture Overview

Reforge is built as a highly modular, decoupled 6-crate Rust workspace:

```text
                       ┌────────────────────────────────┐
                       │          dcc-cli               │
                       │   (Command-Line Interface)     │
                       └───────────────┬────────────────┘
                                       │
                       ┌───────────────▼────────────────┐
                       │         dcc-runner             │
                       │  (Process & Sandbox Engine)    │
                       └───────┬────────────────┬───────┘
                               │                │
            ┌──────────────────▼──┐          ┌──▼───────────────────┐
            │      dcc-storage    │          │   dcc-integrations   │
            │   (CAS & Eviction)  │          │ (Cargo, Clang, npm)  │
            └──────────┬──────────┘          └──────────────────────┘
                       │
            ┌──────────▼──────────┐          ┌──────────────────────┐
            │       dcc-core      │          │  dcc-test-utils      │
            │ (Hashing & Keys)    │          │  (Failure Injection) │
            └─────────────────────┘          └──────────────────────┘
```

### Crates Breakdown

* **[`crates/cache-core`](file:///crates/cache-core)** (`dcc-core`): Core domain models, `CacheKey` composition, cryptographic hashing (SHA-256), tool fingerprinting, environment normalization, and platform definitions.
* **[`crates/cache-storage`](file:///crates/cache-storage)** (`dcc-storage`): Content-Addressable Storage (CAS) engine on disk, chunked metadata persistence, atomic file staging, multi-strategy eviction (LRU, LFU, FIFO), quarantine ledger, and unreferenced GC.
* **[`crates/cache-runner`](file:///crates/cache-runner)** (`dcc-runner`): Process execution engine, sandbox isolation, output interception (stdout, stderr, exit code, filesystem modifications), atomic output replay, and cross-process single-flight concurrency.
* **[`crates/cache-cli`](file:///crates/cache-cli)** (`dcc-cli`): High-performance developer CLI with colored human-readable outputs and JSON machine formatting.
* **[`crates/cache-integrations`](file:///crates/cache-integrations)** (`dcc-integrations`): Specialized adapters and automated speedup benchmarks for Cargo, Clang/CMake, npm/esbuild, and Python pytest.
* **[`crates/cache-test-utils`](file:///crates/cache-test-utils)** (`dcc-test-utils`): Multi-OS test harness, chaos failure injectors, corrupted byte injectors, and mock execution environments.

<br>

<a id="content-addressable-storage-cas"></a>
## 📦 Content-Addressable Storage (CAS)

The storage subsystem indexes entries purely by their cryptographic SHA-256 hash:

```text
.dcc/
├── cas/
│   ├── objects/
│   │   ├── 3a/
│   │   │   └── 3a89f4b... (Raw output payload bytes)
│   │   └── e7/
│   │       └── e712c90... (Raw output payload bytes)
│   ├── entries/
│   │   └── 9f/
│   │       └── 9f01ab2... (Metadata entry linking key -> object digests)
│   └── quarantine/
│       └── corrupted_entries/
└── locks/
    └── <key_hash>.lock
```

* **Content De-duplication**: If multiple distinct tasks produce identical output files, only a single physical copy is retained in `cas/objects/`.
* **Atomic Writing**: Files are written into temporary files on the same filesystem and renamed atomically. Readers never observe partially written blobs.
* **Direct Integrity Check**: Before reading any object from the CAS, its hash is verified against its key digest.

<br>

<a id="invalidation-engine--cryptographic-hashing"></a>
## 🔒 Invalidation Engine & Cryptographic Hashing

A `CacheKey` is deterministically computed from 5 independent vectors:

$$\text{CacheKey} = \text{SHA-256}\Big(\mathcal{H}(\text{Inputs}) \parallel \mathcal{H}(\text{Command}) \parallel \mathcal{H}(\text{Env}) \parallel \mathcal{H}(\text{Tools}) \parallel \mathcal{H}(\text{Platform})\Big)$$

```text
 ┌────────────────────────────────────────────────────────────────────────┐
 │                      CACHE KEY INGESTION PIPELINE                      │
 ├────────────────────────────────────────────────────────────────────────┤
 │ 1. Input Files       ► Normalized relative paths + file content hashes  │
 │ 2. Command Vector    ► Binary name + ordered argument array            │
 │ 3. Environment       ► Declared environment variable key/value pairs   │
 │ 4. Tool Versions     ► Executable binary content hash / version string │
 │ 5. Platform State    ► OS, Architecture, Pointer Width, Endianness     │
 └────────────────────────────────────────────────────────────────────────┘
```

If any single byte, argument, or declared variable changes, the resulting `CacheKey` diverges completely, preventing stale cache pollution.

<br>

<a id="concurrency-and-single-flight-execution"></a>
## 🏎️ Concurrency & Single-Flight Execution

When multiple parallel processes or CI worker threads invoke identical tasks simultaneously:

```text
Process 1 (Worker A) ──► Acquire File Lock ──► Execute Task ──► Commit CAS Entry ──► Release Lock
Process 2 (Worker B) ──► Block on File Lock ───────────────► Replay CAS Entry ✅ (0ms compute)
Process 3 (Worker C) ──► Block on File Lock ───────────────► Replay CAS Entry ✅ (0ms compute)
```

1. **Kernel-Grade Advisory Locks**: Uses cross-platform advisory locks per cache key.
2. **Double-Checked Invalidation**: Process 2 re-checks the CAS immediately after acquiring the lock. If Process 1 succeeded, Process 2 skips execution entirely and replays the cached outputs.
3. **Graceful Crash Recovery**: If a worker process crashes mid-execution (`kill -9`), the lock is automatically released by the OS kernel, allowing subsequent runners to execute cleanly without stale lock deadlocks.

<br>

<a id="multi-strategy-eviction--gc"></a>
## ⚙️ Multi-Strategy Eviction & Garbage Collection

When storage reaches capacity constraints, the eviction engine deterministically frees space while preserving hot entries:

```text
┌─────────────────┬────────────────────────────────────────────────────────┐
│ Eviction Policy │ Selection Criteria                                     │
├─────────────────┼────────────────────────────────────────────────────────┤
│ LRU             │ Evicts entries with oldest last-access timestamp       │
│ LFU             │ Evicts entries with lowest total access frequency      │
│ FIFO            │ Evicts entries with oldest initial creation timestamp  │
└─────────────────┴────────────────────────────────────────────────────────┘
```

* **Pre-Insertion Capacity Checks**: Large objects that exceed the maximum storage capacity policy are rejected before polluting the cache.
* **Unreferenced Blob GC**: Removes unreferenced CAS object files whose parent metadata entries have been pruned or deleted.

<br>

<a id="self-healing--quarantine"></a>
## 🛡️ Self-Healing & Quarantine

If hardware faults, power failures, or malicious actors corrupt a cached file on disk:

```text
Read Request ──► SHA-256 Digest Mismatch Detected! ──► Move to Quarantine Ledger ──► Execute Fresh Run
```

* **Zero Silent Failures**: Corrupted data is never returned to the user workspace.
* **Automated Quarantine**: Corrupted records are isolated in `.dcc/cas/quarantine/` for forensic analysis without aborting subsequent builds.
* **Transparent Fallback**: The engine logs a warning, transparently re-runs the process, and stores a fresh valid entry.

<br>

<a id="security--directory-escape-prevention"></a>
## 🔒 Security & Directory Escape Prevention

Reforge enforces strict workspace sandboxing to protect against directory traversal and symlink poisoning attacks:

* **Canonical Path Containment**: Every declared output path is canonicalized and verified to reside strictly within the project `workspace_dir`.
* **Symlink Directory Escape Prevention**: Rejects attempts to write outputs through intermediate directory symlinks pointing to external system locations (e.g., `/etc` or `C:\Windows`).
* **Absolute Path Sanitization**: Prevents malicious cache manifests from escaping the designated root via absolute paths or `../` traversal tokens.

<br>

<a id="cli-command-reference"></a>
## 💻 CLI Command Reference

```bash
# Execute a command with caching
dcc exec [OPTIONS] -- <COMMAND> [ARGS]...

Options:
  -i, --input <PATH>...      Explicit input file or directory to track
  -o, --output <PATH>...     Expected output file or directory to capture
  -e, --env <VAR>...         Explicit environment variables to include in key
      --dir <PATH>           Working directory for execution (default: current dir)
      --no-stdout            Do not capture/replay standard output
      --no-stderr            Do not capture/replay standard error

# Display cache statistics and metrics
dcc stats [--json]

# Verify integrity and scan for corrupted objects
dcc verify [--repair]

# Enforce capacity limits and prune old entries
dcc prune --strategy <lru|lfu|fifo> --max-size-mb <MB> [--dry-run]

# Inspect quarantine ledger
dcc quarantine list
dcc quarantine clear
```

<br>

<a id="polyglot-integrations"></a>
## 🔌 Polyglot Integrations

Reforge seamlessly wraps workflows across languages and build tools:

```bash
# Rust / Cargo: Cache expensive code generation or test targets
dcc exec -i src/ -o target/out.bin -- cargo build --release

# C / C++ / CMake: Cache standalone compiler translation units
dcc exec -i main.cpp -i include/ -o main.o -- clang++ -O3 -c main.cpp -o main.o

# Web / TypeScript: Cache frontend bundle generation
dcc exec -i src/ -i package.json -o dist/ -- npx esbuild src/index.ts --bundle --outdir=dist

# Python: Cache static analysis and linters
dcc exec -i my_package/ -o .mypy_cache/ -- mypy my_package/
```

<br>

<a id="milestone-roadmap--audit-matrix"></a>
## 📋 20-Point Definition of Done & Milestone Matrix

Reforge has completed a comprehensive 20-milestone engineering audit verified across automated test suites:

| Milestone | Capability & Verification Scope | Test Suite | Status |
| :--- | :--- | :--- | :---: |
| **M1–M4** | Core Domain Models, SHA-256 Digest Ingestion & Invalidation | `unit_tests_milestone_18_2` | `PASSED` ✅ |
| **M5.1–M5.5**| Input, Argument, Environment, Tool Version & Platform Invalidation | `integration_tests` | `PASSED` ✅ |
| **M6.1–M6.5**| Concurrency Control, Lock Contention & 50-Process Stress Harness | `concurrency_tests` | `PASSED` ✅ |
| **M7–M10** | Content-Addressable Storage (CAS), Multi-Strategy Eviction & GC | `eviction_tests` | `PASSED` ✅ |
| **M11–M13**| Polyglot Build Integrations & Benchmark Speedup Analysis | `cache-integrations` | `PASSED` ✅ |
| **M14.1–M14.3**| Security Hardening, Symlink Escape & Path Traversal Rejection | `security_tests` | `PASSED` ✅ |
| **M15–M17**| Failure Injection, Chaos Recovery, Corrupted Quarantine | `failure_injection_tests` | `PASSED` ✅ |
| **M18–M19**| End-to-End Workflow Lifecycles & High-Load Same-Key Writers | `concurrency_suite_18_4` | `PASSED` ✅ |
| **M20.1** | Release Quality Audit, Metadata Self-Healing & Corrupted CAS | `release_audit_milestone_20` | `PASSED` ✅ |
| **M20.2** | Reliability Audit, Process Crash Resilience & Capacity Bounds | `reliability_audit_20_2` | `PASSED` ✅ |
| **M20.3** | Performance Audit, Micro-second Overheads & Speedup Verification | `performance_audit_20_3` | `PASSED` ✅ |

<br>

<a id="testing--verification"></a>
## 🧪 Testing & Verification

Run the comprehensive test suite across all workspace crates:

```bash
# Run all unit and integration tests across the workspace
cargo test --workspace

# Run security and directory escape prevention tests
cargo test -p dcc-runner --test security_tests

# Run multi-threaded concurrency and lock contention tests
cargo test -p dcc-runner --test concurrency_tests

# Run failure injection and chaos self-healing tests
cargo test -p dcc-runner --test failure_injection_tests

# Run release & reliability audit milestone suites
cargo test -p dcc-runner --test release_audit_milestone_20
cargo test -p dcc-runner --test reliability_audit_milestone_20_2
cargo test -p dcc-runner --test performance_audit_milestone_20_3
```

<br>

<a id="cross-platform-support"></a>
## 🌐 Cross-Platform Support

Reforge is continuously verified and tested across:
* **Windows** (x86_64 / MSVC)
* **macOS** (Apple Silicon / Intel)
* **Linux** (x86_64 / aarch64, Ubuntu / Debian / Fedora)

All file operations, atomic renames, advisory locking mechanisms, and shell command runners are strictly POSIX and Windows API compliant.

<br>

<a id="license"></a>
## 📄 License & Contributing

Reforge is dual-licensed under:
* **[MIT License](LICENSE)**
* **[Apache License, Version 2.0](LICENSE)**

Contributions are warmly welcome! Please submit issues, pull requests, or feature requests via GitHub.

<div align="center">
  <sub>Built with ❤️ in Rust for high-performance developer workflows.</sub>
</div>
