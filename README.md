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

## Concurrency & Locking

DCC is architected for multi-process concurrency, allowing parallel terminals, build workers, and CI jobs to access the shared cache safely and simultaneously:

```text
Terminal A -> computation X
Terminal B -> computation X
Terminal C -> computation Y
```

### Concurrent Reads (Milestone 6.1)

- **Immutable CAS Objects**: CAS objects are immutable once committed via atomic two-stage rename. Readers acquire shared read-only handles (`File::open`) without exclusive locks or contention.
- **Lock-Free Read Scaling**: Dozens of concurrent threads or processes can stream the same CAS object or inspect metadata simultaneously.
- **Concurrent Cache Hit Storms**: When multiple concurrent runner processes execute identical computations on a warm cache, 100% of workers experience immediate cache hits (`ExecutionStatus::Hit`), $0\text{ ms}$ execution duration, no child process spawning, and atomic output restoration into their respective workspaces.

### Concurrent Writes (Milestone 6.2)

- **Two-Stage Atomic Write Pipeline**: CAS objects and cache entries are written to unique nonce temporary files in `.cache/tmp/`, fully flushed and synced to disk via `fsync` (`sync_all`), and committed via atomic rename (`fs::rename`).
- **Deduplication Race Safety**: When two processes compute identical outputs simultaneously, both write temporary files and rename to the same CAS path. The destination object is guaranteed to be 100% intact and deduplicated to a single physical file.
- **Zero Partial Reads & Clean Tmp Invariant**: Other processes cannot observe partial or corrupted files because writes occur in `.tmp/` before the atomic rename. Competing temporary files are automatically purged.

### Duplicate Computation Avoidance (Milestone 6.3)

When multiple processes or threads simultaneously miss the same computation key, DCC coordinates execution using per-key computation locks (`ComputationLock`) to prevent redundant work:

```text
A -> MISS                    B -> MISS
      ↓                            ↓
A obtains lock               B waits on lock
A executes command                 :
A stores result to CAS             :
A releases lock                    ↓
                             B acquires lock
                             B re-checks cache
                             B receives HIT (0 ms)
```

- **Locking Mechanism**: Advisory file locking via `fs2` on `.cache/locks/<key>.lock` with RAII lock release and configurable timeouts.
- **Secondary Cache Re-check**: Upon acquiring the lock, `RunnerEngine` immediately re-checks the cache before executing the command.
- **Deduplicated Execution**: Guaranteed that only 1 process runs the command while all other waiting processes receive instant cache hits and restore output artifacts into their respective workspaces.

### Lock Failure Recovery (Milestone 6.4)

DCC guarantees that abnormal process terminations, crashes, timeouts, or corrupted metadata never leave the cache permanently unusable:

- **Kernel Lock Auto-Release**: File locks are backed by operating system kernel locks (`fs2::FileExt`). When a process crashes or is killed (`SIGKILL`), the OS kernel automatically releases held locks.
- **Stale Lock Recovery**: Orphaned lock files on disk from dead processes are claimed and overwritten by the next active process without blocking.
- **Corrupted Metadata Overwrite**: Lock files containing non-JSON or corrupted data are cleared and rewritten atomically with fresh PID and timestamp upon lock acquisition.
- **Configurable Timeouts**: If a lock is held beyond `lock_timeout`, `acquire` returns `CacheError::LockError` instead of deadlocking indefinitely.
- **Stale Lock Pruning**: `ComputationLock::clean_stale_locks` safely tests and purges unlocked lock files exceeding a configured age threshold.

### Concurrency Stress Testing (Milestone 6.5)

DCC is verified under heavy concurrent workloads across multiple scales:

- **10 Concurrent Processes**: Mixed workloads of shared clusters and unique tasks with zero lock contention deadlocks and clean deduplicated execution.
- **50 Concurrent Processes**: 50 simultaneous workers executing multi-artifact builds with secondary side-outputs and verified workspace file integrity.
- **100 Concurrent Operations**: 100 simultaneous operations dispatched via a synchronized barrier, validated with a complete post-stress storage audit (all CAS objects cryptographically verified, all entries satisfy `verify_identity()`, zero leaked `.tmp` files, and zero corrupted objects).

---

## Cache Lifecycle & Eviction (Milestone 7)

A cache that grows indefinitely is not production quality. DCC enforces lifecycle policies and disk usage limits to ensure predictable, bounded footprint.

### Cache Size Limits (`max_size`) (Milestone 7.1)

DCC provides first-class support for configuring cache size limits via the `ByteSize` type and human-readable string expressions:

