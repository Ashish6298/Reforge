# Troubleshooting & Diagnostic Guide

This guide helps you troubleshoot unexpected behavior, unexpected cache misses, storage corruption, and concurrency issues in DCC.

---

## 1. Fast Self-Diagnostics (`dcc doctor`)

Run `dcc doctor` to perform an automated health check on your environment:

```bash
dcc doctor
```

**Checks performed:**
- Cache root path and write permissions.
- Disk space availability vs configured `max_size`.
- Lock manager functionality and lock directory permissions.
- Integrity verification of metadata entries and CAS blobs.

---

## 2. "Why Did I Get a Cache Miss?"

If a computation re-runs when you expected a cache hit, run with `--explain`:

```bash
dcc run --explain --input src/schema.json --output generated/models.rs -- generator src/schema.json
```

### Common Root Causes & Fixes:

### 1. `Reason: input changed`
- **Cause**: The cryptographic content digest of one of your declared input files changed.
- **Diagnostics**: `dcc` prints the exact file path and the previous vs current SHA-256 hashes.
- **Fix**: Check if a build tool or formatter modified the file, or if dynamic timestamps were embedded.

### 2. `Reason: command arguments changed`
- **Cause**: Flags, arguments, or argument ordering differed between runs.
- **Fix**: Ensure your build script passes identical, deterministically ordered arguments.

### 3. `Reason: relevant environment changed`
- **Cause**: A declared environment variable (e.g. `RUSTFLAGS`, `DEBUG`, `TARGET`) has a different value.
- **Fix**: Check your shell environment or CI runner configuration.

### 4. `Reason: tool identity changed`
- **Cause**: The compiler or tool executable was upgraded or rebuilt on disk (executable SHA-256 binary hash mismatch).
- **Fix**: This is expected behavior to prevent using stale binary outputs from older compiler versions.

---

## 3. Storage Corruption & Quarantine

If you encounter `IntegrityError` or corrupted entries:

```bash
# 1. Cryptographically verify all cache entries and objects
dcc verify

# 2. Inspect quarantine logs:
# Corrupted objects are automatically renamed with a *.corrupted extension
ls -la .dcc_cache/objects/*/*.corrupted

# 3. Clean corrupted entries or reset cache if necessary
dcc clean --key <corrupted_key>
# Or wipe entire cache:
dcc clean
```

---

## 4. Stale Locks or Concurrency Timeouts

If a process was abruptly killed (`SIGKILL` or power failure) while holding a computation lock:

1. **Kernel Automatic Release**: OS-level kernel locks are released immediately on process termination.
2. **Stale Lock Overwriting**: Subsequent runs detect dead PIDs and reclaim the lock automatically.
3. **Manual Stale Lock Cleanup**:
   ```bash
   # Remove orphaned locks in .dcc_cache/locks/
   rm -f .dcc_cache/locks/*.lock
   ```

---

## 5. Path Traversal or Sandbox Errors

If `dcc run` fails with `PathTraversal` or `InvalidPath`:
- Verify that your declared input and output paths are **relative to the workspace root**.
- Ensure paths do not contain `../` sequences that resolve outside the workspace directory.
- Avoid absolute paths (e.g. `/tmp/out` or `C:\output.bin`).
