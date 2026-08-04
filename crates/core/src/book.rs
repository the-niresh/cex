//! The order book for a single market.
//!
//! Layout, and why:
//!
//! * `BTreeMap<price, Level>` per side. Best ask is `first_key_value`, best bid is
//!   `last_key_value`, both O(log n).
//! * A `Level` holds only `OrderId`s in a `VecDeque`, never the orders themselves.
//!   The orders live in one `HashMap` arena. This is what makes the match loop
//!   compile: it borrows the level map and the arena as two disjoint fields rather
//!   than fighting over one nested borrow.
//! * Cancel is O(1). It marks the order terminal in the arena and decrements the
//!   level total, leaving a tombstone in the queue that matching steps over. No
//!   scan of a level, ever.
//!
//! The book knows nothing about balances. It reports what happened; the engine
//! layer moves the money.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use cex_proto::{DepthDelta, OrderId, OrderStatus, OrderType, Side, TimeInForce, UserId};
use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::market::Market;
use crate::math::{MathError, Rounding};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    pub id: OrderId,
    pub user_id: UserId,
    pub side: Side,
    pub order_type: OrderType,
    pub time_in_force: TimeInForce,
    /// Limit price in quote atoms per whole base unit. Zero for a market order.
    pub price: i64,
    pub qty: i64,
    pub filled_qty: i64,
    pub status: OrderStatus,
    /// Atoms of the locked asset still reserved against this order — quote for a
    /// buy, base for a sell. Owned here so a cancel refunds the exact amount that
    /// was taken rather than a recomputed approximation. Set by the engine layer.
    pub reserved_remaining: i64,
}

impl Order {
    pub fn limit(
        id: OrderId,
        user_id: UserId,
        side: Side,
        price: i64,
        qty: i64,
        time_in_force: TimeInForce,
    ) -> Self {
        Order {
            id,
            user_id,
            side,
            order_type: OrderType::Limit,
            time_in_force,
            price,
            qty,
            filled_qty: 0,
            status: OrderStatus::Open,
            reserved_remaining: 0,
        }
    }

    /// A market order is immediate-or-cancel by definition — there is no price at
    /// which it could rest.
    pub fn market(id: OrderId, user_id: UserId, side: Side, qty: i64) -> Self {
        Order {
            id,
            user_id,
            side,
            order_type: OrderType::Market,
            time_in_force: TimeInForce::Ioc,
            price: 0,
            qty,
            filled_qty: 0,
            status: OrderStatus::Open,
            reserved_remaining: 0,
        }
    }

    #[inline]
    pub fn remaining(&self) -> i64 {
        self.qty - self.filled_qty
    }

    #[inline]
    pub fn is_live(&self) -> bool {
        !self.status.is_terminal()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Level {
    /// Sum of `remaining()` over the live orders in `queue`. Maintained
    /// incrementally; never recomputed by walking the queue.
    pub total_qty: i64,
    /// Time priority, oldest first. May contain tombstones for cancelled orders.
    pub queue: VecDeque<OrderId>,
}

/// One maker-taker match. `price` is always the maker's resting price.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFill {
    pub maker_order_id: OrderId,
    pub maker_user_id: UserId,
    pub price: i64,
    pub qty: i64,
}

/// A resting order pulled from the book because it would have traded against its
/// own owner. The engine layer refunds its reservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StpCancel {
    pub order_id: OrderId,
    pub user_id: UserId,
    pub unfilled_qty: i64,
}

