# Changelog

All notable changes to the **Developer Computation Cache (`dcc`)** project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [1.0.0] - 2026-09-18

### Added
- **Core Computation Engine**:
  - Deterministic computation modeling (`Computation`, `ComputationBuilder`, `CanonicalComputation`).
  - Cryptographic content-addressed hashing using streaming SHA-256 (`Digest`, `CacheKey`).
  - Composite tool identity tracking combining tool names, semver strings, and binary SHA-256 hashes (`ToolIdentity`).
  - Cross-platform path normalization and workspace boundary enforcement (`PathUtils`).
  - Structured event streaming and pub/sub metrics collection (`EventSubscriber`, `StructuredEvent`).
- **Content-Addressed Storage (CAS)**:
  - Immutable blob storage sharded across 256 two-character hexadecimal directory prefixes (`objects/xx/`).
  - Atomic two-stage write pipeline (`.tmp` $\rightarrow$ `sync_all` $\rightarrow$ atomic filesystem rename).
  - Multi-threaded advisory locking (`ComputationLock` and `ObjectLock`) using `fs2` cross-platform file locks.
  - Multi-policy cache eviction supporting `LRU`, `FIFO`, `LFU`, size ceilings (`max_size_bytes`), and TTL expiration (`Pruner`).
  - Reachability graph garbage collection safely pruning unreferenced CAS objects with active reader coordination.
- **Process Execution & Runtime**:
  - Child process executor capturing exit codes, execution wall-clock time, stdout, and stderr streams (`ProcessExecutor`).
  - Automatic atomic output restoration with scoped cryptographic verification (`OutputRestorer`).
  - Human-readable cache miss explainer (`MissExplainer`, `dcc run --explain`).
  - Explainable miss diagnosis identifying changed inputs, modified arguments, differing environments, and tool upgrades.
- **CLI Commands**:
  - `dcc run`: Primary execution wrapper with caching and `--explain` support.
  - `dcc init`: Cache initialization with configurable size limits (`--max-size`).
  - `dcc inspect`: Deep inspection of cached computation keys and output manifests.
  - `dcc stats`: Storage metrics, blob count, cache size, and largest artifact reporting.
  - `dcc verify`: Complete integrity check across all stored CAS objects and metadata records.
  - `dcc clean`: Full cache evacuation or targeted key deletion.
  - `dcc prune`: Garbage collection of orphaned objects and capacity limit enforcement.
  - `dcc config`: Interactive/declarative cache configuration management.
  - `dcc doctor`: Environment, permission, and directory health diagnostic suite.
  - `dcc diff`: Side-by-side computation input and metadata difference comparator.
- **Security & Sandboxing**:
  - Sandbox path containment rejecting path traversal (`../`, `..\`, absolute roots, drive escapes, null bytes).
  - Symlink attack prevention prohibiting overwriting targets outside the workspace boundary.
  - Automatic sensitive credential scanner (`SensitiveDataDetector`, `SensitiveDataPolicy::Deny`, `SensitiveDataPolicy::Mask`).
  - Integrity quarantine (`*.corrupted`) isolating bitrotted or tampered CAS blobs and metadata files.
  - Untrusted cache verification mode enforcing mandatory hash checks on every cache hit (`TrustMode::Untrusted`).
- **Remote & Tiered Storage Architecture**:
  - Tiered caching abstraction (`TieredCache`, `CacheTier::Local`, `CacheTier::Remote`).
  - Asynchronous remote storage design specification (`docs/remote-cache.md`).
- **CI/CD Integration & Workflows**:
  - Continuous integration workflows across `ubuntu-latest`, `windows-latest`, and `macos-latest` (`.github/workflows/ci.yml`).
  - Multi-stage release automation for tests, lints, packages, builds, and GitHub Releases (`.github/workflows/release.yml`).
  - CI graceful degradation ensuring cache unavailability or corruptions never fail developer builds.
- **Comprehensive Quality Audits & Test Suites**:
  - Infrastructure unit test suites covering hashing, key generation, serialization, validation, storage, metadata, config, and eviction (Milestone 18.1).
  - Full flow integration tests for command misses, hits, input changes, missing outputs, corruption, and concurrency (Milestone 18.2).
  - Fault injection test suite simulating disk full, permission denied, process crashes, partial writes, and corrupted metadata (Milestone 18.3).
  - Concurrency test suite covering many readers, many writers, same-key writers, different-key writers, reader+writer, and pruner+reader (Milestone 18.4).
  - Cross-platform essential test suite with 100% parity across Windows, Linux, and macOS (Milestone 18.5).
  - 10 performance benchmarks measuring small/large file hashing, directory traversal, key derivation, lookup/store/restore throughput (Milestone 13.1).

### Changed
- Refactored internal crate dependencies to use workspace inheritance (`dcc-core`, `dcc-storage`, `dcc-runner`).
- Standardized error handling on typed domain errors (`dcc_core::CacheError`).
- Frozen canonical computation schema version (`DCC_SCHEMA_VERSION = 1`).

### Fixed
- Fixed process lock contention under racing identical cold computations via advisory lock + secondary cache re-check.
- Fixed potential mid-stream deletion during garbage collection using shared read locks (`ObjectLock::acquire_shared`).
- Fixed case-sensitivity and separator discrepancies across Windows backslashes and Unix forward slashes.

### Security
- Added automated scanning for API keys, bearer tokens, passwords, and private certificates before serializing computation metadata.
- Implemented strict containment checks against symlink directory escapes and path traversal attacks.
- Enabled automatic quarantine and non-breaking computation fallback upon detecting corrupted CAS objects.

### Breaking Changes
- Initial stable release (v1.0.0). No previous breaking changes.

---

## [0.3.0] - 2026-09-01
- Beta preview release with tiered storage and remote cache architecture design.

## [0.2.0] - 2026-08-15
- Alpha release introducing multi-policy eviction (`LRU`, `FIFO`, `LFU`) and process execution runner.

## [0.1.0] - 2026-08-01
- Initial prototype demonstrating core content-addressed storage (CAS) and canonical key generation.
