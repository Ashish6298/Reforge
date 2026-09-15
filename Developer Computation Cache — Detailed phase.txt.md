# DEVELOPER COMPUTATION CACHE
## Detailed Implementation Roadmap — phase.txt

---

# 0. PROJECT PRINCIPLE

The project is a Rust-based, local-first developer computation caching engine.

The system should allow developers and developer tools to say:

> "I am about to perform this computation with these inputs, this command/tool, this environment, and these expected outputs. If this exact computation has already been performed safely, give me the previous result instead of doing the work again."

The project must NOT be designed only as a Rust build cache.

Build caching is one important application of the engine.

Other possible applications:

- code generation
- documentation generation
- image processing
- asset compilation
- test result caching
- static analysis
- schema generation
- API client generation
- transpilation
- formatting
- expensive CLI commands
- data transformation
- local developer automation
- CI computation reuse

The fundamental rule is:

> A cache hit must never change the semantic result of the computation.

If the system cannot prove that a cached result is safe to reuse, it must execute the computation again.

---

# MILESTONE 0 — PRODUCT DEFINITION & ENGINEERING CONTRACT

## Objective

Before writing implementation code, establish exactly what the system considers a computation, what can be cached, what cannot be cached, and what guarantees the engine provides.

This prevents the project from gradually turning into an unstructured "build cache CLI."

## 0.1 Define the Computation Model

Create a formal model for a computation.

A computation should contain at minimum:

- operation identifier
- command/tool identity
- arguments
- declared input files
- input content hashes
- declared output files
- environment variables relevant to execution
- working directory information
- platform information
- tool/runtime version information
- cache policy
- metadata

Conceptually:

```text
Computation
    ├── operation
    ├── command
    ├── arguments
    ├── inputs
    ├── environment
    ├── platform
    ├── tool identity
    ├── outputs
    └── policy
```

Do not immediately decide that every environment variable should be part of the key.

Define explicit rules for which environmental information is relevant.

---

## 0.2 Define Cache Identity

The agent must document exactly how two computations are considered equivalent.

Example:

```text
same operation
+ same command
+ same arguments
+ same input contents
+ same relevant environment
+ same tool identity
+ same platform constraints
= same computation
```

The exact serialization must be deterministic.

Do NOT hash an arbitrary Rust struct using an unstable/debug representation.

Instead:

1. normalize the computation
2. serialize it using a canonical format
3. hash the canonical representation

Example conceptual representation:

```json
{
  "version": 1,
  "operation": "codegen",
  "command": "generator",
  "args": ["schema.json", "--language", "rust"],
  "inputs": [
    {
      "path": "schema.json",
      "digest": "..."
    }
  ],
  "environment": {
    "GENERATOR_VERSION": "1.4.0"
  },
  "platform": {
    "os": "windows",
    "arch": "x86_64"
  }
}
```

Then:

```text
SHA-256(canonical_computation)
```

produces the computation key.

---

## 0.3 Define Cache Guarantees

Document guarantees such as:

### Guaranteed

- deterministic cache-key generation
- atomic cache writes
- corruption detection
- cache miss fallback
- safe output restoration
- concurrent access safety
- cache integrity verification
- explainable cache misses

### Not guaranteed

- arbitrary nondeterministic programs
- programs whose behavior depends on undeclared external state
- network responses unless explicitly represented
- hidden system state
- time-dependent computations unless time is part of the computation identity
- secrets embedded into cache outputs

---

## 0.4 Define Non-Goals

Explicitly state that v1 is NOT:

- a remote build farm
- a distributed execution engine
- a Docker replacement
- a package manager
- a full Bazel replacement
- a compiler
- a CI platform
- a cloud service
- a build system

This is important for keeping the implementation focused.

---

## 0.5 Deliverables

Create:

```text
docs/
    architecture.md
    computation-model.md
    cache-correctness.md
    security-model.md
    storage-model.md
```

Create:

```text
project/
    invariants.md
```

The invariants document must contain the rules that implementation code must never violate.

## Exit Criteria

Do not proceed until:

- computation model is documented
- cache-key inputs are documented
- canonical serialization strategy is defined
- cache guarantees are documented
- non-goals are documented
- important invariants are documented

---

# MILESTONE 1 — RUST PROJECT FOUNDATION

## Objective

Create a professional Rust workspace that can evolve into a library + CLI + integrations without restructuring the project later.

## 1.1 Workspace Structure

Prefer a Cargo workspace.

Suggested structure:

```text
developer-cache/
│
├── Cargo.toml
├── Cargo.lock
├── README.md
│
├── crates/
│   ├── cache-core/
│   ├── cache-storage/
│   ├── cache-runner/
│   ├── cache-cli/
│   ├── cache-integrations/
│   └── cache-test-utils/
│
├── docs/
├── examples/
├── tests/
├── benches/
└── .github/
```

The exact crate names may change after final project naming.

Do not over-split crates purely for appearance.

Only create boundaries that represent meaningful architectural responsibilities.

---

## 1.2 Establish Dependency Policy

Prefer a small dependency footprint.

Potential categories:

- hashing
- serialization
- CLI parsing
- filesystem abstraction
- error handling
- logging/tracing
- temporary directories for tests

Before adding every dependency, ask:

1. Is it necessary?
2. Is it maintained?
3. Is it lightweight?
4. Can the functionality be implemented safely without it?

---

## 1.3 Error Architecture

Do not use:

```rust
unwrap()
expect()
panic!()
```

in production paths unless there is a clearly documented invariant.

Create structured errors.

Example conceptual hierarchy:

```text
CacheError
├── StorageError
├── SerializationError
├── IntegrityError
├── LockError
├── ExecutionError
├── KeyGenerationError
├── ConfigurationError
└── ValidationError
```

Errors should contain enough context to explain failures.

---

## 1.4 Logging

Define structured events:

```text
cache.lookup
cache.hit
cache.miss
cache.store
cache.restore
cache.delete
cache.verify
computation.start
computation.finish
computation.failed
```

Do not make verbose logging the only observability mechanism.

---

## 1.5 Exit Criteria

The foundation is complete when:

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
cargo fmt --all -- --check
```

all pass.

No feature implementation should begin before this baseline works.

---

# MILESTONE 2 — CONTENT-ADDRESSED CACHE CORE

## Objective

Implement the fundamental cache abstraction.

This is the heart of the project.

---

## 2.1 Define Core Types

Create types similar to:

```text
CacheKey
Digest
Computation
Input
Output
OutputManifest
CacheEntry
CacheMetadata
CacheResult
CachePolicy
```

Avoid passing raw `String` everywhere.

For example, a digest should be a dedicated type instead of:

```rust
type Digest = String;
```

Prefer a validated representation.

---

## 2.2 Digest System

Implement cryptographic content hashing.

At minimum:

```text
bytes -> SHA-256 -> Digest
```

Support:

```text
hash_bytes()
hash_file()
hash_reader()
```

The implementation should avoid loading very large files entirely into memory.

Use streaming hashing.

---

## 2.3 File Identity

For each input file:

```text
path
+
content digest
+
relevant file metadata
```

However, do not blindly include modification time in semantic identity.

Content should be the primary identity.

Modification timestamps can be used as an optimization but must never cause incorrect cache hits.

---

## 2.4 Computation Key Generation

Implement:

```text
Computation
      ↓
Normalization
      ↓
Canonical serialization
      ↓
SHA-256
      ↓
CacheKey
```

Test that:

```text
same computation => same key
different input => different key
different argument => different key
different tool version => different key
different relevant environment => different key
```

Also test that map ordering does not change the key.

---

## 2.5 Cache Entry Model

A cache entry should conceptually contain:

```text
CacheEntry
├── key
├── schema_version
├── created_at
├── last_accessed
├── computation metadata
├── output manifest
├── stdout reference
├── stderr reference
├── execution metadata
└── integrity information
```

Do not put large output data directly into metadata records.

Metadata should reference stored blobs.

---

## 2.6 Cache API

Design a library-level API roughly like:

```rust
cache.lookup(key)
cache.store(result)
cache.restore(entry)
cache.remove(key)
cache.contains(key)
cache.verify(key)
```

The exact API can evolve.

The important point is that CLI functionality should consume the library rather than implement cache logic itself.

---

## 2.7 Exit Criteria

The core must be able to:

1. create a deterministic computation
2. generate a deterministic key
3. create a cache entry
4. retrieve the entry
5. verify its identity
6. detect corrupted metadata

All of this must work without executing external commands yet.

---

# MILESTONE 3 — CONTENT-ADDRESSED STORAGE ENGINE

## Objective

Build the physical storage layer.

Separate:

```text
"What is cached?"
```

from:

```text
"Where are cached bytes stored?"
```

---

## 3.1 Storage Abstraction

Create a storage trait.

Conceptually:

```rust
trait Storage {
    fn put(...);
    fn get(...);
    fn exists(...);
    fn delete(...);
    fn metadata(...);
}
```

The first implementation should be local filesystem storage.

Do not implement remote storage yet.

---

## 3.2 Storage Layout

Use a content-addressed directory structure.

For example:

```text
.cache/
├── objects/
│   ├── ab/
│   │   └── cdef...
│   ├── 12/
│   │   └── 98af...
│
├── entries/
│   ├── 01/
│   └── 9f/
│
├── metadata/
└── index/
```

Do not create millions of files inside one directory.

Use digest prefixes to distribute entries.

---

## 3.3 Atomic Writes

Never write cache objects directly into their final location.

Use:

```text
temporary file
      ↓
