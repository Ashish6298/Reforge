# Structured Logging & Observability Specification

## Overview

The Developer Computation Cache (`dcc`) avoids relying solely on unstructured verbose stdout logging. Instead, it defines a structured, machine-parsable telemetry and event pipeline.

---

## 1. Structured Event Taxonomy

| Event Name | Schema Type | Description |
| :--- | :--- | :--- |
| `cache.lookup` | `CacheLookup` | Emitted when querying CAS for a given computation key |
| `cache.hit` | `CacheHit` | Emitted when a valid entry is found and outputs are validated |
| `cache.miss` | `CacheMiss` | Emitted when no entry exists or previous entry was invalidated |
| `cache.store` | `CacheStore` | Emitted when outputs and metadata are written to CAS |
| `cache.restore` | `CacheRestore` | Emitted when cached artifacts are restored to workspace |
| `cache.delete` | `CacheDelete` | Emitted when an entry or object is deleted |
| `cache.verify` | `CacheVerify` | Emitted during CAS checksum integrity checks |
| `computation.start` | `ComputationStart` | Emitted before spawning the underlying process |
| `computation.finish` | `ComputationFinish` | Emitted when the child process terminates successfully |
| `computation.failed` | `ComputationFailed` | Emitted when the child process exits with non-zero status |

---

## 2. Event Payload Schema

```json
{
  "timestamp": "2026-09-15T10:45:00.123456Z",
  "event": "cache.hit",
  "key": "c3699bf649f369ebf459c275557429e3ee9e8ec29904732d062767655c1b6b9f",
  "operation": "codegen",
  "duration_ms": 14,
  "message": "Restored 2 outputs safely from CAS",
  "fields": {
    "output_count": "2",
    "total_bytes": "1048576"
  }
}
```

---

## 3. Observability Architecture

- **Subscribers (`EventSubscriber`)**: Decoupled trait allowing programmatic tools, CI monitors, and telemetry bridges to observe events without changing computation logic.
- **Machine-Readable CLI (`--json`)**: All CLI commands output JSON payloads compatible with log collectors.
- **Human-Readable Diagnostics**: `dcc run --explain` provides human explanations on top of structured telemetry.
