//! The history tables.
//!
//! Everything here is written behind the engine and is never on a request path.
//!
//! ## Why a batch is written in one transaction
//!
//! A batch is everything one command produced. A trade and the balance changes
//! it caused are the same fact; half of it is not a smaller truth, it is a
//! false one. Writing the whole batch and recording that it was written in a
//! single transaction means a crash at any point leaves history exactly as it
//! was before, and the redelivery that follows re-does the work cleanly.
//!
//! ## Why the schema handling looks like this
//!
//! Same shape as the api crate's `UserStore`. sqlx 0.9 refuses to build a query
//! from a runtime-constructed string unless it is wrapped in [`AssertSqlSafe`],
//! which is a compile-time nudge to audit for injection. Only `CREATE SCHEMA`
//! needs it, because SQL cannot parameterise an identifier, and the name is
//! validated first. Everything else is a static string with bound parameters.

use cex_proto::{Event, EventBatch, OrderId, OrderStatus, OrderType, Seq, Side, UserId};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgRow};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Row, Transaction};
use std::str::FromStr;
use tracing::warn;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(String),
    #[error("unsafe schema name {0:?}")]
    UnsafeSchema(String),
    #[error("value out of range for the column it is written to: {0}")]
    OutOfRange(String),
}

fn db(e: sqlx::Error) -> StoreError {
    StoreError::Db(e.to_string())
}

// ───────────────────────── column encodings ─────────────────────────
//
// Written out rather than derived from the serde representation. The wire
// format and the storage format are two separate contracts: changing how an
// event is encoded on the stream must never silently rewrite what a column has
// meant for the last year of history.

fn side_str(side: Side) -> &'static str {
    match side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    }
}

fn order_type_str(t: OrderType) -> &'static str {
    match t {
        OrderType::Limit => "LIMIT",
        OrderType::Market => "MARKET",
    }
}

fn status_str(s: OrderStatus) -> &'static str {
    match s {
        OrderStatus::Open => "OPEN",
        OrderStatus::PartiallyFilled => "PARTIALLY_FILLED",
        OrderStatus::Filled => "FILLED",
        OrderStatus::Cancelled => "CANCELLED",
        OrderStatus::Rejected => "REJECTED",
    }
}

/// `Seq` and `OrderId` are `u64` on the wire; Postgres has no unsigned integer.
/// Refuse rather than wrap — a silently negative order id would be far worse
/// than a loud failure that nothing in practice can reach.
fn to_i64(v: u64, what: &str) -> Result<i64, StoreError> {
    i64::try_from(v).map_err(|_| StoreError::OutOfRange(format!("{what} {v}")))
}

/// Only ever reading back what `to_i64` wrote, so this cannot be negative.
fn from_i64(v: i64) -> u64 {
    v as u64
}

// ───────────────────────── row types ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRow {
    pub order_id: OrderId,
    pub user_id: UserId,
    pub symbol: String,
    pub side: String,
    pub order_type: String,
    pub price: Option<i64>,
    pub qty: i64,
    pub filled_qty: i64,
    pub status: String,
    pub last_seq: Seq,
}

