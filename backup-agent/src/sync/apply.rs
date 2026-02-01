//! Delta application to reconstruct files.
//!
//! This module applies deltas to baseline files to reconstruct new versions.

use fast_rsync::apply;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Apply a delta to baseline data to reconstruct the new file
///
/// # Arguments
/// * `baseline_data` - Original (old) file data
/// * `delta` - Delta computed from signature
///
/// # Returns
/// * `Vec<u8>` - Reconstructed new file data
///
/// # Example
/// ```no_run
/// use backup_agent::sync::{
///     signature::generate_signature_from_bytes,
///     delta::compute_delta_from_bytes,
///     apply::apply_delta_to_bytes,
/// };
///
/// let baseline = b"Hello, World!";
/// let modified = b"Hello, Rust!";
///
/// // Generate signature and delta
/// let sig = generate_signature_from_bytes(baseline, None);
/// let delta = compute_delta_from_bytes(&sig, modified);
///
/// // Apply delta to reconstruct modified file
/// let reconstructed = apply_delta_to_bytes(baseline, &delta).unwrap();
/// assert_eq!(reconstructed, modified);
/// ```
pub fn apply_delta_to_bytes(baseline_data: &[u8], delta: &[u8]) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    apply(baseline_data, delta, &mut output)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(output)
}

/// Apply a delta to a baseline file and write the result to a new file
///
/// # Arguments
/// * `baseline_path` - Path to the baseline (old) file
/// * `delta` - Delta data
/// * `output_path` - Path where reconstructed file should be written
///
/// # Returns
/// * `Ok(usize)` - Number of bytes written
/// * `Err(io::Error)` - If files cannot be read/written
pub fn apply_delta_to_file(
    baseline_path: &Path,
    delta: &[u8],
    output_path: &Path,
) -> io::Result<usize> {
    // Read baseline file
    let baseline_data = fs::read(baseline_path)?;

    // Apply delta
    let reconstructed = apply_delta_to_bytes(&baseline_data, delta)?;

    // Write reconstructed file
    let mut output_file = fs::File::create(output_path)?;
    output_file.write_all(&reconstructed)?;

    Ok(reconstructed.len())
}

/// Apply delta in-place (overwrites the baseline file)
///
/// # Arguments
/// * `baseline_path` - Path to baseline file (will be overwritten)
/// * `delta` - Delta data
///
/// # Returns
/// * `Ok(usize)` - Number of bytes written
/// * `Err(io::Error)` - If operation fails
///
/// # Safety
/// This function overwrites the baseline file. Make sure you have a backup
/// if you need to preserve the original.
pub fn apply_delta_in_place(baseline_path: &Path, delta: &[u8]) -> io::Result<usize> {
    // Read baseline
    let baseline_data = fs::read(baseline_path)?;

    // Apply delta
    let reconstructed = apply_delta_to_bytes(&baseline_data, delta)?;

    // Overwrite baseline file
    let mut file = fs::File::create(baseline_path)?;
    file.write_all(&reconstructed)?;

    Ok(reconstructed.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{delta::compute_delta_from_bytes, signature::generate_signature_from_bytes};
    use tempfile::{NamedTempFile, TempDir};
    use std::io::Read;

    #[test]
    fn test_apply_delta_identical_files() -> io::Result<()> {
        let data = b"This is test data for delta application.";

        // Generate signature and delta (for identical file)
        let sig = generate_signature_from_bytes(data, None);
        let delta = compute_delta_from_bytes(&sig, data);

        // Apply delta
        let reconstructed = apply_delta_to_bytes(data, &delta)?;

        // Should reconstruct exact same data
        assert_eq!(reconstructed, data);

        Ok(())
    }

    #[test]
    fn test_apply_delta_modified_file() -> io::Result<()> {
        let baseline = b"Hello, World! This is the baseline version.";
        let modified = b"Hello, Rust! This is the modified version.";

        // Generate signature from baseline
        let sig = generate_signature_from_bytes(baseline, None);

        // Compute delta to modified
        let delta = compute_delta_from_bytes(&sig, modified);

        // Apply delta to reconstruct modified file
        let reconstructed = apply_delta_to_bytes(baseline, &delta)?;

        // Should match the modified version exactly
        assert_eq!(reconstructed, modified);

        Ok(())
    }

    #[test]
    fn test_apply_delta_to_file() -> io::Result<()> {
        let baseline = b"Baseline content";
        let modified = b"Modified content";

        // Create baseline file
        let mut baseline_file = NamedTempFile::new()?;
        baseline_file.write_all(baseline)?;
        baseline_file.flush()?;

        // Generate delta
        let sig = generate_signature_from_bytes(baseline, None);
        let delta = compute_delta_from_bytes(&sig, modified);

        // Apply delta to create output file
        let temp_dir = TempDir::new()?;
        let output_path = temp_dir.path().join("output.txt");

        let bytes_written = apply_delta_to_file(baseline_file.path(), &delta, &output_path)?;

        assert_eq!(bytes_written, modified.len());

        // Verify output file content
        let mut output_content = Vec::new();
        fs::File::open(&output_path)?.read_to_end(&mut output_content)?;

        assert_eq!(output_content, modified);

        Ok(())
    }

    #[test]
    fn test_apply_delta_in_place() -> io::Result<()> {
        let baseline = b"Original content that will be updated";
        let modified = b"Updated content after applying delta";

        // Create a file with baseline content
        let temp_dir = TempDir::new()?;
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, baseline)?;

        // Generate delta
        let sig = generate_signature_from_bytes(baseline, None);
        let delta = compute_delta_from_bytes(&sig, modified);

        // Apply delta in-place
        let bytes_written = apply_delta_in_place(&file_path, &delta)?;

        assert_eq!(bytes_written, modified.len());

        // Verify file was updated
        let updated_content = fs::read(&file_path)?;
        assert_eq!(updated_content, modified);

        Ok(())
    }

    #[test]
    fn test_round_trip_large_data() -> io::Result<()> {
        // Test with larger data to verify correctness
        let baseline = vec![b'A'; 10000];
        let mut modified = baseline.clone();
        // Make some changes in the middle
        for i in 5000..5100 {
            modified[i] = b'B';
        }

        // Generate signature and delta
        let sig = generate_signature_from_bytes(&baseline, None);
        let delta = compute_delta_from_bytes(&sig, &modified);

        // Apply delta
        let reconstructed = apply_delta_to_bytes(&baseline, &delta)?;

        // Verify perfect reconstruction
        assert_eq!(reconstructed, modified);

        Ok(())
    }
}
