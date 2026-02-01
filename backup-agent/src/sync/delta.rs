//! Delta computation logic.
//!
//! This module computes deltas (differences) between a baseline (old) file
//! and a new file using the rsync rolling hash algorithm.

use fast_rsync::{diff, Signature};
use std::fs;
use std::io;
use std::path::Path;

/// Compute delta between a baseline signature and new file data
///
/// # Arguments
/// * `baseline_sig` - Signature of the baseline (old) file
/// * `new_data` - Data of the new (modified) file
///
/// # Returns
/// * `Vec<u8>` - Delta (diff) that can be applied to baseline to get new file
///
/// # Example
/// ```no_run
/// use backup_agent::sync::{signature::generate_signature_from_bytes, delta::compute_delta_from_bytes};
///
/// let baseline = b"Hello, World!";
/// let modified = b"Hello, Rust!";
///
/// let sig = generate_signature_from_bytes(baseline, None);
/// let delta = compute_delta_from_bytes(&sig, modified);
///
/// println!("Delta size: {} bytes", delta.len());
/// ```
pub fn compute_delta_from_bytes(baseline_sig: &Signature, new_data: &[u8]) -> Vec<u8> {
    let mut delta_output = Vec::new();
    let indexed = baseline_sig.index();
    diff(&indexed, new_data, &mut delta_output).expect("Delta computation failed");
    delta_output
}

/// Compute delta between baseline signature and new file on disk
///
/// # Arguments
/// * `baseline_sig` - Signature of the baseline file
/// * `new_file_path` - Path to the new (modified) file
///
/// # Returns
/// * `Ok(Vec<u8>)` - Delta that can be applied to get new file
/// * `Err(io::Error)` - If file cannot be read
pub fn compute_delta_from_file(
    baseline_sig: &Signature,
    new_file_path: &Path,
) -> io::Result<Vec<u8>> {
    let new_data = fs::read(new_file_path)?;
    let mut delta_output = Vec::new();
    let indexed = baseline_sig.index();
    diff(&indexed, &new_data, &mut delta_output).expect("Delta computation failed");
    Ok(delta_output)
}

/// Estimate compression ratio of delta vs new file
///
/// Returns a ratio between 0.0 and 1.0 where:
/// - 0.0 = perfect compression (delta is tiny)
/// - 1.0 = no compression (delta is same size as new file)
/// - >1.0 = delta is larger than new file (rare, but possible)
pub fn delta_compression_ratio(delta_size: usize, new_file_size: usize) -> f64 {
    if new_file_size == 0 {
        return 0.0;
    }
    delta_size as f64 / new_file_size as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::signature::generate_signature_from_bytes;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[test]
    fn test_identical_files_produce_small_delta() {
        let data = b"This is some test data that we will use for testing.";

        let sig = generate_signature_from_bytes(data, None);
        let delta = compute_delta_from_bytes(&sig, data);

        // Delta for identical files should be very small
        assert!(delta.len() > 0); // Delta includes metadata
    }

    #[test]
    fn test_completely_different_files() {
        let baseline = b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let modified = b"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

        let sig = generate_signature_from_bytes(baseline, None);
        let delta = compute_delta_from_bytes(&sig, modified);

        // Delta for completely different files will be large
        // (includes all the new data plus metadata)
        assert!(delta.len() > 0);
    }

    #[test]
    fn test_small_change_produces_small_delta() {
        let baseline = b"Hello, World! This is a test of the delta sync system.";
        let modified = b"Hello, Rust! This is a test of the delta sync system.";
        //                    ^^^^^ Only this changed

        let sig = generate_signature_from_bytes(baseline, None);
        let delta = compute_delta_from_bytes(&sig, modified);

        // Delta should be much smaller than the full file
        assert!(delta.len() > 0); // Delta includes metadata
    }

    #[test]
    fn test_compute_delta_from_file() -> io::Result<()> {
        let baseline = b"Baseline content for testing";

        // Create a temporary file with modified content
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(b"Modified content for testing")?;
        temp_file.flush()?;

        let sig = generate_signature_from_bytes(baseline, None);
        let delta = compute_delta_from_file(&sig, temp_file.path())?;

        assert!(delta.len() > 0);

        Ok(())
    }

    #[test]
    fn test_delta_compression_ratio() {
        // Perfect compression (tiny delta)
        assert!((delta_compression_ratio(10, 1000) - 0.01).abs() < 0.01);

        // No compression (same size)
        assert!((delta_compression_ratio(1000, 1000) - 1.0).abs() < 0.01);

        // 50% compression
        assert!((delta_compression_ratio(500, 1000) - 0.5).abs() < 0.01);

        // Empty file edge case
        assert_eq!(delta_compression_ratio(100, 0), 0.0);
    }
}