write
      ↓
flush
      ↓
sync if required
      ↓
rename
```

The system must never leave a partially-written valid-looking cache object.

---

## 3.4 Corruption Detection

Every stored object should be verifiable.

For example:

```text
expected digest
actual digest
```

If they differ:

```text
CACHE CORRUPTED
```

The engine must remove or quarantine the invalid object rather than returning it as valid.

---

## 3.5 Storage Inspection

Implement internal APIs to answer:

```text
How many objects exist?
How much space do they use?
What is the largest object?
How many cache entries exist?
```

These APIs will later power CLI statistics.

---

## 3.6 Tests

Test:

- empty cache
- one object
- duplicate object
- corrupted object
- interrupted write simulation
- deletion
- concurrent read
- concurrent write
- nested directories
- very large file
- binary files

---

# MILESTONE 4 — COMPUTATION RUNNER

## Objective

Move from storing arbitrary cache entries to actually caching computations.

The runner executes a developer command only when required.

---

## 4.1 Command Model

Create a structured command specification:

```text
CommandSpec
├── executable
├── arguments
├── working_directory
├── environment
├── inputs
├── outputs
└── cache_policy
```

Do not construct shell commands by concatenating untrusted strings.

Prefer direct process execution APIs.

---

## 4.2 Input Declaration

The caller must explicitly tell the engine what inputs matter.

Example:

```text
inputs:
    src/main.rs
    src/lib.rs
    Cargo.toml
```

Hash these inputs before lookup.

---

## 4.3 Output Declaration

The caller must declare expected outputs.

Example:

```text
outputs:
    dist/app.js
    dist/app.js.map
```

The cache engine should verify that declared outputs actually exist after successful execution.

---

## 4.4 Execution Lifecycle

Implement:

```text
prepare
    ↓
collect inputs
    ↓
calculate key
    ↓
lookup cache
    ↓
HIT?
 ┌──┴───┐
YES    NO
 ↓      ↓
restore execute
        ↓
     validate outputs
        ↓
      store
```

---

## 4.5 Cache Hit

On a hit:

1. retrieve metadata
2. verify cache integrity
3. restore outputs
4. restore metadata if requested
5. report HIT
6. do not execute the command

---

## 4.6 Cache Miss

On a miss:

1. report reason
2. execute command
3. capture exit code
4. capture stdout/stderr
5. verify outputs
6. hash outputs
7. store outputs
8. store metadata
9. return execution result

---

## 4.7 Failed Computations

Do not cache failed computations by default.

For example:

```text
exit code != 0
```

should normally mean:

```text
DO NOT STORE
```

Support a future policy for explicitly cacheable failures, but keep v1 conservative.

---

# MILESTONE 5 — CACHE CORRECTNESS & INVALIDATION

## Objective

This is one of the most important milestones.

A fast incorrect cache is worse than no cache.

---

## 5.1 Input Changes

If:

```text
input A = hash X
```

and later:

```text
input A = hash Y
```

the computation must produce a different key.

Test this extensively.

---

## 5.2 Command Changes

These must create different keys:

```text
generator --fast
generator --safe
```

---

## 5.3 Tool Version

A computation using:

```text
compiler 1.80
```

must not silently reuse a result from:

```text
compiler 1.81
```

if tool version affects output.

Define a tool identity mechanism.

Possible identity:

```text
executable path
+
executable digest
+
reported version
```

The exact strategy should be documented.

---

## 5.4 Environment

Do not automatically hash the entire environment.

Instead support:

```text
declared environment
```

Example:

```text
CACHE_ENV:
    NODE_ENV
    GENERATOR_VERSION
    FEATURE_MODE
```

This avoids accidental cache fragmentation.

---

## 5.5 Platform

Platform-sensitive computations must include platform information when necessary.

Potential dimensions:

```text
OS
architecture
target triple
runtime
ABI
compiler
```

Do not unnecessarily include every machine-specific detail because that destroys cache reuse.

---

## 5.6 Explainable Cache Misses

Implement structured miss reasons.

Examples:

```text
MISS: no cache entry exists