- **Supported Formats**:
  * `500 MB`, `500MB`, `500MiB`
  * `2 GB`, `2GB`, `2 GiB`
  * `10 GB`, `10GB`, `10 GiB`
  * `1024 KB`, `4096 B`, raw byte counts (e.g. `10737418240`)
  * Fractional representations (e.g. `1.5 GB`, `0.5 MB`)
- **Storage Configuration**:
  * Default limit: `10 GB`
  * Programmatic builders: `StorageConfig::new(root).with_max_size_str("2 GB")?` or `with_max_size(ByteSize::gb(2))`
- **CLI Commands**:
  * `dcc init --max-size "2 GB"`
  * `dcc prune --max-size "500 MB"`

### Eviction Strategy (Milestone 7.2)

DCC avoids complex, brittle heuristics in favor of deterministic, understandable eviction policies:

- **LRU (Least Recently Used - Default)**: Evicts cache entries with the oldest `last_accessed_at` timestamp.
- **FIFO (First In, First Out)**: Evicts cache entries with the oldest `created_at` timestamp.
- **LFU (Least Frequently Used)**: Evicts cache entries with the lowest `hit_count`, tie-breaking on oldest access.
- **TTL / Expiration**: Purges stale entries older than a configurable duration (`evict_expired`).
- **Policy Enforcement**: `Pruner::enforce_policy(policy)` and `Pruner::evict_with_strategy(strategy, max_size)`.
- **CLI Commands**:
  * `dcc prune --strategy lru --max-size "2 GB"`
  * `dcc prune --strategy fifo --max-size "1 GB"`
  * `dcc prune --strategy lfu --max-size "500 MB"`

### Garbage Collection (Milestone 7.3)

Automatic and manual removal of CAS objects no longer referenced by valid cache entries:

- **Reachability Graph Analysis**: Traverses all entry shards and aggregates referenced output blobs and execution stream captures (`stdout`/`stderr`).
- **Orphan Identification**: Discovers all unreferenced physical files in `.dcc_cache/objects/`.
- **Safe Shared Blob Retention**: Protects deduplicated payloads referenced by other active cache entries.
- **Dry-Run Inspection**: Allows inspecting reclaimable files and byte counts before performing deletions (`dcc prune --dry-run`).
- **Convenience API**: `Pruner::prune_unreferenced_objects()`, `CasStorage::prune_unreferenced()`, `Cache::prune()`.
- **CLI Commands**:
  * `dcc prune`
  * `dcc prune --dry-run`
  * `dcc prune --dry-run --max-size "1 GB"`

### Manual Maintenance (Milestone 7.4)

Granular maintenance operations available via CLI and library APIs:

- **`cache clean` / `dcc clean`**: Wipe all cached data (`clean_all()`) or delete specific computation keys (`--key <key>`).
- **`cache prune` / `dcc prune`**: Garbage-collect unreferenced objects (`prune()`) and enforce size limits (`--max-size`) with dry-run inspection (`--dry-run`).
- **`cache verify` / `dcc verify`**: Complete cryptographic integrity audit (`verify_all()`) of all CAS objects and metadata records with automatic corruption quarantine.
- **`cache stats` / `dcc stats`**: Real-time storage telemetry (`stats()`), reporting total entries, object counts, disk usage, and largest object footprint.
- **CLI Commands**:
  * `dcc cache clean` or `dcc clean --key <key>`
  * `dcc cache prune --dry-run` or `dcc prune --max-size "2 GB"`
  * `dcc cache verify` or `dcc verify`
  * `dcc cache stats` or `dcc stats`

### Safe Deletion (Milestone 7.5)

To prevent race conditions and data corruption across concurrent processes, objects currently being read or used are **never deleted**:

- **Reader-Writer Coordination (`ObjectLock`)**:
  * Active consumers acquire shared read locks (`ObjectLock::acquire_shared`) when reading or streaming CAS objects.
  * Pruning, eviction, and blob deletion routines acquire exclusive deletion locks (`ObjectLock::try_acquire_exclusive` / `ObjectLock::acquire_exclusive`) before removing objects.
- **Non-Blocking Eviction Safety**:
  * During automated garbage collection and size eviction, unreferenced objects actively held by readers are safely skipped rather than torn down mid-stream (`delete_object_safe(&digest, None)`).
  * Once readers finish and drop their locks, subsequent GC passes cleanly purge orphaned objects.
- **Computation Entry Lock Coordination**:
  * Cache entry deletions (`delete_entry(&key)`) acquire exclusive `ComputationLock` before removing metadata records, preventing deletion of entries currently being computed or committed.
