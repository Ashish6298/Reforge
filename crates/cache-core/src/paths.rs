use crate::error::{CacheError, Result};
use std::path::{Component, Path, PathBuf};

/// Cross-platform path abstraction utilities for DCC (Milestone 12.1).
///
/// Ensures deterministic, safe, platform-agnostic path operations across
/// Windows, Linux, and macOS without hardcoded assumptions about separators (`/` vs `\`)
/// or root prefixes (`C:\` vs `/`).
pub struct PathUtils;

impl PathUtils {
    /// Normalizes any path into a canonical, forward-slash representation (`/`)
    /// suitable for deterministic computation key hashing and metadata persistence.
    ///
    /// Strips leading `./` components and converts all platform-specific separators.
    pub fn to_normalized_string<P: AsRef<Path>>(path: P) -> String {
        let p = path.as_ref();
        let mut components = Vec::new();

        for component in p.components() {
            match component {
                Component::CurDir => continue, // ignore leading or redundant '.'
                Component::Normal(c) => components.push(c.to_string_lossy().into_owned()),
                Component::ParentDir => components.push("..".to_string()),
                Component::RootDir => components.push("".to_string()),
                Component::Prefix(prefix) => {
                    components.push(prefix.as_os_str().to_string_lossy().into_owned())
                }
            }
        }

        if components.is_empty() {
            return String::new();
        }

        // Check if original path was root-rooted
        let joined = components.join("/");
        if p.is_absolute() && !joined.starts_with('/') && !joined.contains(':') {
            format!("/{}", joined)
        } else {
            joined
        }
    }

    /// Convert a normalized relative string path back into a native OS `PathBuf`.
    pub fn to_native_path(normalized: &str) -> PathBuf {
        let mut buf = PathBuf::new();
        for segment in normalized.split('/') {
            if !segment.is_empty() {
                buf.push(segment);
            }
        }
        buf
    }

    /// Validates that a relative path does not escape a target root directory,
    /// rejecting directory traversal (`../`), absolute paths (`/foo`, `C:\foo`),
    /// or Windows drive-prefix tricks.
    pub fn sanitize_relative_path<P: AsRef<Path>>(base_dir: P, rel_path: &str) -> Result<PathBuf> {
        let normalized = rel_path.replace('\\', "/");

        // Reject absolute or escaping prefixes
        if normalized.starts_with('/')
            || normalized.starts_with("../")
            || normalized.contains("/../")
            || normalized == ".."
            || (normalized.len() >= 2 && normalized.as_bytes()[1] == b':')
        {
            return Err(CacheError::PathTraversal(format!(
                "Illegal path component or traversal in relative path: '{}'",
                rel_path
            )));
        }

        let base = base_dir.as_ref();
        let native_sub = Self::to_native_path(&normalized);
        let combined = base.join(native_sub);

        // Verify logical encapsulation
        let mut depth: isize = 0;
        for c in Path::new(&normalized).components() {
            match c {
                Component::ParentDir => depth -= 1,
                Component::Normal(_) => depth += 1,
                Component::RootDir | Component::Prefix(_) => {
                    return Err(CacheError::PathTraversal(format!(
                        "Absolute path components forbidden in relative path: '{}'",
                        rel_path
                    )));
                }
                Component::CurDir => {}
            }
            if depth < 0 {
                return Err(CacheError::PathTraversal(format!(
                    "Path traversal escapes root: '{}'",
                    rel_path
                )));
            }
        }

        Ok(combined)
    }

    /// Validates a path declared on a computation or build action.
    /// Rejects directory traversal (`../`, `/../`) while allowing safe relative paths and canonical absolute paths.
    pub fn validate_computation_path(path_str: &str) -> Result<()> {
        let norm = path_str.replace('\\', "/");
        if norm.starts_with("../") || norm.contains("/../") || norm == ".." {
            return Err(CacheError::PathTraversal(format!(
                "Path contains invalid directory traversal: '{}'",
                path_str
            )));
        }
        Ok(())
    }

