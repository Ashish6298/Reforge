# CLI Reference (`dcc`)

The Developer Computation Cache provides a powerful, consistent command-line interface designed for both interactive developer usage and automated CI scripts.

---

## 1. Global Options

| Option | Description |
| :--- | :--- |
| `--json` | Emit machine-readable JSON output for script and CI consumption |
| `-v`, `--verbose` | Enable verbose debug logging of caching lifecycle stages |
| `-h`, `--help` | Print help information |
| `-V`, `--version` | Print version information |

---

## 2. Command Index

| Command | Description |
| :--- | :--- |
| [`dcc init`](#dcc-init) | Initialize local cache directory structure and configuration |
| [`dcc run`](#dcc-run) | Execute a computation with transparent content-addressed caching |
| [`dcc inspect`](#dcc-inspect) | Inspect stored computation metadata and I/O manifests |
| [`dcc stats`](#dcc-stats) | Display storage metrics, hit ratios, and cumulative time savings |
| [`dcc verify`](#dcc-verify) | Cryptographically audit CAS objects and metadata records |
| [`dcc clean`](#dcc-clean) | Wipe all cached entries or delete a specific computation key |
| [`dcc prune`](#dcc-prune) | Garbage collect unreferenced blobs and enforce max size limits |
| [`dcc config`](#dcc-config) | Query or inspect active cache configuration parameters |
| [`dcc doctor`](#dcc-doctor) | Run environment and filesystem health diagnostics |

---

## 3. Command Details

### `dcc init`

Initializes the `.dcc_cache/` directory and creates `config.json`.

```bash
dcc init [--max-size <limit>] [--json]
```

**Options:**
- `--max-size <limit>`: Set cache size limit (e.g. `500 MB`, `2 GB`, `10 GB`). Default: `10 GB`.

---

### `dcc run`

Executes a child process with content-addressed caching.

```bash
dcc run [OPTIONS] -- <COMMAND> [ARGS...]
```

**Options:**
- `-i`, `--input <PATH>`: Declare an input file or directory (repeatable).
- `-o`, `--output <PATH>`: Declare an expected output file or directory (repeatable).
- `-e`, `--env <KEY=VAL>`: Declare an environment variable affecting cache identity (repeatable).
- `--explain`: Print root-cause explanation if a cache miss occurs.
- `--policy <read-write|read-only|write-only|bypass|force-recompute>`: Set cache execution policy. Default: `read-write`.
- `--json`: Output execution results in structured JSON.

**Example:**
```bash
dcc run \
  --input src/schema.json \
  --output generated/models.rs \
  --env CODEGEN_OPT=2 \
  --explain \
  -- cargo run --bin codegen -- src/schema.json
```

---

### `dcc inspect`

Inspects detailed computation metadata, input digests, and output manifests.

```bash
dcc inspect <CACHE_KEY> [--json]
```

---

### `dcc stats`

Displays cache efficiency, storage utilization, and cumulative time savings.

```bash
dcc stats [--json]
```

---

### `dcc verify`

Performs an exhaustive cryptographic audit of all CAS blobs and metadata entries.

```bash
dcc verify [--json]
```

---

### `dcc clean`

Removes cached entries and objects.

```bash
# Wipe entire cache
dcc clean [--json]

# Remove specific computation key
dcc clean --key <CACHE_KEY> [--json]
```

---

### `dcc prune`

Garbage collects unreferenced CAS objects and evicts entries to enforce size limits.

```bash
dcc prune [--max-size <limit>] [--strategy <lru|fifo|lfu>] [--dry-run] [--json]
```

**Options:**
- `--max-size <limit>`: Target size limit to enforce.
- `--strategy <lru|fifo|lfu>`: Eviction strategy. Default: `lru`.
- `--dry-run`: Preview reclaimable bytes without deleting files.

---

### `dcc doctor`

Performs end-to-end diagnostics on the host environment:
- Cache directory writability and permissions.
- Disk space availability.
- Lock manager functionality.
- Integrity scan of active entries.

```bash
dcc doctor [--json]
```

---

## 4. Exit Codes Reference

| Exit Code | Identifier | Description |
| :---: | :--- | :--- |
| `0` | `Success` | Clean execution, successful cache HIT, or valid maintenance operation |
| `1` | `ComputationFailed` | Target process exited with non-zero exit code, or key not found |
| `2` | `InvalidConfiguration` | Invalid configuration parameters or unparseable max size string |
| `3` | `CacheError` | Storage system I/O error or permission denied |
| `4` | `IntegrityFailure` | Corrupted CAS object detected, hash mismatch, or invalid metadata |
| `5` | `InvalidArguments` | Missing required CLI arguments or unrecognized options |