- **Programmatic APIs**:
  * `Cache::lock_object(&digest, timeout)`
  * `Cache::delete_blob(&digest)`
  * `CasStorage::delete_object_safe(&digest, timeout_opt)`

---

## Workspace Architecture

- **[`crates/cache-core`](crates/cache-core)**: Core domain models (`Digest`, `CacheKey`, `Computation`, `CacheEntry`, `StructuredEvent`, `ByteSize`), streaming hashing, and canonical key derivation.
- **[`crates/cache-storage`](crates/cache-storage)**: Content-Addressed Storage (CAS) with 2-char hex prefix sharding, two-stage atomic writes (`.tmp` $\rightarrow$ `fsync` $\rightarrow$ rename), checksum verification, corrupted object isolation, multi-strategy eviction (`Pruner` supporting LRU/FIFO/LFU), maintenance suite (`clean`, `prune`, `verify`, `stats`), and `fs2` multi-process locking.
- **[`crates/cache-runner`](crates/cache-runner)**: Direct OS process execution, sandboxed output restoration with path-traversal protection, and structured miss explainer.
- **[`crates/cache-cli`](crates/cache-cli)**: CLI binary (`dcc`) supporting `init`, `run`, `inspect`, `stats`, `verify`, `clean`, `prune`, `doctor`, and nested `cache` maintenance subcommands.
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

---

## Professional CLI (Milestone 8)

The Developer Computation Cache engine is exposed through a polished, robust command-line interface (`dcc`):

| Command | Purpose | Key Flags / Arguments |
| :--- | :--- | :--- |
| `dcc init` | Initialize local cache directory & configuration | `--max-size <limit>` |
| `dcc run` | Execute computation with sandboxed caching | `--input <f>`, `--output <f>`, `--env <k=v>`, `--explain`, `--policy <p>`, `-- <cmd...>` |
| `dcc inspect <key>` | Inspect detailed computation metadata and I/O manifests | `<key>`, `--json` |
| `dcc stats` | Inspect entry count, CAS objects, disk usage, and largest blob | `--json` |
| `dcc verify` | Cryptographically audit CAS objects and metadata integrity | `--json` |
| `dcc clean` | Wipe all cached data or remove specific computation key | `--key <key>`, `--json` |
| `dcc prune` | Garbage collect unreferenced objects and enforce max size | `--max-size <limit>`, `--strategy <lru/fifo/lfu>`, `--dry-run`, `--json` |
| `dcc config` | Inspect active storage configuration and path layout | `--get <max_size/cache_dir>`, `--json` |
| `dcc doctor` | Perform comprehensive environment and health diagnostics | `--json` |

### `dcc init` (Milestone 8.2)

Creates and validates the local cache environment:
- **Determine Default Cache Directory**: Automatically checks `DCC_CACHE_DIR`, system user home directories (`USERPROFILE` / `HOME`), and fallbacks.
- **Create Required Hierarchy**: Ensures `objects/`, `entries/`, `locks/`, and `tmp/` directories exist with sharded partitioning.
- **Create Local Configuration**: Persists active cache settings to `.dcc_cache/config.json`.
- **Validate Storage**: Performs an atomic probe to confirm filesystem writability and storage integrity.
- **Print Configuration Summary**: Outputs clear, human-readable layout summary or structured JSON (`--json`).

### `dcc run` (Milestone 8.3)

Executes computations through content-addressed caching:

```bash
dcc run \
  --input src/schema.json \
  --output generated/client.rs \
  -- generator src/schema.json
```

#### Execution Lifecycle
```text
calculate key
→ lookup
→ HIT: restore
→ MISS: execute
→ validate
→ store
```

1. **Calculate Key**: Hashes declared inputs, normalizes arguments, environment, tool identity, and platform runtime to form a canonical SHA-256 `CacheKey`.
2. **Lookup**: Checks CAS storage for matching `CacheEntry`.
3. **HIT: Restore**: Safely restores all declared output artifacts via atomic staging and re-streams stdout/stderr without re-running the command (`execution_time_ms = 0`).
4. **MISS: Execute**: Spawns command, captures stdout/stderr, monitors exit code.
5. **Validate**: Confirms all declared required output files exist and are intact.
6. **Store**: Hashes outputs into CAS objects, writes `CacheEntry` JSON atomically, and records execution metadata.

### `dcc stats` (Milestone 8.4)

Displays comprehensive cache storage metrics, efficiency metrics, and cumulative time savings:

```bash
dcc stats
dcc stats --json
```

Output Metrics:
- **`entries`**: Total number of registered computation metadata records.
- **`objects`**: Total physical CAS objects stored.
- **`disk usage`**: Combined disk usage formatted in human-readable units (with breakdown of objects vs entries).
- **`hits`**: Cumulative cache hit count across all entries.
- **`misses`**: Cumulative cache miss count (initial executions).
- **`hit ratio`**: Aggregate hit ratio percentage (`hits / (hits + misses)`).
- **`bytes restored`**: Total byte volume restored during cache hits without recomputation.
- **`bytes stored`**: Total output byte volume stored in CAS.
- **`estimated time saved`**: Total estimated execution time saved by serving hits from cache.
- **`largest object`**: Size in bytes of the largest stored CAS blob.

### `dcc inspect` (Milestone 8.5)

Allows developers to inspect stored computation records and manifests:

```bash
dcc inspect <key>
dcc inspect <key> --json
```

Displayed Fields:
- **`key`**: Canonical SHA-256 computation key.
- **`operation`**: High-level logical operation identifier.
- **`command`**: Target executable binary or script.
- **`arguments`**: Full argument list.
- **`inputs`**: Manifest of input files (paths, sizes, SHA-256 digests).
- **`outputs`**: Manifest of produced output files (paths, sizes, CAS SHA-256 digests).
- **`tool identity`**: Name, version, and binary executable hash.
- **`environment`**: Explicitly declared environment variables.
- **`created`**: UTC timestamp when computation was first recorded.
- **`last accessed`**: UTC timestamp of the most recent cache hit/access.
- **`size`**: Aggregate size of produced outputs in bytes and KB/MB.
- **`integrity`**: Cryptographic validation status checking declared key against canonical computation digest.

### JSON Output (`--json`) (Milestone 8.6)

Every major command supports machine-readable structured JSON output via the global `--json` flag:

```bash
dcc stats --json
dcc inspect <key> --json
dcc run --json --input src/schema.json --output generated/models.rs -- generator src/schema.json
dcc init --json
dcc doctor --json
dcc verify --json
dcc config --json
dcc prune --json
dcc clean --json
```

Enables seamless integration with CI/CD runners, build automation pipelines, and metrics aggregators.

### Stable Exit Codes (Milestone 8.7)

DCC implements documented, stable exit codes for deterministic process orchestration:

| Exit Code | Constant / Identifier | Meaning & Triggers |
| :--- | :--- | :--- |
| `0` | `ExitCode::Success` | Clean execution, successful cache HIT restoration, or valid maintenance operation |
| `1` | `ExitCode::ComputationFailed` | Target computation process exited with non-zero code, or cache entry key not found during inspect |
| `2` | `ExitCode::InvalidConfiguration` | Invalid configuration parameters or unparseable max size string |
| `3` | `ExitCode::CacheError` | Storage system I/O error or permission denied |
| `4` | `ExitCode::IntegrityFailure` | Corrupted CAS object detected, hash mismatch, or invalid computation entry integrity |
| `5` | `ExitCode::InvalidArguments` | Missing required CLI arguments, unrecognized command options, or invalid subcommands |

---

## Observability & Cache Explanation (Milestone 9)

Provides full visibility into cache efficiency, telemetry, and execution performance.

### Hit / Miss Metrics (Milestone 9.1)

Tracks quantitative cache activity across all computations:

- **`total requests`**: Total computation cache queries processed (`hits + misses`).
- **`hits`**: Number of requests resolved directly from cache without recomputation.
- **`misses`**: Number of computations that resulted in a cache miss.
- **`hit ratio`**: Cache reuse ratio percentage (`hits / total requests`).
- **`execution count`**: Number of external command executions performed.
- **`cache restore count`**: Number of output restoration events from CAS storage.
- **`cache store count`**: Number of completed computations stored into CAS.

Viewable via `dcc stats` and `dcc stats --json`.

### Timing Metrics & Time Saved (Milestone 9.2)

DCC tracks granular lifecycle timings for both cache misses and cache hits:

- **`computation execution time` (`execution_time_ms`)**: Time spent executing the actual compiler, generator, or tool process.
- **`cache lookup time` (`lookup_time_ms`)**: Time spent querying the CAS entry index and locating cached records.
- **`cache restore time` (`restore_time_ms`)**: Time spent restoring output files and directories from CAS blobs to disk.
- **`cache store time` (`store_time_ms`)**: Time spent hashing outputs, committing CAS objects, and saving entry metadata.
- **`time saved` (`time_saved_ms`)**: Net time saved on a cache hit, calculated as:
  $$\text{Time Saved} = \text{Execution Time} - (\text{Lookup Time} + \text{Restore Time})$$

