//! Backoff for a loop that keeps failing on the same error.

use std::time::Duration;

const INITIAL: Duration = Duration::from_millis(250);
const MAX: Duration = Duration::from_secs(30);

/// Grows from 250 ms to 30 s and logs only when the wait changes.
pub struct Backoff {
    wait: Duration,
    logged_wait: Option<Duration>,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

impl Backoff {
    pub fn new() -> Self {
        Backoff {
            wait: INITIAL,
            logged_wait: None,
        }
    }

    /// Reset after a successful step.
    pub fn reset(&mut self) {
        self.wait = INITIAL;
        self.logged_wait = None;
    }

    /// Sleep before the next retry. Returns whether a new wait was logged.
    pub async fn sleep(&mut self) -> bool {
        let should_log = self.logged_wait != Some(self.wait);
        if should_log {
            self.logged_wait = Some(self.wait);
        }
        tokio::time::sleep(self.wait).await;
        self.wait = (self.wait * 2).min(MAX);
        should_log
    }
}
