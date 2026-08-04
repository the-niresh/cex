//! Engine state and the `apply` function.
//!
//! `apply` is the entire mutating surface of the engine: a pure function of
//! `(state, command)` returning the events that command produced. No clock, no
//! I/O, no randomness — which is what makes snapshot-and-replay recovery exact.
//!
//! ## Settlement
//!
//! Fees are charged in the asset each side *receives*. That is what lets a buyer
//! reserve only the quote notional, with no extra headroom for a fee whose rate
//! is not known until the order either rests or takes.
//!
//! Every credit is derived from the matching debit rather than recomputed, so
//! sub-atom rounding can never create or destroy value.
//!
//! ## Reservations
//!
//! A maker is touched at most once per incoming command, so its reservation can
//! be drawn down per fill. A *taker* can appear in many fills of one command, so
//! its reservation is settled once at the end against the total — drawing it down
//! per fill would let per-fill rounding accumulate past what was locked.

use std::collections::{BTreeMap, BTreeSet};

use cex_proto::{
    Command, DepthSnapshot, Event, Fill, MarketView, OrderId, OrderType, OrderView, Query,
    ResponseBody, Seq, Side, TimeInForce, UserId,
};
use serde::{Deserialize, Serialize};

use crate::balances::Balances;
use crate::book::{Order, OrderBook};
use crate::error::EngineError;
use crate::market::{Market, MarketRegistry};
use crate::math::{checked_sub, Rounding};

/// Fees accrue to the nil UUID. Keeping them inside the same ledger is what makes
/// the conservation check meaningful: fees are moved, never destroyed.
pub const FEE_ACCOUNT: UserId = UserId::nil();

const DEFAULT_DEPTH_LIMIT: usize = 50;

/// The outcome of one accepted command.
#[derive(Debug, Clone)]
pub struct Applied {
    pub seq: Seq,
    pub response: ResponseBody,
    pub events: Vec<Event>,
}

/// Bumped whenever the shape of [`State`] changes. A snapshot written by a newer
/// build is refused rather than misread — replaying the log is always available
/// as a fallback, and a silently misinterpreted field is not.
pub const SNAPSHOT_VERSION: u32 = 1;

/// Engine state plus the log position it was taken at.
///
/// The position is the whole point: without it there is no way to know which
/// commands still need replaying, and the snapshot is worthless.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    /// Id of the last command applied before this snapshot was taken.
    pub last_stream_id: String,
    pub state: State,
}

