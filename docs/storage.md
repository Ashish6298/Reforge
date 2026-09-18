# Storage Model & Content-Addressed Storage (CAS)

DCC manages computation artifacts and execution metadata through a high-performance, crash-consistent, local-first storage architecture.

---

## 1. Architectural Model

DCC completely decouples **Action Metadata** from **Physical Byte Blobs**:

```text
.dcc_cache/
├── objects/            # Content-Addressed Storage (CAS) - Immutable output blobs
│   ├── ab/
│   │   └── ab34cdef...
│   └── 12/
│       └── 1298af7b...
├── entries/            # Action Cache metadata records (JSON)
│   ├── 01/
│   │   └── 01a4e2...json
│   └── 9f/
│       └── 9f5c88...json
├── metadata/           # Storage configuration and indices
├── index/              # Secondary lookup indices
├── tmp/                # Staging directory for crash-safe atomic writes
└── locks/              # Multi-process concurrency and deletion locks
```

### 1.1 256-Shard Partitioning
To prevent scalability bottlenecks from storing tens of thousands of files in a single directory, DCC partitions both CAS objects (`objects/xx/`) and computation entries (`entries/xx/`) across 256 subdirectories based on the first two hexadecimal characters of their SHA-256 digest.

---

## 2. Two-Stage Atomic Writes & Crash Consistency

To guarantee that DCC never leaves half-written or corrupted artifacts in the live cache, all writes follow a strict atomic pipeline:

```text
1. Staging Write (.dcc_cache/tmp/*.tmp)
           ↓
2. Flush Buffer
           ↓
3. sync_all (fsync)
           ↓
4. Atomic Rename (.dcc_cache/objects/ab/<hash>)
```

- **In-flight Interruptions**: If a process crashes or is killed during a write, the incomplete file remains in `.tmp/` and is ignored by readers.
- **Atomic Promotion**: The file is only moved to its destination path in `objects/` or `entries/` via an atomic filesystem rename (`fs::rename`).
- **Deduplication Races**: If two concurrent processes compute identical outputs and attempt to write the same CAS blob simultaneously, both safely write to separate `.tmp` files and atomically rename to the same destination. The final file is 100% intact and deduplicated.

---

## 3. Storage Trait & Pluggable Backends

Physical storage is defined via the `Storage` trait, allowing seamless backend interchangeability:

```rust
use dcc_storage::{BlobMetadata, Digest, Storage};
use std::io::Read;
use std::path::Path;

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

The default implementation is `CasStorage`, providing local filesystem CAS with sharding, caching, and integrity verification.

---

## 4. Corruption Detection & Automatic Quarantining

Every CAS blob read by the engine is cryptographically verified against its expected SHA-256 hash.

```text
CAS Read ──► Compute SHA-256 Digest ──► Equals Expected?
                                               │
               ┌───────────────────────────────┴───────────────────────────────┐
               ▼                                                               ▼
             [YES]                                                            [NO]
        Stream / Restore                                     1. Rename to *.corrupted
                                                             2. Return IntegrityError
                                                             3. Fallback to fresh execution
```

If a file fails checksum verification (due to disk corruption, bitrot, or tampering):
1. DCC immediately renames the offending object to `<digest>.corrupted` to prevent further access.
2. The runner engine treats the corrupted object as a cache miss.
3. The computation re-executes cleanly and stores fresh, valid artifacts.

---

## 5. Cache Lifecycle, Eviction & Garbage Collection

### 5.1 Size Limits (`max_size`)
DCC enforces configurable storage bounds (e.g. `500 MB`, `2 GB`, `10 GB`):

```bash
dcc init --max-size "5 GB"
dcc prune --max-size "2 GB"
```

### 5.2 Eviction Policies
When disk usage exceeds `max_size`, DCC evicts old metadata entries using deterministic policies:
- **LRU (Least Recently Used - Default)**: Evicts entries with the oldest `last_accessed_at` timestamp.
- **FIFO (First In, First Out)**: Evicts entries with the oldest `created_at` timestamp.
- **LFU (Least Frequently Used)**: Evicts entries with the lowest `hit_count`.

### 5.3 Garbage Collection (Reachability Analysis)
After metadata entries are evicted or deleted, CAS objects they previously referenced may become orphaned. DCC's garbage collector (`dcc prune`) performs full reachability analysis:

1. Scans all active metadata entries in `entries/`.
2. Computes the complete set of reachable CAS digests (referenced outputs + stdout + stderr).
3. Scans all physical objects in `objects/`.
4. Safely deletes any unreferenced objects not actively held by readers.

### 5.4 Safe Deletion with Object Locks
To prevent race conditions where a pruning process deletes an object while another process is actively reading or restoring it, DCC coordinates deletions with shared/exclusive locks (`ObjectLock`). Active readers hold shared locks, causing eviction passes to safely skip in-use objects without blocking.