### Explain Mode (Milestone 9.3)

With `dcc run --explain`, developers get clear, actionable root-cause analysis answering *"Why didn't my cache work?"*:

```text
Cache lookup

Result: MISS

Reason:
  input changed

Changed:
  src/parser.rs

Previous:
  sha256: abc...

Current:
  sha256: def...
```

### Debug Mode (Milestone 9.4)

With `dcc run --verbose` (or `-v`), developers inspect the engine's step-by-step caching lifecycle:

```text
[INPUT] hashing files
[KEY] generating computation key
[LOOKUP] checking cache
[MISS] no entry
[EXEC] running command
[OUTPUT] validating outputs
[STORE] writing objects
[DONE] stored result
```

---

## CLI Usage

```bash
# Initialize local cache directory with custom max size
dcc init --max-size "2 GB"

# Execute a computation with caching
dcc run --input src/schema.json --output generated/models.rs -- generator src/schema.json

# Explain cache miss reasons
dcc run --explain --input src/schema.json --output generated/models.rs -- generator src/schema.json

# Run with verbose stage-by-stage debugging
dcc run --verbose --input src/schema.json --output generated/models.rs -- generator src/schema.json

# View cache storage statistics (human-readable or JSON)
dcc stats
dcc stats --json

# Inspect a specific computation by key
dcc inspect <key>
dcc inspect <key> --json

# Inspect cache configuration
dcc config
dcc config --get max_size

# Verify storage integrity
dcc verify

# Prune unreferenced objects and enforce max size limit
dcc prune --max-size "500 MB" --dry-run
dcc prune --strategy lru --max-size "2 GB"

# Clean specific key or entire cache
dcc clean --key <key>
dcc clean

# Run health diagnostics
dcc doctor
```

---

## Generic Developer Integration API (Rust Library)

DCC provides a first-class, idiomatic Rust public API for developers to embed caching directly into custom build tools, code generators, compilers, and linters without calling CLI subprocesses:

```rust
use dcc_integrations::{Cache, ComputationBuilder, GenericIntegration};
use std::path::Path;

// 1. Open the cache workspace (auto-creates layout if missing)
let cache = Cache::open(Path::new("./.dcc_cache"))?;

// 2. Build computation specifications fluently
let spec = ComputationBuilder::new("codegen-models")
    .with_arguments(vec!["--schema".into(), "schema.json".into()])
    .with_inputs(vec![Path::new("schema.json").to_path_buf()])
    .with_outputs(vec![Path::new("generated/models.rs").to_path_buf()])
    .build();

// 3. Execute or lookup using the integration runner
let runner = GenericIntegration::from_cache(&cache, Default::default());
let result = runner.execute(&spec, Path::new("."))?;

if result.was_hit {
    println!("Computation restored from cache!");
} else {
    println!("Computation executed and stored in cache!");
}

// 4. Or interact with the low-level Cache API directly
let key = spec.canonical_key();
if let Some(entry) = cache.lookup(&key)? {
    println!("Found cached entry with {} outputs", entry.outputs.len());
}
```

---

## Ergonomic Builder API (`Computation::builder()`)

DCC provides a fluent, ergonomic builder API for constructing `Computation` instances, with pre-execution validation against invalid configurations (e.g. empty operation or command, path traversal attempts):

```rust
use dcc_core::Computation;

let computation = Computation::builder()
    .operation("codegen")
    .command("generator")
    .args(vec!["--schema", "schema.json", "--opt"])
    .arg("--fast")
    .input("schema.json", digest, size)
    .output("models.rs", true)
    .env("TARGET_LANG", "rust")
    .meta("author", "dcc-dev")
    .build()?;
```

### Validation Guarantees:
- **Operation & Command Validation**: Operation and executable command must not be empty or whitespace-only.
- **Path Traversal Protection**: Inputs and outputs containing traversal prefixes (`../`, `..`) or absolute paths (`/`) are rejected at construction before execution starts.
- **Canonical Ordering**: Inputs and outputs are canonically sorted by path during `.build()`.

---

## Developer Integration Examples (`examples/`)

Realistic developer integration examples demonstrating first-class usage of the DCC Rust library:

```bash
# 1. Code Generator (Schema -> Models)
cargo run --example cached_codegen

# 2. Static Code Analysis & Linter (Source + Rules -> Analysis Report)
cargo run --example cached_analysis

# 3. Data Transformation & Asset Minification (CSV -> JSON Pipeline)
cargo run --example cached_transform

# 4. Rust Compiler Build Action Integration (Source + Compiler Config + Tool Identity)
cargo run --example cached_rust_build

# 5. Milestone 11.3 Reproducible Build Demonstration (Build #1 MISS -> Build #2 HIT -> Build #3 Source Mod MISS)
cargo run --example build_demonstration
```

Each example illustrates:
1. Opening the local cache using `Cache::open(cache_dir)`.
2. Initializing a `GenericIntegration` runner bound to the workspace.
3. Defining inputs, outputs, environment declarations, and metadata with `Computation::builder()`.
4. Cold execution causing a cache **MISS** and storing generated artifacts.
5. Deleting local outputs and running the second execution to demonstrate instantaneous cache **HIT** and deterministic output restoration.

---

## Generic Developer Integration Contract

When integrating DCC into custom build systems, compilers, linters, or code generators, adhere to the following contract rules:

### 1. How to Declare Inputs
- Declare all files whose contents affect the output of the computation.
- Use relative or normalized workspace paths.
- Each input requires a cryptographic content digest (`Digest::hash_file`) and byte size. Helpers like `.input_path(path)` compute this automatically.

### 2. How to Declare Outputs
- Declare all files or directories created or modified by the computation in the workspace.
- Specify whether the output is strictly required (`required = true`).
- On a cache **HIT**, DCC guarantees atomic restoration of all declared outputs from CAS.

### 3. How to Declare Environment
- Explicitly declare only the environment variables that alter the computation result (e.g. `TARGET_ARCH`, `OPTIMIZATION_LEVEL`, `DEBUG`).
- Use `.declared_env("VAR_NAME")` to capture ambient variables if present.
- Undeclared system environment variables are excluded from the canonical identity, preventing unnecessary cache fragmentation.

### 4. How Cache Identity Works
- Computation identity (`CacheKey`) is computed deterministically by canonical JSON serialization and SHA-256 hashing.
- Path representations are normalized to `/` across all operating systems.
- Input order and environment map order are sorted canonically, making key generation invariant to declaration sequence.

### 5. How Errors Work
- **Validation Errors**: Malformed configurations (empty command/operation, directory traversal outside workspace) return `CacheError::ValidationError` or `CacheError::PathTraversal` prior to execution.
- **Execution Failures**: If a command exits with a non-zero code, DCC by default (`FailurePolicy::DoNotCache`) avoids caching the failed state.
- **Integrity & Quarantine**: If stored CAS blobs fail checksum verification, DCC automatically marks the object as quarantined and safely re-computes the result.

### 6. How Cache Misses Work
- When no entry matches the `CacheKey`, DCC runs the computation command in the working directory.
- Upon successful execution (exit code 0), DCC captures outputs and stdout/stderr into CAS, and records a new `CacheEntry`.
- Use `--explain` or `MissExplainer` to inspect why a cache miss occurred (e.g., input modified, arguments altered, platform change).

### 7. How to Disable or Bypass Caching
- Configure the runtime `CachePolicy`:
  * `CachePolicy::ReadWrite`: Default normal caching (lookup then store).
  * `CachePolicy::ReadOnly`: Lookup cached results, but do not write new ones.
  * `CachePolicy::WriteOnly`: Always execute, overwrite cache entries with fresh outputs.
  * `CachePolicy::Bypass`: Completely skip cache lookups and writes.
  * `CachePolicy::ForceRecompute`: Force process re-execution but update the cache with new results.

---

## Build Action Model (`BuildAction`)

DCC provides a dedicated action-oriented build caching model (`BuildAction`) representing compiler invocations:

```rust
use dcc_integrations::{BuildAction, Cache, GenericIntegration};

let action = BuildAction::builder()
    .compiler("rustc")
    .arguments(vec!["src/main.rs", "--crate-type", "bin", "-O"])
    .source_input("src/main.rs", source_digest, source_size)
    .dependency_input("target/deps/libcore.rlib", dep_digest, dep_size)
    .compiler_version("1.80.0")
    .target("x86_64-unknown-linux-gnu")
    .env("RUSTFLAGS", "-C opt-level=3")
    .output("target/main", true)
    .build()?;

let key = action.compute_key()?;
let result = integration.execute_build_action(action)?;
```

