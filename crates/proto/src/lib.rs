//! Every message that crosses a process boundary.
//!
//! Two request families, deliberately separated:
//!
//! * [`Command`] mutates state. It is appended to the durable Redis stream and
//!   is what gets replayed on recovery.
//! * [`Query`] reads state. It never touches the log — logging reads would bloat
//!   the stream and slow every replay down for no benefit.
//!
//! Both are answered on the loopback channel keyed by `request_id`.

use serde::{Deserialize, Serialize};

pub type UserId = uuid::Uuid;
pub type OrderId = u64;
pub type RequestId = uuid::Uuid;

/// Sequence number assigned by the engine to every applied command. Monotonic,
/// gap-free, and the ordering authority for everything downstream.
pub type Seq = u64;

// ─────────────────────────── primitives ───────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    #[inline]
    pub fn opposite(self) -> Side {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderType {
    Limit,
    Market,
}

/// Time in force. Only the two that matter for a spot book today; the rest are a
/// small addition once the core is right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TimeInForce {
    /// Rest on the book until filled or cancelled.
    Gtc,
    /// Match what is available immediately, cancel the remainder.
    Ioc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderStatus {
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
}

impl OrderStatus {
    #[inline]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            OrderStatus::Filled | OrderStatus::Cancelled | OrderStatus::Rejected
        )
    }
}

// ─────────────────────────── commands ───────────────────────────

/// State-mutating requests. Appended to the durable log, replayed on recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Credit an account. Stands in for a real deposit pipeline.
    Deposit {
        request_id: RequestId,
        user_id: UserId,
        asset: String,
        /// Atomic units of `asset`.
        amount: i64,
    },
    Withdraw {
        request_id: RequestId,
        user_id: UserId,
        asset: String,
        amount: i64,
    },
    PlaceOrder {
        request_id: RequestId,
        user_id: UserId,
        symbol: String,
        side: Side,
        order_type: OrderType,
        #[serde(default)]
        time_in_force: Option<TimeInForce>,
        /// Required for `Limit`, ignored for `Market`. Quote atoms per whole base unit.
        #[serde(default)]
        price: Option<i64>,
        /// Atomic units of the base asset.
        qty: i64,
    },
    CancelOrder {
        request_id: RequestId,
        user_id: UserId,
        order_id: OrderId,
    },
}

impl Command {
    pub fn request_id(&self) -> RequestId {
        match self {
            Command::Deposit { request_id, .. }
            | Command::Withdraw { request_id, .. }
            | Command::PlaceOrder { request_id, .. }
            | Command::CancelOrder { request_id, .. } => *request_id,
        }
    }
}

// ─────────────────────────── queries ───────────────────────────

/// Read-only requests. Answered from the same single-threaded state as commands,
/// so a read is always consistent with the last applied command — but never logged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "query", rename_all = "snake_case")]
pub enum Query {
    Depth {
        request_id: RequestId,
        symbol: String,
        #[serde(default)]
        limit: Option<usize>,
    },
    Balances {
        request_id: RequestId,
        user_id: UserId,
    },
    Order {
        request_id: RequestId,
        user_id: UserId,
        order_id: OrderId,
    },
    OpenOrders {
        request_id: RequestId,
        user_id: UserId,
        #[serde(default)]
        symbol: Option<String>,
    },
    Markets {
        request_id: RequestId,
    },
}

impl Query {
    pub fn request_id(&self) -> RequestId {
        match self {
            Query::Depth { request_id, .. }
            | Query::Balances { request_id, .. }
            | Query::Order { request_id, .. }
            | Query::OpenOrders { request_id, .. }
            | Query::Markets { request_id } => *request_id,
        }
    }
}

// ─────────────────────────── events ───────────────────────────

/// A single match between a resting maker and an incoming taker.
///
/// `price` is always the **maker's** resting price — price improvement belongs to
/// the taker, not to whichever side happened to be cheaper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fill {
    pub symbol: String,
    pub price: i64,
    pub qty: i64,
    pub maker_order_id: OrderId,
    pub taker_order_id: OrderId,
    pub maker_user_id: UserId,
    pub taker_user_id: UserId,
    /// Side of the aggressing order.
    pub taker_side: Side,
    /// Quote atoms paid by the taker to the maker, before fees.
    pub notional: i64,
    pub maker_fee: i64,
    pub taker_fee: i64,
}

