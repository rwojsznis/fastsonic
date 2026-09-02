//! What the interface shows about the client's own traffic.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Live view of the client's traffic, shared with the interface so it can
/// show that the app is talking to the server rather than being slow itself.
pub struct NetActivity {
    started_at: Instant,
    in_flight: AtomicUsize,
    /// Milliseconds since `started_at` when the oldest current burst began.
    busy_since_ms: AtomicU64,
}

impl Default for NetActivity {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            in_flight: AtomicUsize::new(0),
            busy_since_ms: AtomicU64::new(0),
        }
    }
}

impl NetActivity {
    fn now_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    pub(crate) fn begin(&self) {
        if self.in_flight.fetch_add(1, Ordering::SeqCst) == 0 {
            self.busy_since_ms.store(self.now_ms(), Ordering::SeqCst);
        }
    }

    pub(crate) fn end(&self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }

    /// Requests have been in flight continuously for at least `for_at_least`.
    pub fn busy(&self, for_at_least: Duration) -> bool {
        self.in_flight.load(Ordering::SeqCst) > 0
            && self
                .now_ms()
                .saturating_sub(self.busy_since_ms.load(Ordering::SeqCst))
                >= for_at_least.as_millis() as u64
    }
}

/// Decrements the in-flight count even if the request future is dropped.
pub(crate) struct ActivityGuard<'a>(pub(crate) &'a NetActivity);

impl Drop for ActivityGuard<'_> {
    fn drop(&mut self) {
        self.0.end();
    }
}