MISS: input changed
  src/parser.rs

MISS: command arguments changed

MISS: tool identity changed

MISS: relevant environment changed

MISS: platform changed

MISS: cached output failed integrity verification
```

This should become one of the project's strongest developer-experience features.

---

## 5.7 Correctness Test Matrix

Create tests for:

```text
same inputs
different input contents
different paths
different arguments
different environment
different tool version
different platform
missing output
modified cached output
corrupted metadata
partial cache
```

No later milestone should bypass correctness tests.

---

# MILESTONE 6 — CONCURRENCY & LOCKING

## Objective

Multiple processes must be able to use the cache safely.

Example:

```text
Terminal A -> computation X
Terminal B -> computation X
Terminal C -> computation Y
```

---

## 6.1 Concurrent Reads

Multiple readers should be able to access the same cache object safely.

---

## 6.2 Concurrent Writes

Two processes writing the same object must never corrupt it.

Use atomic object creation and/or locking.

---

## 6.3 Duplicate Computation

If two processes simultaneously miss the same key:

```text
A -> MISS
B -> MISS
```

avoid both performing expensive work when possible.

Potential flow:

```text
A obtains computation lock
B waits
A executes
A stores result
B checks cache again
B receives HIT
```

This is an important systems-level feature.

---

## 6.4 Lock Failure Recovery

Handle:

- stale locks
- crashed processes
- process termination
- lock timeout
- corrupted lock metadata

Never leave the cache permanently unusable because one process crashed.

---

## 6.5 Stress Testing

Create stress tests that launch:

```text
10 processes
50 processes
100 concurrent operations
```

against the same cache.

Verify:

- no corruption
- no deadlocks
- no inconsistent metadata
- no lost outputs

---

# MILESTONE 7 — CACHE LIFECYCLE & EVICTION

## Objective

A cache that grows forever is not production quality.

---

## 7.1 Cache Size Limits

Support configuration:

```text
max_size
```

Examples:

```text
500 MB
2 GB
10 GB
```

---

## 7.2 Eviction Strategy

Start with an understandable strategy such as:

```text
LRU / least recently used
```

or another justified policy.

Do not prematurely implement complex predictive eviction.

---

## 7.3 Garbage Collection

Implement:

```text
prune
```

that removes objects no longer referenced by valid cache entries.

---

## 7.4 Manual Maintenance

Support operations such as:

```text
cache clean
cache prune
cache verify
cache stats
```

---

## 7.5 Safe Deletion

Never delete an object currently being used.

Coordinate deletion with locking.

---

# MILESTONE 8 — PROFESSIONAL CLI

## Objective

Expose the engine through a polished command-line interface.

---

## 8.1 Commands

Initial command structure:

```text
dcc init
dcc run
dcc inspect
dcc stats
dcc verify
dcc clean
dcc prune
dcc config
dcc doctor
```

The final binary name can be changed after project naming.

---

## 8.2 `init`

Creates a local cache configuration.

Example:

```bash
dcc init
```

Should:

- determine default cache directory
- create required directories
- create configuration
- validate storage
- print configuration summary

---

## 8.3 `run`

Example concept:

```bash
dcc run \
  --input src/schema.json \
  --output generated/client.rs \
  -- generator src/schema.json
```

Behavior:

```text
calculate key
→ lookup
→ HIT: restore
→ MISS: execute
→ validate
→ store
```

---

## 8.4 `stats`

Show:

```text
entries
objects
disk usage
hits
misses
hit ratio
bytes restored
bytes stored
estimated time saved
```

---

## 8.5 `inspect`

Allow developers to inspect a computation:

```bash
dcc inspect <key>
```

Display:

```text
key
operation
command
arguments
inputs
outputs
tool identity
environment
created
last accessed
size
integrity
```

---

## 8.6 JSON Output

Every major command should support machine-readable output:

```bash
dcc stats --json
dcc inspect <key> --json
dcc run --json
```

This makes the CLI useful for automation and CI.

---

## 8.7 Stable Exit Codes

Define documented exit codes.

For example:

```text
0 = success
1 = computation failed
2 = invalid configuration
3 = cache error
4 = integrity failure
5 = invalid arguments
```

Do not randomly change these later.

---

# MILESTONE 9 — OBSERVABILITY & CACHE EXPLANATION

## Objective

Make the cache understandable.

Performance tools are frustrating when developers cannot answer:

> "Why didn't my cache work?"

---

## 9.1 Hit/Miss Metrics

Track:

```text
total requests
hits
misses
hit ratio
execution count
cache restore count
cache store count
```

---

## 9.2 Timing Metrics

Track:

```text
computation execution time
cache lookup time
cache restore time
cache store time
```

Then calculate:

```text
time saved
```

---

## 9.3 Explain Mode

Add:

```bash
dcc run --explain
```

Example:

```text
Cache lookup

