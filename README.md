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

All 6 core exit criteria (deterministic computation modeling, canonical key generation, cache entry creation, retrieval, identity verification, and corrupted metadata detection) are fully verified and tested without external command dependencies.
