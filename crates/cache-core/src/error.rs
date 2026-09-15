use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Invalid digest format: {0}")]
    InvalidDigest(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Integrity error: expected digest {expected}, actual {actual} for path {path}")]
    IntegrityMismatch {
        expected: String,
        actual: String,
        path: String,
    },

    #[error("Corrupted cache entry at {0}: {1}")]
    CorruptedEntry(PathBuf, String),

    #[error("Path traversal detected: {0}")]
    PathTraversal(String),

    #[error("Declared output not produced by computation: {0}")]
    MissingOutput(String),

    #[error("Computation execution failed with exit code: {0}")]
    ExecutionFailed(i32),

    #[error("Process terminated by signal")]
    ProcessTerminated,

    #[error("Lock error: {0}")]
    LockError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Cache miss: {0}")]
    Miss(String),
}

pub type Result<T> = std::result::Result<T, CacheError>;