    /// Normalizes paths for canonical computation key generation, ensuring invariant
    /// sorting and hash equality across Windows, Linux, and macOS.
    pub fn canonicalize_for_key(path_str: &str) -> String {
        path_str
            .replace('\\', "/")
            .trim_start_matches("./")
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_normalized_string_windows_and_unix() {
        let win_path = r"src\components\button.rs";
        let norm = PathUtils::to_normalized_string(win_path);
        assert_eq!(norm, "src/components/button.rs");

        let unix_path = "src/components/button.rs";
        let norm_u = PathUtils::to_normalized_string(unix_path);
        assert_eq!(norm_u, "src/components/button.rs");

        let dot_slash = r".\nested\dir\file.txt";
        let norm_dot = PathUtils::to_normalized_string(dot_slash);
        assert_eq!(norm_dot, "nested/dir/file.txt");
    }

    #[test]
    fn test_to_native_path() {
        let norm = "target/debug/build/output.rlib";
        let native = PathUtils::to_native_path(norm);

        #[cfg(windows)]
        assert_eq!(
            native.to_string_lossy().replace('/', "\\"),
            r"target\debug\build\output.rlib"
        );

        #[cfg(not(windows))]
        assert_eq!(native.to_string_lossy(), "target/debug/build/output.rlib");
    }

    #[test]
    fn test_sanitize_relative_path_valid() {
        let base = Path::new("workspace");
        let safe = PathUtils::sanitize_relative_path(base, "src/main.rs").unwrap();
        assert_eq!(safe, base.join("src").join("main.rs"));

        let win_style = PathUtils::sanitize_relative_path(base, r"dist\bundle.js").unwrap();
        assert_eq!(win_style, base.join("dist").join("bundle.js"));
    }

    #[test]
    fn test_sanitize_relative_path_traversal_rejection() {
        let base = Path::new("workspace");

        assert!(PathUtils::sanitize_relative_path(base, "../secret.txt").is_err());
        assert!(PathUtils::sanitize_relative_path(base, r"..\secret.txt").is_err());
        assert!(PathUtils::sanitize_relative_path(base, "a/../../secret.txt").is_err());
        assert!(PathUtils::sanitize_relative_path(base, "/etc/passwd").is_err());
        assert!(PathUtils::sanitize_relative_path(base, r"C:\Windows\System32").is_err());
        assert!(PathUtils::sanitize_relative_path(base, "D:/secrets").is_err());
    }

    #[test]
    fn test_canonicalize_for_key() {
        assert_eq!(
            PathUtils::canonicalize_for_key(r".\foo\bar\baz.txt"),
            "foo/bar/baz.txt"
        );
        assert_eq!(
            PathUtils::canonicalize_for_key("foo/bar/baz.txt"),
            "foo/bar/baz.txt"
        );
    }

    #[test]
    fn test_validate_computation_path() {
        assert!(PathUtils::validate_computation_path("src/lib.rs").is_ok());
        assert!(PathUtils::validate_computation_path(r"src\nested\mod.rs").is_ok());
        assert!(PathUtils::validate_computation_path("target/debug/output.rlib").is_ok());
        assert!(PathUtils::validate_computation_path("C:/project/src/main.rs").is_ok());
        assert!(PathUtils::validate_computation_path("/usr/local/bin/rustc").is_ok());

        assert!(PathUtils::validate_computation_path("../escape.rs").is_err());
        assert!(PathUtils::validate_computation_path(r"..\escape.rs").is_err());
        assert!(PathUtils::validate_computation_path("foo/../../escape.rs").is_err());
    }

    #[test]
    fn test_cross_platform_key_invariance() {
        let win_path = r"src\components\button.rs";
        let unix_path = "src/components/button.rs";

        assert_eq!(
            PathUtils::canonicalize_for_key(win_path),
            PathUtils::canonicalize_for_key(unix_path)
        );
    }
}
