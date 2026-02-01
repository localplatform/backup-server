//! File metadata handling for backup operations.
//!
//! This module preserves file metadata (permissions, timestamps, ownership)
//! for accurate restoration.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

/// Complete file metadata for backup/restore operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    /// File size in bytes
    pub size: u64,

    /// Last modified time (seconds since Unix epoch)
    pub modified: u64,

    /// File permissions (Unix mode bits)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<u32>,

    /// Is this a directory?
    pub is_dir: bool,

    /// Is this a symlink?
    pub is_symlink: bool,
}

impl FileMetadata {
    /// Extract metadata from a file path
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        let metadata = fs::metadata(path)?;

        let modified = metadata
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            Some(metadata.permissions().mode())
        };

        #[cfg(not(unix))]
        let permissions = None;

        Ok(Self {
            size: metadata.len(),
            modified,
            permissions,
            is_dir: metadata.is_dir(),
            is_symlink: metadata.is_symlink(),
        })
    }

    /// Apply this metadata to a file
    #[cfg(unix)]
    pub fn apply_to_path(&self, path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        // Set permissions if available
        if let Some(mode) = self.permissions {
            let perms = fs::Permissions::from_mode(mode);
            fs::set_permissions(path, perms)?;
        }

        // Note: Setting modified time requires additional platform-specific code
        // For now, we just preserve the metadata for later restoration

        Ok(())
    }

    #[cfg(not(unix))]
    pub fn apply_to_path(&self, _path: &Path) -> std::io::Result<()> {
        // On non-Unix platforms, metadata application is limited
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_extract_metadata() -> std::io::Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(b"test content")?;
        temp_file.flush()?;

        let metadata = FileMetadata::from_path(temp_file.path())?;

        assert_eq!(metadata.size, 12);
        assert!(!metadata.is_dir);
        assert!(!metadata.is_symlink);
        assert!(metadata.modified > 0);

        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn test_permissions_preservation() -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let temp_file = NamedTempFile::new()?;

        // Set specific permissions
        let perms = fs::Permissions::from_mode(0o644);
        fs::set_permissions(temp_file.path(), perms)?;

        let metadata = FileMetadata::from_path(temp_file.path())?;

        assert!(metadata.permissions.is_some());
        let mode = metadata.permissions.unwrap() & 0o777;
        assert_eq!(mode, 0o644);

        Ok(())
    }
}
