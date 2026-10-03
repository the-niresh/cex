//! In-memory rate limits for endpoints that must not be abused.
//!
//! No external store: these counters reset when the process restarts, which is
//! acceptable for guest creation where the goal is slowing a flood rather than
//! a perfect global quota.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How many guest accounts one IP may create per window.
pub const GUEST_LIMIT: usize = 5;

/// How long a guest-creation window lasts.
pub const GUEST_WINDOW: Duration = Duration::from_secs(3600);

/// Per-IP sliding window of recent guest creations.
pub struct GuestRateLimit {
    hits: Mutex<HashMap<IpAddr, Vec<Instant>>>,
}

impl GuestRateLimit {
    pub fn new() -> Self {
        GuestRateLimit {
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Record one attempt. Returns `false` when the IP is over the limit.
    pub fn allow(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut map = self.hits.lock().expect("guest rate limit lock");
        let entry = map.entry(ip).or_default();
        entry.retain(|t| now.duration_since(*t) < GUEST_WINDOW);
        if entry.len() >= GUEST_LIMIT {
            return false;
        }
        entry.push(now);
        true
    }
}

impl Default for GuestRateLimit {
    fn default() -> Self {
        Self::new()
    }
}
