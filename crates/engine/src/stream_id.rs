//! Redis stream ids.
//!
//! A stream id is `<milliseconds>-<sequence>`. Both halves are numbers, so they
//! must be compared as numbers: sorted as text, `"9-0"` sorts *after* `"10-0"`,
//! and an engine that trusted that would recover from an older snapshot and
//! replay commands it had already applied.

use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamId {
    pub ms: u64,
    pub seq: u64,
}

impl StreamId {
    /// The beginning of the stream. Used as the resume position when no usable
    /// snapshot exists, meaning "replay everything".
    pub const ZERO: StreamId = StreamId { ms: 0, seq: 0 };

    pub fn parse(s: &str) -> Option<StreamId> {
        let (ms, seq) = s.split_once('-')?;
        if ms.is_empty() || seq.is_empty() {
            return None;
        }
        Some(StreamId {
            ms: ms.parse().ok()?,
            seq: seq.parse().ok()?,
        })
    }
}

impl Ord for StreamId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ms.cmp(&other.ms).then(self.seq.cmp(&other.seq))
    }
}

impl PartialOrd for StreamId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.ms, self.seq)
    }
}
