use dcc_core::{CacheError, Digest};
use dcc_storage::{CasStorage, StorageConfig};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::sync::Arc;
use std::thread;

#[test]
fn test_storage_suite_empty_cache() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    let dummy_digest = Digest::from_bytes(b"non-existent");
    assert!(!storage.has_object(&dummy_digest));
    assert!(storage.verify_object(&dummy_digest).is_err());
    assert_eq!(storage.count_objects().unwrap(), 0);
    assert_eq!(storage.count_entries().unwrap(), 0);
    assert_eq!(storage.total_size_bytes().unwrap(), 0);
}

#[test]
fn test_storage_suite_one_object() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    let payload = b"single object data payload";
    let (digest, size) = storage.store_object_bytes(payload).unwrap();

    assert_eq!(size, payload.len() as u64);
    assert!(storage.has_object(&digest));
    assert!(storage.verify_object(&digest).is_ok());
    assert_eq!(storage.count_objects().unwrap(), 1);

    let mut reader = storage.get_object_reader(&digest).unwrap();
    let mut retrieved = Vec::new();
    reader.read_to_end(&mut retrieved).unwrap();
    assert_eq!(retrieved, payload);
}

#[test]
fn test_storage_suite_duplicate_object() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    let payload = b"duplicate test data payload";
    let (digest1, size1) = storage.store_object_bytes(payload).unwrap();
    let (digest2, size2) = storage.store_object_bytes(payload).unwrap();

    assert_eq!(digest1, digest2);
    assert_eq!(size1, size2);
    assert_eq!(
        storage.count_objects().unwrap(),
        1,
        "Deduplication must keep only 1 object"
    );
}

#[test]
fn test_storage_suite_corrupted_object() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    let payload = b"valid data to be corrupted";
    let (digest, _) = storage.store_object_bytes(payload).unwrap();
    let obj_path = storage.object_path(&digest);

    // Corrupt on-disk content
    fs::write(&obj_path, b"corrupted bytes").unwrap();

    let verify_res = storage.verify_object(&digest);
    assert!(verify_res.is_err());
    assert!(matches!(
        verify_res.unwrap_err(),
        CacheError::IntegrityError { .. }
    ));

    // Must be quarantined
    assert!(!obj_path.exists());
    assert!(obj_path.with_extension("corrupted").is_file());
}

#[test]
fn test_storage_suite_interrupted_write_simulation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    // Create a dangling .tmp file in tmp directory to simulate an interrupted/crashed process
    let tmp_file = storage.tmp_dir().join("interrupted_write_9999.tmp");
    fs::write(&tmp_file, b"partial incomplete write bytes").unwrap();
    assert!(tmp_file.is_file());

    // Normal CAS operations must remain completely unaffected and not see the incomplete .tmp file
    assert_eq!(storage.count_objects().unwrap(), 0);

    let payload = b"clean new object write";
    let (digest, _) = storage.store_object_bytes(payload).unwrap();
    assert!(storage.has_object(&digest));
    assert_eq!(storage.count_objects().unwrap(), 1);
}

#[test]
fn test_storage_suite_deletion() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    let payload = b"deletable object payload";
    let (digest, _) = storage.store_object_bytes(payload).unwrap();
    assert!(storage.has_object(&digest));

    let path = storage.object_path(&digest);
    assert!(fs::remove_file(path).is_ok());
    assert!(!storage.has_object(&digest));
    assert_eq!(storage.count_objects().unwrap(), 0);
}

#[test]
fn test_storage_suite_concurrent_reads() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = Arc::new(CasStorage::new(config).unwrap());

    let payload = b"concurrent read test data";
    let (digest, _) = storage.store_object_bytes(payload).unwrap();

    let mut handles = Vec::new();
    for _ in 0..16 {
        let storage_clone = Arc::clone(&storage);
        let digest_clone = digest.clone();
        let handle = thread::spawn(move || {
            for _ in 0..10 {
                let mut reader = storage_clone.get_object_reader(&digest_clone).unwrap();
                let mut buf = Vec::new();
                reader.read_to_end(&mut buf).unwrap();
                assert_eq!(buf, payload);
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn test_storage_suite_concurrent_writes() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = Arc::new(CasStorage::new(config).unwrap());

    let payload = b"concurrent write identical data";
    let mut handles = Vec::new();

    for i in 0..16 {
        let storage_clone = Arc::clone(&storage);
        let handle = thread::spawn(move || {
            if i % 2 == 0 {
                // Racing writes of the exact same content (deduplication race)
                let (d, s) = storage_clone.store_object_bytes(payload).unwrap();
                assert_eq!(s, payload.len() as u64);
                assert!(storage_clone.verify_object(&d).is_ok());
            } else {
                // Racing writes of distinct content
                let unique_data = format!("unique concurrent data {}", i);
                let (d, s) = storage_clone
                    .store_object_bytes(unique_data.as_bytes())
                    .unwrap();
                assert_eq!(s, unique_data.len() as u64);
                assert!(storage_clone.verify_object(&d).is_ok());
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().unwrap();
    }

    // tmp dir must be clean after concurrent writes
    assert!(fs::read_dir(storage.tmp_dir()).unwrap().next().is_none());
}

#[test]
fn test_storage_suite_nested_directories() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    let nested_dir = temp_dir
        .path()
        .join("deeply")
        .join("nested")
        .join("sub")
        .join("path");
    fs::create_dir_all(&nested_dir).unwrap();
    let nested_file = nested_dir.join("artifact.bin");
    let payload = b"nested directory content";
    fs::write(&nested_file, payload).unwrap();

    let (digest, size) = storage.store_object_from_file(&nested_file).unwrap();
    assert_eq!(size, payload.len() as u64);
    assert!(storage.has_object(&digest));
    assert!(storage.verify_object(&digest).is_ok());
}

#[test]
fn test_storage_suite_very_large_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    // Create 8 MB synthetic chunked test file
    let chunk_size = 64 * 1024; // 64 KB
    let total_chunks = 128; // 128 * 64KB = 8 MB
    let large_file_path = temp_dir.path().join("large_file.dat");
    {
        let mut f = File::create(&large_file_path).unwrap();
        let chunk = vec![0xABu8; chunk_size];
        for _ in 0..total_chunks {
            f.write_all(&chunk).unwrap();
        }
        f.flush().unwrap();
    }

    let (digest, size) = storage.store_object_from_file(&large_file_path).unwrap();
    assert_eq!(size, (chunk_size * total_chunks) as u64);
    assert!(storage.has_object(&digest));
    assert!(storage.verify_object(&digest).is_ok());

    let (largest_size, largest_path) = storage.largest_object().unwrap();
    assert_eq!(largest_size, size);
    assert_eq!(largest_path, Some(storage.object_path(&digest)));
}

#[test]
fn test_storage_suite_binary_files() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = StorageConfig {
        root_dir: temp_dir.path().to_path_buf(),
        max_size_bytes: None,
    };
    let storage = CasStorage::new(config).unwrap();

    // Byte pattern covering all 0..=255 values, null bytes, unicode invalid sequences
    let mut binary_data = Vec::with_capacity(1024);
    for i in 0..1024 {
        binary_data.push((i % 256) as u8);
    }

    let (digest, size) = storage.store_object_bytes(&binary_data).unwrap();
    assert_eq!(size, binary_data.len() as u64);
    assert!(storage.has_object(&digest));
    assert!(storage.verify_object(&digest).is_ok());

    let mut reader = storage.get_object_reader(&digest).unwrap();
    let mut read_back = Vec::new();
    reader.read_to_end(&mut read_back).unwrap();
    assert_eq!(read_back, binary_data);
}
