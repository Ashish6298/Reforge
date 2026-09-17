# Developer Computation Cache (`dcc`)

A high-performance, local-first, content-addressed developer computation caching engine written in pure Rust.

## Core Guarantee

> **A cache hit must never change the semantic result of the computation.**
> If the system cannot prove that a cached result is safe to reuse, it executes the computation again.

---

## Cache Entry Model (`CacheEntry`)

Metadata records never embed raw binary output data directly. Instead, entries reference CAS blobs via cryptographic digests:

```text
CacheEntry
├── key: CacheKey (SHA-256 computation digest)
├── schema_version: u32
├── created_at: DateTime<Utc>
├── last_accessed_at: DateTime<Utc>
├── hit_count: u64
├── computation: Computation (operation, command, args, inputs, env, tool, platform)
├── outputs: Vec<OutputManifestItem> (manifest mapping paths to CAS SHA-256 digests)
├── execution: ExecutionMetadata (exit code, duration, stdout_digest, stderr_digest)
└── integrity: Option<IntegrityInfo> (record verification checksum)
```

---

## Library-Level Cache API (`Cache`)

High-level library API decoupling callers (CLI, runners, build tools) from CAS internals:

```rust
use dcc_storage::{Cache, CasStorage, StorageConfig};

let storage = CasStorage::new(StorageConfig::default())?;
let cache = Cache::new(storage);

// Core Cache API operations
let entry_opt = cache.lookup(&key)?;
cache.store(&entry)?;
cache.restore(&entry, Path::new("./workspace"))?;
let was_deleted = cache.remove(&key)?;
let exists = cache.contains(&key);
cache.verify(&key)?;
```

---

## Physical Storage Abstraction (`Storage`)

Decouples *what* is cached from *where* and *how* physical bytes are stored:

```rust
use dcc_storage::{BlobMetadata, Storage};

pub trait Storage: Send + Sync {
    fn put(&self, bytes: &[u8]) -> Result<(Digest, u64)>;
    fn put_file(&self, source_path: &Path) -> Result<(Digest, u64)>;
    fn get(&self, digest: &Digest) -> Result<Box<dyn Read + Send>>;
    fn get_bytes(&self, digest: &Digest) -> Result<Vec<u8>>;
    fn exists(&self, digest: &Digest) -> bool;
    fn delete(&self, digest: &Digest) -> Result<bool>;
    fn metadata(&self, digest: &Digest) -> Result<Option<BlobMetadata>>;
    fn verify(&self, digest: &Digest) -> Result<()>;
}
```

The primary implementation is local filesystem content-addressed storage (`CasStorage`).

---

## Storage Directory Layout

To prevent scalability bottlenecks from storing millions of files in a single folder, DCC distributes objects and metadata entries using 2-character hex prefixes (256 shards):

```text
.dcc_cache/
├── objects/        # Content-Addressed Storage (CAS) for outputs/stdout/stderr
│   ├── ab/
│   │   └── ab34cdef...
│   └── 12/
│       └── 1298af7b...
├── entries/        # Computation metadata records (JSON)
│   ├── 01/
│   │   └── 01a4e2...json
│   └── 9f/
│       └── 9f5c88...json
├── metadata/       # General cache metadata and indices
├── index/          # Fast lookup indices
├── tmp/            # Atomic write staging directory
└── locks/          # Multi-process concurrency locks
```

---

## Atomic Writes & Crash Consistency

To guarantee that DCC never leaves partially written or corrupted artifacts in the live cache, all writes follow a strict two-stage atomic pipeline:

```text
temporary file (.dcc_cache/tmp/*.tmp)
          ↓
        write
          ↓
        flush
          ↓
   sync (fsync / sync_all)
          ↓
    atomic rename (.dcc_cache/objects/ab/...)
```

If an error or process interruption occurs during write or sync, temporary staging files are cleaned up and the live cache remains intact.

---

## Corruption Detection & Quarantining

DCC validates the cryptographic hash of every CAS object before reading or restoring:

- If `expected_digest != actual_digest`:
  - A structured `CacheError::IntegrityError` is generated.
  - The corrupted object is immediately isolated and renamed to `*.corrupted`.
  - The runner engine detects the miss/corruption and automatically falls back to re-executing the computation rather than returning invalid data.

---

## Storage Inspection APIs

The storage engine exposes dedicated inspection methods that power `dcc stats` and telemetry:

```rust
let stats = storage.stats()?;
let obj_count = storage.count_objects()?;
let entry_count = storage.count_entries()?;
let total_bytes = storage.total_size_bytes()?;
let (largest_size, largest_path) = storage.largest_object()?;
```

---

## Structured Command Model (`CommandSpec`)

Computations are specified through structured process definitions rather than unsafe shell string concatenation:

