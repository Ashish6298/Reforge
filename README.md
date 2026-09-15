# Developer Computation Cache (`dcc`)

A high-performance, local-first, content-addressed developer computation caching engine written in pure Rust.

## Core Guarantee

> **A cache hit must never change the semantic result of the computation.**
> If the system cannot prove that a cached result is safe to reuse, it executes the computation again.

## Features

- **Content-Addressed Storage (CAS)**: SHA-256 sharded storage for inputs, outputs, logs, and computation metadata.
- **Deterministic Key Derivation**: Canonical JSON normalization and SHA-256 hashing.
- **Explainable Cache Misses**: Precise human and machine-readable reasons when a computation must re-execute.
- **Multi-Process Concurrency Safety**: File-locking and duplicate-execution guards prevent redundant work across concurrent terminals or CI jobs.
- **Integrity Validation & Quarantine**: Validates checksums before restoration; automatically detects and quarantines tampered or corrupted artifacts.
- **Path Traversal Protection**: Prevents malicious directory escape attempts.
- **CLI & Rust Library**: First-class developer API and CLI with JSON output mode.

## Quickstart

```bash
# Initialize local cache
dcc init

# Run a computation with caching
dcc run --input src/schema.json --output generated/models.rs -- generator src/schema.json

# View cache statistics
dcc stats

# Inspect a computation
dcc inspect <key>

# Prune unreferenced objects
dcc prune
```
