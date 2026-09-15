# Developer Computation Cache (`dcc`)

A high-performance, local-first, content-addressed developer computation caching engine written in pure Rust.

## Core Guarantee

> **A cache hit must never change the semantic result of the computation.**
> If the system cannot prove that a cached result is safe to reuse, it executes the computation again.

---

## Core Domain Models (`cache-core`)

DCC avoids unstructured raw strings by employing strongly typed domain models:

- **`Digest`**: Validated 64-character lowercase hexadecimal SHA-256 content hash.
- **`CacheKey`**: Strongly typed cryptographic identifier derived from canonical JSON computation identity.
- **`Computation`**: Declarative model containing operation, executable, arguments, inputs, outputs, env, tool identity, platform, and policy.
- **`InputFile` & `OutputFile`**: Declared inputs with streaming digests and declared required outputs.
- **`OutputManifest`**: Collection of generated artifacts with validated digests and sizes.
- **`CacheEntry`**: Immutable metadata record containing computation specification, output manifest, timestamps, and execution metrics.
- **`CacheMetadata` & `ExecutionMetadata`**: Access times, hit counts, exit codes, and stdout/stderr references.
- **`CacheResult<T>`**: Strongly typed lookup and execution outcomes (`Hit`, `Miss`, `Bypassed`).
- **`CachePolicy`**: Execution caching directives (`ReadWrite`, `ReadOnly`, `WriteOnly`, `Bypass`, `ForceRecompute`).
- **`StructuredEvent`**: High-performance telemetry event taxonomy.

---

## Workspace Architecture

- **[`crates/cache-core`](crates/cache-core)**: Core domain models, deterministic normalization, structured logging, and canonical SHA-256 key generation.
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
