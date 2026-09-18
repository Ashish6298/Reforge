# Security & Threat Model

DCC treats cached artifacts as **untrusted data**. A computation caching engine must never blindly execute commands or extract files without strict cryptographic and boundary validation.

---

## 1. Security Invariants

1. **Integrity Invariant**: No unverified or tampered bytes may ever be returned from cache or extracted into a workspace.
2. **Containment Invariant**: Cached outputs must never create, modify, or delete files outside the target workspace directory.
3. **Identity Invariant**: A metadata record must mathematically prove that its embedded computation hashes exactly to the cache key under which it is indexed.
4. **Secret Invariant**: Authentication credentials, private keys, and sensitive tokens must never be inadvertently persisted into cache metadata.

---

## 2. Cache Poisoning Defense

DCC enforces multi-layered cryptographic verification across all storage and restoration pipelines:

```text
       [ Cached Entry JSON ]
                │
         verify_identity()  ──► FAILS? ──► REJECT (Forged metadata / Key spoofing prevented)
                │
                ▼
       [ CAS Object on Disk ]
                │
          verify_object()   ──► FAILS? ──► QUARANTINE to .corrupted & REJECT (Malicious blob isolated)
                │
                ▼
       [ Atomic Staging .tmp ]
                │
          verify_stream()   ──► FAILS? ──► ROLLBACK & CLEAN (Destination workspace untouched)
                │
                ▼
       [ Restored Artifact ]
```

### 2.1 Metadata Identity Verification
When an entry is loaded from disk, `entry.verify_identity()` recalculates the canonical SHA-256 key from its embedded `Computation` record. If an attacker manually alters the JSON (e.g. changing the command or output digest while retaining the key), the key mismatch is detected immediately and the entry is rejected.

### 2.2 CAS Hash Verification & Quarantine
Every referenced CAS blob is hashed before and during extraction. If the SHA-256 digest does not match the manifest entry, the file is renamed with a `.corrupted` extension and isolated, triggering a fallback recomputation.

---

## 3. Path Traversal Defense

DCC prevents malicious or malformed computations from writing files outside the intended workspace boundary:

```text
       [ Manifest Output Path ]
                 │
       PathUtils::sanitize_relative_path()
                 ├── Starts with / or C:\ ?   ──► REJECT (Absolute paths forbidden)
                 ├── Starts with \\ or // ?   ──► REJECT (UNC network shares forbidden)
                 ├── Injected \0 byte ?       ──► REJECT (Null byte injection forbidden)
                 └── Depth check (.. escapes) ──► REJECT (Parent directory traversal forbidden)
                 │
                 ▼
       [ Normalized Safe Workspace-Relative Path ]
                 │
       OutputRestorer::restore_entry()
                 ▼
       [ Atomic Extraction to Workspace Subdirectory ]
```

- **Parent Traversal (`..`)**: All path components are inspected. Any path attempting to escape upward past the workspace root is rejected with `CacheError::PathTraversal`.
- **Absolute Paths & Drives**: Paths starting with `/`, `C:\`, `D:\`, or UNC shares (`\\server\share`) are rejected.
- **Null Byte Injections**: Injected `\0` bytes are caught and rejected during sanitization.

---

## 4. Symlink Attack Defense

A malicious cache entry or repository must not be able to cause arbitrary file overwrite through a pre-existing or created symlink:

```text
       [ Output Restoration Target Path ]
                      │
       PathUtils::sanitize_relative_path()
                      │
       ├── verify_symlink_safety():
       │   Inspects intermediate directory components.
       │   Resolves canonical symlink targets.
       │   Target escapes workspace root? ──► REJECT (PathTraversal error)
       │
       └── safe_prepare_target_path():
           Is target already an existing symlink?
           ├── YES: Unlinks symlink node directly (never follows into external file)
           └── NO:  Cleans regular file before atomic rename
                      │
                      ▼
       [ Atomic Rename: Extracted Payload Safely Replaces Link Node ]
```

1. **Intermediate Directory Symlink Check**: `verify_symlink_safety` checks every component along the path. If an intermediate directory is a symlink pointing outside the workspace, restoration is aborted.
2. **Pre-Existing Target Symlink Removal**: If a destination file path is an existing symlink pointing to an external file (e.g. `/etc/passwd`), `safe_prepare_target_path` removes the symlink node itself rather than opening or writing through it.

---

## 5. Sensitive Information Protection

To prevent accidental leakage of secrets across team members or CI environments, DCC provides secret scanning and policy enforcement:

```rust
use dcc_core::{SensitiveDataDetector, SensitiveDataPolicy};

// Detect sensitive keys (PASSWORD, TOKEN, API_KEY, SSH_KEY, etc.)
// and values (PEM headers, GitHub/GitLab tokens, DB URIs)
let detector = SensitiveDataDetector::new();
```

### Configurable Policies (`SensitiveDataPolicy`)
- **`Allow`**: Default caller-responsible mode.
- **`Warn`**: Logs warnings to stderr when credentials or private keys are detected in arguments or environment variables.
- **`Deny`**: Strictly rejects caching computations containing detected credentials.
- **`Mask`**: Redacts detected secret values to `[REDACTED]` prior to key computation and metadata persistence.

---

## 6. Layered Trust Modes (`TrustMode`)

DCC supports different levels of trust for local vs shared/remote cache sources:

- **`TrustMode::TrustedLocal`**: Optimized local development mode with standard integrity checks.
- **`TrustMode::Untrusted`**: Strict mode for unauthenticated or third-party cache sources. Requires multi-pass cryptographic verification and strict manifest audits.
- **`TrustMode::ReadOnly`**: Safe shared consumption mode; strictly forbids writes or mutations to cache storage.
