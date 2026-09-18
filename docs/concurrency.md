# Concurrency & Multi-Process Synchronization

DCC is architected from the ground up for multi-process concurrency, allowing parallel terminals, background build daemons, and CI workers to access the shared cache simultaneously and safely without corruption or deadlocks.

---

## 1. Concurrency Model

```text
Terminal A ──► dcc run --input data.json -- generator data.json
Terminal B ──► dcc run --input data.json -- generator data.json  (Same computation)
Terminal C ──► dcc run --input src/lib.rs -- compiler src/lib.rs (Different computation)
```

DCC divides concurrent access into three distinct synchronization tiers:

1. **Lock-Free Concurrent Reads**: Multiple processes can read, lookup, and restore identical or distinct CAS objects simultaneously with zero lock contention.
2. **Atomic Concurrent Writes**: Two-stage writes (`.tmp` $\rightarrow$ `sync_all` $\rightarrow$ atomic rename) guarantee that writers never expose partially written or corrupt files to readers or each other.
3. **Computation Deduplication Locks**: When multiple processes experience a cache miss on the exact same computation key simultaneously, DCC coordinates execution so only one worker executes the command while the others wait and receive instant cache hits.

---

## 2. Duplicate Computation Avoidance (`ComputationLock`)

When 10 parallel build jobs or terminal sessions miss the same cache key at the same moment, running 10 identical heavy compilations wastes massive CPU. DCC prevents duplicate execution via advisory file locking:

```text
Process A (MISS)                    Process B (MISS)
      │                                   │
Acquire Lock (key X) ───► SUCCESS         │
      │                                   │
Spawns & runs command                     Acquire Lock (key X) ───► WAITING
      │                                                               :
Ingests outputs into CAS                                              :
Writes CacheEntry record                                              :
Releases Lock ────────────────────────────────────────────────────────┘
                                                                      │
                                                           Acquires Lock
                                                           Re-checks Cache
                                                           Receives HIT (0ms)
                                                           Restores files & exits
```

### 2.1 Secondary Cache Re-Check Invariant
Upon successfully acquiring the computation lock, `RunnerEngine` immediately queries the cache a second time before executing the child process. If a preceding process completed the computation while the current process was waiting on the lock, the second check immediately returns a cache hit (`ExecutionStatus::Hit`) with 0ms execution time, completely bypassing command execution.

---

## 3. Crash Consistency & Lock Failure Recovery

DCC guarantees that abnormal process terminations, sudden kills (`SIGKILL`, `taskkill`), power cuts, or unhandled panics never leave the cache locked or corrupted:

1. **OS Kernel Lock Auto-Release**: All locks utilize operating system advisory file locks (`fs2::FileExt`). When a process terminates abruptly, the OS kernel immediately and automatically releases all held file descriptors and locks.
2. **Stale Lock Overwrite**: Orphaned lock files on disk containing dead PIDs or timestamps are claimed and overwritten atomically by active processes without blocking.
3. **Corrupted Lock Self-Healing**: Non-JSON or malformed lock files are detected and overwritten cleanly upon acquisition.
4. **Configurable Timeouts**: If a lock is held longer than the configured timeout duration (`lock_timeout`, default 30s), acquisition returns `CacheError::LockError` rather than hanging indefinitely.
5. **Stale Lock Pruning**: `ComputationLock::clean_stale_locks()` allows background maintenance routines to safely purge inactive lock files older than a specified threshold.

---

## 4. Concurrent Reader-Writer Safety (`ObjectLock`)

When garbage collection (`dcc prune`) or manual deletion (`dcc clean`) runs concurrently with active builds:

- **Active Readers**: Acquire a shared read lock (`ObjectLock::acquire_shared`) when opening or streaming a CAS blob.
- **Deleters / Pruners**: Attempt to acquire an exclusive lock (`ObjectLock::try_acquire_exclusive`) before deleting an object.
- **Non-Blocking Eviction**: If an unreferenced object is actively being streamed by a reader, the pruner safely skips deleting it during the current pass and will reclaim it in the next GC cycle once the reader has finished.

---

## 5. Concurrency Stress Validation

DCC's multi-process synchronization is verified under synthetic stress workloads:

| Scale | Scenario | Verification Result |
| :--- | :--- | :--- |
| **10 Workers** | Mixed concurrent read/write workloads with identical and distinct computation keys | 0 deadlocks, clean deduplication, all output files byte-identical |
| **50 Workers** | 50 simultaneous workers executing multi-artifact builds with secondary side outputs | 1 process executes cold miss, 49 processes receive instant warm hits |
| **100 Operations** | Synchronized barrier dispatching 100 parallel operations | 0 leaked `.tmp` files, 0 corrupted objects, 100% integrity validation pass |
