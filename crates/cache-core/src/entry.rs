use crate::computation::Computation;
use crate::digest::{CacheKey, Digest};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputManifestItem {
    pub path: String,
    pub digest: Digest,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_executable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionMetadata {
    pub exit_code: i32,
    pub execution_time_ms: u64,
    pub stdout_digest: Option<Digest>,
    pub stderr_digest: Option<Digest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheMetadata {
    pub created_at: DateTime<Utc>,
    pub last_accessed_at: DateTime<Utc>,
    pub hit_count: u64,
    pub execution: ExecutionMetadata,
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
            },
        }
    }

    pub fn total_output_size(&self) -> u64 {
        self.outputs.iter().map(|o| o.size).sum()
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