#[derive(Debug, Clone, Default)]
pub struct MatchOutcome {
    pub fills: Vec<RawFill>,
    pub stp_cancelled: Vec<StpCancel>,
    /// Price levels whose aggregate quantity changed, for the diff feed.
    pub touched: Vec<(Side, i64)>,
    /// True once the taker has been inserted into the book as a resting order.
    pub rested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelOutcome {
    pub order_id: OrderId,
    pub user_id: UserId,
    pub side: Side,
    pub price: i64,
    pub unfilled_qty: i64,
    /// Atoms of the locked asset to return to the owner's available balance.
    pub refund: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Simulation {
    /// How much of the requested quantity the book can actually fill.
    pub fillable_qty: i64,
    /// Quote atoms required (buy) or received before fees (sell).
    pub cost: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub symbol: String,
    bids: BTreeMap<i64, Level>,
    asks: BTreeMap<i64, Level>,
    orders: HashMap<OrderId, Order>,
    /// Monotonic per-symbol counter stamped on every depth diff, so a client that
    /// misses a message can tell instead of drifting silently.
    depth_seq: u64,
    last_price: Option<i64>,
}

impl OrderBook {
    pub fn new(symbol: impl Into<String>) -> Self {
        OrderBook {
            symbol: symbol.into(),
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            orders: HashMap::new(),
            depth_seq: 0,
            last_price: None,
        }
    }

    pub fn depth_seq(&self) -> u64 {
        self.depth_seq
    }

    pub fn last_price(&self) -> Option<i64> {
        self.last_price
    }

    pub fn order(&self, id: OrderId) -> Option<&Order> {
        self.orders.get(&id)
    }

    pub fn order_mut(&mut self, id: OrderId) -> Option<&mut Order> {
        self.orders.get_mut(&id)
    }

    pub fn orders(&self) -> impl Iterator<Item = &Order> {
        self.orders.values()
    }

    pub fn best_bid(&self) -> Option<i64> {
        self.bids.last_key_value().map(|(p, _)| *p)
    }

    pub fn best_ask(&self) -> Option<i64> {
        self.asks.first_key_value().map(|(p, _)| *p)
    }

    fn side_map(&mut self, side: Side) -> &mut BTreeMap<i64, Level> {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }

    fn side_map_ref(&self, side: Side) -> &BTreeMap<i64, Level> {
        match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        }
    }

    /// Aggregate resting quantity on one side, best price first.
    pub fn depth(&self, side: Side, limit: usize) -> Vec<[i64; 2]> {
        let map = self.side_map_ref(side);
        let iter: Box<dyn Iterator<Item = (&i64, &Level)> + '_> = match side {
            Side::Buy => Box::new(map.iter().rev()), // highest bid first
            Side::Sell => Box::new(map.iter()),      // lowest ask first
        };
        iter.filter(|(_, l)| l.total_qty > 0)
            .take(limit)
            .map(|(p, l)| [*p, l.total_qty])
            .collect()
    }

    /// Total resting quantity a taker on `side` could consume, ignoring price.
    pub fn liquidity_against(&self, taker_side: Side) -> i64 {
        self.side_map_ref(taker_side.opposite())
            .values()
            .map(|l| l.total_qty)
            .sum()
    }

    /// Walk the book without mutating it and report what filling `qty` as a taker
    /// would actually cost (buy) or yield (sell), in quote atoms.
    ///
    /// This is how a market buy gets an exact reservation instead of guessing from
    /// an average price. `skip_user` excludes the taker's own resting orders, since
    /// self-trade prevention will pull those rather than match them.
    pub fn simulate(
        &self,
        market: &Market,
        taker_side: Side,
        qty: i64,
        skip_user: Option<UserId>,
    ) -> Result<Simulation, EngineError> {
        let map = self.side_map_ref(taker_side.opposite());
        let levels: Box<dyn Iterator<Item = (&i64, &Level)> + '_> = match taker_side {
            Side::Buy => Box::new(map.iter()),        // cheapest asks first
            Side::Sell => Box::new(map.iter().rev()), // richest bids first
        };

        // Charge the buyer the remainder, credit the seller the floor. The
        // exchange is never the side left short.
        let rounding = match taker_side {
            Side::Buy => Rounding::Up,
            Side::Sell => Rounding::Down,
        };

        let mut remaining = qty;
        let mut filled = 0i64;
        let mut cost = 0i64;

