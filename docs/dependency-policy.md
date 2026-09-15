# Dependency Policy & Architecture Specification

## Overview

The Developer Computation Cache (`dcc`) enforces a strict, minimalist dependency footprint. Every third-party dependency introduced into the workspace must pass an explicit 4-point vetting evaluation.

---

## 1. The 4-Point Dependency Vetting Checklist

Before adding any dependency to `Cargo.toml`, the engineering team must document answers to:

1. **Is it necessary?** Does it solve a genuine domain requirement that would otherwise require complex, security-sensitive, or error-prone custom implementations (e.g. SHA-256 cryptographic primitives, standard JSON serialization, cross-platform file locking)?
2. **Is it maintained?** Is the crate widely adopted, actively maintained by reputable community teams, with regular security audits and minimal transitive dependencies?
3. **Is it lightweight?** Does it compile quickly without bloating binary sizes or pulling deep, unneeded dependency subtrees?
4. **Can it be implemented safely without it?** If the functionality is trivial (e.g. basic string manipulation or simple path helpers), it must be implemented directly in internal crates rather than adding third-party dependencies.

---

## 2. Approved Dependency Registry (v1.0)

| Category | Crate | Purpose | Justification |
| :--- | :--- | :--- | :--- |
| **Hashing** | `sha2` (v0.10) | Standard SHA-256 cryptographic content hashing | Audited, pure Rust, zero unsafe C bindings, fast streaming API. |
| **Hashing** | `hex` (v0.4) | Fast hex encoding & decoding for digests and keys | Ultra-lightweight, zero transitive dependencies. |
| **Serialization** | `serde` & `serde_json` | Deterministic canonical JSON serialization | Standard Rust ecosystem serialization with `preserve_order` support. |
| **Time** | `chrono` (v0.4) | RFC 3339 / UTC timestamp handling for cache entry metadata | Stable time representation for LRU timestamps. |
| **Filesystem / Locking** | `fs2` (v0.4) | Cross-platform OS advisory file locking | Reliable Windows (`LockFileEx`) and POSIX (`flock`) process synchronization. |
| **Filesystem / Traversal** | `walkdir` (v2.4) | Robust directory traversal for CAS and cache entries | Safe recursive directory walking with cycle detection. |
| **CLI Parsing** | `clap` (v4.4) | Declarative command-line argument parser | Type-safe CLI parsing with shell completion and help generation. |
| **Error Handling** | `thiserror` (v1.0) | Structured error definitions in library crates | Zero-runtime-overhead derive macro for typed library errors. |
| **Error Handling** | `anyhow` (v1.0) | Error context handling in binary applications | Idiomatic error handling in CLI binary layer. |
| **Test Utilities** | `tempfile` (v3.10) | RAII-based temporary directories for tests | Ensures complete cleanup of temporary cache folders in tests. |

---

## 3. Dependency Prohibitions

- **No Network / Async Runtime in Core**: No `tokio`, `hyper`, or `reqwest` in `cache-core` or `cache-storage`. The core engine must remain synchronous, fast, and local-first.
- **No Unstable / Heavy Crypto**: Avoid complex, unmaintained cryptographic suites. Standard SHA-256 satisfies all content-addressing and integrity requirements.
- **No Heavy Shell Wrappers**: Rely on standard library `std::process::Command` rather than complex shell abstraction layers.
