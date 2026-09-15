# Cache Correctness & Guarantees Specification

## Fundamental Rule

> **A cache hit must never change the semantic result of a computation.**
> If the system cannot prove that a cached result is safe to reuse, it must execute the computation again.

---

## 1. What DCC Guarantees

DCC provides strict, verifiable guarantees backed by cryptographic content-addressed storage, deterministic serialization, and atomic filesystem semantics:

### 1. Deterministic Cache-Key Generation
- Computations with identical normalized inputs, commands, arguments, environment, platform, and tool identity produce the exact same SHA-256 `CacheKey`.
- Sorting of dictionary keys and paths eliminates memory layout and insertion-order variance.

### 2. Atomic Cache Writes
- Objects and entries are written to temporary staging files (`.tmp`), flushed, synced to disk (`fsync`), and atomically renamed into their final paths.
- Partially written, interrupted, or crashed writes never result in corrupted valid-looking entries.

### 3. Corruption Detection & Quarantine
- Every stored CAS object and entry metadata record is validated against its cryptographic SHA-256 digest before use.
- Tampered or corrupted cache artifacts are immediately detected, quarantined/rejected, and safely bypassed.

### 4. Cache Miss Fallback
- When a cache lookup fails, when objects are missing, or when integrity checks fail, the engine gracefully falls back to direct command execution.
- Cache failure never crashes or breaks the user's build process.

### 5. Safe Output Restoration
- Restored outputs are sanitized against path traversal (`../`, `..\\`) and absolute path escapes.
- Restored files are verified against the declared output manifest before completing execution.

### 6. Concurrent Access Safety
- Multiple readers can simultaneously read cached objects without locking contention.
- Multi-process write races and duplicate cold computations on the same key are protected via file locks (`fs2`), preventing race conditions and duplicated work.

### 7. Cache Integrity Verification
- Built-in verification utilities (`dcc verify`) audit every object in the CAS and report any bit-rot or tampering.

### 8. Explainable Cache Misses
- Detailed miss taxonomy explains exactly why a computation missed (e.g., input changed, command args modified, environment variable updated, tool version mismatch).

---

## 2. What DCC Does NOT Guarantee (Non-Guarantees)

DCC operates strictly on declared inputs and pure computation assumptions. The following cannot be guaranteed:

### 1. Arbitrary Nondeterministic Programs
- Programs that embed random numbers, random UUIDs, or unseeded pseudo-random generators without recording them in declared inputs.

### 2. Undeclared External State
- Programs whose behavior depends on undeclared files, unmanaged system files, global registries, or external databases.

### 3. Network Responses
- External network requests or remote API endpoints unless explicitly fetched, saved to a declared input file, and hashed.

### 4. Hidden System State
- Dynamic kernel states, hardware clock ticks, CPU core count adaptations, or undeclared system configuration files.

### 5. Time-Dependent Computations
- Commands that embed the current wall-clock timestamp into output files unless that timestamp is part of the computation identity.

### 6. Secrets Embedded in Cache Outputs
- DCC does not provide automated secret redaction. Callers must avoid storing sensitive tokens or private credentials in declared output artifacts.