Result: MISS

Reason:
  input changed

Changed:
  src/parser.rs

Previous:
  sha256: abc...

Current:
  sha256: def...
```

---

## 9.4 Debug Mode

Add structured debugging:

```bash
dcc run --verbose
```

Potential stages:

```text
[INPUT] hashing files
[KEY] generating computation key
[LOOKUP] checking cache
[MISS] no entry
[EXEC] running command
[OUTPUT] validating outputs
[STORE] writing objects
[DONE] stored result
```

---

# MILESTONE 10 — GENERIC DEVELOPER INTEGRATION API

## Objective

Make the engine useful outside its own CLI.

The Rust library should be a first-class product.

---

## 10.1 Public Library API

Design a clean API such as:

```rust
Cache::open(...)
ComputationBuilder::new(...)
cache.execute(...)
cache.lookup(...)
cache.store(...)
```

The API should hide storage implementation details.

---

## 10.2 Builder API

Prefer ergonomic construction:

```rust
Computation::builder()
    .operation("codegen")
    .command("generator")
    .args(...)
    .input(...)
    .output(...)
    .env(...)
    .build()
```

Validate invalid configurations before execution.

---

## 10.3 Integration Example

Create examples:

```text
examples/
    cached_codegen.rs
    cached_analysis.rs
    cached_transform.rs
