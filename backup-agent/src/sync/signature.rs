//! File signature generation using fast_rsync.
//!
//! This module uses the fast_rsync library to generate signatures for files
//! using a rolling hash algorithm (similar to rsync's algorithm).

use fast_rsync::{Signature, SignatureOptions};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Default block size for signature generation (16KB)
const DEFAULT_BLOCK_SIZE: u32 = 16 * 1024;

/// Generate a signature for a file at the given path.
///
/// Note: This reads the entire file into memory. For very large files,
/// consider using streaming approaches or chunking.
///
/// # Arguments
/// * `path` - Path to the file to generate a signature for
/// * `block_size` - Optional block size (defaults to 16KB)
///
/// # Returns
/// * `Ok(Signature)` - The generated signature
/// * `Err(io::Error)` - If the file cannot be read
pub fn generate_signature(path: &Path, block_size: Option<u32>) -> io::Result<Signature> {
    let data = fs::read(path)?;

    let options = SignatureOptions {
        block_size: block_size.unwrap_or(DEFAULT_BLOCK_SIZE),
        crypto_hash_size: 8, // Strong hash (8 bytes = 64 bits)
    };

    let signature = Signature::calculate(&data, options);

    Ok(signature)
}

/// Generate a signature from a byte buffer.
///
/// # Arguments
/// * `data` - Byte slice to generate a signature for
/// * `block_size` - Optional block size (defaults to 16KB)
///
/// # Returns
/// * `Signature` - The generated signature
pub fn generate_signature_from_bytes(data: &[u8], block_size: Option<u32>) -> Signature {
    let options = SignatureOptions {
        block_size: block_size.unwrap_or(DEFAULT_BLOCK_SIZE),
        crypto_hash_size: 8,
    };

    Signature::calculate(data, options)
}

/// Serialize a signature to bytes for transmission or storage.
pub fn serialize_signature(signature: &Signature) -> Vec<u8> {
    signature.serialized().to_vec()
}

/// Get the size of a serialized signature in bytes.
pub fn signature_size(signature: &Signature) -> usize {
    signature.serialized().len()
}

// ============================================================================
// Directory Tree Signature Generation
// ============================================================================

/// Signature information for a single file in a directory tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSignatureInfo {
    /// Relative path from directory root
    pub path: PathBuf,

    /// File size in bytes
    pub size: u64,

    /// Serialized signature
    pub signature: Vec<u8>,
}

/// Signature for an entire directory tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorySignature {
    /// Map of relative paths to their signature info
    pub files: HashMap<PathBuf, FileSignatureInfo>,

    /// Total number of files
    pub total_files: usize,

    /// Total size of all files in bytes
    pub total_bytes: u64,
}

/// Generate signatures for all files in a directory tree
///
/// # Arguments
/// * `root` - Root directory to scan
/// * `block_size` - Optional block size for signatures
///
/// # Returns
/// * `Ok(DirectorySignature)` - Signatures for all files
/// * `Err(io::Error)` - If directory cannot be read
pub fn generate_directory_signature(
    root: &Path,
    block_size: Option<u32>,
) -> io::Result<DirectorySignature> {
    use crate::fs::walker::{walk_directory, WalkOptions};

    let files_list = walk_directory(root, WalkOptions::default())?;

    let mut files = HashMap::new();
    let mut total_bytes = 0u64;

    for file_info in &files_list {
        // Generate signature for this file
        let signature = generate_signature(&file_info.path, block_size)?;
        let serialized = serialize_signature(&signature);

        total_bytes += file_info.size;

        let sig_info = FileSignatureInfo {
            path: file_info.relative_path.clone(),
            size: file_info.size,
            signature: serialized,
        };

        files.insert(file_info.relative_path.clone(), sig_info);
    }

    Ok(DirectorySignature {
        files,
        total_files: files_list.len(),
        total_bytes,
    })
}

/// Generate signatures for a directory tree with progress callback
///
/// # Arguments
/// * `root` - Root directory to scan
/// * `block_size` - Optional block size for signatures
/// * `progress` - Callback called for each file processed (current_file, total_files)
///
/// # Returns
/// * `Ok(DirectorySignature)` - Signatures for all files
/// * `Err(io::Error)` - If directory cannot be read
pub fn generate_directory_signature_with_progress<F>(
    root: &Path,
    block_size: Option<u32>,
    mut progress: F,
) -> io::Result<DirectorySignature>
where
    F: FnMut(usize, usize),
{
    use crate::fs::walker::{walk_directory, WalkOptions};

    let files_list = walk_directory(root, WalkOptions::default())?;
    let total_files = files_list.len();

    let mut files = HashMap::new();
    let mut total_bytes = 0u64;

    for (idx, file_info) in files_list.iter().enumerate() {
        // Report progress
        progress(idx + 1, total_files);

        // Generate signature for this file
        let signature = generate_signature(&file_info.path, block_size)?;
        let serialized = serialize_signature(&signature);

        total_bytes += file_info.size;

        let sig_info = FileSignatureInfo {
            path: file_info.relative_path.clone(),
            size: file_info.size,
            signature: serialized,
        };

        files.insert(file_info.relative_path.clone(), sig_info);
    }

    Ok(DirectorySignature {
        files,
        total_files,
        total_bytes,
    })
}

