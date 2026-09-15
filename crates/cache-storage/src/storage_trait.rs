use dcc_core::{Digest, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Metadata regarding a stored blob in Content-Addressed Storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobMetadata {
    pub digest: Digest,
    pub size_bytes: u64,
    pub path: Option<PathBuf>,
}

/// Abstract storage trait defining the physical storage engine interface.
///
/// Decouples *what* is cached from *where* and *how* cached bytes are stored.
/// The primary implementation is local filesystem CAS (`LocalCasStorage` / `CasStorage`),
/// while remaining future-extensible for alternative backends (e.g., S3, memory, network cache).
pub trait Storage: Send + Sync {
    /// Put raw bytes into the storage, returning the computed digest and stored size.
    fn put(&self, bytes: &[u8]) -> Result<(Digest, u64)>;

    /// Put content from a local file into storage, returning the computed digest and stored size.
    fn put_file(&self, source_path: &Path) -> Result<(Digest, u64)>;

    /// Retrieve a readable stream for a blob by digest after verifying its integrity.
    fn get(&self, digest: &Digest) -> Result<Box<dyn Read + Send>>;

    /// Retrieve raw bytes for a blob by digest.
    fn get_bytes(&self, digest: &Digest) -> Result<Vec<u8>> {
        let mut reader = self.get(digest)?;
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Check if a blob with the specified digest exists in storage.
    fn exists(&self, digest: &Digest) -> bool;

    /// Delete a blob by its digest. Returns true if the blob was present and deleted.
    fn delete(&self, digest: &Digest) -> Result<bool>;

    /// Retrieve metadata for a blob if present.
    fn metadata(&self, digest: &Digest) -> Result<Option<BlobMetadata>>;

    /// Verify cryptographic integrity of the blob against its expected digest.
    fn verify(&self, digest: &Digest) -> Result<()>;
}

impl Storage for crate::cas::CasStorage {
    fn put(&self, bytes: &[u8]) -> Result<(Digest, u64)> {
        self.store_object_bytes(bytes)
    }

    fn put_file(&self, source_path: &Path) -> Result<(Digest, u64)> {
        self.store_object_from_file(source_path)
    }

    fn get(&self, digest: &Digest) -> Result<Box<dyn Read + Send>> {
        let reader = self.get_object_reader(digest)?;
        Ok(Box::new(reader))
    }

    fn exists(&self, digest: &Digest) -> bool {
        self.has_object(digest)
    }

    fn delete(&self, digest: &Digest) -> Result<bool> {
        let path = self.object_path(digest);
        if path.is_file() {
            std::fs::remove_file(path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn metadata(&self, digest: &Digest) -> Result<Option<BlobMetadata>> {
        let path = self.object_path(digest);
        if !path.is_file() {
            return Ok(None);
        }
        let meta = std::fs::metadata(&path)?;
        Ok(Some(BlobMetadata {
            digest: digest.clone(),
            size_bytes: meta.len(),
            path: Some(path),
        }))
    }

    fn verify(&self, digest: &Digest) -> Result<()> {
        self.verify_object(digest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::{CasStorage, StorageConfig};

    #[test]
    fn test_storage_trait_operations() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().to_path_buf(),
            max_size_bytes: None,
        };
        let storage: Box<dyn Storage> = Box::new(CasStorage::new(config).unwrap());

        let test_data = b"storage trait content payload";
        let (digest, size) = storage.put(test_data).unwrap();
        assert_eq!(size, test_data.len() as u64);

        // exists
        assert!(storage.exists(&digest));

        // metadata
        let meta = storage
            .metadata(&digest)
            .unwrap()
            .expect("should find metadata");
        assert_eq!(meta.digest, digest);
        assert_eq!(meta.size_bytes, size);

        // get_bytes & get
        let retrieved = storage.get_bytes(&digest).unwrap();
        assert_eq!(retrieved, test_data);

        // verify
        assert!(storage.verify(&digest).is_ok());

        // delete
        assert!(storage.delete(&digest).unwrap());
        assert!(!storage.exists(&digest));
        assert_eq!(storage.metadata(&digest).unwrap(), None);
    }

    #[test]
    fn test_storage_put_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = StorageConfig {
            root_dir: temp_dir.path().join("storage"),
            max_size_bytes: None,
        };
        let storage = CasStorage::new(config).unwrap();

        let src_file = temp_dir.path().join("source.txt");
        let content = b"file-based storage put test";
        std::fs::write(&src_file, content).unwrap();

        let (digest, size) = storage.put_file(&src_file).unwrap();
        assert_eq!(size, content.len() as u64);
        assert!(storage.exists(&digest));
        assert_eq!(storage.get_bytes(&digest).unwrap(), content);
    }
}
