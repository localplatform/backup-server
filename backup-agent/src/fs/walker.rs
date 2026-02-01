//! Directory traversal with metadata preservation.
//!
//! This module provides efficient directory traversal with full metadata
//! preservation for backup operations.

use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

/// Options for directory walking
#[derive(Debug, Clone)]
pub struct WalkOptions {
    /// Follow symbolic links
    pub follow_links: bool,

    /// Maximum depth (None = unlimited)
    pub max_depth: Option<usize>,

    /// Exclude patterns (glob-style)
    pub exclude_patterns: Vec<String>,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            follow_links: false,
            max_depth: None,
            exclude_patterns: vec![
                // Common excludes
                ".git".to_string(),
                "node_modules".to_string(),
                ".DS_Store".to_string(),
            ],
        }
    }
}

/// Information about a file discovered during walking
#[derive(Debug, Clone)]
pub struct FileInfo {
    /// Full path to the file
    pub path: PathBuf,

    /// Relative path from the root
    pub relative_path: PathBuf,

    /// File size in bytes
    pub size: u64,

    /// Is this a directory?
    pub is_dir: bool,

    /// Is this a symlink?
    pub is_symlink: bool,

    /// File depth from root
    pub depth: usize,
}

impl FileInfo {
    /// Create FileInfo from a DirEntry.
    /// For symlinks, resolves to the target to get the real file size.
    /// Returns None if the symlink target is a directory or cannot be resolved.
    fn from_entry(entry: &DirEntry, root: &Path) -> std::io::Result<Option<Self>> {
        let raw_metadata = entry.metadata()?;
        let path = entry.path().to_path_buf();
        let relative_path = path.strip_prefix(root)
            .unwrap_or(&path)
            .to_path_buf();
        let is_symlink = raw_metadata.is_symlink();

        // For symlinks, resolve to get the real file metadata
        let (size, is_dir) = if is_symlink {
            match std::fs::metadata(&path) {
                Ok(resolved) => {
                    if resolved.is_dir() {
                        // Symlink to directory — skip it
                        return Ok(None);
                    }
                    (resolved.len(), false)
                }
                Err(_) => {
                    // Broken symlink — skip it
                    return Ok(None);
                }
            }
        } else {
            (raw_metadata.len(), raw_metadata.is_dir())
        };

        Ok(Some(Self {
            path,
            relative_path,
            size,
            is_dir,
            is_symlink,
            depth: entry.depth(),
        }))
    }
}

/// Walk a directory tree and collect all files
///
/// # Arguments
/// * `root` - Root directory to start walking from
/// * `options` - Walking options (filters, depth, etc.)
///
/// # Returns
/// * `Ok(Vec<FileInfo>)` - List of all files found
/// * `Err(io::Error)` - If directory cannot be read
///
/// # Example
/// ```no_run
/// use backup_agent::fs::walker::{walk_directory, WalkOptions};
/// use std::path::Path;
///
/// let files = walk_directory(Path::new("/data"), WalkOptions::default()).unwrap();
/// println!("Found {} files", files.len());
/// ```
pub fn walk_directory(root: &Path, options: WalkOptions) -> std::io::Result<Vec<FileInfo>> {
    let mut files = Vec::new();

    let mut walker = WalkDir::new(root)
        .follow_links(options.follow_links);

    if let Some(max_depth) = options.max_depth {
        walker = walker.max_depth(max_depth);
    }

    for entry in walker {
        let entry = entry?;

        // Skip if matches exclude pattern
        if should_exclude(&entry, &options.exclude_patterns) {
            continue;
        }

        // Skip directories (we only want files for backup)
        if entry.file_type().is_dir() {
            continue;
        }

        if let Some(file_info) = FileInfo::from_entry(&entry, root)? {
            files.push(file_info);
        }
    }

    Ok(files)
}