fn row_to_order(row: PgRow) -> OrderRow {
    OrderRow {
        order_id: from_i64(row.get("order_id")),
        user_id: row.get("user_id"),
        symbol: row.get("symbol"),
        side: row.get("side"),
        order_type: row.get("order_type"),
        price: row.get("price"),
        qty: row.get("qty"),
        filled_qty: row.get("filled_qty"),
        status: row.get("status"),
        last_seq: from_i64(row.get("last_seq")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillRow {
    pub seq: Seq,
    pub idx: i32,
    pub symbol: String,
    pub price: i64,
    pub qty: i64,
    pub maker_order_id: OrderId,
    pub taker_order_id: OrderId,
    pub maker_user_id: UserId,
    pub taker_user_id: UserId,
    pub taker_side: String,
    pub notional: i64,
    pub maker_fee: i64,
    pub taker_fee: i64,
    /// When the persister wrote the row, in milliseconds since the epoch.
    ///
    /// Not when the trade matched — the engine has no clock, deliberately, so
    /// there is no exchange-authoritative timestamp to report. This is close
    /// enough to order a chart by and honest about what it is.
    pub created_at_ms: i64,
}

fn row_to_fill(row: PgRow) -> FillRow {
    FillRow {
        seq: from_i64(row.get("seq")),
        idx: row.get("idx"),
        symbol: row.get("symbol"),
        price: row.get("price"),
        qty: row.get("qty"),
        maker_order_id: from_i64(row.get("maker_order_id")),
        taker_order_id: from_i64(row.get("taker_order_id")),
        maker_user_id: row.get("maker_user_id"),
        taker_user_id: row.get("taker_user_id"),
        taker_side: row.get("taker_side"),
        notional: row.get("notional"),
        maker_fee: row.get("maker_fee"),
        taker_fee: row.get("taker_fee"),
        created_at_ms: row.get("created_at_ms"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceChangeRow {
    pub seq: Seq,
    pub idx: i32,
    pub user_id: UserId,
    pub asset: String,
    pub available: i64,
    pub locked: Option<i64>,
    pub reason: String,
    pub delta: Option<i64>,
}

fn row_to_balance_change(row: PgRow) -> BalanceChangeRow {
    BalanceChangeRow {
        seq: from_i64(row.get("seq")),
        idx: row.get("idx"),
        user_id: row.get("user_id"),
        asset: row.get("asset"),
        available: row.get("available"),
        locked: row.get("locked"),
        reason: row.get("reason"),
        delta: row.get("delta"),
    }
}

const SCHEMA_SQL: &str = include_str!("../migrations/0002_history.sql");

// ───────────────────────── the store ─────────────────────────

#[derive(Clone)]
pub struct HistoryStore {
    pool: PgPool,
}

impl HistoryStore {
    /// Connect and bring the schema up to date.
    pub async fn connect(database_url: &str) -> Result<Self, StoreError> {
        Self::connect_to_schema(database_url, "public").await
    }

    /// Connect against a named schema. Tests use a fresh schema per run so they
    /// neither collide nor need tearing down.
    pub async fn connect_to_schema(database_url: &str, schema: &str) -> Result<Self, StoreError> {
        // Audited: the identifier cannot contain a quote, a semicolon, or
        // whitespace, so it cannot terminate the statement it is spliced into.
        if schema.is_empty()
            || !schema
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(StoreError::UnsafeSchema(schema.to_string()));
        }

        // The search path is a *connection parameter*, not SQL — so it applies
        // to every pooled connection without a statement, and cannot be injected.
        let opts = PgConnectOptions::from_str(database_url)
            .map_err(db)?
            .options([("search_path", format!("{schema},public"))]);

        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .map_err(db)?;

        sqlx::raw_sql(AssertSqlSafe(format!(
            "CREATE SCHEMA IF NOT EXISTS {schema}"
        )))
        .execute(&pool)
        .await
        .map_err(db)?;

        // Static SQL, unqualified names — the search path puts them in place.
        // `IF NOT EXISTS` throughout, so every boot can safely run it.
        sqlx::raw_sql(SCHEMA_SQL).execute(&pool).await.map_err(db)?;

        Ok(HistoryStore { pool })
    }

    /// Write every batch, all in one transaction.
    ///
    /// Returns how many were new. A batch whose `seq` is already recorded is
    /// skipped: the engine republishes events after a restart, and Redis
    /// redelivers anything unacknowledged, so seeing the same batch twice is
    /// routine rather than exceptional.
    ///
    /// An error means nothing at all was written, including the batches that
    /// would have succeeded. That is deliberate — the caller must not
    /// acknowledge, and the retry then starts from a clean slate rather than
    /// from a partial one.
    pub async fn write_batches(&self, batches: &[EventBatch]) -> Result<usize, StoreError> {
        if batches.is_empty() {
            return Ok(0);
        }

        let mut tx = self.pool.begin().await.map_err(db)?;
        let mut written = 0usize;

        for batch in batches {
            let seq = to_i64(batch.seq, "seq")?;

            // The guard and the rows it guards commit together, which is the
            // whole reason redelivery is safe.
            let claimed = sqlx::query(
                "INSERT INTO event_batches (seq, request_id) VALUES ($1, $2)
                 ON CONFLICT (seq) DO NOTHING",
            )
            .bind(seq)
            .bind(batch.request_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;

            if claimed.rows_affected() == 0 {
                continue; // Already written by an earlier delivery.
            }
            written += 1;

            let mut fill_idx = 0i32;
            let mut balance_idx = 0i32;
            for event in &batch.events {
                write_event(&mut tx, seq, event, &mut fill_idx, &mut balance_idx).await?;
            }
        }

        tx.commit().await.map_err(db)?;
        Ok(written)
    }

    /// Every batch seq written so far, ascending.
    pub async fn written_seqs(&self) -> Result<Vec<Seq>, StoreError> {
        sqlx::query("SELECT seq FROM event_batches ORDER BY seq")
            .fetch_all(&self.pool)
            .await
            .map(|rows| rows.into_iter().map(|r| from_i64(r.get("seq"))).collect())
            .map_err(db)
    }

    pub async fn order(&self, order_id: OrderId) -> Result<Option<OrderRow>, StoreError> {
        sqlx::query(
            "SELECT order_id, user_id, symbol, side, order_type, price, qty,
                    filled_qty, status, last_seq
             FROM orders WHERE order_id = $1",
        )
        .bind(to_i64(order_id, "order id")?)
        .fetch_optional(&self.pool)
        .await
        .map(|opt| opt.map(row_to_order))
        .map_err(db)
    }

    /// A user's orders, newest first.
    pub async fn orders_for_user(
        &self,
        user_id: UserId,
        limit: i64,
    ) -> Result<Vec<OrderRow>, StoreError> {
        sqlx::query(
            "SELECT order_id, user_id, symbol, side, order_type, price, qty,
                    filled_qty, status, last_seq
             FROM orders WHERE user_id = $1 ORDER BY order_id DESC LIMIT $2",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map(|rows| rows.into_iter().map(row_to_order).collect())
        .map_err(db)
    }

    /// Trade history for one market, newest first. The source `GET /trades/:symbol`
    /// has been waiting for.
    pub async fn fills_for_symbol(
        &self,
        symbol: &str,
        limit: i64,
    ) -> Result<Vec<FillRow>, StoreError> {
        sqlx::query(
            "SELECT seq, idx, symbol, price, qty, maker_order_id, taker_order_id,
                    maker_user_id, taker_user_id, taker_side, notional,
                    maker_fee, taker_fee,
                    (EXTRACT(EPOCH FROM created_at) * 1000)::BIGINT AS created_at_ms
             FROM fills WHERE symbol = $1 ORDER BY seq DESC, idx DESC LIMIT $2",
        )
        .bind(symbol)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map(|rows| rows.into_iter().map(row_to_fill).collect())
        .map_err(db)
    }

    pub async fn balance_changes_for(
        &self,
        user_id: UserId,
    ) -> Result<Vec<BalanceChangeRow>, StoreError> {
        sqlx::query(
            "SELECT seq, idx, user_id, asset, available, locked, reason, delta
             FROM balance_changes WHERE user_id = $1 ORDER BY seq, idx",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map(|rows| rows.into_iter().map(row_to_balance_change).collect())
        .map_err(db)
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

// ───────────────────────── one event ─────────────────────────

/// Project one event onto the history tables.
///
/// Events with no history to write — depth updates are live market data, and a
/// rejection carries no order id to key a row on — write nothing and say so,
/// rather than being silently swallowed by a catch-all arm.
async fn write_event(
    tx: &mut Transaction<'_, Postgres>,
    seq: i64,
    event: &Event,
    fill_idx: &mut i32,
    balance_idx: &mut i32,
) -> Result<(), StoreError> {
    match event {
        Event::Deposited {
            user_id,
            asset,
            amount,
            available,
        } => {
            insert_balance_change(
                tx, seq, balance_idx, *user_id, asset, *available, None, "deposit",
                Some(*amount),
            )
            .await?;
        }

        Event::Withdrawn {
            user_id,
            asset,
            amount,
            available,
        } => {
            // A withdrawal leaves the account, so its delta is negative.
            insert_balance_change(
                tx,
                seq,
                balance_idx,
                *user_id,
                asset,
                *available,
                None,
                "withdrawal",
                Some(-*amount),
            )
            .await?;
        }

        Event::BalanceUpdated {
            user_id,
            asset,
            available,
            locked,
        } => {
            insert_balance_change(
                tx,
                seq,
                balance_idx,
                *user_id,
                asset,
                *available,
                Some(*locked),
                "update",
                None,
            )
            .await?;
        }

        Event::OrderAccepted {
            order_id,
            user_id,
            symbol,
            side,
            order_type,
            price,
            qty,
        } => {
            // `ON CONFLICT` cannot fire in practice — order ids are unique and
            // a repeated batch is skipped before we get here — but if it ever
            // does, the identity of the order is refreshed while its progress
            // is left alone. Re-accepting must never un-fill an order.
            sqlx::query(
                "INSERT INTO orders
                     (order_id, user_id, symbol, side, order_type, price, qty,
                      filled_qty, status, last_seq)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, 0, $8, $9)
                 ON CONFLICT (order_id) DO UPDATE SET
                     user_id    = EXCLUDED.user_id,
                     symbol     = EXCLUDED.symbol,
                     side       = EXCLUDED.side,
                     order_type = EXCLUDED.order_type,
                     price      = EXCLUDED.price,
                     qty        = EXCLUDED.qty,
                     last_seq   = EXCLUDED.last_seq,
                     updated_at = now()
                 WHERE orders.last_seq <= EXCLUDED.last_seq",
            )
            .bind(to_i64(*order_id, "order id")?)
            .bind(user_id)
            .bind(symbol)
            .bind(side_str(*side))
            .bind(order_type_str(*order_type))
            .bind(price)
            .bind(qty)
            .bind(status_str(OrderStatus::Open))
            .bind(seq)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
        }

        Event::OrderUpdated {
            order_id,
            filled_qty,
            qty,
            status,
            ..
        } => {
            let touched = sqlx::query(
                "UPDATE orders
                 SET filled_qty = $2, qty = $3, status = $4, last_seq = $5,
                     updated_at = now()
                 WHERE order_id = $1 AND last_seq <= $5",
            )
            .bind(to_i64(*order_id, "order id")?)
            .bind(filled_qty)
            .bind(qty)
            .bind(status_str(*status))
            .bind(seq)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
            warn_if_untouched(touched.rows_affected(), *order_id, seq, "update");
        }

        Event::OrderCancelled { order_id, .. } => {
            let touched = sqlx::query(
                "UPDATE orders
                 SET status = $2, last_seq = $3, updated_at = now()
                 WHERE order_id = $1 AND last_seq <= $3",
            )
            .bind(to_i64(*order_id, "order id")?)
            .bind(status_str(OrderStatus::Cancelled))
            .bind(seq)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
            warn_if_untouched(touched.rows_affected(), *order_id, seq, "cancel");
        }

        Event::Trades { fills, .. } => {
            for fill in fills {
                sqlx::query(
                    "INSERT INTO fills
                         (seq, idx, symbol, price, qty, maker_order_id, taker_order_id,
                          maker_user_id, taker_user_id, taker_side, notional,
                          maker_fee, taker_fee)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
                )
                .bind(seq)
                .bind(*fill_idx)
                .bind(&fill.symbol)
                .bind(fill.price)
                .bind(fill.qty)
                .bind(to_i64(fill.maker_order_id, "maker order id")?)
                .bind(to_i64(fill.taker_order_id, "taker order id")?)
                .bind(fill.maker_user_id)
                .bind(fill.taker_user_id)
                .bind(side_str(fill.taker_side))
                .bind(fill.notional)
                .bind(fill.maker_fee)
                .bind(fill.taker_fee)
                .execute(&mut **tx)
                .await
                .map_err(db)?;
                *fill_idx += 1;
            }
        }

        // Live market data. `ws` fans it out; it is not history.
        Event::DepthUpdated { .. } => {}

        // A rejection never became an order, so there is no row to key on it.
        // The batch is still recorded, so it is never reprocessed.
        Event::OrderRejected { .. } => {}
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_balance_change(
    tx: &mut Transaction<'_, Postgres>,
    seq: i64,
    idx: &mut i32,
    user_id: UserId,
    asset: &str,
    available: i64,
    locked: Option<i64>,
    reason: &str,
    delta: Option<i64>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO balance_changes
             (seq, idx, user_id, asset, available, locked, reason, delta)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(seq)
    .bind(*idx)
    .bind(user_id)
    .bind(asset)
    .bind(available)
    .bind(locked)
    .bind(reason)
    .bind(delta)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    *idx += 1;
    Ok(())
}

/// An update that matched nothing is not an error, but it is worth saying out
/// loud: either the event carried an older seq than the row already holds, or
/// the order was accepted before this persister had a stream to read.
fn warn_if_untouched(rows: u64, order_id: OrderId, seq: i64, what: &str) {
    if rows == 0 {
        warn!(
            order_id,
            seq, kind = what, "order {what} matched no row; older seq, or the order predates this persister"
        );
    }
}