impl Snapshot {
    pub fn of(state: &State, last_stream_id: impl Into<String>) -> Self {
        Snapshot {
            version: SNAPSHOT_VERSION,
            last_stream_id: last_stream_id.into(),
            state: state.clone(),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, EngineError> {
        serde_json::to_vec(self).map_err(|e| EngineError::Snapshot(e.to_string()))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, EngineError> {
        let snap: Snapshot =
            serde_json::from_slice(bytes).map_err(|e| EngineError::Snapshot(e.to_string()))?;
        if snap.version != SNAPSHOT_VERSION {
            return Err(EngineError::Snapshot(format!(
                "snapshot version {}, this build understands {}",
                snap.version, SNAPSHOT_VERSION
            )));
        }
        Ok(snap)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    markets: MarketRegistry,
    books: BTreeMap<String, OrderBook>,
    balances: Balances,
    seq: Seq,
    next_order_id: OrderId,
    /// Net deposited per asset. The conservation check compares the ledger's total
    /// supply against this; any divergence is money created or lost.
    minted: BTreeMap<String, i64>,
}

impl State {
    pub fn new(markets: MarketRegistry) -> Self {
        let books = markets
            .symbols()
            .map(|s| (s.clone(), OrderBook::new(s.clone())))
            .collect();
        State {
            markets,
            books,
            balances: Balances::new(),
            seq: 0,
            next_order_id: 1,
            minted: BTreeMap::new(),
        }
    }

    pub fn seq(&self) -> Seq {
        self.seq
    }

    pub fn balances(&self) -> &Balances {
        &self.balances
    }

    pub fn markets(&self) -> &MarketRegistry {
        &self.markets
    }

    pub fn book(&self, symbol: &str) -> Result<&OrderBook, EngineError> {
        self.books
            .get(symbol)
            .ok_or_else(|| EngineError::UnknownMarket(symbol.to_string()))
    }

    /// Every live order across every market.
    pub fn open_order_ids(&self) -> Vec<OrderId> {
        let mut ids: Vec<OrderId> = self
            .books
            .values()
            .flat_map(|b| b.orders().filter(|o| o.is_live()).map(|o| o.id))
            .collect();
        ids.sort_unstable();
        ids
    }

    pub fn order_owner(&self, id: OrderId) -> Option<UserId> {
        self.books
            .values()
            .find_map(|b| b.order(id).map(|o| o.user_id))
    }

    fn symbol_of_order(&self, id: OrderId) -> Option<String> {
        self.books
            .iter()
            .find(|(_, b)| b.order(id).is_some())
            .map(|(s, _)| s.clone())
    }

    fn user_of_order(&self, symbol: &str, id: OrderId) -> Result<UserId, EngineError> {
        self.books
            .get(symbol)
            .and_then(|b| b.order(id))
            .map(|o| o.user_id)
            .ok_or(EngineError::UnknownOrder(id))
    }

    // ───────────────────────── apply ─────────────────────────

    /// Apply one command. On `Err` the state is unchanged — every check that can
    /// fail runs before the first mutation.
    pub fn apply(&mut self, cmd: Command) -> Result<Applied, EngineError> {
        let (response, events) = match cmd {
            Command::Deposit {
                user_id,
                asset,
                amount,
                ..
            } => self.deposit(user_id, &asset, amount)?,
            Command::Withdraw {
                user_id,
                asset,
                amount,
                ..
            } => self.withdraw(user_id, &asset, amount)?,
            Command::PlaceOrder {
                user_id,
                symbol,
                side,
                order_type,
                time_in_force,
                price,
                qty,
                ..
            } => self.place_order(
                user_id,
                &symbol,
                side,
                order_type,
                time_in_force.unwrap_or(TimeInForce::Gtc),
                price,
                qty,
            )?,
            Command::CancelOrder {
                user_id, order_id, ..
            } => self.cancel_order(user_id, order_id)?,
        };

        self.seq += 1;
        Ok(Applied {
            seq: self.seq,
            response,
            events,
        })
    }

    fn deposit(
        &mut self,
        user: UserId,
        asset: &str,
        amount: i64,
    ) -> Result<(ResponseBody, Vec<Event>), EngineError> {
        if amount <= 0 {
            return Err(EngineError::NonPositiveAmount);
        }
        if !self.markets.has_asset(asset) {
            return Err(EngineError::UnknownAsset(asset.to_string()));
        }

        self.balances.credit(user, asset, amount)?;
        *self.minted.entry(asset.to_string()).or_insert(0) += amount;

        let bal = self.balances.get(user, asset);
        Ok((
            ResponseBody::Ack,
            vec![
                Event::Deposited {
                    user_id: user,
                    asset: asset.to_string(),
                    amount,
                    available: bal.available,
                },
                Event::BalanceUpdated {
                    user_id: user,
                    asset: asset.to_string(),
                    available: bal.available,
                    locked: bal.locked,
                },
            ],
        ))
    }

    fn withdraw(
        &mut self,
        user: UserId,
        asset: &str,
        amount: i64,
    ) -> Result<(ResponseBody, Vec<Event>), EngineError> {
        if amount <= 0 {
            return Err(EngineError::NonPositiveAmount);
        }
        if !self.markets.has_asset(asset) {
            return Err(EngineError::UnknownAsset(asset.to_string()));
        }

        // Fails without mutating if `available` is short. Locked funds are not
        // reachable, which is the whole point of keeping the two halves apart.
        self.balances.debit(user, asset, amount)?;
        *self.minted.entry(asset.to_string()).or_insert(0) -= amount;

        let bal = self.balances.get(user, asset);
        Ok((
            ResponseBody::Ack,
            vec![
                Event::Withdrawn {
                    user_id: user,
                    asset: asset.to_string(),
                    amount,
                    available: bal.available,
                },
                Event::BalanceUpdated {
                    user_id: user,
                    asset: asset.to_string(),
                    available: bal.available,
                    locked: bal.locked,
                },
            ],
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn place_order(
        &mut self,
        user: UserId,
        symbol: &str,
        side: Side,
        order_type: OrderType,
        tif: TimeInForce,
        price: Option<i64>,
        qty: i64,
    ) -> Result<(ResponseBody, Vec<Event>), EngineError> {
        let market = self.markets.get(symbol)?.clone();
        market.validate_qty(qty)?;

        let limit_price = match order_type {
            OrderType::Limit => {
                let p = price.ok_or(EngineError::MissingPrice)?;
                market.validate_price(p)?;
                if market.notional(p, qty, Rounding::Down)? < market.min_notional {
                    return Err(EngineError::BelowMinNotional(market.min_notional));
                }
                p
            }
            OrderType::Market => 0,
        };

        // Size the reservation before touching anything. A market order needs the
        // book walked first, because there is no limit price to multiply by.
        let book = self.book(symbol)?;
        let (lock_asset, lock_amount) = match (side, order_type) {
            (Side::Buy, OrderType::Limit) => (
                market.quote.clone(),
                market.notional(limit_price, qty, Rounding::Up)?,
            ),
            (Side::Buy, OrderType::Market) => {
                let sim = book.simulate(&market, Side::Buy, qty, Some(user))?;
                if sim.fillable_qty == 0 {
                    return Err(EngineError::InsufficientLiquidity {
                        requested: qty,
                        available: 0,
                    });
                }
                (market.quote.clone(), sim.cost)
            }
            (Side::Sell, OrderType::Market) => {
                let sim = book.simulate(&market, Side::Sell, qty, Some(user))?;
                if sim.fillable_qty == 0 {
                    return Err(EngineError::InsufficientLiquidity {
                        requested: qty,
                        available: 0,
                    });
                }
                (market.base.clone(), qty)
            }
            (Side::Sell, OrderType::Limit) => (market.base.clone(), qty),
        };

        // First mutation. Nothing above this point changed any state.
        self.balances.lock(user, &lock_asset, lock_amount)?;

        let order_id = self.next_order_id;
        self.next_order_id += 1;

        let mut order = match order_type {
            OrderType::Limit => Order::limit(order_id, user, side, limit_price, qty, tif),
            OrderType::Market => Order::market(order_id, user, side, qty),
        };
        order.reserved_remaining = lock_amount;

        let outcome = self
            .books
            .get_mut(symbol)
            .expect("book checked above")
            .place(order);

        let mut events = vec![Event::OrderAccepted {
            order_id,
            user_id: user,
            symbol: symbol.to_string(),
            side,
            order_type,
            price: (order_type == OrderType::Limit).then_some(limit_price),
            qty,
        }];

        // A resting order pulled by self-trade prevention gets its reservation back.
        for stp in &outcome.stp_cancelled {
            self.release_reservation(symbol, stp.order_id)?;
            events.push(Event::OrderCancelled {
                order_id: stp.order_id,
                user_id: stp.user_id,
                symbol: symbol.to_string(),
                unfilled_qty: stp.unfilled_qty,
            });
        }

        let taker_is_buyer = side == Side::Buy;
        let mut wire_fills = Vec::with_capacity(outcome.fills.len());
        let mut filled_qty = 0i64;
        let mut notional_sum: i128 = 0;
        let mut taker_buy_cost = 0i64;
        let mut counterparties: BTreeSet<UserId> = BTreeSet::new();

        for raw in &outcome.fills {
            let cost = market.notional(raw.price, raw.qty, Rounding::Up)?;
            let maker_user = raw.maker_user_id;
            counterparties.insert(maker_user);

            let (buyer, seller) = if taker_is_buyer {
                (user, maker_user)
            } else {
                (maker_user, user)
            };

            // Draw down the sell side's base reservation. Correct per fill for
            // either role: a seller always gives up exactly `qty`.
            let sell_order_id = if taker_is_buyer {
                raw.maker_order_id
            } else {
                order_id
            };
            self.balances.settle_locked(seller, &market.base, raw.qty)?;
            self.reduce_reservation(symbol, sell_order_id, raw.qty);

            // Draw down the buy side's quote reservation. A maker is touched once
            // per command so it can settle here; the taker is deferred to after
            // the loop where the total is known.
            if taker_is_buyer {
                taker_buy_cost = taker_buy_cost
                    .checked_add(cost)
                    .ok_or(crate::math::MathError::Overflow)?;
            } else {
                self.settle_maker_buy(&market, symbol, raw.maker_order_id, cost)?;
            }

            // Fees come out of what each side receives.
            let buyer_bps = if taker_is_buyer {
                market.taker_fee_bps
            } else {
                market.maker_fee_bps
            };
            let seller_bps = if taker_is_buyer {
                market.maker_fee_bps
            } else {
                market.taker_fee_bps
            };
            let buyer_fee = market.fee(raw.qty, buyer_bps)?;
            let seller_fee = market.fee(cost, seller_bps)?;

            self.balances
                .credit(buyer, &market.base, checked_sub(raw.qty, buyer_fee)?)?;
            self.balances
                .credit(seller, &market.quote, checked_sub(cost, seller_fee)?)?;
            self.balances.credit(FEE_ACCOUNT, &market.base, buyer_fee)?;
            self.balances
                .credit(FEE_ACCOUNT, &market.quote, seller_fee)?;

            filled_qty += raw.qty;
            notional_sum += (raw.price as i128) * (raw.qty as i128);

            wire_fills.push(Fill {
                symbol: symbol.to_string(),
                price: raw.price,
                qty: raw.qty,
                maker_order_id: raw.maker_order_id,
                taker_order_id: order_id,
                maker_user_id: maker_user,
                taker_user_id: user,
                taker_side: side,
                notional: cost,
                maker_fee: if taker_is_buyer { seller_fee } else { buyer_fee },
                taker_fee: if taker_is_buyer { buyer_fee } else { seller_fee },
            });

            if let Some(m) = self.books.get(symbol).and_then(|b| b.order(raw.maker_order_id)) {
                events.push(Event::OrderUpdated {
                    order_id: m.id,
                    user_id: m.user_id,
                    filled_qty: m.filled_qty,
                    qty: m.qty,
                    status: m.status,
                });
            }
        }

        // Settle the taker's quote reservation once, against the total.
        if taker_is_buyer && taker_buy_cost > 0 {
            self.settle_taker_buy(&market, symbol, order_id, taker_buy_cost, limit_price)?;
        }

        let taker_status = self
            .book(symbol)?
            .order(order_id)
            .expect("taker is in the arena")
            .status;

        // Whatever the taker did not fill and will not rest on comes back.
        if !outcome.rested {
            self.release_reservation(symbol, order_id)?;
        }

        if !wire_fills.is_empty() {
            events.push(Event::Trades {
                symbol: symbol.to_string(),
                fills: wire_fills,
            });
        }

        events.push(Event::OrderUpdated {
            order_id,
            user_id: user,
            filled_qty,
            qty,
            status: taker_status,
        });

        if !outcome.touched.is_empty() {
            let book = self.book(symbol)?;
            events.push(Event::DepthUpdated {
                symbol: symbol.to_string(),
                depth_seq: book.depth_seq(),
                deltas: book.deltas_for(&outcome.touched),
            });
        }

        events.extend(self.balance_events(user, &market));
        for other in counterparties {
            events.extend(self.balance_events(other, &market));
        }

        let avg_price = if filled_qty > 0 {
            Some((notional_sum / filled_qty as i128) as i64)
        } else {
            None
        };

        Ok((
            ResponseBody::OrderPlaced {
                order_id,
                status: taker_status,
                filled_qty,
                qty,
                avg_price,
            },
            events,
        ))
    }

    fn cancel_order(
        &mut self,
        user: UserId,
        order_id: OrderId,
    ) -> Result<(ResponseBody, Vec<Event>), EngineError> {
        let symbol = self
            .symbol_of_order(order_id)
            .ok_or(EngineError::UnknownOrder(order_id))?;

        // Ownership and liveness are checked before anything mutates.
        {
            let order = self
                .book(&symbol)?
                .order(order_id)
                .ok_or(EngineError::UnknownOrder(order_id))?;
            if order.user_id != user {
                return Err(EngineError::NotOrderOwner(order_id));
            }
            if !order.is_live() {
                return Err(EngineError::OrderClosed(order_id));
            }
        }

        let market = self.markets.get(&symbol)?.clone();
        let side = self
            .book(&symbol)?
            .order(order_id)
            .expect("checked above")
            .side;

        let outcome = self
            .books
            .get_mut(&symbol)
            .expect("checked above")
            .cancel(order_id)?;

        let asset = match side {
            Side::Buy => market.quote.clone(),
            Side::Sell => market.base.clone(),
        };
        if outcome.refund > 0 {
            self.balances.unlock(user, &asset, outcome.refund)?;
        }

        let mut events = vec![Event::OrderCancelled {
            order_id,
            user_id: user,
            symbol: symbol.clone(),
            unfilled_qty: outcome.unfilled_qty,
        }];

        let book = self.book(&symbol)?;
        events.push(Event::DepthUpdated {
            symbol: symbol.clone(),
            depth_seq: book.depth_seq(),
            deltas: book.deltas_for(&[(outcome.side, outcome.price)]),
        });
        events.extend(self.balance_events(user, &market));

        Ok((ResponseBody::Ack, events))
    }

    // ───────────────────────── reservation handling ─────────────────────────

    fn reduce_reservation(&mut self, symbol: &str, order_id: OrderId, amount: i64) {
        if let Some(o) = self.books.get_mut(symbol).and_then(|b| b.order_mut(order_id)) {
            o.reserved_remaining = (o.reserved_remaining - amount).max(0);
        }
    }

    /// A resting buy order fills at its own price, so there is no improvement to
    /// refund and the draw-down is exactly the cost.
    fn settle_maker_buy(
        &mut self,
        market: &Market,
        symbol: &str,
        order_id: OrderId,
        cost: i64,
    ) -> Result<(), EngineError> {
        let user = self.user_of_order(symbol, order_id)?;
        self.balances.settle_locked(user, &market.quote, cost)?;
        self.reduce_reservation(symbol, order_id, cost);
        Ok(())
    }

    /// Settle an aggressing buy against the total of all its fills.
    ///
    /// The reservation was sized at the limit price; the fills happened at or
    /// below it. The difference is price improvement and goes straight back to
    /// available — this is the case `cex` dropped on the floor.
    fn settle_taker_buy(
        &mut self,
        market: &Market,
        symbol: &str,
        order_id: OrderId,
        total_cost: i64,
        limit_price: i64,
    ) -> Result<(), EngineError> {
        let (user, reserved, remaining, order_type) = {
            let o = self
                .book(symbol)?
                .order(order_id)
                .ok_or(EngineError::UnknownOrder(order_id))?;
            (o.user_id, o.reserved_remaining, o.remaining(), o.order_type)
        };

        // How much of the reservation this command consumed. For a limit order
        // that is whatever is no longer needed to back the unfilled remainder.
        let consumed = match order_type {
            OrderType::Limit => {
                let still_needed = market.notional(limit_price, remaining, Rounding::Up)?;
                reserved.saturating_sub(still_needed)
            }
            OrderType::Market => reserved,
        };
        // Never settle less than was actually spent, never more than was locked.
        let consumed = consumed.max(total_cost).min(reserved);

        self.balances.settle_locked(user, &market.quote, total_cost)?;
        let improvement = consumed - total_cost;
        if improvement > 0 {
            self.balances.unlock(user, &market.quote, improvement)?;
        }
        self.reduce_reservation(symbol, order_id, consumed);
        Ok(())
    }

    /// Return an order's outstanding reservation to its owner's available balance.
    fn release_reservation(&mut self, symbol: &str, order_id: OrderId) -> Result<(), EngineError> {
        let market = self.markets.get(symbol)?.clone();
        let Some(order) = self.books.get_mut(symbol).and_then(|b| b.order_mut(order_id)) else {
            return Ok(());
        };

        let amount = order.reserved_remaining;
        if amount == 0 {
            return Ok(());
        }
        let (user, side) = (order.user_id, order.side);
        order.reserved_remaining = 0;

        let asset = match side {
            Side::Buy => market.quote,
            Side::Sell => market.base,
        };
        self.balances.unlock(user, &asset, amount)?;
        Ok(())
    }

    fn balance_events(&self, user: UserId, market: &Market) -> Vec<Event> {
        [&market.base, &market.quote]
            .iter()
            .map(|asset| {
                let b = self.balances.get(user, asset);
                Event::BalanceUpdated {
                    user_id: user,
                    asset: (*asset).clone(),
                    available: b.available,
                    locked: b.locked,
                }
            })
            .collect()
    }

    // ───────────────────────── queries ─────────────────────────

    /// Answer a read-only request. Never mutates, never logged.
    pub fn query(&self, q: &Query) -> Result<ResponseBody, EngineError> {
        match q {
            Query::Depth { symbol, limit, .. } => {
                let book = self.book(symbol)?;
                let n = limit.unwrap_or(DEFAULT_DEPTH_LIMIT);
                Ok(ResponseBody::Depth(DepthSnapshot {
                    symbol: symbol.clone(),
                    depth_seq: book.depth_seq(),
                    bids: book.depth(Side::Buy, n),
                    asks: book.depth(Side::Sell, n),
                }))
            }
            Query::Balances { user_id, .. } => {
                Ok(ResponseBody::Balances(self.balances.for_user(*user_id)))
            }
            Query::Order {
                user_id, order_id, ..
            } => {
                let view = self
                    .books
                    .iter()
                    .find_map(|(sym, b)| b.order(*order_id).map(|o| (sym, o)))
                    .filter(|(_, o)| o.user_id == *user_id)
                    .map(|(sym, o)| order_view(sym, o))
                    .ok_or(EngineError::UnknownOrder(*order_id))?;
                Ok(ResponseBody::Order(view))
            }
            Query::OpenOrders {
                user_id, symbol, ..
            } => {
                let mut out: Vec<OrderView> = Vec::new();
                for (sym, book) in &self.books {
                    if symbol.as_ref().is_some_and(|want| want != sym) {
                        continue;
                    }
                    out.extend(
                        book.orders()
                            .filter(|o| o.user_id == *user_id && o.is_live())
                            .map(|o| order_view(sym, o)),
                    );
                }
                out.sort_by_key(|o| o.order_id);
                Ok(ResponseBody::Orders(out))
            }
            Query::Markets { .. } => Ok(ResponseBody::Markets(
                self.markets.iter().map(Market::view).collect::<Vec<MarketView>>(),
            )),
        }
    }

    // ───────────────────────── invariants ─────────────────────────

    /// Assert the two properties that make the ledger trustworthy:
    ///
    /// 1. Every asset's total supply equals deposits net of withdrawals.
    /// 2. Every locked atom is backed by a live order that can release it.
    ///
    /// The second matters as much as the first: locked funds with no order behind
    /// them are money the user can never get back, and the totals still balance.
    pub fn check_invariants(&self) -> Result<(), String> {
        for asset in self.markets.assets() {
            let expected = self.minted.get(&asset).copied().unwrap_or(0);
            let actual = self.balances.total_supply(&asset);
            if expected != actual {
                return Err(format!(
                    "supply of {asset}: ledger holds {actual}, deposits net to {expected}"
                ));
            }
        }

        for (user, asset, bal) in self.balances.accounts() {
            if bal.available < 0 || bal.locked < 0 {
                return Err(format!("negative balance for {user} {asset}: {bal:?}"));
            }
        }

        let mut reserved: BTreeMap<(UserId, String), i64> = BTreeMap::new();
        for (symbol, book) in &self.books {
            let market = self
                .markets
                .get(symbol)
                .map_err(|e| format!("market {symbol}: {e}"))?;
            for o in book.orders().filter(|o| o.is_live()) {
                let asset = match o.side {
                    Side::Buy => market.quote.clone(),
                    Side::Sell => market.base.clone(),
                };
                *reserved.entry((o.user_id, asset)).or_insert(0) += o.reserved_remaining;
            }
        }

        let keys: BTreeSet<(UserId, String)> = reserved
            .keys()
            .cloned()
            .chain(
                self.balances
                    .accounts()
                    .filter(|(_, _, b)| b.locked != 0)
                    .map(|(user, asset, _)| (user, asset.to_string())),
            )
            .collect();

        for key in keys {
            let want = reserved.get(&key).copied().unwrap_or(0);
            let have = self.balances.get(key.0, &key.1).locked;
            if want != have {
                return Err(format!(
                    "locked {} for {}: ledger says {have}, open orders reserve {want}",
                    key.1, key.0
                ));
            }
        }

        Ok(())
    }
}

fn order_view(symbol: &str, o: &Order) -> OrderView {
    OrderView {
        order_id: o.id,
        user_id: o.user_id,
        symbol: symbol.to_string(),
        side: o.side,
        order_type: o.order_type,
        price: (o.order_type == OrderType::Limit).then_some(o.price),
        qty: o.qty,
        filled_qty: o.filled_qty,
        status: o.status,
    }
}
