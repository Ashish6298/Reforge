# Security Model Specification

## Overview

The Developer Computation Cache (`dcc`) treats cached computation outputs, stored blobs, and user-provided paths as potentially hazardous operations that require strict defense-in-depth sanitization, integrity verification, and execution isolation.

---

## 1. Threat Vectors & Mitigations

### 1.1 Path Traversal & Escapes
- **Threat**: Malicious or malformed cache metadata specifying output paths such as `../../Windows/System32/evil.dll` or `/etc/shadow`.
- **Mitigation**: Strict pre-execution and pre-restoration path normalization. Any output path containing relative directory escapes (`../`, `..\\`), leading root slashes (`/`, `\`), Windows drive letters (`C:`), or paths resolving outside the designated working directory boundary is rejected with `CacheError::PathTraversal`.

### 1.2 Cache Poisoning & Bit-Rot Corruption
- **Threat**: Tampered or corrupted Content-Addressed Storage (CAS) objects being restored to disk.
- **Mitigation**: Mandatory SHA-256 cryptographic verification before any CAS object is read or copied to the output destination. If an actual digest does not match the manifest digest, it is immediately quarantined and rejected with `CacheError::IntegrityMismatch`.

### 1.3 Symlink Attacks
- **Threat**: Output paths pointing to pre-existing symlinks designed to overwrite arbitrary files outside the project root.
- **Mitigation**: Atomic replacement of target output files. Restoration writes to an isolated `.tmp_restore` file and performs an atomic rename, preventing symlink follower overwrites.

### 1.4 Command Execution Safety
- **Threat**: Shell injection vulnerabilities from maliciously constructed argument strings.
- **Mitigation**: Process execution uses direct OS APIs (`std::process::Command`) passing argument arrays directly to `CreateProcess` / `execve`, avoiding raw shell string concatenation (`cmd.exe /c` or `sh -c`).

### 1.5 Multi-Process Race Conditions
- **Threat**: Multiple concurrent processes modifying the same cache entry or CAS blob simultaneously.
- **Mitigation**: Two-stage atomic write commits (write to `.tmp` $\rightarrow$ `fsync` $\rightarrow$ atomic rename) combined with advisory file locking (`fs2`).

---

## 2. Sensitive Data Responsibility
- DCC operates locally without automatic secret scanning. Developers and CI workflows are advised to exclude secret-bearing files (e.g., `.env`, credentials, private keys) from declared cached outputs.
