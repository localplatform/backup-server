//! Job tracking for managing running backup jobs.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;

/// Entry for a tracked job
struct TrackedJob {
    abort_handle: AbortHandle,
    cancel_token: CancellationToken,
}

/// Tracks running backup jobs and provides cancellation mechanism
#[derive(Clone)]
pub struct JobTracker {
    jobs: Arc<RwLock<HashMap<String, TrackedJob>>>,
}

impl JobTracker {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new job with its abort handle and cancellation token
    pub async fn register(&self, job_id: String, handle: AbortHandle, token: CancellationToken) {
        let mut jobs = self.jobs.write().await;
        jobs.insert(job_id, TrackedJob {
            abort_handle: handle,
            cancel_token: token,
        });
    }

    /// Cancel a running job by its ID
    pub async fn cancel(&self, job_id: &str) -> bool {
        let mut jobs = self.jobs.write().await;
        if let Some(tracked) = jobs.remove(job_id) {
            // Signal all tasks to stop via the cancellation token
            tracked.cancel_token.cancel();
            // Also abort the top-level task as a fallback
            tracked.abort_handle.abort();
            true
        } else {
            false
        }
    }

    /// Remove a job from tracking (called when job completes naturally)
    pub async fn complete(&self, job_id: &str) {
        let mut jobs = self.jobs.write().await;
        jobs.remove(job_id);
    }

    /// Get count of running jobs
    pub async fn running_count(&self) -> usize {
        let jobs = self.jobs.read().await;
        jobs.len()
    }
}

impl Default for JobTracker {
    fn default() -> Self {
        Self::new()
    }
}
