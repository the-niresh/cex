//! The public API: REST in front, loopback to the engine behind.
//!
//! Holds no exchange state of its own — every balance, order, and price lives in
//! the engine. That is what makes it safe to run as many copies as you need.

pub mod auth;
pub mod loopback;
pub mod routes;
pub mod users;

pub use auth::{hash_password, verify_password, Tokens};
pub use loopback::{Loopback, LoopbackConfig, LoopbackError};
pub use routes::{build_router, build_router_with_cors, AppState, CorsSettings, InvalidOrigin};
pub use users::{User, UserStore, UsersError};
