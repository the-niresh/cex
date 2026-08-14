//! Development-only load generator for the exchange.
//!
//! Not part of the deployed image (see `Dockerfile` and this crate's manifest
//! comment). Task 6 built the sample recorder in [`report`]; Task 7 adds the
//! driver that places orders and records into it.

pub mod quotes;
pub mod report;
pub mod venue;
