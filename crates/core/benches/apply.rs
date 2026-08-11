//! What matching actually costs, with no I/O anywhere near it.
//!
//! Read `tests/bench_guards.rs` before changing this file. Two things here look
//! like ceremony and are not: every iteration gets a fresh `request_id`, because
//! a repeat short-circuits on the idempotency log, and every iteration gets a
//! cloned `State`, because `apply` takes `&mut self` and a shared one would
//! drain or grow as the benchmark ran.
//!
//! Re-derive with: cargo bench -p cex-core

use std::hint::black_box;

use cex_core::{MarketRegistry, State};
use cex_proto::{Command, OrderType, Side, TimeInForce};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use uuid::Uuid;

const SYMBOL: &str = "BTC_USDT";
/// 50,000 USDT per BTC, in quote atoms. A multiple of the 10,000 tick.
const MID: i64 = 50_000_000_000;
const TICK: i64 = 10_000;
/// 0.001 BTC in base atoms. A multiple of the 1,000 lot.
const QTY: i64 = 100_000;

struct Fixture {
    state: State,
    taker: Uuid,
    maker: Uuid,
}

/// A book carrying `depth` resting price levels on each side, both users funded
/// far beyond anything the measured command can spend.
fn fixture(depth: i64) -> Fixture {
    let mut state = State::new(MarketRegistry::with_defaults());
    let maker = Uuid::new_v4();
    let taker = Uuid::new_v4();

    for user in [maker, taker] {
        for asset in ["USDT", "BTC"] {
            state
                .apply(Command::Deposit {
                    request_id: Uuid::new_v4(),
                    user_id: user,
                    asset: asset.to_string(),
                    amount: 1_000_000_000_000_000,
                })
                .expect("deposit");
        }
    }

    for i in 0..depth {
        state
            .apply(limit(maker, Side::Buy, MID - (i + 1) * TICK, QTY))
            .expect("seed bid");
        state
            .apply(limit(maker, Side::Sell, MID + (i + 1) * TICK, QTY))
            .expect("seed ask");
    }

    Fixture {
        state,
        taker,
        maker,
    }
}

fn limit(user: Uuid, side: Side, price: i64, qty: i64) -> Command {
    Command::PlaceOrder {
        request_id: Uuid::new_v4(),
        user_id: user,
        symbol: SYMBOL.to_string(),
        side,
        order_type: OrderType::Limit,
        time_in_force: Some(TimeInForce::Gtc),
        price: Some(price),
        qty,
    }
}

fn market(user: Uuid, side: Side, qty: i64) -> Command {
    Command::PlaceOrder {
        request_id: Uuid::new_v4(),
        user_id: user,
        symbol: SYMBOL.to_string(),
        side,
        order_type: OrderType::Market,
        time_in_force: None,
        price: None,
        qty,
    }
}

fn bench_apply(c: &mut Criterion) {
    let mut group = c.benchmark_group("apply");

    for depth in [10i64, 100, 1_000] {
        let f = fixture(depth);

        // The cheap path and the common one: a buy far below the best bid,
        // which rests without touching anything.
        group.bench_with_input(BenchmarkId::new("limit_rest", depth), &f, |b, f| {
            b.iter_batched(
                || {
                    (
                        f.state.clone(),
                        limit(f.taker, Side::Buy, MID - (depth + 10) * TICK, QTY),
                    )
                },
                |(mut state, cmd)| black_box(state.apply(cmd)),
                BatchSize::SmallInput,
            )
        });

        // One match against the best ask.
        group.bench_with_input(BenchmarkId::new("limit_cross_one", depth), &f, |b, f| {
            b.iter_batched(
                || (f.state.clone(), limit(f.taker, Side::Buy, MID + TICK, QTY)),
                |(mut state, cmd)| black_box(state.apply(cmd)),
                BatchSize::SmallInput,
            )
        });

        // Where the cost actually scales: a market buy sized to eat half the
        // resting asks.
        let sweep_qty = QTY * (depth / 2).max(1);
        group.bench_with_input(BenchmarkId::new("market_sweep_half", depth), &f, |b, f| {
            b.iter_batched(
                || (f.state.clone(), market(f.taker, Side::Buy, sweep_qty)),
                |(mut state, cmd)| black_box(state.apply(cmd)),
                BatchSize::SmallInput,
            )
        });

        // Lookup and removal, no matching. Order ids start at 1 and the fixture
        // seeds them in order, so id 1 is the maker's first bid.
        group.bench_with_input(BenchmarkId::new("cancel", depth), &f, |b, f| {
            b.iter_batched(
                || {
                    (
                        f.state.clone(),
                        Command::CancelOrder {
                            request_id: Uuid::new_v4(),
                            user_id: f.maker,
                            order_id: 1,
                        },
                    )
                },
                |(mut state, cmd)| black_box(state.apply(cmd)),
                BatchSize::SmallInput,
            )
        });
    }

    group.finish();
}

criterion_group!(benches, bench_apply);
criterion_main!(benches);