        for (price, level) in levels {
            if remaining == 0 {
                break;
            }

            let mut level_qty = 0i64;
            for id in &level.queue {
                let Some(o) = self.orders.get(id) else { continue };
                if !o.is_live() || o.remaining() == 0 {
                    continue;
                }
                if skip_user == Some(o.user_id) {
                    continue;
                }
                level_qty += o.remaining();
            }

            let take = level_qty.min(remaining);
            if take == 0 {
                continue;
            }

            cost = cost
                .checked_add(market.notional(*price, take, rounding)?)
                .ok_or(MathError::Overflow)?;
            filled += take;
            remaining -= take;
        }

        Ok(Simulation {
            fillable_qty: filled,
            cost,
        })
    }

    /// Match `taker` against the book, then rest the remainder if it should rest.
    ///
    /// The taker is moved into the arena either way, so a fully filled order is
    /// still queryable afterwards.
    pub fn place(&mut self, mut taker: Order) -> MatchOutcome {
        let mut out = MatchOutcome::default();

        let last = self.match_taker(&mut taker, &mut out);
        if let Some(p) = last {
            self.last_price = Some(p);
        }

        let should_rest = taker.order_type == OrderType::Limit
            && taker.time_in_force == TimeInForce::Gtc
            && taker.remaining() > 0;

        if should_rest {
            let (id, side, price, remaining) =
                (taker.id, taker.side, taker.price, taker.remaining());

            let level = self.side_map(side).entry(price).or_default();
            level.total_qty += remaining;
            level.queue.push_back(id);

            taker.status = if taker.filled_qty > 0 {
                OrderStatus::PartiallyFilled
            } else {
                OrderStatus::Open
            };
            out.touched.push((side, price));
            out.rested = true;
        } else if taker.remaining() > 0 {
            // An IOC remainder, or a market order that ran out of book.
            taker.status = if taker.filled_qty > 0 {
                OrderStatus::PartiallyFilled
            } else {
                OrderStatus::Cancelled
            };
        } else {
            taker.status = OrderStatus::Filled;
        }

        self.orders.insert(taker.id, taker);
        self.finalise_touched(&mut out);
        out
    }

    /// The match loop. Returns the last traded price, if anything traded.
    ///
    /// Borrowing note: `book` and `orders` are taken as two `&mut` to disjoint
    /// fields up front, and `last_price` is returned rather than assigned. Reaching
    /// through `&mut self` inside the loop is what makes this impossible to write —
    /// it is the exact spot the Rust reference project stalled on.
    fn match_taker(&mut self, taker: &mut Order, out: &mut MatchOutcome) -> Option<i64> {
        let taker_side = taker.side;
        let book = match taker_side {
            Side::Buy => &mut self.asks,
            Side::Sell => &mut self.bids,
        };
        let orders = &mut self.orders;
        let mut last_price = None;

        while taker.remaining() > 0 {
            // Copy the key out so the immutable borrow of `book` ends right here.
            let best = match taker_side {
                Side::Buy => book.first_key_value().map(|(p, _)| *p),
                Side::Sell => book.last_key_value().map(|(p, _)| *p),
            };
            let Some(best) = best else { break };

            // The crossing test. A buy at P trades against asks at or below P; a
            // sell at P trades against bids at or above P. A market order takes
            // whatever is there.
            let crosses = match taker.order_type {
                OrderType::Market => true,
                OrderType::Limit => match taker_side {
                    Side::Buy => best <= taker.price,
                    Side::Sell => best >= taker.price,
                },
            };
            if !crosses {
                break;
            }

            let Some(level) = book.get_mut(&best) else { break };
            let mut level_touched = false;

            while taker.remaining() > 0 {
                let Some(&maker_id) = level.queue.front() else { break };

                let Some(maker) = orders.get_mut(&maker_id) else {
                    level.queue.pop_front();
                    continue;
                };

                // A tombstone left behind by a cancel — drop it and carry on.
                if !maker.is_live() || maker.remaining() == 0 {
                    level.queue.pop_front();
                    continue;
                }

                // Self-trade prevention: pull the resting order rather than let a
                // user trade with themselves, then keep matching behind it.
                if maker.user_id == taker.user_id {
                    let unfilled = maker.remaining();
                    maker.status = OrderStatus::Cancelled;
                    level.total_qty -= unfilled;
                    level.queue.pop_front();
                    out.stp_cancelled.push(StpCancel {
                        order_id: maker_id,
                        user_id: maker.user_id,
                        unfilled_qty: unfilled,
                    });
                    level_touched = true;
                    continue;
                }

                let qty = maker.remaining().min(taker.remaining());
                maker.filled_qty += qty;
                maker.status = if maker.remaining() == 0 {
                    OrderStatus::Filled
                } else {
                    OrderStatus::PartiallyFilled
                };

                taker.filled_qty += qty;
                level.total_qty -= qty;
                level_touched = true;
                last_price = Some(best);

                out.fills.push(RawFill {
                    maker_order_id: maker_id,
                    maker_user_id: maker.user_id,
                    // The maker's resting price — not the taker's, and not the
                    // minimum of the two. Price improvement belongs to the taker.
                    price: best,
                    qty,
                });

                if maker.remaining() == 0 {
                    level.queue.pop_front();
                }
            }

            if level_touched {
                out.touched.push((taker_side.opposite(), best));
            }

            if level.total_qty <= 0 {
                book.remove(&best);
            } else if taker.remaining() > 0 {
                // The level still holds quantity we could not take — everything
                // left is the taker's own, already handled. Stop, or we spin.
                break;
            }
        }

        last_price
    }

    /// Cancel a resting order. O(1): the queue entry is left as a tombstone for
    /// the match loop to skip.
    pub fn cancel(&mut self, order_id: OrderId) -> Result<CancelOutcome, EngineError> {
        let order = self
            .orders
            .get_mut(&order_id)
            .ok_or(EngineError::UnknownOrder(order_id))?;

        if !order.is_live() {
            return Err(EngineError::OrderClosed(order_id));
        }

        let unfilled = order.remaining();
        let refund = order.reserved_remaining;
        let side = order.side;
        let price = order.price;
        let user_id = order.user_id;

        order.status = OrderStatus::Cancelled;
        order.reserved_remaining = 0;

        let map = self.side_map(side);
        if let Some(level) = map.get_mut(&price) {
            level.total_qty -= unfilled;
            if level.total_qty <= 0 {
                map.remove(&price);
            }
        }

        self.depth_seq += 1;

        Ok(CancelOutcome {
            order_id,
            user_id,
            side,
            price,
            unfilled_qty: unfilled,
            refund,
        })
    }

    /// Collapse the touched-level list and bump the depth sequence once for the
    /// whole command. A command that changed nothing must not bump it.
    fn finalise_touched(&mut self, out: &mut MatchOutcome) {
        if out.touched.is_empty() {
            return;
        }
        out.touched
            .sort_by_key(|(s, p)| (matches!(s, Side::Sell), *p));
        out.touched.dedup();
        self.depth_seq += 1;
    }

    /// Turn the touched-level list into the deltas that go on the wire. A level
    /// that is gone reports `qty: 0` so a diff consumer knows to delete it.
    pub fn deltas_for(&self, touched: &[(Side, i64)]) -> Vec<DepthDelta> {
        touched
            .iter()
            .map(|(side, price)| DepthDelta {
                side: *side,
                price: *price,
                qty: self
                    .side_map_ref(*side)
                    .get(price)
                    .map(|l| l.total_qty)
                    .unwrap_or(0),
            })
            .collect()
    }

    /// Drop terminal orders that no level still references, so the arena does not
    /// grow without bound. Called between commands, never during a match.
    pub fn compact(&mut self) {
        let referenced: HashSet<OrderId> = self
            .bids
            .values()
            .chain(self.asks.values())
            .flat_map(|l| l.queue.iter().copied())
            .collect();
        self.orders
            .retain(|id, o| o.is_live() || referenced.contains(id));
    }
}
