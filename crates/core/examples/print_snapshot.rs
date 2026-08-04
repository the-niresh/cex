//! Print a snapshot so you can look at one.
//!
//!     cargo run -p cex-core --example print_snapshot
//!
//! Useful when debugging recovery: a snapshot is just bytes, and being able to
//! read them is the difference between "recovery is broken" and "the locked
//! balance on line 40 is wrong".

use cex_core::state::{Snapshot, State};
use cex_core::MarketRegistry;
use cex_proto::{Command, OrderType, Side, TimeInForce};
use uuid::Uuid;

fn main() {
    let mut state = State::new(MarketRegistry::with_defaults());
    let alice = Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();

    state
        .apply(Command::Deposit {
            request_id: Uuid::nil(),
            user_id: alice,
            asset: "USDT".into(),
            amount: 1_000_000_000, // 1,000 USDT
        })
        .unwrap();

    state
        .apply(Command::PlaceOrder {
            request_id: Uuid::nil(),
            user_id: alice,
            symbol: "BTC_USDT".into(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            time_in_force: Some(TimeInForce::Gtc),
            price: Some(50_000_000_000), // 50,000.00 USDT
            qty: 100_000,                // 0.001 BTC
        })
        .unwrap();

    let snap = Snapshot::of(&state, "1699999999999-0");
    let bytes = snap.encode().unwrap();

    println!("encoded size: {} bytes\n", bytes.len());

    // Re-parse and pretty-print, purely so it is readable here. The real
    // encoding is the compact form above.
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}