/// Emitted for every price level the book touched while applying one command.
/// A `qty` of zero means the level is now empty and should be dropped by the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepthDelta {
    pub side: Side,
    pub price: i64,
    pub qty: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Deposited {
        user_id: UserId,
        asset: String,
        amount: i64,
        available: i64,
    },
    Withdrawn {
        user_id: UserId,
        asset: String,
        amount: i64,
        available: i64,
    },
    OrderAccepted {
        order_id: OrderId,
        user_id: UserId,
        symbol: String,
        side: Side,
        order_type: OrderType,
        price: Option<i64>,
        qty: i64,
    },
    OrderRejected {
        user_id: UserId,
        symbol: String,
        reason: String,
    },
    Trades {
        symbol: String,
        fills: Vec<Fill>,
    },
    OrderUpdated {
        order_id: OrderId,
        user_id: UserId,
        filled_qty: i64,
        qty: i64,
        status: OrderStatus,
    },
    OrderCancelled {
        order_id: OrderId,
        user_id: UserId,
        symbol: String,
        /// Quantity that was still resting when the cancel landed.
        unfilled_qty: i64,
    },
    /// Incremental book update. `depth_seq` is per-symbol and monotonic, so a
    /// client that sees a gap knows to resync rather than drift silently.
    DepthUpdated {
        symbol: String,
        depth_seq: u64,
        deltas: Vec<DepthDelta>,
    },
    BalanceUpdated {
        user_id: UserId,
        asset: String,
        available: i64,
        locked: i64,
    },
}

/// An event as it appears on the output stream: the engine's sequence number plus
/// the command that produced it, so `persist` can deduplicate on redelivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StampedEvent {
    pub seq: Seq,
    pub event: Event,
}

// ─────────────────────────── responses ───────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepthSnapshot {
    pub symbol: String,
    pub depth_seq: u64,
    /// Best first: descending price.
    pub bids: Vec<[i64; 2]>,
    /// Best first: ascending price.
    pub asks: Vec<[i64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceView {
    pub asset: String,
    pub available: i64,
    pub locked: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderView {
    pub order_id: OrderId,
    pub user_id: UserId,
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub price: Option<i64>,
    pub qty: i64,
    pub filled_qty: i64,
    pub status: OrderStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketView {
    pub symbol: String,
    pub base: String,
    pub quote: String,
    pub base_decimals: u32,
    pub quote_decimals: u32,
    pub tick_size: i64,
    pub lot_size: i64,
    pub min_notional: i64,
    pub maker_fee_bps: i64,
    pub taker_fee_bps: i64,
}

/// Result payload delivered back over the loopback channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ok", rename_all = "snake_case")]
pub enum ResponseBody {
    Ack,
    Balances(Vec<BalanceView>),
    Depth(DepthSnapshot),
    Order(OrderView),
    Orders(Vec<OrderView>),
    Markets(Vec<MarketView>),
    OrderPlaced {
        order_id: OrderId,
        status: OrderStatus,
        filled_qty: i64,
        qty: i64,
        /// Volume-weighted average fill price, `None` if nothing filled.
        avg_price: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub request_id: RequestId,
    #[serde(flatten)]
    pub result: ResponseResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseResult {
    Ok { data: ResponseBody },
    Err { error: String },
}

impl Response {
    pub fn ok(request_id: RequestId, data: ResponseBody) -> Self {
        Response {
            request_id,
            result: ResponseResult::Ok { data },
        }
    }

    pub fn err(request_id: RequestId, error: impl Into<String>) -> Self {
        Response {
            request_id,
            result: ResponseResult::Err {
                error: error.into(),
            },
        }
    }
}

// ─────────────────────────── channel names ───────────────────────────

/// Durable input log. Source of truth; replayed on recovery.
pub const STREAM_COMMANDS: &str = "cex:commands";
/// Durable output log. Read by `ws` and `persist` via independent consumer groups.
pub const STREAM_EVENTS: &str = "cex:events";
/// Ephemeral read requests. Never logged.
pub const QUEUE_QUERIES: &str = "cex:queries";
/// Loopback replies, keyed by `request_id` on the subscriber side.
pub const CHANNEL_RESPONSES: &str = "cex:responses";
/// Field name used for the JSON payload inside every stream entry.
pub const FIELD_PAYLOAD: &str = "payload";
