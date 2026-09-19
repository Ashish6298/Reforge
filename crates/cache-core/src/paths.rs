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
        let p_str = path.as_ref().to_string_lossy();
        let replaced = p_str.replace('\\', "/");
        let p = Path::new(&replaced);
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
    /// Windows drive-prefix tricks, or malicious symlink escapes (Milestone 14.2 & 14.3).
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
        let mut target = base.to_path_buf();
        for part in normalized.split('/') {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                return Err(CacheError::PathTraversal(format!(
                    "Relative path contains forbidden '..' traversal component: '{}'",
                    rel_path
                )));
            }
            target.push(part);
        }

        Ok(target)
    }

    /// Validates and verifies that a computation's input and output paths are strictly
    /// contained within the workspace root without symlink dereference attacks.
    pub fn validate_computation_path<P: AsRef<Path>>(
        workspace_root: P,
        declared_path: &str,
    ) -> Result<PathBuf> {
        let root = workspace_root.as_ref();
        let sanitized = Self::sanitize_relative_path(root, declared_path)?;

        // If path already exists, verify it doesn't resolve outside sandbox via symlink
        if sanitized.exists() {
            Self::verify_symlink_safety(root, &sanitized)?;
        }

        Ok(sanitized)
    }

    /// Inspects whether a path or any ancestor component is a symlink pointing outside `sandbox_root`.
    pub fn verify_symlink_safety<P: AsRef<Path>, Q: AsRef<Path>>(
        sandbox_root: P,
        target_path: Q,
    ) -> Result<()> {
        let root = sandbox_root.as_ref();
        let target = target_path.as_ref();

        let canonical_root = root
            .canonicalize()
            .map_err(|e| CacheError::StorageError(e))?;
        let canonical_target = target
            .canonicalize()
            .map_err(|e| CacheError::StorageError(e))?;

        if !canonical_target.starts_with(&canonical_root) {
            return Err(CacheError::SymlinkAttack(format!(
                "Symlink resolution of '{}' escaped sandbox root '{}'",
                target.display(),
                root.display()
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
            native,
            PathBuf::from("target\\debug\\build\\output.rlib")
        );

        #[cfg(not(windows))]
        assert_eq!(
            native,
            PathBuf::from("target/debug/build/output.rlib")
        );
    }

    #[test]
    fn test_sanitize_relative_path_valid() {
        let base = Path::new("/workspace");
        let sanitized = PathUtils::sanitize_relative_path(base, "src/main.rs").unwrap();
        assert_eq!(sanitized, base.join("src").join("main.rs"));

        let win_style = PathUtils::sanitize_relative_path(base, r"src\module\lib.rs").unwrap();
        assert_eq!(win_style, base.join("src").join("module").join("lib.rs"));
    }

    #[test]
    fn test_sanitize_relative_path_traversal_rejection() {
        let base = Path::new("/workspace");
        assert!(PathUtils::sanitize_relative_path(base, "../secret.txt").is_err());
        assert!(PathUtils::sanitize_relative_path(base, "src/../../etc/passwd").is_err());
        assert!(PathUtils::sanitize_relative_path(base, "/absolute/path").is_err());
        assert!(PathUtils::sanitize_relative_path(base, "C:\\Windows\\System32").is_err());
    }

    #[test]
    fn test_validate_computation_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let valid_file = temp_dir.path().join("source.rs");
        std::fs::write(&valid_file, b"content").unwrap();

        let res = PathUtils::validate_computation_path(temp_dir.path(), "source.rs");
        assert!(res.is_ok());

        let invalid = PathUtils::validate_computation_path(temp_dir.path(), "../outside.txt");
        assert!(invalid.is_err());
    }

    #[test]
    fn test_verify_symlink_safety_rejection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_outside = temp_dir.path().join("outside.txt");
        std::fs::write(&target_outside, b"outside").unwrap();

        let sandbox = temp_dir.path().join("sandbox");
        std::fs::create_dir_all(&sandbox).unwrap();

        let symlink_path = sandbox.join("link_to_outside.txt");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            if symlink(&target_outside, &symlink_path).is_ok() {
                let check = PathUtils::verify_symlink_safety(&sandbox, &symlink_path);
                assert!(check.is_err(), "Escaping symlink must be rejected");
            }
        }

        #[cfg(windows)]
        {
            use std::os::windows::fs::symlink_file;
            if symlink_file(&target_outside, &symlink_path).is_ok() {
                let check = PathUtils::verify_symlink_safety(&sandbox, &symlink_path);
                assert!(check.is_err(), "Escaping symlink must be rejected");
            }
        }
    }
}