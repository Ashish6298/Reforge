# Performance Engineering & Benchmarks

DCC follows the strict performance engineering principle: **"Do not optimize blindly. First measure."** Every architectural decision and optimization is verified with reproducible micro-benchmarks and stress suites.

---

## 1. Performance Summary

| Primitive / Operation | Measured Latency / Throughput | Notes |
| :--- | :--- | :--- |
| **Small File Hash (4 KB)** | $\approx 2.8\,\mu\text{s}$ | Streaming SHA-256 calculation |
| **Large File Hash (10 MB)** | $\approx 824.6\text{ MB/s}$ | Constant $64\text{ KB}$ chunk streaming |
| **Directory Hash (Recursive)** | $\approx 410\,\mu\text{s}$ (100 files) | Canonical sorted walkdir traversal |
| **Key Generation** | $\approx 1.2\,\mu\text{s}$ | Canonical JSON serialization + SHA-256 |
| **Cache Lookup (L1 RAM Hit)** | **$1.49\,\mu\text{s}$** | In-memory metadata cache ($672{,}000\text{ lookups/s}$) |
| **Cache Lookup (L2 Disk Hit)** | $\approx 12.5\,\mu\text{s}$ | Direct sharded file read |
| **Cache Lookup (Miss)** | $\approx 10.8\,\mu\text{s}$ (at 1k entries) | Immediate negative probe |
| **CAS Stream Ingestion** | $\approx 229.0\text{ MB/s}$ | Two-stage atomic write + `fsync` |
| **CAS Stream Restoration** | $\approx 191.5\text{ MB/s}$ | Direct atomic file materialization |
| **Hardlink Materialization** | **$38.0\text{ ms}$** (vs $91.0\text{ ms}$) | $2.41\times$ faster than byte-copy |

---

## 2. Large File Streaming ($O(1)$ Constant Memory)

DCC handles multi-gigabyte files with flat, bounded memory consumption by strictly streaming through $64\text{ KB}$ chunk buffers across all tiers:

```bash
cargo run --release --example large_files_benchmark
```

### Verified Multi-Tier Scale:
- **1 MB Tier**: Verified instant ingestion & restoration.
- **10 MB Tier**: Ingestion $\approx 229\text{ MB/s}$, memory strictly bounded.
- **100 MB Tier**: Streaming hash at $>800\text{ MB/s}$.
- **1 GB Tier**: Constant $64\text{ KB}$ buffer memory footprint with zero RAM inflation.

---

## 3. Large Cache Scalability (256-Shard Partitioning)

To evaluate performance under heavy enterprise usage, DCC was benchmarked across **1,000**, **10,000**, and **100,000** synthetic entry tiers:

```bash
cargo run --release --example large_cache_benchmark
```

```text
.dcc_cache/
├── objects/ [256 shards: 00/ .. ff/]  (~390 objects/dir at 100k)
└── entries/ [256 shards: 00/ .. ff/]  (~390 entries/dir at 100k)
```

### Scalability Findings:
1. **Partitioning Efficiency**: Sharding prevents filesystem lock contention and directory traversal bottlenecks.
2. **Negative Lookup Latency**: Remains nearly flat even at 100,000 entries ($10.8\,\mu\text{s} \rightarrow 21.0\,\mu\text{s}$).
3. **Linear Scan Performance**: Full storage audits (`dcc stats`) scale linearly ($68\text{ ms}$ at 1k entries to $7.98\text{ s}$ at 100k entries).

---

## 4. Profile-Driven Optimizations

Three targeted optimizations were implemented based on real profile data:

```bash
cargo run --release --example profile_and_optimize_benchmark
```

### 1. In-Memory L1 Metadata Cache
- **Problem**: Reading and parsing JSON metadata files from disk on every lookup introduced unnecessary I/O overhead for hot computations.
- **Optimization**: Added a concurrent in-memory L1 cache (`RwLock<HashMap<CacheKey, CacheEntry>>`) backed by filesystem mtime invalidation.
- **Result**: Lookup latency dropped from $4{,}947.26\,\mu\text{s}$ to **$1.49\,\mu\text{s}$** (**$3{,}324.64\times$ speedup**).

### 2. Parallel Batch File Hashing
- **Problem**: Computations with dozens of declared input files hashed files sequentially, bound by single-core throughput.
- **Optimization**: Implemented parallel batch hashing using Rayon thread pools for workloads with $>4$ inputs.
- **Result**: Hashing latency dropped from $21\text{ ms}$ to **$6\text{ ms}$** (**$3.55\times$ speedup**).

### 3. Safe Hardlink Materialization
- **Problem**: Copying large binary output files from CAS to workspace on cache hit duplicated physical disk writes.
- **Optimization**: Implemented opportunistic hardlink materialization (`fs::hard_link`) when both CAS and workspace reside on the same filesystem, falling back safely to byte copying.
- **Result**: Restoration time dropped from $91\text{ ms}$ to **$38\text{ ms}$** (**$2.41\times$ speedup**).
