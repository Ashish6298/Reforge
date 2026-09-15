# Cache Identity Specification

## Overview

Cache identity establishes the mathematical and cryptographic definition of equivalence between two developer computations.

If and only if two computations share an identical `CacheKey`, they are considered semantically equivalent, and the previous execution result may be reused safely without executing the tool again.

---

## 1. Computation Equivalence Formula

Two computations $C_1$ and $C_2$ are equivalent ($C_1 \equiv C_2$) if and only if all of the following components match identically:

```text
  same operation identifier
+ same command / executable
+ same arguments (ordered sequence)
+ same declared input files (normalized relative paths + SHA-256 digests)
+ same relevant environment variables (key-value pairs)
+ same tool identity (name + version + optional binary hash)
+ same platform constraints (os + arch + target)
= same computation identity (CacheKey)
```

---

## 2. Deterministic Serialization & Hashing Pipeline

To guarantee that compiler memory layout differences, field ordering variations, or debug formatting cannot alter the identity, `dcc` employs a 3-step normalization and canonicalization pipeline:

```text
┌───────────────────────────────┐
│     Runtime Computation       │
└───────────────┬───────────────┘
                │
                ▼
┌───────────────────────────────┐
│       1. Normalization        │
│  - Path normalization ('/')   │
│  - Lexicographical input sort │
│  - Lexicographical output sort│
│  - Key-sorted environment map │
└───────────────┬───────────────┘
                │
                ▼
┌───────────────────────────────┐
│  2. Canonical JSON Encoding   │
│     (Ordered Canonical Schema)│
└───────────────┬───────────────┘
                │
                ▼
┌───────────────────────────────┐
│     3. SHA-256 Hash Digest    │
│      CacheKey(Hex String)     │
└───────────────────────────────┘
```

---

## 3. Canonical JSON Schema Representation

```json
{
  "schema_version": 1,
  "operation": "codegen",
  "command": "generator",
  "args": ["schema.json", "--language", "rust"],
  "inputs": [
    {
      "path": "schema.json",
      "digest": "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
      "size": 1024
    }
  ],
  "outputs": [
    {
      "path": "generated/models.rs",
      "required": true
    }
  ],
  "env": {
    "GENERATOR_VERSION": "1.4.0",
    "NODE_ENV": "production"
  },
  "platform": {
    "os": "windows",
    "arch": "x86_64",
    "target": null
  },
  "tool": {
    "name": "generator",
    "version": "1.4.0",
    "digest": null
  }
}
```

The resulting computation key is computed as:

$$\text{CacheKey} = \text{SHA-256}(\text{CanonicalJSON})$$

---

## 4. Key Invariants

1. **Map Invariance**: Insertion order of environment variables or metadata produces identical canonical JSON because maps are sorted lexicographically by key (`BTreeMap`).
2. **Path Delimiter Invariance**: Input and output paths using Windows backslashes (`\`) or POSIX slashes (`/`) normalize to identical forward-slash paths before hashing.
3. **No Hidden State**: Ambient system state, time, or undeclared directories are never implicitly hashed.