```

Each should demonstrate a realistic use case.

---

## 10.4 Integration Contract

Document:

```text
How to declare inputs
How to declare outputs
How to declare environment
How cache identity works
How errors work
How cache misses work
How to disable caching
```

---

# MILESTONE 11 — BUILD CACHE AS THE FIRST MAJOR APPLICATION

## Objective

Now use the generic engine to demonstrate a serious real-world application: build caching.

This comes AFTER the generic computation engine.

That architectural ordering is deliberate.

Cargo already maintains build artifacts in `target` and intermediate build artifacts in its build directory, while tools such as sccache already address compiler caching. Therefore this project should demonstrate a broader computation abstraction rather than simply reproducing those tools.

---

## 11.1 Build Action Model

Represent a build action as:

```text
compiler
arguments
source inputs
dependency inputs
compiler version
target
environment
outputs
```

This is similar to the action-oriented model used by established build-cache systems.

---

## 11.2 Rust Build Integration

Start with a controlled demonstration.

Do not immediately attempt to replace Cargo internals.

Possible initial integration:

```text
source files
+
compiler configuration
+
tool identity
→ computation key
→ cached artifact
```

---

## 11.3 Build Demonstration

Create a benchmark project.

Run:

```text
build #1
```

Result:

```text
MISS
compile
store
```

Then:

```text
build #2
```

Result:

```text
HIT
restore
```

Modify one source file:

```text
build #3
```

Result:

```text
MISS
```

This should be reproducible.

---

## 11.4 Benchmark

Measure:

```text
cold build
warm build without cache
warm build with cache
```

Report:

```text
execution time
cache lookup time
restore time
storage size
speedup
```

---

# MILESTONE 12 — CROSS-PLATFORM SUPPORT

## Objective

Support:

```text
Windows
Linux
macOS
```

from a single codebase.

---

## 12.1 Path Handling

Never assume:

```text
/
```

or:

```text
C:\
```

Use Rust path abstractions.

---

## 12.2 Process Handling

Account for:

- Windows command execution
- Unix command execution
- environment handling
- process termination
- stdout/stderr
- executable discovery

---

## 12.3 File Semantics

Test:

- symlinks
- permissions
- executable bits
- case-sensitive paths
- case-insensitive paths
- path separators

---

## 12.4 CI Matrix

Create CI jobs for:

```text
ubuntu-latest
windows-latest
macos-latest
```

Every release candidate must pass the full matrix.

---

# MILESTONE 13 — PERFORMANCE ENGINEERING

## Objective

Do not optimize blindly.

First measure.

---

## 13.1 Benchmarks

Create benchmarks for:

```text
hash small file
hash large file
hash directory
generate key
lookup cache
store cache
restore cache
serialize metadata
deserialize metadata
concurrent lookup
```

---

## 13.2 Large Files

Test:

```text
1 MB
10 MB
100 MB
1 GB
```

where practical.

Measure memory usage.

The system should stream large files instead of loading them entirely into memory.

---

## 13.3 Large Cache

Create synthetic caches containing:

```text
1,000 entries
10,000 entries
100,000 entries
```

Measure lookup and maintenance performance.

---

## 13.4 Optimize Only After Profiling

Potential future optimizations:

- directory sharding
- metadata index
- memory cache
- parallel hashing
- batching
- filesystem optimization
- hardlinks/reflinks where safe
- compression

Do not implement all of them automatically.

Every optimization should have:

```text
benchmark before
implementation
benchmark after
```

---

# MILESTONE 14 — SECURITY & SAFETY

## Objective

Treat cached artifacts as potentially dangerous data.

---

## 14.1 Cache Poisoning

Consider:

```text
malicious cache object
malicious metadata
tampered output
```

Verify integrity before restoration.

---

## 14.2 Path Traversal

Never allow cached output metadata to restore files outside the intended workspace.

Reject paths such as:

```text
../../important-file
```

or equivalent platform-specific traversal.

---

## 14.3 Symlink Attacks

Carefully handle symlinks.

A cached entry must not be able to cause arbitrary file overwrite through a malicious symlink.

---

## 14.4 Sensitive Information

Do not accidentally cache:

```text
passwords
tokens
private keys
credentials
```

Document the responsibility of callers.

Consider adding an opt-in sensitive-data warning or policy later.

---

## 14.5 Untrusted Cache Mode

Future architecture should allow:

```text
trusted local cache
untrusted cache
read-only cache
```

An untrusted cache should receive stricter validation.

---

# MILESTONE 15 — REMOTE CACHE ABSTRACTION

## Objective

Do NOT build the remote server yet.

Prepare the architecture so remote storage can be introduced later.

Remote caches commonly separate an action/result mapping from content-addressable storage, which is a useful architectural model for this project.

---

## 15.1 Storage Backend Trait

The storage abstraction should allow:

```text
LocalFilesystemStorage
RemoteStorage
```

without changing computation logic.

---

## 15.2 Backend Capabilities

Define capabilities such as:

```text
read
write
delete
exists
stream
```

Potentially later:

```text
batch_get
batch_put
```

---

## 15.3 Local-First Architecture

The system should conceptually support:

```text
L1: memory
L2: local disk
L3: remote cache
```

But v1 should only require local disk.

---

## 15.4 Remote Design Document

Write:

```text
docs/remote-cache.md
```

Define:

- protocol possibilities
- authentication
- integrity
- upload/download
- cache namespaces
- versioning
- compatibility
- security
- failure behavior

Do not implement a server until the local engine is stable.

---

# MILESTONE 16 — CI/CD INTEGRATION

## Objective

Demonstrate that the cache can be useful in automated environments.

---

## 16.1 CI Behavior

CI should support:

```text
cache available
cache unavailable
cache corrupted
cache partially available
```

---

## 16.2 Graceful Degradation

If cache access fails:

```text
cache error
      ↓
fallback to computation
```

Do not make developer builds unusable because the cache is unavailable.

---

## 16.3 CI Example

Provide an example GitHub Actions workflow.

Conceptually:

```text
checkout
↓
restore cache
↓
run dcc
↓
tests/build
↓
store cache
```

The exact implementation should remain provider-neutral.

---

# MILESTONE 17 — DOCUMENTATION

## Objective

The project should be understandable to someone who did not build it.

---

## Required Documentation

```text
README.md

