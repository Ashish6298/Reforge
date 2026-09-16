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