```rust
use dcc_runner::CommandSpec;

let spec = CommandSpec::builder("rustc")
    .arg("main.rs")
    .arg("--crate-type=bin")
    .current_dir("./workspace")
    .input_paths(vec!["src/main.rs", "src/lib.rs", "Cargo.toml"])
    .output_path("target/main.exe")
    .output_optional("target/main.pdb")
    .env("RUST_LOG", "debug")
    .build()?;
```

- **Declared Inputs**: Validated, streamed in 64 KB chunks, and cryptographically hashed (SHA-256) before cache lookup.
- **Declared Outputs**: Verified for physical existence on disk after command completion. If any required output is missing, an error (`CacheError::MissingOutput`) is returned and caching is aborted.

---

## Execution Lifecycle State Machine

The runner engine coordinates computation execution and cache reuse via an explicit, deterministic state machine:

```text
prepare
    ↓
collect inputs
    ↓
calculate key
    ↓
lookup cache
    ↓
HIT?
 ┌──┴───┐
YES    NO
 ↓      ↓
restore execute
        ↓
     validate outputs
        ↓
      store
```

1. **Prepare**: Validate `CommandSpec`, verify working directory, and prepare child process context.
2. **Collect Inputs**: Stream and compute SHA-256 digests for all declared input files.
3. **Calculate Key**: Compute canonical SHA-256 `CacheKey` incorporating executable, args, input hashes, env vars, and tool versions.
4. **Lookup Cache**: Check CAS metadata index for an existing entry matching the key.
5. **Branch HIT**: Restore output artifacts and captured stdout/stderr from CAS directly to workspace.
6. **Branch MISS**:
   - Acquire execution lock.
   - Execute child process directly (no shell concatenation).
   - **Validate Outputs**: Verify all declared required output files physically exist on disk.
   - **Store**: Ingest output files into CAS and write an immutable `CacheEntry` record atomically.

---

## Cache Hit Guarantees

When a cache key resolves to an existing valid entry, the engine executes a strict 6-step hit sequence:

1. **Retrieve Metadata**: Reads `CacheEntry` record from storage and updates access statistics.
2. **Verify Cache Integrity**: Verifies metadata identity (`verify_identity`) and ensures every referenced output CAS blob matches its SHA-256 hash.
3. **Restore Outputs**: Restores output files/directories atomically to their target workspace locations with verified bitstreams and Unix permissions.
4. **Restore Metadata**: Replays captured stdout/stderr streams and exit code from previous execution.
5. **Report HIT**: Emits structured hit telemetry and returns `ExecutionStatus::Hit` with `execution_time_ms = 0`.
6. **Bypass Execution**: The underlying command process is **never** executed.

---

## Cache Miss Lifecycle

When a computation cannot be resolved from the cache, the engine executes the strict 9-step miss sequence:

1. **Report Reason**: Captures and reports why the miss occurred (`NoEntryFound`, `InputChanged`, `ForcedRecompute`, `CorruptedCache`, etc.).
2. **Execute Command**: Spawns and executes the child process directly with specified args and environment.
3. **Capture Exit Code**: Records the integer exit code of the completed child process.
4. **Capture stdout/stderr**: Captures the complete byte buffers of standard output and standard error.
5. **Verify Outputs**: Checks that all declared required output files physically exist on disk.
6. **Hash Outputs**: Streams and hashes verified output files to compute their SHA-256 digests.
7. **Store Outputs**: Atomically ingests output files and non-empty stdout/stderr streams into CAS blobs.
8. **Store Metadata**: Constructs and atomically commits an immutable `CacheEntry` JSON record.
9. **Return Execution Result**: Returns `ExecutionResult` with status `Miss`, execution metadata, manifests, and captured logs.

---

## Failed Computations Policy

To maintain strict correctness, DCC **never caches failed computations by default**:

```text
exit code != 0  ──►  DO NOT STORE
```

- If a command process exits with a non-zero exit code, DCC captures the exit code, duration, stdout, and stderr, but skips storing CAS blobs or `CacheEntry` metadata records.
- Subsequent runs of the same failing computation will always re-execute rather than serving a cached failure.
- A configurable `FailurePolicy` (`FailurePolicy::DoNotCache` vs `FailurePolicy::CacheIfExplicit`) supports future selective failure caching while preserving conservative defaults in v1.

---

## Cache Correctness & Input Invalidation

A fast incorrect cache is worse than no cache. DCC strictly guarantees that any alteration to input state invalidates cached computations:

```text
input A = hash X  ──►  CacheKey 1
input A = hash Y  ──►  CacheKey 2  (Key 1 ≠ Key 2)
```

