# Remote Cache Design Document (`docs/remote-cache.md`)

## Milestone 15.4 — Developer Computation Cache (DCC) Remote Protocol & Architecture Specification

---

## 1. Executive Summary & Objective

The **Developer Computation Cache (DCC)** is designed as a **local-first** computation caching engine. The local execution engine, Content-Addressable Storage (CAS), and metadata models are fully decoupled from storage transport details via the `Storage` and `TieredCache` abstractions (Milestones 15.1 – 15.3).

This document specifies the design, protocols, security model, and operational semantics for integrating **Remote Distributed Caches** into DCC without modifying core computation logic or local offline capabilities.

---

## 2. Protocol Possibilities

Remote caching in DCC separates two distinct operational planes:
1. **Action Cache (AC)**: Key-to-manifest mapping (`CacheKey` -> `CacheEntry` JSON metadata).
2. **Content-Addressable Storage (CAS)**: Immutable blob data addressed strictly by cryptographic digest (`Digest` / SHA-256).

### A. HTTP/REST & gRPC (Recommended Protocols)

| Protocol | Action Cache (AC) Endpoints | CAS Endpoints | Advantages | Tradeoffs |
| :--- | :--- | :--- | :--- | :--- |
| **HTTP/1.1 & HTTP/2 (REST)** | `GET /ac/{key}`<br>`PUT /ac/{key}`<br>`HEAD /ac/{key}` | `GET /cas/{digest}`<br>`PUT /cas/{digest}`<br>`HEAD /cas/{digest}` | Standard CDN compatibility, HTTP reverse proxies, simple TLS termination, chunked transfer encoding. | Higher header overhead for fine-grained small artifact calls. |
| **gRPC (Bazel REAPI Compatible)** | `GetActionResult`<br>`UpdateActionResult` | `BatchReadBlobs`<br>`BatchUpdateBlobs`<br>`ByteStream.Read / Write` | High-throughput multiplexing, native streaming, bi-directional batching, Bazel ecosystem alignment. | Requires HTTP/2 infrastructure and gRPC client tooling. |
| **S3 / Blob Storage Protocol** | S3 object keys under prefix `ac/{key}.json` | S3 object keys under prefix `cas/{digest}` | Direct integration with AWS S3, GCS, Cloudflare R2, MinIO; built-in lifecycle and tiering. | Requires pre-signed URLs or IAM credentials; metadata update consistency models. |

---

## 3. Authentication & Authorization

All remote cache communications must be authenticated and authorized:

```text
                  [ Client / CLI / CI Runner ]
                               │
            HTTPS + Authorization Header / mTLS
                               │
            ┌──────────────────┴──────────────────┐
            ▼                                     ▼
   [ Bearer Token / API Key ]             [ Mutual TLS (mTLS) ]
- Header: `Authorization: Bearer <token>`  - X.509 client certificates
- Scoped to Namespace & Role (Read/Write)  - Zero-trust network topologies
```

1. **Bearer Tokens & API Keys**:
   - Transmitted via standard `Authorization: Bearer <TOKEN>` HTTP headers or gRPC metadata.
   - Tokens carry role-based access control (RBAC):
     * `Role::ReadOnly`: Permitted only to query `HEAD` / `GET` on AC and CAS.
     * `Role::ReadWrite`: Permitted to query and publish `PUT` / `POST` entries and blobs.
     * `Role::Admin`: Permitted to delete, prune, or manage namespaces.
2. **Mutual TLS (mTLS)**:
   - For enterprise CI clusters, mTLS provides identity verification and channel encryption at the transport layer.
3. **Pre-Signed URLs**:
   - For direct cloud blob backends (S3/GCS), AC responds with short-lived (e.g., 15-minute) pre-signed upload/download URLs to bypass proxy bottlenecks.

---

## 4. Cryptographic Integrity & Anti-Poisoning

Remote caches are treated as **untrusted data sources** (Milestone 14.5 `TrustMode::Untrusted`).

```text
                     [ Remote Download Stream ]
                                 │
                     Compute Streaming SHA-256
                                 │
                 ┌───────────────┴───────────────┐
                 ▼                               ▼
     [ Matches Expected Digest ]     [ Hash Mismatch Detected ]
                 │                               │
       Write to Local CAS              Quarantine & Abort Download
                 │                               │
    Re-Verify Canonical Entry Key       Emit Integrity Alert to CI/CLI
```

1. **CAS Blob Verification**:
   - The remote storage server responds with raw bytes for a given `Digest`.
   - The client computes the SHA-256 digest on the incoming stream. If `actual_digest != requested_digest`, the download is immediately aborted and discarded.
2. **Action Cache Identity Verification**:
   - Before applying a remote `CacheEntry`, DCC re-computes `CanonicalComputation::from_computation(&entry.computation).compute_key()`.
   - If the canonical key does not match the AC key, the entry is rejected as poisoned metadata (`CacheError::IntegrityError`).
