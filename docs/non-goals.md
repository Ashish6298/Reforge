# Product Scope & Non-Goals Specification (v1.0)

## Objective & Focus

The primary purpose of **DCC (Developer Computation Cache)** in v1 is to provide a local-first, content-addressed developer computation caching engine that deterministically caches and restores the results of expensive operations (build steps, code generators, transpilations, test runs, and CLI tasks).

To maintain engineering focus and system reliability, the project explicitly defines the following boundaries and non-goals for v1.

---

## Explicit Non-Goals for v1.0

### 1. Not a Remote Build Farm
- DCC v1 does not coordinate distributed worker fleets or balance execution across remote machines. It focuses on single-host local developer machines and CI runner environments.

### 2. Not a Distributed Execution Engine
- DCC v1 executes computations directly on the local machine where the CLI/library runs, rather than dispatching tasks across a cluster or RPC grid.

### 3. Not a Docker Replacement
- DCC does not provide OS-level containerization, cgroups isolation, namespace sandboxing, or filesystem overlay layers. It manages computation artifacts based on declared input/output boundaries.

### 4. Not a Package Manager
- DCC does not resolve version graphs, download external dependencies from registries (like crates.io or npm), or manage dependency lockfiles.

### 5. Not a Full Bazel / Buck Replacement
- DCC does not define a multi-language graph build DSL, target query language, or fine-grained compiler dependency graph parser. It is a universal caching primitive that developer tools and build systems can integrate with.

### 6. Not a Compiler
- DCC does not parse source code, produce machine code, or perform linking. It wraps existing compilers and developer tools.

### 7. Not a CI Platform
- DCC does not orchestrate CI pipelines, schedule workflows, manage runners, or report pull request statuses. It is designed to be invoked inside existing CI systems (GitHub Actions, GitLab CI, Buildkite).

### 8. Not a Cloud Service
- DCC core requires zero cloud subscriptions, centralized proprietary accounts, or mandatory network connectivity. It is completely local-first and self-contained.

### 9. Not a Build System
- DCC does not orchestrate dependency DAG execution or replace tools like `cargo`, `make`, `cmake`, or `ninja`. It acts as an execution wrapper and caching engine for individual computation tasks.

---

## Architectural Boundaries

```text
┌─────────────────────────────────────────────────────────┐
│                     Build System                        │
│          (Cargo, Make, Turbo, Custom Scripts)           │
└────────────────────────────┬────────────────────────────┘
                             │
                             ▼ Invokes computation task
┌─────────────────────────────────────────────────────────┐
│           Developer Computation Cache (DCC)             │
│   [Normalizer] -> [CAS Storage] -> [Atomic Restorer]   │
└────────────────────────────┬────────────────────────────┘
                             │ (On cache miss)
                             ▼
┌─────────────────────────────────────────────────────────┐
│                 OS Process Execution                    │
│            (rustc, protoc, esbuild, pytest)             │
└─────────────────────────────────────────────────────────┘
```