- **Content-Primary Hashing**: Input identity is computed via streaming SHA-256 digests of actual file bytes, invariant across file timestamps (`mtime`).
- **Path & Permission Sensitivity**: Relative input paths and executable permission bits form part of canonical identity.
- **Alphabetical Normalization**: Input manifest declarations are canonically sorted by path, making key derivation invariant to input specification order.
- **Miss Explanation**: When input changes occur, `MissExplainer` pinpoints changed paths along with prior and current digests (`MissReason::InputChanged`).

---

## Command & Argument Invalidation

Modifications to executable names, command-line arguments, argument ordering, or flags strictly alter computation keys:

```text
generator --fast  ──►  CacheKey A
generator --safe  ──►  CacheKey B  (Key A ≠ Key B)
```

- **Ordered Arguments**: Argument sequence is preserved verbatim (`["--opt", "--debug"]` ≠ `["--debug", "--opt"]`).
- **Option Flag Sensitivity**: Flag variations, additions, or omissions create distinct cache identities.
- **Independent Cache Isolation**: Computations with different commands or arguments never collide or cross-contaminate stored artifacts.
- **Miss Explanation**: `MissExplainer` identifies changed arguments (`MissReason::ArgumentsChanged`) or executables (`MissReason::CommandChanged`).

---

## Tool Identity & Version Invalidation

A computation using one tool or compiler version must not silently reuse results produced by another version:

```text
compiler 1.80  ──►  CacheKey A
compiler 1.81  ──►  CacheKey B  (Key A ≠ Key B)
```

Tool identity is modeled as a composite structure incorporated into canonical key derivation:

```text
ToolIdentity
├── name: String              # Logical tool identifier (e.g. "rustc", "gcc", "clang")
├── version: Option<String>   # Reported version (e.g. "1.80.0", "13.2.0")
└── digest: Option<Digest>    # SHA-256 binary hash of the executable file
```

- **Binary Digest Invalidation**: If the compiler binary on disk changes (e.g. rebuild or patch), the computed digest invalidates existing cache entries even if the reported version string remains identical.
- **Miss Explanation**: `MissExplainer` pinpoints tool and version mismatches via `MissReason::ToolChanged`.

---

## Declared Environment Invalidation

To avoid accidental cache fragmentation, DCC **never hashes the entire ambient process environment**. Instead, only explicitly declared environment variables are captured and hashed:

```text
CACHE_ENV / Declared Variables:
    NODE_ENV
    GENERATOR_VERSION
    FEATURE_MODE
```

- **Selective Isolation**: System variables like `USER`, `PWD`, `SSH_AUTH_SOCK`, and ephemeral tokens do not affect cache identity.
- **Declared Variable Sensitivity**: Mutating any declared variable (e.g. `FEATURE_MODE=0` vs `FEATURE_MODE=1`) alters the canonical cache key.
- **Lexicographical Normalization**: Declared environment pairs are stored in a `BTreeMap` and serialized in alphabetical order, guaranteeing canonical stability regardless of insertion sequence.
- **Miss Explanation**: `MissExplainer` pinpoints environment differences via `MissReason::EnvironmentChanged`.

---

## Platform Constraints & Invalidation

Platform-sensitive computations must capture meaningful platform dimensions without including unnecessary host machine metrics (which destroys cache reuse across developers and CI):

```text
Platform Dimensions:
    OS              (e.g., linux, windows, macos)
    Architecture    (e.g., x86_64, aarch64, arm)
    Target Triple   (e.g., x86_64-unknown-linux-musl vs x86_64-unknown-linux-gnu)
    Runtime         (e.g., node20, python3.11, jvm21)
    ABI             (e.g., glibc, musl, msvc)
    Compiler        (e.g., rustc 1.80.0, clang 17.0.6)
```

- **Targeted Discrimination**: Changing target triples or operating systems automatically invalidates cache entries and prevents binary mismatch errors.
- **Machine Neutrality**: Hostnames, process IDs, CPU core count, and local paths are excluded, preserving maximum cache sharing between workstations and CI workers.
- **Fluent Runner Integration**: Configure via `CommandSpecBuilder::platform`, `.platform_target(...)`, `.platform_runtime(...)`, or `.platform_abi(...)`.

---

## Explainable Cache Misses

Cache misses are never unexplained in DCC. When a computation produces a cache miss or recomputation, `MissExplainer` provides human-readable and structured diagnostic reasons:

```text
MISS: no cache entry exists

MISS: input changed
  src/parser.rs (was 3a8f..., now 9b2c...)

MISS: command arguments changed
  ["main.rs"] -> ["main.rs", "--release"]

MISS: tool identity changed
  tool version changed: Some("1.80.0") -> Some("1.81.0")

MISS: relevant environment changed
  OPTIMIZATION_LEVEL changed from "2" to "3"

MISS: platform changed
  Target triple changed: "x86_64-unknown-linux-gnu" -> "aarch64-unknown-linux-gnu"

MISS: cached output failed integrity verification
  CAS object 4a2b missing or integrity hash failed
```

