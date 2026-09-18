# CI/CD Integration & Provider-Neutral Caching Guide

## Milestone 16.3 — Developer Computation Cache (`dcc`) CI Workflow Integration

---

## 1. Overview

This document provides a reference architecture and concrete implementation examples for integrating the **Developer Computation Cache (`dcc`)** into Continuous Integration (CI) pipelines.

The caching lifecycle follows a standard, provider-neutral pattern:
```text
  1. Checkout Code
         ↓
  2. Restore Cache (DCC cache directory)
         ↓
  3. Execute Build / Test / Codegen steps via DCC (`dcc run ...`)
         ↓
  4. Inspect Cache Metrics & Miss Explanations (`dcc stats`, `dcc explain`)
         ↓
  5. Save / Store Cache back to CI Cache Storage
```

---

## 2. GitHub Actions Reference Workflow

Below is a complete, provider-ready GitHub Actions workflow demonstrating how DCC accelerates build steps and tests with persistent cache restore/save actions:

```yaml
name: DCC Accelerated CI Pipeline

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

env:
  CARGO_TERM_COLOR: always
  DCC_CACHE_DIR: ${{ github.workspace }}/.dcc_cache

jobs:
  build-and-test:
    name: Build & Test with DCC
    runs-on: ubuntu-latest

    steps:
      # Step 1: Checkout Repository
      - name: Checkout repository
        uses: actions/checkout@v4

      # Step 2: Setup Toolchains
      - name: Install Rust Toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Build DCC CLI
        run: cargo build --release --bin dcc

      # Step 3: Restore DCC Cache
      - name: Restore DCC Cache
        uses: actions/cache/restore@v4
        id: dcc-cache
        with:
          path: ${{ env.DCC_CACHE_DIR }}
          key: dcc-${{ runner.os }}-${{ hashFiles('**/Cargo.lock', '**/build.rs') }}
          restore-keys: |
            dcc-${{ runner.os }}-

      # Step 4: Run Computations & Builds via DCC
      - name: Run Cached Code Generation
        run: |
          ./target/release/dcc run \
            --cache-dir "${{ env.DCC_CACHE_DIR }}" \
            --input "schema.json" \
            --output "src/generated.rs" \
            -- python scripts/generate_models.py

      - name: Run Cached Compilation & Tests
        run: |
          ./target/release/dcc run \
            --cache-dir "${{ env.DCC_CACHE_DIR }}" \
            --input "src/**" \
            --input "Cargo.toml" \
            --output "target/debug/my_app" \
            -- cargo build --bin my_app

      # Step 5: Inspect Cache Efficiency
      - name: Cache Statistics & Hit Rate
        if: always()
        run: |
          ./target/release/dcc stats --cache-dir "${{ env.DCC_CACHE_DIR }}"

      # Step 6: Save DCC Cache (Only on main branch / non-PR if desired)
      - name: Save DCC Cache
        uses: actions/cache/save@v4
        if: always()
        with:
          path: ${{ env.DCC_CACHE_DIR }}
          key: dcc-${{ runner.os }}-${{ hashFiles('**/Cargo.lock', '**/build.rs') }}-${{ github.run_id }}
```

---

## 3. Provider-Neutral Integration Models

DCC works identically across all modern CI providers:

### GitLab CI (`.gitlab-ci.yml`)
```yaml
dcc_build_job:
  stage: build
  cache:
    key: dcc-$CI_COMMIT_REF_SLUG
    paths:
      - .dcc_cache/
  script:
    - cargo build --release --bin dcc
    - ./target/release/dcc run --cache-dir .dcc_cache --input "src/**" --output "dist/bundle" -- npm run build
    - ./target/release/dcc stats --cache-dir .dcc_cache
```

### Generic Docker / Container Runner (`entrypoint.sh`)
```bash
#!/usr/bin/env bash
set -euo pipefail

CACHE_DIR="${CACHE_DIR:-/mnt/cache/dcc}"
mkdir -p "${CACHE_DIR}"

echo "Executing build via DCC..."
dcc run --cache-dir "${CACHE_DIR}" --input "src/**" --output "bin/app" -- make build
dcc stats --cache-dir "${CACHE_DIR}"
```

---

## 4. Operational Invariants

1. **Non-Blocking Resilience**: If the CI runner experiences cache restore failure, network timeouts, or missing blobs, DCC executes computations locally without failing the job (Milestone 16.2).
2. **Deterministic Inputs**: CI should declare input files (`--input <path>`) and environment dependencies (`--env <KEY>`) to ensure maximum cache hit reuse across builds.
3. **Multi-Platform Support**: Cross-platform path normalization ensures cache entries remain reproducible across Windows, Linux, and macOS.
