use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use dcc_core::{CacheEntry, CacheError, Digest, Result};
use dcc_storage::CasStorage;

pub struct OutputRestorer;

impl OutputRestorer {
    pub fn sanitize_path(base_dir: &Path, rel_path: &str) -> Result<PathBuf> {
        let norm = rel_path.replace('\\', "/");
        if norm.starts_with('/') || norm.starts_with("../") || norm.contains("/../") || norm == ".." {
            return Err(CacheError::PathTraversal(format!("Illegal path component in output path: {}", rel_path)));
        }

        let full_path = base_dir.join(rel_path);
        // Ensure path stays within base_dir
        if !full_path.starts_with(base_dir) {
            return Err(CacheError::PathTraversal(format!("Path {} escapes base directory {}", rel_path, base_dir.display())));
        }

        Ok(full_path)
    }

    pub fn restore_entry(
        storage: &CasStorage,
        entry: &CacheEntry,
        destination_dir: &Path,
    ) -> Result<()> {
        for output in &entry.outputs {
            let target_path = Self::sanitize_path(destination_dir, &output.path)?;

            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }

            // Verify CAS object before writing
            storage.verify_object(&output.digest)?;

            let cas_path = storage.object_path(&output.digest);
            let mut src = BufReader::new(File::open(cas_path)?);

            // Write to a temporary file alongside destination
            let parent_dir = target_path.parent().unwrap_or(destination_dir);
            let tmp_path = parent_dir.join(format!(".tmp_restore_{}", output.digest.prefix(8)));

            {
                let mut dst = BufWriter::new(
                    OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(&tmp_path)?,
                );
                io::copy(&mut src, &mut dst)?;
                dst.flush()?;
                dst.get_ref().sync_all()?;
            }

            // Verify restored file checksum
            let check_file = File::open(&tmp_path)?;
            let check_digest = Digest::from_reader(BufReader::new(check_file))?;
            if check_digest != output.digest {
                let _ = fs::remove_file(&tmp_path);
                return Err(CacheError::IntegrityError {
                    expected: output.digest.as_str().to_string(),
                    actual: check_digest.as_str().to_string(),
                    path: target_path.display().to_string(),
                });
            }

            // Atomically replace target
            fs::rename(&tmp_path, &target_path)?;

            #[cfg(unix)]
            if let Some(true) = output.is_executable {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&target_path)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&target_path, perms)?;
            }
        }

        Ok(())
    }
}