- **CLI Flag**: Run with `dcc run --explain ...` to inspect the exact trigger for execution.
- **Machine Readability**: Included in JSON outputs (`"miss_reason": { ... }`) for tooling and CI telemetry.

---

## Correctness Test Matrix

DCC enforces an exhaustive 11-dimension correctness test matrix to guarantee that no code path can bypass cache integrity, identity isolation, or invalidation semantics:

| # | Correctness Dimension | Invariant & Verified Behavior |
|---|------------------------|-------------------------------|
| 1 | **Same inputs** | Cold run computes & stores; second run reports `Hit` (0 ms exec), skips process execution, and restores output files. |
| 2 | **Different input contents** | Altering file bytes modifies input digest, yields a different `CacheKey`, reports `Miss`, and computes/stores fresh outputs. |
| 3 | **Different paths** | Same file bytes at distinct paths derive distinct keys and prevent improper cross-path cache reuse. |
| 4 | **Different arguments** | Changing flags or arguments produces distinct computation keys and isolated cached entries. |
| 5 | **Different environment** | Changing declared environment variables alters keys without leaking or fragmenting ambient variables. |
| 6 | **Different tool version** | Differing compiler/tool versions or executable digests invalidate cache and produce isolated results. |
| 7 | **Different platform** | Target triple / OS / arch mismatches derive distinct keys and avoid runtime ABI issues. |
| 8 | **Missing output** | Commands failing to produce declared outputs trigger strict validation errors and reject storage. |
| 9 | **Modified cached output** | Tampered or bitrotted CAS blobs fail checksum verification, are quarantined to `.corrupted`, report `MissReason::CorruptedCache`, and recompute. |
| 10 | **Corrupted metadata** | Malformed or identity-mismatched entry JSON is detected, purged, reported as corrupted miss, and self-healed. |
| 11 | **Partial cache** | Entries with missing CAS object references fail restoration gracefully, report corrupted miss, recompute, and repair storage. |

---

## Workspace Architecture

- **[`crates/cache-core`](crates/cache-core)**: Core domain models (`Digest`, `CacheKey`, `Computation`, `CacheEntry`, `StructuredEvent`), streaming hashing, and canonical key derivation.
- **[`crates/cache-storage`](crates/cache-storage)**: Content-Addressed Storage (CAS) with 2-char hex prefix sharding, two-stage atomic writes (`.tmp` $\rightarrow$ `fsync` $\rightarrow$ rename), checksum verification, corrupted object isolation, LRU eviction, and `fs2` multi-process locking.
- **[`crates/cache-runner`](crates/cache-runner)**: Direct OS process execution, sandboxed output restoration with path-traversal protection, and structured miss explainer.
- **[`crates/cache-cli`](crates/cache-cli)**: CLI binary (`dcc`) supporting `init`, `run`, `inspect`, `stats`, `verify`, `clean`, `prune`, and `doctor`.
- **[`crates/cache-integrations`](crates/cache-integrations)**: Developer adapters for code generators, build systems, and tools.
- **[`crates/cache-test-utils`](crates/cache-test-utils)**: Test harnesses, synthetic workspace generators, and failure injectors.

---

## Documentation

- [Project Invariants](project/invariants.md)
- [System Architecture](docs/architecture.md)
- [Computation Model](docs/computation-model.md)
- [Cache Identity & Keys](docs/cache-keys.md)
- [Cache Correctness & Guarantees](docs/cache-correctness.md)
- [Storage Model & CAS](docs/storage-model.md)
- [Security Model](docs/security-model.md)
- [Scope & Non-Goals](docs/non-goals.md)
- [Dependency Policy](docs/dependency-policy.md)
- [Structured Logging](docs/logging.md)

---

## CLI Usage

```bash
# Initialize local cache directory
dcc init

# Execute a computation with caching
dcc run --input src/schema.json --output generated/models.rs -- generator src/schema.json

# Explain cache miss reasons
dcc run --explain --input src/schema.json --output generated/models.rs -- generator src/schema.json

# View cache storage statistics
dcc stats

# Inspect a specific computation by key
dcc inspect <key>

# Verify storage integrity
dcc verify

# Run health diagnostics
dcc doctor

# Prune unreferenced objects and enforce max size
dcc prune --max-size 10737418240
```

---

## Quality Gates & Verification

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
cargo fmt --all -- --check
```

All 6 core exit criteria (deterministic computation modeling, canonical key generation, cache entry creation, retrieval, identity verification, and corrupted metadata detection) and all 11 physical storage scenarios (empty cache, single object, deduplication, corruption quarantine, interrupted write isolation, deletion, concurrent read/write races, deeply nested paths, multi-MB large files, and binary byte safety) are fully verified and tested.
