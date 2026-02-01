//! Progress-tracking stream wrapper for real-time upload progress.

use bytes::Bytes;
use futures_util::Stream;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::time::{Instant, Duration};

/// Callback for progress updates
pub type ProgressCallback = Arc<dyn Fn(u64) + Send + Sync>;

/// Stream wrapper that tracks bytes transferred and calls a progress callback
pub struct ProgressStream<S> {
    inner: S,
    bytes_transferred: u64,
    last_update: Instant,
    update_interval: Duration,
    callback: ProgressCallback,
}

impl<S> ProgressStream<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>>,
{
    /// Create a new progress stream
    pub fn new(inner: S, callback: ProgressCallback) -> Self {
        Self {
            inner,
            bytes_transferred: 0,
            last_update: Instant::now(),
            update_interval: Duration::from_millis(250), // 4 updates per second
            callback,
        }
    }

    /// Get total bytes transferred
    pub fn bytes_transferred(&self) -> u64 {
        self.bytes_transferred
    }
}

impl<S> Stream for ProgressStream<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let inner = Pin::new(&mut self.inner);

        match inner.poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                let chunk_size = bytes.len() as u64;
                self.bytes_transferred += chunk_size;

                // Call progress callback if enough time has passed
                let now = Instant::now();
                if now.duration_since(self.last_update) >= self.update_interval {
                    (self.callback)(self.bytes_transferred);
                    self.last_update = now;
                }

                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => {
                // Final update on completion
                (self.callback)(self.bytes_transferred);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
