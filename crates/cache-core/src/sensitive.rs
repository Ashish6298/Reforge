use serde::{Deserialize, Serialize};

/// Policy governing the detection and handling of sensitive information (Milestone 14.4).
///
/// Ensures credentials, passwords, tokens, and private keys are never accidentally
/// serialized into computation models or cached on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SensitiveDataPolicy {
    /// No automated scanning; caller assumes sole responsibility for sanitization.
    #[default]
    Allow,
    /// Detects sensitive keys/values and logs a warning to subscribers/stderr.
    Warn,
    /// Detects sensitive keys/values and rejects computation modeling or execution with an error.
    Deny,
    /// Automatically masks/redacts identified sensitive values (e.g., "[REDACTED]") in computation records.
    Mask,
}

/// Known sensitive variable and argument patterns.
pub const SENSITIVE_KEY_PATTERNS: &[&str] = &[
    "PASSWORD",
    "PASSWD",
    "SECRET",
    "TOKEN",
    "API_KEY",
    "APIKEY",
    "AUTH",
    "AUTHORIZATION",
    "BEARER",
    "PRIVATE_KEY",
    "PRIVKEY",
    "CREDENTIAL",
    "CREDENTIALS",
    "ACCESS_KEY",
    "SECRET_KEY",
    "SSH_KEY",
    "CERTIFICATE",
    "DATABASE_URL",
];

/// Utility for detecting sensitive information in environment variables, arguments, and metadata.
pub struct SensitiveDataDetector;

impl SensitiveDataDetector {
    /// Checks if a key name suggests sensitive content.
    pub fn is_sensitive_key(key: &str) -> bool {
        let upper = key.to_ascii_uppercase();
        for pattern in SENSITIVE_KEY_PATTERNS {
            if upper.contains(pattern) {
                return true;
            }
        }
        false
    }

    /// Checks if a string value appears to contain raw sensitive material
    /// (e.g. PEM private keys, bearer tokens).
    pub fn is_sensitive_value(val: &str) -> bool {
        let trimmed = val.trim();
        if trimmed.starts_with("-----BEGIN") && trimmed.contains("PRIVATE KEY-----") {
            return true;
        }
        if trimmed.starts_with("ghp_")
            || trimmed.starts_with("glpat-")
            || trimmed.starts_with("npm_")
        {
            return true;
        }
        if trimmed.starts_with("Bearer ") || trimmed.starts_with("bearer ") {
            return true;
        }
        false
    }

    /// Inspects an environment variable pair `(key, value)`.
    /// Returns `Some(finding_description)` if sensitive information is detected.
    pub fn scan_env_var(key: &str, value: &str) -> Option<String> {
        if Self::is_sensitive_key(key) {
            Some(format!(
                "Environment variable key '{}' matches sensitive credential pattern",
                key
            ))
        } else if Self::is_sensitive_value(value) {
            Some(format!(
                "Environment variable '{}' contains sensitive payload pattern",
                key
            ))
        } else {
            None
        }
    }

    /// Inspects command-line arguments for inline credentials (e.g. `--token=xyz`, `--password=abc`).
    pub fn scan_argument(arg: &str) -> Option<String> {
        let upper = arg.to_ascii_uppercase();
        for pattern in SENSITIVE_KEY_PATTERNS {
            if upper.contains(pattern) && (arg.contains('=') || arg.starts_with("--")) {
                return Some(format!(
                    "Command argument matches sensitive pattern '{}'",
                    pattern
                ));
            }
        }
        if Self::is_sensitive_value(arg) {
            return Some("Command argument contains raw credential payload pattern".to_string());
        }
        None
    }

    /// Redacts a sensitive string value.
    pub fn redact_value(_val: &str) -> String {
        "[REDACTED]".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_sensitive_key() {
        assert!(SensitiveDataDetector::is_sensitive_key(
            "AWS_SECRET_ACCESS_KEY"
        ));
        assert!(SensitiveDataDetector::is_sensitive_key("GITHUB_TOKEN"));
        assert!(SensitiveDataDetector::is_sensitive_key("DB_PASSWORD"));
        assert!(SensitiveDataDetector::is_sensitive_key("SSL_PRIVATE_KEY"));
        assert!(SensitiveDataDetector::is_sensitive_key("NPM_AUTH_TOKEN"));
        assert!(!SensitiveDataDetector::is_sensitive_key("PATH"));
        assert!(!SensitiveDataDetector::is_sensitive_key("RUST_LOG"));
        assert!(!SensitiveDataDetector::is_sensitive_key("CARGO_PKG_NAME"));
    }

    #[test]
    fn test_is_sensitive_value() {
        let priv_key =
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaA==\n-----END OPENSSH PRIVATE KEY-----";
        assert!(SensitiveDataDetector::is_sensitive_value(priv_key));
        assert!(SensitiveDataDetector::is_sensitive_value(
            "ghp_1234567890abcdef"
        ));
        assert!(SensitiveDataDetector::is_sensitive_value(
            "Bearer secret-jwt-token"
        ));
        assert!(!SensitiveDataDetector::is_sensitive_value(
            "target/debug/app"
        ));
    }

    #[test]
    fn test_scan_argument() {
        assert!(SensitiveDataDetector::scan_argument("--password=secret123").is_some());
        assert!(SensitiveDataDetector::scan_argument("--api-token=xyz").is_some());
        assert!(SensitiveDataDetector::scan_argument("-o").is_none());
        assert!(SensitiveDataDetector::scan_argument("main.rs").is_none());
    }
}
