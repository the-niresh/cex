//! The market data process: `cex:events` in, WebSocket subscribers out.
//!
//! Reads the event stream **once**, through its own consumer group, and hands
//! every subscriber a copy through a `tokio::sync::broadcast` channel. The
//! alternative — one Redis reader per connection — would multiply load on the
//! stream by the number of clients, which is exactly backwards for the one
//! component whose job is to have a lot of clients.
//!
//! ## The two rules this crate exists to keep
//!
//! **A public channel never carries a user id.** Enforced by type: the public
//! trade message has no user fields, so forwarding a `Fill` to `trades@SYMBOL`
//! does not compile. See [`wire`].
//!
//! **A slow subscriber is dropped, never allowed to stall the others.** A
//! broadcast channel gives every receiver its own cursor into a shared ring
//! buffer, so one connection that stops reading falls behind on its own. When
//! it falls off the end its connection is closed, telling it to reconnect and
//! resync rather than silently receiving a feed with a hole in it.

pub mod config;
pub mod feed;
pub mod route;
pub mod server;
pub mod session;
pub mod wire;

pub use config::Config;
pub use feed::Feed;
pub use route::{route, Update};
pub use server::{build_router, AppState};
pub use session::{Session, SessionError};
pub use wire::{Channel, ClientMessage, Envelope, Payload, ServerMessage};
