use crate::computation::Computation;
use crate::digest::{CacheKey, Digest};
use crate::error::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CachePolicy {
    #[default]
    ReadWrite,
    ReadOnly,
    WriteOnly,
    Bypass,
    ForceRecompute,
}

/// Trust and validation level for a cache instance or storage backend (Milestone 14.5).
///
/// Untrusted caches receive stricter validation during lookups and restoration:
/// - Re-verifies computation canonical key identity against declared key.
/// - Cryptographically verifies all referenced output CAS blobs before and after staging.
/// - Enforces safe workspace path constraints and verifies intermediate path symlinks.
/// - When in `ReadOnly` mode, prevents write operations from modifying local or remote stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrustMode {
    /// Trusted local cache: default high-performance operation with standard integrity checks.
    #[default]
    TrustedLocal,
    /// Untrusted cache: enforces strict, multi-pass validation on metadata and all artifact blobs.
    Untrusted,
    /// Read-only cache: disallows mutations and entry storage, safely consuming validated artifacts.
    ReadOnly,
}

impl TrustMode {
    /// Returns true if this trust mode requires strict validation checks.
    pub fn requires_strict_validation(&self) -> bool {
        matches!(self, Self::Untrusted)
    }

    /// Returns true if this trust mode permits writing / storing cache entries.
    pub fn allows_writes(&self) -> bool {
        !matches!(self, Self::ReadOnly)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputManifestItem {
    pub path: String,
    pub digest: Digest,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_executable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OutputManifest {
    pub items: Vec<OutputManifestItem>,
}

impl OutputManifest {
    pub fn new(items: Vec<OutputManifestItem>) -> Self {
        Self { items }
    }

    pub fn total_size(&self) -> u64 {
        self.items.iter().map(|i| i.size).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheResult<T> {
    Hit(T),
    Miss(MissReason),
    Bypassed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TimingMetrics {
    #[serde(default)]
    pub execution_time_ms: u64,
    #[serde(default)]
    pub lookup_time_ms: u64,
    #[serde(default)]
    pub restore_time_ms: u64,
    #[serde(default)]
    pub store_time_ms: u64,
}

impl TimingMetrics {
    /// Calculate estimated wall-clock time saved by cache hit:
    /// time_saved = execution_time_ms - (lookup_time_ms + restore_time_ms)
    pub fn calculate_time_saved_ms(&self) -> u64 {
        let hit_overhead = self.lookup_time_ms.saturating_add(self.restore_time_ms);
        self.execution_time_ms.saturating_sub(hit_overhead)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExecutionMetadata {
    pub exit_code: i32,
    pub execution_time_ms: u64,
    pub stdout_digest: Option<Digest>,
    pub stderr_digest: Option<Digest>,
    #[serde(default)]
    pub timings: TimingMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrityInfo {
    pub entry_digest: Digest,
    pub verified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheMetadata {
    pub created_at: DateTime<Utc>,
    pub last_accessed_at: DateTime<Utc>,
    pub hit_count: u64,
    pub execution: ExecutionMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity: Option<IntegrityInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    pub schema_version: u32,
    pub key: CacheKey,
    pub computation: Computation,
    pub outputs: Vec<OutputManifestItem>,
    pub metadata: CacheMetadata,
}

impl CacheEntry {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    pub fn new(
        key: CacheKey,
        computation: Computation,
        outputs: Vec<OutputManifestItem>,
        execution: ExecutionMetadata,
    ) -> Self {
        let now = Utc::now();
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            key,
            computation,
            outputs,
            metadata: CacheMetadata {
                created_at: now,
                last_accessed_at: now,
                hit_count: 0,
                execution,
                integrity: None,
            },
        }
    }

    pub fn total_output_size(&self) -> u64 {
        self.outputs.iter().map(|o| o.size).sum()
    }

    pub fn stdout_digest(&self) -> Option<&Digest> {
        self.metadata.execution.stdout_digest.as_ref()
    }

    pub fn stderr_digest(&self) -> Option<&Digest> {
        self.metadata.execution.stderr_digest.as_ref()
    }

    /// Compute the canonical cache key derived from the embedded computation specification.
    pub fn compute_key(&self) -> Result<CacheKey> {
        let canonical = crate::canonical::CanonicalComputation::from_computation(&self.computation);
        canonical.compute_key()
    }

    /// Verify that the entry's declared key exactly matches the key derived canonically
    /// from its embedded computation. Returns an error if an identity mismatch is detected.
    pub fn verify_identity(&self) -> Result<()> {
        let expected_key = self.compute_key()?;
        if self.key != expected_key {
            return Err(crate::error::CacheError::IntegrityError {
                expected: expected_key.as_str().to_string(),
                actual: self.key.as_str().to_string(),
                path: format!("entry:{}", self.key),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissReason {
    NoEntryFound,
    InputChanged {
        path: String,
        old_digest: Option<String>,
        new_digest: String,
    },
    InputAdded {
        path: String,
    },
    InputRemoved {
        path: String,
    },
    CommandChanged {
        old: String,
        new: String,
    },
    ArgumentsChanged {
        old: Vec<String>,
        new: Vec<String>,
    },
    EnvironmentChanged {
        key: String,
        old: Option<String>,
        new: Option<String>,
    },
    ToolChanged {
        reason: String,
    },
    PlatformChanged {
        reason: String,
    },
    CorruptedCache {
        reason: String,
    },
    ForcedRecompute,
}

impl MissReason {
    /// Formats the miss reason in structured explain mode matching Milestone 9.3 spec.
    pub fn format_explain(&self) -> String {
        let mut out = String::new();
        out.push_str("Cache lookup\n\nResult: MISS\n\n");
        match self {
            Self::InputChanged {
                path,
                old_digest,
                new_digest,
            } => {
                out.push_str("Reason:\n  input changed\n\n");
                out.push_str(&format!("Changed:\n  {}\n\n", path));
                out.push_str(&format!(
                    "Previous:\n  sha256: {}\n\n",
                    old_digest.as_deref().unwrap_or("<none>")
                ));
                out.push_str(&format!("Current:\n  sha256: {}", new_digest));
            }
            Self::InputAdded { path } => {
                out.push_str("Reason:\n  input added\n\n");
                out.push_str(&format!("Added:\n  {}", path));
            }
            Self::InputRemoved { path } => {
                out.push_str("Reason:\n  input removed\n\n");
                out.push_str(&format!("Removed:\n  {}", path));
            }
            Self::CommandChanged { old, new } => {
                out.push_str("Reason:\n  command changed\n\n");
                out.push_str(&format!("Previous:\n  {}\n\n", old));
                out.push_str(&format!("Current:\n  {}", new));
            }
            Self::ArgumentsChanged { old, new } => {
                out.push_str("Reason:\n  arguments changed\n\n");
                out.push_str(&format!("Previous:\n  {:?}\n\n", old));
                out.push_str(&format!("Current:\n  {:?}", new));
            }
            Self::EnvironmentChanged { key, old, new } => {
                out.push_str("Reason:\n  environment changed\n\n");
                out.push_str(&format!("Variable:\n  {}\n\n", key));
                out.push_str(&format!(
                    "Previous:\n  {}\n\n",
                    old.as_deref().unwrap_or("<unset>")
                ));
                out.push_str(&format!(
                    "Current:\n  {}",
                    new.as_deref().unwrap_or("<unset>")
                ));
            }
            Self::ToolChanged { reason } => {
                out.push_str("Reason:\n  tool identity changed\n\n");
                out.push_str(&format!("Details:\n  {}", reason));
            }
            Self::PlatformChanged { reason } => {
                out.push_str("Reason:\n  platform constraints changed\n\n");
                out.push_str(&format!("Details:\n  {}", reason));
            }
            Self::CorruptedCache { reason } => {
                out.push_str("Reason:\n  cache integrity corrupted\n\n");
                out.push_str(&format!("Details:\n  {}", reason));
            }
            Self::ForcedRecompute => {
                out.push_str("Reason:\n  forced recompute policy");
            }
            Self::NoEntryFound => {
                out.push_str("Reason:\n  no previous cache entry found");
            }
        }
        out
    }
}

impl std::fmt::Display for MissReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoEntryFound => {
                write!(f, "No previous cache entry found for this computation key")
            }
            Self::InputChanged {
                path,
                old_digest,
                new_digest,
            } => {
                write!(
                    f,
                    "Input file changed: {} (was {}, now {})",
                    path,
                    old_digest.as_deref().unwrap_or("<none>"),
                    new_digest
                )
            }
            Self::InputAdded { path } => write!(f, "New input file declared: {}", path),
            Self::InputRemoved { path } => write!(f, "Previous input file missing: {}", path),
            Self::CommandChanged { old, new } => {
                write!(f, "Command changed from '{}' to '{}'", old, new)
            }
            Self::ArgumentsChanged { old, new } => {
                write!(f, "Arguments changed from {:?} to {:?}", old, new)
            }
            Self::EnvironmentChanged { key, old, new } => {
                write!(
                    f,
                    "Environment variable '{}' changed from {:?} to {:?}",
                    key, old, new
                )
            }
            Self::ToolChanged { reason } => write!(f, "Tool identity changed: {}", reason),
            Self::PlatformChanged { reason } => write!(f, "Platform changed: {}", reason),
            Self::CorruptedCache { reason } => write!(f, "Corrupted cache entry: {}", reason),
            Self::ForcedRecompute => write!(f, "Forced recompute requested by policy"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_entry_references_blobs_not_raw_data() {
        let comp = Computation::builder().build().unwrap();
        let key = CacheKey::from_bytes(b"key");
        let item = OutputManifestItem {
            path: "target/bin".into(),
            digest: Digest::from_bytes(b"blob data"),
            size: 1024,
            is_executable: Some(true),
        };

        let entry = CacheEntry::new(
            key,
            comp,
            vec![item],
            ExecutionMetadata {
                exit_code: 0,
                execution_time_ms: 50,
                stdout_digest: Some(Digest::from_bytes(b"stdout content")),
                stderr_digest: None,
                timings: TimingMetrics {
                    execution_time_ms: 50,
                    lookup_time_ms: 2,
                    restore_time_ms: 3,
                    store_time_ms: 5,
                },
            },
        );

        assert_eq!(entry.total_output_size(), 1024);
        assert!(entry.stdout_digest().is_some());
        assert!(entry.stderr_digest().is_none());
        assert_eq!(
            entry.metadata.execution.timings.calculate_time_saved_ms(),
            45
        );
    }

    #[test]
    fn test_timing_metrics_calculation() {
        let timings = TimingMetrics {
            execution_time_ms: 1000,
            lookup_time_ms: 10,
            restore_time_ms: 40,
            store_time_ms: 25,
        };

        // time saved = execution_time (1000) - overhead (10 + 40) = 950ms
        assert_eq!(timings.calculate_time_saved_ms(), 950);
    }

    #[test]
    fn test_miss_reason_format_explain() {
        let miss_input = MissReason::InputChanged {
            path: "src/parser.rs".to_string(),
            old_digest: Some("abc12345".to_string()),
            new_digest: "def67890".to_string(),
        };

        let explained = miss_input.format_explain();
        assert!(explained.contains("Cache lookup"));
        assert!(explained.contains("Result: MISS"));
        assert!(explained.contains("Reason:\n  input changed"));
        assert!(explained.contains("Changed:\n  src/parser.rs"));
        assert!(explained.contains("Previous:\n  sha256: abc12345"));
        assert!(explained.contains("Current:\n  sha256: def67890"));
    }
}