/// Serialize a directory signature to JSON
pub fn serialize_directory_signature(sig: &DirectorySignature) -> serde_json::Result<String> {
    serde_json::to_string(sig)
}

/// Deserialize a directory signature from JSON
pub fn deserialize_directory_signature(json: &str) -> serde_json::Result<DirectorySignature> {
    serde_json::from_str(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_signature_from_bytes() {
        let data = b"Hello, World!";
        let sig = generate_signature_from_bytes(data, None);

        // Signature should be serializable
        let serialized = serialize_signature(&sig);
        assert!(serialized.len() > 0);
    }

    #[test]
    fn test_signature_with_custom_block_size() {
        let data = vec![0u8; 1024 * 64]; // 64KB of zeros
        let sig = generate_signature_from_bytes(&data, Some(4096)); // 4KB blocks

        // Verify signature was created
        let size = signature_size(&sig);
        assert!(size > 0);
    }

    #[test]
    fn test_signature_from_file() -> io::Result<()> {
        // Create a temporary file
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(b"Test file content for signature generation")?;
        temp_file.flush()?;

        // Generate signature from file
        let sig = generate_signature(temp_file.path(), None)?;

        // Verify signature was created
        assert!(signature_size(&sig) > 0);

        Ok(())
    }

    #[test]
    fn test_serialize_signature() {
        let data = b"Data to sign";
        let sig = generate_signature_from_bytes(data, None);
        let serialized = serialize_signature(&sig);

        // Serialized data should be non-empty
        assert!(!serialized.is_empty());
    }

    #[test]
    fn test_directory_signature() -> io::Result<()> {
        use tempfile::TempDir;
        use std::fs;

        // Create a temporary directory with some files
        let temp_dir = TempDir::new()?;
        fs::write(temp_dir.path().join("file1.txt"), b"content1")?;
        fs::write(temp_dir.path().join("file2.txt"), b"content2 longer")?;

        // Generate directory signature
        let dir_sig = generate_directory_signature(temp_dir.path(), None)?;

        // Verify results
        assert_eq!(dir_sig.total_files, 2);
        assert_eq!(dir_sig.total_bytes, 8 + 15); // content1 + content2 longer
        assert_eq!(dir_sig.files.len(), 2);

        // Verify signatures were generated
        assert!(dir_sig.files.contains_key(Path::new("file1.txt")));
        assert!(dir_sig.files.contains_key(Path::new("file2.txt")));

        Ok(())
    }

    #[test]
    fn test_directory_signature_with_progress() -> io::Result<()> {
        use tempfile::TempDir;
        use std::fs;
        use std::sync::{Arc, Mutex};

        let temp_dir = TempDir::new()?;
        fs::write(temp_dir.path().join("file1.txt"), b"test1")?;
        fs::write(temp_dir.path().join("file2.txt"), b"test2")?;

        // Track progress calls
        let progress_calls = Arc::new(Mutex::new(Vec::new()));
        let progress_calls_clone = progress_calls.clone();

        let dir_sig = generate_directory_signature_with_progress(
            temp_dir.path(),
            None,
            move |current, total| {
                progress_calls_clone.lock().unwrap().push((current, total));
            },
        )?;

        assert_eq!(dir_sig.total_files, 2);

        // Verify progress was called
        let calls = progress_calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], (1, 2));
        assert_eq!(calls[1], (2, 2));

        Ok(())
    }

    #[test]
    fn test_serialize_directory_signature() -> io::Result<()> {
        use tempfile::TempDir;
        use std::fs;

        let temp_dir = TempDir::new()?;
        fs::write(temp_dir.path().join("test.txt"), b"test content")?;

        let dir_sig = generate_directory_signature(temp_dir.path(), None)?;

        // Serialize to JSON
        let json = serialize_directory_signature(&dir_sig).unwrap();
        assert!(!json.is_empty());

        // Deserialize back
        let deserialized = deserialize_directory_signature(&json).unwrap();
        assert_eq!(deserialized.total_files, dir_sig.total_files);
        assert_eq!(deserialized.total_bytes, dir_sig.total_bytes);

        Ok(())
    }
}