/// Walk a directory tree with a callback for each file (for progress reporting)
///
/// # Arguments
/// * `root` - Root directory to start walking from
/// * `options` - Walking options
/// * `callback` - Called for each file discovered
///
/// # Returns
/// * `Ok(())` - If walk completed successfully
/// * `Err(io::Error)` - If directory cannot be read
pub fn walk_directory_with_callback<F>(
    root: &Path,
    options: WalkOptions,
    mut callback: F,
) -> std::io::Result<()>
where
    F: FnMut(&FileInfo),
{
    let mut walker = WalkDir::new(root)
        .follow_links(options.follow_links);

    if let Some(max_depth) = options.max_depth {
        walker = walker.max_depth(max_depth);
    }

    for entry in walker {
        let entry = entry?;

        if should_exclude(&entry, &options.exclude_patterns) {
            continue;
        }

        if entry.file_type().is_dir() {
            continue;
        }

        if let Some(file_info) = FileInfo::from_entry(&entry, root)? {
            callback(&file_info);
        }
    }

    Ok(())
}

/// Count files in a directory (fast, without collecting)
pub fn count_files(root: &Path, options: WalkOptions) -> std::io::Result<usize> {
    let mut count = 0;

    walk_directory_with_callback(root, options, |_| {
        count += 1;
    })?;

    Ok(count)
}

/// Calculate total size of all files in a directory
pub fn calculate_total_size(root: &Path, options: WalkOptions) -> std::io::Result<u64> {
    let mut total_size = 0u64;

    walk_directory_with_callback(root, options, |file| {
        total_size += file.size;
    })?;

    Ok(total_size)
}

/// Check if a directory entry should be excluded based on patterns
fn should_exclude(entry: &DirEntry, patterns: &[String]) -> bool {
    let file_name = entry.file_name().to_string_lossy();

    for pattern in patterns {
        if file_name.contains(pattern) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_walk_empty_directory() -> std::io::Result<()> {
        let temp_dir = TempDir::new()?;
        let files = walk_directory(temp_dir.path(), WalkOptions::default())?;
        assert_eq!(files.len(), 0);
        Ok(())
    }

    #[test]
    fn test_walk_with_files() -> std::io::Result<()> {
        let temp_dir = TempDir::new()?;

        // Create some test files
        fs::write(temp_dir.path().join("file1.txt"), b"content1")?;
        fs::write(temp_dir.path().join("file2.txt"), b"content2")?;

        let files = walk_directory(temp_dir.path(), WalkOptions::default())?;
        assert_eq!(files.len(), 2);

        Ok(())
    }

    #[test]
    fn test_walk_with_subdirectories() -> std::io::Result<()> {
        let temp_dir = TempDir::new()?;

        fs::create_dir(temp_dir.path().join("subdir"))?;
        fs::write(temp_dir.path().join("file1.txt"), b"content1")?;
        fs::write(temp_dir.path().join("subdir/file2.txt"), b"content2")?;

        let files = walk_directory(temp_dir.path(), WalkOptions::default())?;
        assert_eq!(files.len(), 2);

        Ok(())
    }

    #[test]
    fn test_count_files() -> std::io::Result<()> {
        let temp_dir = TempDir::new()?;

        fs::write(temp_dir.path().join("file1.txt"), b"test")?;
        fs::write(temp_dir.path().join("file2.txt"), b"test")?;

        let count = count_files(temp_dir.path(), WalkOptions::default())?;
        assert_eq!(count, 2);

        Ok(())
    }

    #[test]
    fn test_calculate_total_size() -> std::io::Result<()> {
        let temp_dir = TempDir::new()?;

        fs::write(temp_dir.path().join("file1.txt"), b"12345")?;     // 5 bytes
        fs::write(temp_dir.path().join("file2.txt"), b"1234567")?;   // 7 bytes

        let total = calculate_total_size(temp_dir.path(), WalkOptions::default())?;
        assert_eq!(total, 12);

        Ok(())
    }

    #[test]
    fn test_exclude_patterns() -> std::io::Result<()> {
        let temp_dir = TempDir::new()?;

        fs::write(temp_dir.path().join("file.txt"), b"keep")?;
        fs::write(temp_dir.path().join(".DS_Store"), b"exclude")?;

        let files = walk_directory(temp_dir.path(), WalkOptions::default())?;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].relative_path.to_str().unwrap(), "file.txt");

        Ok(())
    }
}
