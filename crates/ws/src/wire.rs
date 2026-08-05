//! What crosses the WebSocket, in both directions.
//!
//! ## Why the public feed does not reuse `cex_proto::Fill`
//!
//! A `Fill` names both counterparties: `maker_user_id` and `taker_user_id`.
//! Forwarding one to `trades@SYMBOL` would tell every connected client who
//! traded with whom. The public message below is a separate type carrying only
//! what a market data feed is entitled to say — symbol, price, quantity, and
//! which side was the aggressor.
//!
//! Keeping it a distinct type rather than a filtered copy means the compiler is
//! what stops a user id reaching the public channel, not somebody's memory.

use cex_proto::{DepthDelta, OrderId, OrderStatus, OrderType, Seq, Side};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

// ───────────────────────── channels ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Channel {
    /// Public incremental book updates for one market.
    Depth(String),
    /// Public trade prints for one market.
    Trades(String),
    /// Private. Your own orders and your own fills, and nobody else's.
    Orders,
}

impl Channel {
    /// Whether a subscription requires an authenticated connection.
    pub fn is_private(&self) -> bool {
        matches!(self, Channel::Orders)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown channel {0:?}")]
pub struct UnknownChannel(pub String);

impl FromStr for Channel {
    type Err = UnknownChannel;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "orders" {
            return Ok(Channel::Orders);
        }
        match s.split_once('@') {
            // An empty symbol would subscribe to a market that cannot exist and
            // would quietly never deliver anything. Refuse it instead.
            Some((_, "")) => Err(UnknownChannel(s.to_string())),
            Some(("depth", symbol)) => Ok(Channel::Depth(symbol.to_string())),
            Some(("trades", symbol)) => Ok(Channel::Trades(symbol.to_string())),
            _ => Err(UnknownChannel(s.to_string())),
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Channel::Depth(s) => write!(f, "depth@{s}"),
            Channel::Trades(s) => write!(f, "trades@{s}"),
            Channel::Orders => write!(f, "orders"),
        }
    }
}

// ───────────────────────── client → server ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Attach an identity to this connection. Required before subscribing to
    /// any private channel.
    Auth { token: String },
    Subscribe { channels: Vec<String> },
    Unsubscribe { channels: Vec<String> },
}

// ───────────────────────── server → client ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Everything this connection is now subscribed to, after the change.
    Subscribed { channels: Vec<String> },
    Authenticated,
    Error { error: String },
}

/// One market data or private update, as it reaches a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub channel: String,
    /// The engine's gap-free command counter. A client that sees a jump knows
    /// it missed something.
    pub seq: Seq,
    pub data: Payload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum Payload {
    Depth(DepthUpdate),
    Trade(PublicTrade),
    Order(OrderUpdate),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepthUpdate {
    pub symbol: String,
    /// Per-symbol and monotonic. A gap here means resync rather than drift.
    pub depth_seq: u64,
    pub deltas: Vec<DepthDelta>,
}

/// A trade print, as the whole world may see it.
///
/// Deliberately has no user fields, and must never grow any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicTrade {
    pub symbol: String,
    pub price: i64,
    pub qty: i64,
    /// Side of the aggressing order.
    pub taker_side: Side,
}

/// Something that happened to one user's own order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum OrderUpdate {
    Accepted {
        order_id: OrderId,
        symbol: String,
        side: Side,
        order_type: OrderType,
        price: Option<i64>,
        qty: i64,
    },
    Updated {
        order_id: OrderId,
        filled_qty: i64,
        qty: i64,
        status: OrderStatus,
    },
    Cancelled {
        order_id: OrderId,
        symbol: String,
        unfilled_qty: i64,
    },
    Rejected {
        symbol: String,
        reason: String,
    },
    /// One of this user's orders traded. Carries their own side, their own fee
    /// and their own role — never the counterparty's identity.
    Fill {
        order_id: OrderId,
        symbol: String,
        price: i64,
        qty: i64,
        side: Side,
        fee: i64,
        role: Role,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Maker,
    Taker,
}