3. **Workspace Path Containment**:
   - All restored artifact paths are sanitized via `PathUtils::sanitize_relative_path` to prevent path traversal (`../../`) and symlink overwrite attacks (Milestones 14.2 & 14.3).

---

## 5. Upload & Download Flows

### Download / Cache Retrieval Flow
```text
Client                        Action Cache                   CAS Storage
  │                                 │                             │
  │─── 1. GET /ac/{key} ───────────►│                             │
  │◄── 2. 200 OK (CacheEntry) ──────│                             │
  │                                                               │
  │─── 3. Verify computation key matches declared key ───────────┐│
  │    (Fail -> abort)                                           ││
  │◄─────────────────────────────────────────────────────────────┘│
  │                                                               │
  │─── 4. Query CAS for output digests (Batch / Stream) ─────────►│
  │◄── 5. Stream Blobs with streaming SHA-256 verification ───────│
  │                                                               │
  │─── 6. Atomically restore into workspace ──────────────────────┘
```

### Upload / Cache Publication Flow
```text
Client                        Action Cache                   CAS Storage
  │                                 │                             │
  │─── 1. HEAD /cas/{digest} (Check missing blobs) ──────────────►│
  │◄── 2. 404 Not Found (Missing) ────────────────────────────────│
  │                                                               │
  │─── 3. PUT /cas/{digest} (Upload missing output blobs) ───────►│
  │◄── 4. 201 Created ────────────────────────────────────────────│
  │                                                               │
  │─── 5. PUT /ac/{key} (Publish Action Cache entry) ────────────►│
  │◄── 6. 200 OK ─────────────────────────────────────────────────│
```

---

## 6. Cache Namespaces & Multi-Tenancy

Namespaces isolate cache entries across teams, repositories, branches, and environments:

1. **Namespace Hierarchy**:
   - URL path format: `/{namespace}/ac/{key}` and `/{namespace}/cas/{digest}`
   - Example namespaces:
     * `org-core/ref-main/linux-x86_64`
     * `org-core/pull-requests/darwin-arm64`
     * `team-infra/staging`
2. **Cross-Namespace CAS Deduplication**:
   - While Action Cache mappings (`ac/{key}`) are strictly isolated per namespace, Content-Addressable Storage (`cas/{digest}`) can be globally deduplicated across namespaces because blobs are cryptographically content-addressed and immutable.

---

## 7. Versioning & Compatibility

1. **Schema Versioning**:
   - `CacheEntry` embeds `schema_version: u32` (current: `1`).
   - If a client encounters a remote entry with an unsupported schema version, it logs a warning and treats the lookup as a cache miss (`MissReason::CorruptedCache`).
2. **Protocol Negotiation**:
   - Clients send `X-DCC-Protocol-Version: 1.0` and `User-Agent: dcc/{version}` headers.
   - Servers advertise capabilities via `OPTIONS /` or `X-DCC-Capabilities: batch_get,batch_put,stream`.

---

## 8. Security & Secret Protection

1. **Sensitive Information Policy (Milestone 14.4)**:
   - Callers configure `SensitiveDataPolicy` (`Deny` / `Warn` / `Mask`).
   - The runner ensures passwords, authentication tokens (`ghp_`, `Bearer`), and private keys are never persisted in the Action Cache or CAS.
2. **Payload Encryption at Rest & in Transit**:
   - In-transit: Mandatory TLS 1.3 encryption.
   - At-rest: Server-side envelope encryption with customer-managed KMS keys.

---

## 9. Failure Behavior & Graceful Degradation

A remote cache failure must **never break a build**:

| Failure Scenario | Client Behavior | CI / Local Impact |
| :--- | :--- | :--- |
| **Connection Timeout** | Abort remote lookup after timeout (e.g. 500ms); fall through to local computation. | Transparent fallback (Cold Miss). |
| **5xx Server Error** | Log diagnostic warning to `stderr`; execute computation locally. | Build succeeds without remote hit. |
| **403 Forbidden on Upload** | Log warning; retain artifacts in local L1/L2 cache; do not fail build. | Build succeeds without remote store. |
| **Corrupted Blob / Digest Mismatch** | Quarantine downloaded blob; delete local corrupted entry; fall back to local computation. | Guaranteed correctness; build unaffected. |
| **Network Partition Mid-Transfer** | Discard temporary staging files; proceed to local execution. | Zero partial/corrupted files left in workspace. |

---

## 10. Conclusion

This specification provides the blueprints for implementing high-performance, secure, and resilient remote cache servers for DCC. The decoupled architecture developed across Milestones 15.1 – 15.3 ensures seamless pluggability whenever remote server deployments begin.
