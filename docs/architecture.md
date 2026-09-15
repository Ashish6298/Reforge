# DCC Architecture Specification

## Overview

The Developer Computation Cache (`dcc`) is a high-performance, local-first, content-addressed caching engine for developer tasks, builds, code generators, transformations, and CLI tools.

## System Architecture Diagram

```text
                 ┌──────────────────────────────────────┐
                 │ Developer CLI / CI / API Integration  │
                 └──────────────────┬───────────────────┘
                                    │
                                    ▼
                 ┌──────────────────────────────────────┐
                 │     Computation Definition & Spec    │
                 └──────────────────┬───────────────────┘
                                    │
                                    ▼
                 ┌──────────────────────────────────────┐
                 │       Normalization & Canonical      │
                 │          Key Derivation (SHA-256)    │
                 └──────────────────┬───────────────────┘
                                    │
                                    ▼
                         ┌──────────────────────┐
                         │ Cache Lookup (CAS)   │
                         └──────────┬───────────┘
                                    │
                         ┌──────────┴──────────┐
                         │                     │
                     [Cache HIT]          [Cache MISS]
                         │                     │
                         ▼                     ▼
             ┌───────────────────────┐ ┌───────────────────────┐
             │ Verify Object & Entry │ │ Acquire Exec Lock     │
             │ SHA-256 Checksums     │ │ Execute Process       │
             └───────────┬───────────┘ └───────────┬───────────┘
                         │                         │
                         │                         ▼
                         │             ┌───────────────────────┐
                         │             │ Validate Outputs      │
                         │             │ Store to CAS Atomically│
                         │             └───────────┬───────────┘
                         │                         │
                         └───────────┬─────────────┘
                                     │
                                     ▼
                         ┌───────────────────────┐
                         │ Restore Outputs & Log │
                         └───────────────────────┘
```

## Core Crates

1. **`cache-core`**: Defines pure domain types (`Digest`, `CacheKey`, `Computation`, `CacheEntry`, `OutputManifest`). Contains deterministic JSON canonicalizer and hashing pipeline.
2. **`cache-storage`**: Content-Addressed Storage engine. Manages directory sharding, temporary writes, atomic renames, checksum validation, disk statistics, and LRU pruning.
3. **`cache-runner`**: Execution engine orchestrator. Handles file discovery, process execution, stdout/stderr capture, output restoration, explainable miss generation, and lock management.
4. **`cache-cli`**: Command-line binary (`dcc`) with full interactive and machine-readable (`--json`) commands.
5. **`cache-test-utils`**: Test fixtures, mock tools, corruption injectors, and concurrency stress harnesses.
