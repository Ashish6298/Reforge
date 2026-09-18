pub mod build_action;
pub mod canonical;
pub mod computation;
pub mod digest;
pub mod entry;
pub mod error;
pub mod event;
pub mod paths;
pub mod sensitive;
pub mod size;

pub use build_action::{BuildAction, BuildActionBuilder};
pub use canonical::CanonicalComputation;
pub use computation::{
    Computation, ComputationBuilder, InputFile, OutputFile, PlatformConstraints, ToolIdentity,
};
pub use digest::{CacheKey, Digest};
pub use entry::{
    CacheEntry, CacheMetadata, CachePolicy, CacheResult, ExecutionMetadata, IntegrityInfo,
    MissReason, OutputManifest, OutputManifestItem, TimingMetrics, TrustMode,
};
pub use error::{CacheError, Result};
pub use event::{EventKind, EventSubscriber, StructuredEvent};
pub use paths::PathUtils;
pub use sensitive::{SensitiveDataDetector, SensitiveDataPolicy, SENSITIVE_KEY_PATTERNS};
pub use size::{ByteSize, SizeParseError};

/// Current SemVer version of DCC (Milestone 19.1).
pub const DCC_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Current canonical computation schema version (Milestone 19.1 & Milestone 2.2).
pub const DCC_SCHEMA_VERSION: u32 = 1;

/// Checks if a schema version is compatible with this version of the DCC engine.
pub fn is_schema_compatible(version: u32) -> bool {
    version == DCC_SCHEMA_VERSION
}
