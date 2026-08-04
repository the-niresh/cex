//! The public API: REST in front, loopback to the engine behind.
//!
//! Holds no exchange state of its own — every balance, order, and price lives in
//! the engine. That is what makes it safe to run as many copies as you need.

pub mod loopback;

pub use loopback::{Loopback, LoopbackConfig, LoopbackError};
