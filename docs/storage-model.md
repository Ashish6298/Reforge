# Storage Model Specification

## Overview

The Developer Computation Cache (`dcc`) separates **"What is cached?"** (metadata and action mapping in `entries/`) from **"Where are cached bytes stored?"** (content-addressed blobs in `objects/`).

---

## 1. Directory Structure

```text
.dcc_cache/
├── objects/               <-- Content-Addressed Storage (CAS)
│   ├── 0a/
│   │   └── 0af89...      <-- Binary blob keyed by SHA-256 digest
│   ├── b9/
│   │   └── b94d2...
│   └── ff/
│
├── entries/               <-- Computation key -> Entry Metadata
│   ├── c3/
│   │   └── c3699....json  <-- CacheEntry record containing computation & manifest
│   └── 9f/
│
├── locks/                 <-- Computation execution locks
│   └── c3699....lock      <-- Prevents redundant duplicate executions
│
└── tmp/                   <-- Staging directory for atomic writes
    └── 1048_172638....tmp
```

---

## 2. Directory Sharding
- To avoid filesystem performance degradation when millions of objects exist in a single directory, both `objects/` and `entries/` are sharded into 256 subdirectories based on the first **2 hex characters** of their SHA-256 digest (e.g. `00` through `ff`).

---

## 3. Two-Stage Atomic Write Protocol

No object or cache entry is ever written directly to its final path. Every write follows this strict lifecycle:

```text
1. Generate unique nonce temporary filename in .dcc_cache/tmp/
2. Write bytes / stream data into temporary file
3. Flush application buffers (writer.flush())
4. Synchronize physical file to disk storage (file.sync_all())
5. Atomically rename temporary file to destination path (.dcc_cache/objects/ab/cdef...)
```

If a process terminates or power is lost during steps 1–4, only an unreferenced file in `tmp/` remains, which is cleaned on maintenance. Partially written valid-looking artifacts can never exist in `objects/` or `entries/`.

---

## 4. Eviction & Maintenance
- **LRU Eviction**: Sorts entries by `last_accessed_at` timestamp and removes oldest entries when total cache size exceeds `max_size_bytes`.
- **Garbage Collection (Prune)**: Sweeps `objects/` and removes any unreferenced blobs not listed in any active `entries/` manifest.
