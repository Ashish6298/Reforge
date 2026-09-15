# Workspace Invariants — Developer Computation Cache (`dcc`)

These invariants represent non-negotiable engineering laws for the `dcc` codebase. Every commit, PR, and release must strictly obey them.

## 1. Correctness Invariants
1. **Zero Semantic Shift**: A cache hit must never change the semantic result of any computation. If safety cannot be mathematically and cryptographically proven, the computation must execute.
2. **Deterministic Canonical Key**: The computation key is strictly `SHA-256(canonical_json(normalized_computation))`. Struct hashing must never rely on compiler-dependent memory layouts or debug representations. Map keys and inputs must be lexicographically sorted before key derivation.
3. **No Blind Environment Inclusion**: Only explicitly declared relevant environment variables or system-level constraints (`os`, `arch`) may influence cache identity. Undeclared environment variables must not pollute or fragment the cache key.
4. **No Failed Computation Caching**: By default, commands with non-zero exit codes or aborted processes must never be cached.

## 2. Storage & Integrity Invariants
5. **Two-Stage Atomic Commits**: All object writes in CAS (`objects/`) and entry writes (`entries/`) must write to a temporary file (`.tmp`), flush, fsync, and atomic-rename into place. Partially written files must never exist in valid paths.
6. **Mandatory Object Verification**: No cached object or output file may be restored without verifying its SHA-256 digest against its manifest. If a digest mismatch occurs, it is immediately flagged as `CorruptionError` and the entry is quarantined/rejected.
7. **Directory Sharding**: Objects in the CAS are sharded by the first 2 hex characters of their digest (e.g., `objects/a4/f89b...`) to avoid filesystem degradation.

## 3. Security & Isolation Invariants
8. **Strict Path Traversal Protection**: Outputs cannot be restored outside the declared working directory or destination tree. Any output containing relative path escapes (`../`, `..\\`) or absolute root redirects must be rejected with `PathTraversalError`.
9. **Safe Process Execution**: Process execution must pass arguments directly to the OS process API (`std::process::Command`), never through concatenated, unescaped shell strings.

## 4. Reliability & Concurrency Invariants
10. **Multi-Process Concurrency**: Multiple readers and writers must operate without deadlock or corruption. File locks and duplicate-execution guards must gracefully handle stale locks and process termination.
11. **Graceful Fallback**: Cache lookup/store errors or temporary I/O failures must never break the developer's build; the system falls back to direct execution when configured with a resilient policy.