### Build Action Components:
1. **Compiler**: Target compiler binary executable (`rustc`, `gcc`, `clang`, `cl.exe`).
2. **Arguments**: Full list of compiler flags, target definitions, and optimization switches.
3. **Source Inputs**: Primary source files (`.rs`, `.c`, `.cpp`) with cryptographic content digests.
4. **Dependency Inputs**: Intermediate libraries, header files, and crate archives (`.rlib`, `.a`, `.h`).
5. **Compiler Version & Tool Digest**: Explicit version string and tool binary hash.
6. **Target & Platform**: Target triple (`x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, etc.).
7. **Environment**: Declared compilation environment variables (`RUSTFLAGS`, `CFLAGS`, `CC`).
8. **Outputs**: Expected build artifacts (`.exe`, `.rlib`, `.so`, `.o`, `.pdb`).

---

## Build Cache Benchmarking (`BenchmarkMetrics`)

DCC provides built-in benchmarking utilities for measuring compiler cache acceleration and storage footprint:

```rust
use dcc_integrations::{BuildAction, GenericIntegration};

let integration = GenericIntegration::new(cache_dir, workspace_dir)?;
let metrics = integration.run_benchmark(build_action)?;

println!("Cold Build:        {} ms", metrics.cold_build_time_ms);
println!("Warm (No Cache):   {} ms", metrics.warm_build_without_cache_time_ms);
println!("Warm (With Cache): {} ms", metrics.warm_build_with_cache_time_ms);
println!("Lookup Time:       {} ms", metrics.cache_lookup_time_ms);
println!("Restore Time:      {} ms", metrics.restore_time_ms);
println!("Storage Size:      {} bytes", metrics.storage_size_bytes);
println!("Speedup:           {:.2}x", metrics.speedup);
```

Run the benchmark example:

```bash
cargo run --example build_benchmark
```

---

## Cross-Platform Path Handling (`PathUtils`)

DCC provides robust, platform-agnostic path normalization and sanitization across Windows, Linux, and macOS without hardcoded assumptions about separators (`/` vs `\`) or drive prefixes (`C:\`):

```rust
use dcc_core::PathUtils;
use std::path::Path;

// Canonical forward-slash normalization for invariant key hashing
let normalized = PathUtils::to_normalized_string(r"src\components\button.rs");
assert_eq!(normalized, "src/components/button.rs");

// Convert normalized paths back to OS-native PathBuf
let native = PathUtils::to_native_path("src/components/button.rs");

// Safe workspace encapsulation and directory traversal rejection
let safe_path = PathUtils::sanitize_relative_path(Path::new("./workspace"), "dist/bundle.js")?;
```

---

## Cross-Platform Process Execution (`ProcessExecutor`)

DCC provides robust, platform-agnostic process spawning, environment isolation, executable discovery, and lifecycle supervision across Windows, Linux, and macOS:

```rust
use dcc_runner::ProcessExecutor;
use std::collections::BTreeMap;
use std::time::Duration;

// Executable discovery across PATH with platform extension resolution (.exe, .cmd, .bat)
let rustc_bin = ProcessExecutor::discover_executable("rustc");

// Execute command with stdout/stderr capture and timeout protection
let output = ProcessExecutor::execute_with_timeout(
    "rustc",
    &["--version".to_string()],
    &BTreeMap::new(),
    None,
    Some(Duration::from_secs(10)),
)?;

println!("Exit code: {}", output.exit_code);
println!("Stdout: {}", String::from_utf8_lossy(&output.stdout));
```

---

## Cross-Platform File Semantics

DCC handles cross-platform file system semantics consistently across Windows, Linux, and macOS:

- **Symlinks**: Transparently followed and hashed based on underlying target content.
- **Permissions & Executable Bits**: Unix executable permission bits (`0o755`) are preserved across CAS storage and atomic restorations.
- **Case Sensitivity**: Exact casing is maintained for key identity while supporting platform-native lookups.
- **Path Separators**: Canonical forward-slash normalization eliminates platform disparities.

---

## Cross-Platform CI Matrix

Every build and release candidate of DCC is automatically verified across all major operating systems via GitHub Actions (`.github/workflows/ci.yml`):

| Operating System | CI Runner | Toolchain | Status |
|---|---|---|---|
| **Linux** | `ubuntu-latest` | `stable` | Active |
| **Windows** | `windows-latest` | `stable` | Active |
| **macOS** | `macos-latest` | `stable` | Active |

Quality pipeline steps executed on every OS:
1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
3. `cargo test --workspace --verbose`
4. Example suite execution (`cached_codegen`, `cached_analysis`, `cached_transform`, `cached_rust_build`, `build_demonstration`, `build_benchmark`)

---

## Performance Benchmarks (`performance_benchmarks`)

DCC provides comprehensive performance benchmarks covering all core primitives and operational paths:

```bash
cargo run --example performance_benchmarks
```

Measured operations include:
1. **Hash Small File (4 KB)**: Microsecond-scale streaming SHA-256 calculation.
2. **Hash Large File (10 MB)**: High-throughput constant-memory SHA-256 processing.
3. **Hash Directory (Recursive)**: Canonical walkdir traversal and deterministic combined hashing.
4. **Generate Key**: Computation specification normalization and canonical JSON serialization.
5. **Lookup Cache**: Entry retrieval and access metadata updates.
6. **Store Cache**: Atomic metadata commit and CAS output blob indexing.
7. **Restore Cache**: Atomic materialization and SHA-256 verification.
8. **Serialize & Deserialize Metadata**: Zero-loss JSON conversion efficiency.
9. **Concurrent Lookup**: Multi-threaded read throughput under active thread contention.

---

## Large Files Streaming Performance (`large_files_benchmark`)

DCC guarantees constant $O(1)$ memory usage when processing large artifacts (tested across **1 MB**, **10 MB**, **100 MB**, and **1 GB** tiers) by strictly streaming files through fixed-size $64\text{ KB}$ chunk buffers:

```bash
cargo run --release --example large_files_benchmark
```

### Measured Large File Throughput:
- **Streaming SHA-256 Hashing**: Up to $\approx 824.6\text{ MB/s}$
- **CAS Stream Ingestion**: Up to $\approx 229.0\text{ MB/s}$
- **CAS Stream Restoration**: Up to $\approx 191.5\text{ MB/s}$
- **Memory Footprint**: Flat, bounded $64\text{ KB}$ buffer memory across all tiers.

---

## Large Cache Scalability (`large_cache_benchmark`)

DCC scales efficiently across high-density cache volumes, benchmarked and verified across **1,000**, **10,000**, and **100,000** synthetic entry tiers:

```bash
cargo run --release --example large_cache_benchmark
```

### Key Scalability Results:
- **256-Shard Directory Layout**: Entry metadata files (`.dcc_cache/entries/xx/`) and CAS objects (`.dcc_cache/objects/xx/`) are partitioned across 256 subdirectories, ensuring uniform distribution (~390 files/dir at 100k entries) and avoiding filesystem lock bottlenecks.
- **Microsecond Negative Lookup**: Miss lookup latency remains bounded and near-instantaneous ($10.8\,\mu\text{s}$ at 1k entries $\rightarrow$ $21.0\,\mu\text{s}$ at 100k entries).
- **Linear Index Inspection**: Full storage stats scan scales linearly with entry volume ($68\text{ ms}$ at 1k entries $\rightarrow$ $7.98\text{ s}$ at 100k entries).
- **Safe Maintenance & GC**: Complete reachability graph analysis and safe unreferenced object pruning scales deterministically without memory leaks or race conditions.

---

## Profile-Driven Optimizations (`profile_and_optimize_benchmark`)

DCC follows the strict performance engineering principle: **"Do not optimize blindly. First measure."** Every optimization is backed by empirical before/after benchmarks:

```bash
cargo run --release --example profile_and_optimize_benchmark
```

### Targeted Optimization Results:

| Optimization Pipeline | Baseline (Before) | Optimized (After) | Speedup Gain |
| :--- | :--- | :--- | :--- |
| **In-Memory L1 Metadata Cache** | $4{,}947.26\,\mu\text{s}$ ($202\text{ lookups/s}$) | **$1.49\,\mu\text{s}$** ($672{,}016\text{ lookups/s}$) | **$3{,}324.64\times$** |
| **Parallel Batch File Hashing** | $21.00\text{ ms}$ ($1{,}177.78\text{ MB/s}$) | **$6.00\text{ ms}$** ($4{,}181.44\text{ MB/s}$) | **$3.55\times$** |
| **Safe Hardlink Materialization** | $91.00\text{ ms}$ ($218.57\text{ MB/s}$) | **$38.00\text{ ms}$** ($525.83\text{ MB/s}$) | **$2.41\times$** |

---

## Quality Gates & Verification

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
cargo fmt --all -- --check
```

All 6 core exit criteria (deterministic computation modeling, canonical key generation, cache entry creation, retrieval, identity verification, and corrupted metadata detection) and all 11 physical storage scenarios (empty cache, single object, deduplication, corruption quarantine, interrupted write isolation, deletion, concurrent read/write races, deeply nested paths, multi-MB large files, and binary byte safety) are fully verified and tested.