docs/
├── architecture.md
├── getting-started.md
├── computation-model.md
├── cache-keys.md
├── cache-correctness.md
├── storage.md
├── concurrency.md
├── security.md
├── performance.md
├── cli.md
├── library-api.md
├── integrations.md
├── ci.md
├── troubleshooting.md
└── remote-cache.md
```

---

## README Must Explain

Within the first screen:

```text
What problem does this solve?
Why is it different?
How does it work?
How do I install it?
How do I run it?
What does a cache hit look like?
```

Do not write generic marketing copy.

Use concrete examples.

---

# MILESTONE 18 — COMPLETE TESTING & QUALITY AUDIT

## Objective

Before release, test the system as infrastructure rather than as a simple CLI.

---

## 18.1 Unit Tests

Cover:

```text
hashing
key generation
serialization
validation
storage
metadata
configuration
eviction
```

---

## 18.2 Integration Tests

Test complete flows:

```text
command miss
command hit
input changed
output missing
cache corruption
concurrent access
failed command
```

---

## 18.3 Failure Injection

Simulate:

```text
disk full
permission denied
process crash
partial write
corrupted metadata
corrupted object
missing output
invalid configuration
```

The system must fail safely.

---

## 18.4 Concurrency Tests

Run:

```text
many readers
many writers
same-key writers
different-key writers
reader + writer
pruner + reader
```

---

## 18.5 Cross-Platform Tests

Every supported OS must pass the same essential test suite.

---

## 18.6 Quality Gates

Release is blocked if any of these fail:

```text
cargo fmt
cargo check
cargo test
cargo clippy
documentation build
integration tests
cross-platform CI
benchmarks
security tests
```

---

# MILESTONE 19 — RELEASE ENGINEERING

## Objective

Prepare the project as a real open-source Rust product.

---

## 19.1 Versioning

Use SemVer.

Suggested progression:

```text
0.1.0
0.2.0
0.3.0
...
1.0.0
```

Do not claim stability before the API and cache format are stable.

---

## 19.2 Package Validation

Before publishing:

```bash
cargo package
```

Verify:

- package contents
- README
- license
- repository metadata
- documentation
- examples
- binaries
- library API

---

## 19.3 Release Automation

Create GitHub Actions for:

```text
test
lint
build
package
release
```

---

## 19.4 Binary Distribution

Eventually provide binaries for:

```text
Windows x64
Linux x64
macOS
```

Potential architecture-specific releases can follow.

---

## 19.5 Changelog

Maintain:

```text
CHANGELOG.md
```

Every release should describe:

```text
added
changed
fixed
security
breaking changes
```

---

# MILESTONE 20 — V1.0.0 RELEASE AUDIT

## Objective

Do not release v1.0 merely because the feature list is complete.

Perform an engineering audit.

---

# 20.1 Correctness Audit

Verify:

```text
same computation → same key

different computation → different key

changed input → cache miss

changed relevant environment → cache miss

changed tool identity → cache miss

corrupted cache → detected

missing cache → safe miss

failed computation → not cached by default
```

---

# 20.2 Reliability Audit

Verify:

```text
process crash
disk failure
partial writes
concurrent processes
cache corruption
cache deletion
large cache
```

---

# 20.3 Performance Audit

Measure:

```text
cold execution
cache lookup
cache hit
cache restore
cache store
large files
large cache
concurrent workloads
```

Document real numbers.

Do not invent performance claims.

---

# 20.4 Developer Experience Audit

Verify:

```text
CLI understandable
errors useful
miss reasons understandable
JSON output stable
documentation complete
installation simple
configuration predictable
```

---

# 20.5 API Audit

Review every public Rust API.

For each public type/function ask:

```text
Is this necessary?
Is naming clear?
Is error behavior documented?
Can this API evolve?
Is this accidentally exposing implementation details?
```

Remove unnecessary public APIs.

---

# 20.6 Security Audit

Review:

```text
path traversal
symlink handling
cache poisoning
integrity validation
unsafe code
command execution
environment handling
temporary files
permissions
sensitive data
```

Run:

```bash
cargo audit
```

if appropriate for the project's dependency policy.

---

# 20.7 Release Audit Report

Create:

```text
reports/
    v1.0.0-release-audit.md
```

The report must contain:

```text
Project
Version
Date

Architecture Status
Correctness Status
Storage Status
Concurrency Status
Security Status
Performance Status
Cross-Platform Status
Testing Status
Documentation Status
Release Status

Known Limitations

Final Decision:
GO / NO-GO
```

---

# POST-V1 ROADMAP

Do not implement these before v1 unless there is a strong reason.

---

# V1.1 — Advanced Diagnostics

Potential features:

```text
cache why
cache explain
cache diff
cache inspect
cache trace
```

Example:

```text
Why did this computation miss?

Changed:
  compiler version

Previous:
  rustc 1.90

Current:
  rustc 1.91
```

---

# V1.2 — Storage Optimization

Potential features:

```text
compression
hardlinks
reflinks
parallel restore
parallel hashing
metadata index
memory cache
```

Only implement features supported safely by the platform.

---

# V1.3 — Plugin / Integration API

Allow external developer tools to integrate without directly depending on internal storage.

Possible integrations:

```text
code generators
linters
documentation tools
asset pipelines
test frameworks
build systems
```

---

# V1.4 — Advanced Cache Policies

Potential policies:

```text
read-only
write-only
no-cache
force-recompute
cache-failed
cache-success-only
maximum-entry-size
TTL
```

---

# V2 — REMOTE CACHE

Potential architecture:

```text
Developer
   ↓
Local Cache
   ↓ miss
