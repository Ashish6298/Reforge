# Continuous Integration & Automated Pipelines (`ci.md`)

This guide explains how to integrate DCC into CI/CD workflows (GitHub Actions, GitLab CI, Jenkins, and Container Runners) to accelerate automated builds and test suites.

---

## 1. Core CI Workflow Pattern

Caching in CI follows a standard, provider-neutral 5-step pipeline:

```text
1. Checkout source code
          ↓
2. Restore cache (.dcc_cache/) from CI storage
          ↓
3. Run computation via dcc (dcc run ...)
          ↓
4. Run test suites / downstream builds
          ↓
5. Store updated cache back to CI storage
```

---

## 2. CI Behavioral Matrix (4 States)

DCC handles all operational states robustly in automated CI runners:

| CI State | Condition | DCC Behavior | Pipeline Impact |
| :--- | :--- | :--- | :--- |
| **1. Cache Available** | Warm cache hit | Restores output artifacts in $0\text{ ms}$; skips process execution. | Massive time savings (5x–50x speedup). |
| **2. Cache Unavailable** | Cold cache / clean runner | Executes computation normally; ingests outputs into `.dcc_cache/`. | Baseline execution time; builds cache. |
| **3. Cache Corrupted** | Tampered or broken object | Detects hash mismatch, quarantines object, and falls back to clean execution. | Build completes safely (no pipeline failure). |
| **4. Cache Partially Available** | Incomplete cache fetch | Identifies missing CAS blobs during restoration and falls back to execution. | Build completes safely (no pipeline failure). |

---

## 3. Graceful Degradation Guarantee

> **A cache failure must NEVER cause a CI build to fail.**

If the cache directory is unwritable, a CAS object is corrupted, or a network storage transfer fails:
1. DCC logs a diagnostic warning to stderr.
2. DCC transparently executes the command directly as if the cache were cold.
3. The build and test steps complete normally with exit code 0.

---

## 4. GitHub Actions Reference Workflow

Create `.github/workflows/ci.yml`:

```yaml
name: CI with DCC Cache

on:
  push:
    branches: [ main ]
  pull_request:
    branches: [ main ]

jobs:
  build-and-test:
    name: Build & Test (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]

    steps:
      - name: 1. Checkout Repository
        uses: actions/checkout@v4

      - name: 2. Setup Rust Toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: 3. Restore DCC Cache
        uses: actions/cache/restore@v4
        id: restore-dcc-cache
        with:
          path: .dcc_cache
          key: dcc-cache-${{ runner.os }}-${{ hashFiles('**/Cargo.lock', '**/schema.json') }}
          restore-keys: |
            dcc-cache-${{ runner.os }}-

      - name: 4. Build DCC Engine
        run: cargo build --release --bin dcc

      - name: 5. Execute Cached Computations
        run: |
          ./target/release/dcc run \
            --input src/schema.json \
            --output generated/models.rs \
            --explain \
            -- cargo run --bin codegen -- src/schema.json

      - name: 6. Run Workspace Test Suite
        run: cargo test --workspace

      - name: 7. Save Updated DCC Cache
        uses: actions/cache/save@v4
        if: always()
        with:
          path: .dcc_cache
          key: dcc-cache-${{ runner.os }}-${{ hashFiles('**/Cargo.lock', '**/schema.json') }}-${{ github.run_id }}
```

---

## 5. GitLab CI Reference Pipeline

Add to `.gitlab-ci.yml`:

```yaml
stages:
  - build

dcc-build:
  stage: build
  image: rust:latest
  cache:
    key: "dcc-$CI_COMMIT_REF_SLUG"
    paths:
      - .dcc_cache/
    policy: pull-push
  script:
    - cargo build --release --bin dcc
    - ./target/release/dcc run --input src/schema.json --output generated/models.rs -- cargo run --bin codegen
    - cargo test --workspace
```
