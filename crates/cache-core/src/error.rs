use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Storage error: {0}")]
    StorageError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Integrity error: expected digest {expected}, actual {actual} for path {path}")]
    IntegrityError {
        expected: String,
        actual: String,
        path: String,
    },

    #[error("Corrupted cache entry at {0}: {1}")]
    CorruptedEntry(PathBuf, String),

    #[error("Lock error: {0}")]
    LockError(String),

    #[error("Execution error: process exited with code {0}")]
    ExecutionError(i32),

    #[error("Process terminated by signal")]
    ProcessTerminated,

    #[error("Key generation error: {0}")]
    KeyGenerationError(String),

    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Path traversal rejected: {0}")]
    PathTraversal(String),

    #[error("Declared output not produced by computation: {0}")]
    MissingOutput(String),

    #[error("Invalid digest format: {0}")]
    InvalidDigest(String),

    #[error("Sensitive data policy violation: {0}")]
    SensitiveDataError(String),

    #[error("Cache miss: {0}")]
    Miss(String),
}

pub type Result<T> = std::result::Result<T, CacheError>;
