//! Byte-level progress tracking for backup operations.
//!
//! This module provides precise progress tracking with transfer speeds,
//! completion estimates, and real-time updates.

use std::time::{Duration, Instant};

/// Progress information for a file transfer operation
#[derive(Debug, Clone)]
pub struct TransferProgress {
    /// Total bytes to transfer
    pub total_bytes: u64,

    /// Bytes transferred so far
    pub transferred_bytes: u64,

    /// Current transfer speed in bytes/second
    pub bytes_per_second: u64,

    /// Estimated time remaining (seconds)
    pub eta_seconds: u64,

    /// Percentage complete (0-100)
    pub percent_complete: f64,

    /// Number of files processed
    pub files_processed: usize,

    /// Total number of files
    pub total_files: usize,

    /// Current file being processed
    pub current_file: Option<String>,
}

impl TransferProgress {
    /// Create a new progress tracker
    pub fn new(total_bytes: u64, total_files: usize) -> Self {
        Self {
            total_bytes,
            transferred_bytes: 0,
            bytes_per_second: 0,
            eta_seconds: 0,
            percent_complete: 0.0,
            files_processed: 0,
            total_files,
            current_file: None,
        }
    }

    /// Update progress with new transferred bytes
    pub fn update(&mut self, transferred_bytes: u64) {
        self.transferred_bytes = transferred_bytes;
        self.percent_complete = if self.total_bytes > 0 {
            (self.transferred_bytes as f64 / self.total_bytes as f64) * 100.0
        } else {
            0.0
        };
    }

    /// Set current file being processed
    pub fn set_current_file(&mut self, file_path: String) {
        self.current_file = Some(file_path);
    }

    /// Increment files processed counter
    pub fn increment_files(&mut self) {
        self.files_processed += 1;
    }

    /// Check if transfer is complete
    pub fn is_complete(&self) -> bool {
        self.transferred_bytes >= self.total_bytes
    }
}

/// Progress tracker with time-based speed calculation
pub struct ProgressTracker {
    start_time: Instant,
    last_update_time: Instant,
    last_bytes: u64,
    progress: TransferProgress,
}

impl ProgressTracker {
    /// Create a new progress tracker
    pub fn new(total_bytes: u64, total_files: usize) -> Self {
        let now = Instant::now();
        Self {
            start_time: now,
            last_update_time: now,
            last_bytes: 0,
            progress: TransferProgress::new(total_bytes, total_files),
        }
    }

    /// Update progress and calculate speed
    pub fn update(&mut self, transferred_bytes: u64) -> &TransferProgress {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update_time).as_secs_f64();

        // Calculate instantaneous speed
        if elapsed > 0.0 {
            let bytes_diff = transferred_bytes.saturating_sub(self.last_bytes);
            self.progress.bytes_per_second = (bytes_diff as f64 / elapsed) as u64;
        }

        // Calculate ETA
        if self.progress.bytes_per_second > 0 {
            let remaining_bytes = self.progress.total_bytes.saturating_sub(transferred_bytes);
            self.progress.eta_seconds = remaining_bytes / self.progress.bytes_per_second;
        }

        self.progress.update(transferred_bytes);
        self.last_update_time = now;
        self.last_bytes = transferred_bytes;

        &self.progress
    }

    /// Get total elapsed time
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Get average speed since start
    pub fn average_speed(&self) -> u64 {
        let elapsed = self.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            (self.progress.transferred_bytes as f64 / elapsed) as u64
        } else {
            0
        }
    }

    /// Get current progress
    pub fn progress(&self) -> &TransferProgress {
        &self.progress
    }

    /// Get mutable progress
    pub fn progress_mut(&mut self) -> &mut TransferProgress {
        &mut self.progress
    }
}

/// Format bytes as human-readable string
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    format!("{:.2} {}", size, UNITS[unit_index])
}

/// Format speed as human-readable string
pub fn format_speed(bytes_per_second: u64) -> String {
    format!("{}/s", format_bytes(bytes_per_second))
}

/// Format duration as human-readable string
pub fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_transfer_progress_new() {
        let progress = TransferProgress::new(1000, 10);
        assert_eq!(progress.total_bytes, 1000);
        assert_eq!(progress.transferred_bytes, 0);
        assert_eq!(progress.percent_complete, 0.0);
        assert_eq!(progress.total_files, 10);
    }

    #[test]
    fn test_transfer_progress_update() {
        let mut progress = TransferProgress::new(1000, 10);
        progress.update(500);
        assert_eq!(progress.transferred_bytes, 500);
        assert!((progress.percent_complete - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_transfer_progress_complete() {
        let mut progress = TransferProgress::new(1000, 10);
        assert!(!progress.is_complete());

        progress.update(1000);
        assert!(progress.is_complete());
        assert!((progress.percent_complete - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_progress_tracker() {
        let mut tracker = ProgressTracker::new(1000, 5);

        // First update
        let prog = tracker.update(100);
        assert_eq!(prog.transferred_bytes, 100);

        // Second update after a delay
        thread::sleep(Duration::from_millis(100));
        let prog = tracker.update(500);
        assert_eq!(prog.transferred_bytes, 500);
        assert!(prog.bytes_per_second > 0);
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0.00 B");
        assert_eq!(format_bytes(1023), "1023.00 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.00 GB");
    }

    #[test]
    fn test_format_speed() {
        assert_eq!(format_speed(1024), "1.00 KB/s");
        assert_eq!(format_speed(1024 * 1024), "1.00 MB/s");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(30), "30s");
        assert_eq!(format_duration(90), "1m 30s");
        assert_eq!(format_duration(3665), "1h 1m");
    }

    #[test]
    fn test_progress_increment_files() {
        let mut progress = TransferProgress::new(1000, 10);
        assert_eq!(progress.files_processed, 0);

        progress.increment_files();
        assert_eq!(progress.files_processed, 1);
    }

    #[test]
    fn test_progress_set_current_file() {
        let mut progress = TransferProgress::new(1000, 10);
        assert!(progress.current_file.is_none());

        progress.set_current_file("test.txt".to_string());
        assert_eq!(progress.current_file.as_deref(), Some("test.txt"));
    }
}