Remote Cache
   ↓ miss
Execute
   ↓
Local Cache
   ↓
Remote Cache
```

Potential backend:

```text
HTTP
object storage
self-hosted server
```

Remote caching should remain optional.

The product should still be useful with zero server infrastructure.

---

# FUTURE ADVANCED FEATURES

Potential future research directions:

## Predictive Cache Warming

Predict which computations will be needed and prepare them before execution.

---

## Dependency-Aware Prefetching

If:

```text
A → B → C
```

and C changes, determine which cached computations remain reusable.

---

## Cache Federation

Multiple local caches could share results securely.

---

## Distributed Cache

Allow teams to share computation results.

---

## Reproducibility Verification

Run a computation independently and compare:

```text
actual output digest
vs
cached output digest
```

This could detect nondeterministic computations.

---

## Automatic Dependency Discovery

Instead of requiring every input to be manually declared, integrations could discover dependencies.

This must be implemented carefully because incorrect dependency discovery can create incorrect cache hits.

---

# FINAL ARCHITECTURAL MODEL

The final system should conceptually look like:

```text
                 ┌──────────────────────┐
                 │ Developer / CI / Tool │
                 └──────────┬───────────┘
                            │
                            ▼
                 ┌──────────────────────┐
                 │ Computation API      │
                 └──────────┬───────────┘
                            │
                            ▼
                 ┌──────────────────────┐
                 │ Normalizer           │
                 │ + Key Generator      │
                 └──────────┬───────────┘
                            │
                            ▼
                 ┌──────────────────────┐
                 │ Cache Lookup         │
                 └──────────┬───────────┘
                            │
                  ┌─────────┴─────────┐
                  │                   │
                 HIT                 MISS
                  │                   │
                  ▼                   ▼
          ┌──────────────┐    ┌──────────────┐
          │ Verify       │    │ Execute      │
          │ Integrity    │    │ Computation  │
          └──────┬───────┘    └──────┬───────┘
                 │                    │
                 │                    ▼
                 │             ┌──────────────┐
                 │             │ Validate     │
                 │             │ Outputs      │
                 │             └──────┬───────┘
                 │                    │
                 │                    ▼
                 │             ┌──────────────┐
                 │             │ Store Result │
                 │             └──────┬───────┘
                 │                    │
                 └─────────┬──────────┘
                           ▼
                 ┌──────────────────────┐
                 │ Restore / Return     │
                 │ Computation Result   │
                 └──────────────────────┘
```

---

# CORE ENGINEERING RULES

The implementation agent MUST follow these rules throughout the project:

1. Never sacrifice correctness for cache hit rate.

2. Never treat a cache hit as valid without integrity verification.

3. Never use nondeterministic serialization for cache keys.

4. Never make the CLI contain business logic that belongs in the library.

5. Never make the storage implementation dictate the computation model.

6. Never assume all environment variables affect computation identity.

7. Never silently cache failed computations.

8. Never restore outputs outside the declared workspace/output boundaries.

9. Never optimize without benchmarks.

10. Never introduce a remote server requirement into the core product.

11. Every major feature must have tests.

12. Every cache-corruption scenario must fail safely.

13. Every public API should be intentional and documented.

14. Preserve backward compatibility once the project reaches stable API status.

15. Do not add features merely to increase the feature count.

16. Prefer a small, reliable implementation over a large fragile implementation.

---

# DEFINITION OF DONE

The project is considered genuinely v1.0-ready only when a developer can:

```text
1. Install the tool.

2. Initialize a local cache.

3. Define a computation.

4. Run it.

5. Receive a MISS on first execution.

6. Have the result stored safely.

7. Run the same computation again.

8. Receive a HIT.

9. Modify an input.

10. Receive a MISS.

11. Inspect why the previous cache entry was invalid.

12. Restore cached outputs safely.

13. Run multiple processes concurrently.

14. Recover from corrupted cache data.

15. Limit cache size.

16. Inspect cache statistics.

17. Use JSON output in automation.

18. Run the system on Windows, Linux, and macOS.

19. Use the Rust library directly.

20. Understand the entire system from the documentation.
```

The final product should feel like a **real developer infrastructure component**, not a CLI project with a cache folder.

The strongest resume-level engineering signals should be:

```text
Rust
Systems Programming
Content-Addressable Storage
Deterministic Computation
Cache Correctness
Concurrency
File-System Engineering
Process Execution
Integrity Verification
Cross-Platform Engineering
Performance Engineering
Developer Tooling
CLI + Library Architecture
Testing & Failure Injection
CI/CD
Security
```