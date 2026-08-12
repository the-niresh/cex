//! Drives real orders through a running exchange and reports what they cost.
//!
//! Two latencies per order. `ack` is the HTTP response returning. `visible` is
//! the order appearing on the private feed, which is when a real trader would
//! see it. They are different numbers and the gap between them is the event
//! path, so both are reported. A third histogram, `engine`, records the
//! server's own `x-cex-engine-us` header value — the exchange's view of
//! itself, independent of where this process happens to be running.
//!
//! Correlation is done on the private `orders` feed, which carries the
//! order's own id, not on the public depth feed: several orders can land
//! inside one depth delta, so a delta cannot be attributed to one order. See
//! `is_order_accepted` below.
//!
//! Re-derive with:
//!   cargo run -p cex-loadgen -- --host http://localhost:8080 \
//!     --ws ws://localhost:8081 --count 2000 --out target/latency

use std::time::Instant;

use anyhow::{Context, Result};
use cex_loadgen::report::Samples;
use clap::Parser;
use uuid::Uuid;

const MID: i64 = 50_000_000_000;
const TICK: i64 = 10_000;
const QTY: i64 = 100_000;

#[derive(Parser)]
struct Args {
    /// Base URL of the API, e.g. http://localhost:8080
    #[arg(long)]
    host: String,
    /// Base URL of the WebSocket, e.g. ws://localhost:8081
    #[arg(long)]
    ws: String,
    /// How many measured orders to send.
    #[arg(long, default_value_t = 2000)]
    count: usize,
    /// Directory for the histogram CSVs.
    #[arg(long, default_value = "target/latency")]
    out: String,
    #[arg(long, default_value = "BTC_USDT")]
    symbol: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let http = reqwest::Client::new();

    // Two users. The maker seeds resting liquidity so every measured order has
    // something to hit; the taker sends the orders being measured. Measuring a
    // taker against an empty book would time the resting path and call it
    // matching.
    let maker = register(&http, &args.host)
        .await
        .context("register maker")?;
    let taker = register(&http, &args.host)
        .await
        .context("register taker")?;

    fund(&http, &args.host, &maker).await?;
    fund(&http, &args.host, &taker).await?;
    seed_book(&http, &args.host, &maker, &args.symbol, args.count).await?;

    let mut orders_feed = subscribe_private_orders(&args.ws, &taker).await?;

    let mut ack = Samples::new();
    let mut visible = Samples::new();
    // The exchange's own view of itself, straight off x-cex-engine-us. This is
    // the only one of the three that is independent of where this process is
    // running, which is why the screen's degraded threshold is taken from it.
    let mut engine = Samples::new();

    for _ in 0..args.count {
        // A market buy, so it takes from the seeded asks rather than resting.
        let started = Instant::now();
        let (order_id, engine_us) =
            place(&http, &args.host, &taker, &args.symbol, "BUY", None).await?;
        ack.record(started.elapsed().as_micros() as u64);
        if let Some(us) = engine_us {
            engine.record(us);
        }

        wait_for_order(&mut orders_feed, order_id).await?;
        visible.record(started.elapsed().as_micros() as u64);
    }

    std::fs::create_dir_all(&args.out)?;
    ack.write_csv(std::fs::File::create(format!("{}/ack.csv", args.out))?)?;
    visible.write_csv(std::fs::File::create(format!("{}/visible.csv", args.out))?)?;
    engine.write_csv(std::fs::File::create(format!("{}/engine.csv", args.out))?)?;

    println!("host    {}", args.host);
    println!("orders  {}", args.count);
    println!("ack     {:?}", ack.summary());
    println!("visible {:?}", visible.summary());
    println!("engine  {:?}", engine.summary());
    println!("csv     {}/", args.out);

    Ok(())
}

struct User {
    token: String,
    #[allow(dead_code)] // kept for parity with the brief / future correlation needs
    id: Uuid,
}

async fn register(http: &reqwest::Client, host: &str) -> Result<User> {
    let username = format!("load{}", Uuid::new_v4().simple());
    let body: serde_json::Value = http
        .post(format!("{host}/register"))
        .json(&serde_json::json!({ "username": username, "password": "a-good-password" }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(User {
        token: body["token"].as_str().context("token")?.to_string(),
        id: body["user_id"].as_str().context("user_id")?.parse()?,
    })
}

async fn fund(http: &reqwest::Client, host: &str, who: &User) -> Result<()> {
    for asset in ["USDT", "BTC"] {
        http.post(format!("{host}/deposit"))
            .bearer_auth(&who.token)
            .header("idempotency-key", Uuid::new_v4().to_string())
            .json(&serde_json::json!({ "asset": asset, "amount": 1_000_000_000_000_000i64 }))
            .send()
            .await?
            .error_for_status()?;
    }
    Ok(())
}

/// One resting ask per measured order, so no measured order ever finds an empty
/// book. A taker against nothing would time the resting path and report it as
/// matching.
async fn seed_book(
    http: &reqwest::Client,
    host: &str,
    maker: &User,
    symbol: &str,
    levels: usize,
) -> Result<()> {
    for i in 0..levels {
        place(
            http,
            host,
            maker,
            symbol,
            "SELL",
            Some(MID + (i as i64 + 1) * TICK),
        )
        .await?;
    }
    Ok(())
}

/// Returns the new order's id and the server's own `x-cex-engine-us`, which is
/// `None` if the header was absent. A fresh `idempotency-key` every time.
///
/// A repeated key is answered straight from the idempotency log without the
/// engine ever seeing it, which would record a cache hit as a matching time.
/// Same trap as the benchmark in Task 2, one layer out.
async fn place(
    http: &reqwest::Client,
    host: &str,
    who: &User,
    symbol: &str,
    side: &str,
    price: Option<i64>,
) -> Result<(u64, Option<u64>)> {
    let mut body = serde_json::json!({
        "symbol": symbol,
        "side": side,
        "order_type": if price.is_some() { "LIMIT" } else { "MARKET" },
        "qty": QTY,
    });
    if let Some(p) = price {
        body["time_in_force"] = "GTC".into();
        body["price"] = p.into();
    }

    let raw = http
        .post(format!("{host}/orders"))
        .bearer_auth(&who.token)
        .header("idempotency-key", Uuid::new_v4().to_string())
        .json(&body)
        .send()
        .await?
        .error_for_status()?;

    let engine_us = raw
        .headers()
        .get("x-cex-engine-us")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    let response: serde_json::Value = raw.json().await?;
    let order_id = response["order_id"]
        .as_u64()
        .context("no order_id in the order response")?;

    Ok((order_id, engine_us))
}

type Feed =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn subscribe_private_orders(ws: &str, who: &User) -> Result<Feed> {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;

    let (mut socket, _) = tokio_tungstenite::connect_async(format!("{ws}/ws")).await?;
    socket
        .send(Message::Text(
            serde_json::json!({ "op": "auth", "token": who.token })
                .to_string()
                .into(),
        ))
        .await?;
    socket
        .send(Message::Text(
            serde_json::json!({ "op": "subscribe", "channels": ["orders"] })
                .to_string()
                .into(),
        ))
        .await?;
    Ok(socket)
}

/// True when this envelope is the `Accepted` event for exactly this order.
///
/// Correlating on the public depth channel would be wrong: several orders can
/// land inside one depth delta, so a delta cannot be attributed to one order.
/// `OrderUpdate::Accepted` carries the order's own id, so this predicate
/// belongs to exactly one order — matching on event type alone (ignoring the
/// id) would attribute a previous order's acceptance to this one and report
/// `visible` latencies that are impossibly fast, even faster than `ack`.
fn is_order_accepted(envelope: &cex_ws::wire::Envelope, order_id: u64) -> bool {
    use cex_ws::wire::{OrderUpdate, Payload};

    matches!(
        &envelope.data,
        Payload::Order(OrderUpdate::Accepted { order_id: id, .. }) if *id == order_id
    )
}

/// Blocks until this exact order is acknowledged on the private feed.
async fn wait_for_order(feed: &mut Feed, order_id: u64) -> Result<()> {
    use cex_ws::wire::Envelope;
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;

    while let Some(frame) = feed.next().await {
        let Message::Text(text) = frame? else {
            continue;
        };
        let Ok(envelope) = serde_json::from_str::<Envelope>(&text) else {
            // Subscribed / Authenticated / Error acknowledgements are not
            // envelopes. Skipping them is expected, not a failure.
            continue;
        };
        if is_order_accepted(&envelope, order_id) {
            return Ok(());
        }
    }
    anyhow::bail!("the private feed closed before order {order_id} was acknowledged")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cex_proto::{OrderStatus, OrderType, Side};
    use cex_ws::wire::{DepthUpdate, Envelope, OrderUpdate, Payload};

    fn envelope(data: Payload) -> Envelope {
        Envelope {
            channel: "orders".to_string(),
            seq: 1,
            data,
        }
    }

    #[test]
    fn matches_the_accepted_event_for_this_order_id() {
        let env = envelope(Payload::Order(OrderUpdate::Accepted {
            order_id: 42,
            symbol: "BTC_USDT".to_string(),
            side: Side::Buy,
            order_type: OrderType::Market,
            price: None,
            qty: QTY,
        }));

        assert!(is_order_accepted(&env, 42));
    }

    /// The exact bug the brief warns about: matching an Accepted event that
    /// belongs to a *different* order would make `visible` look faster than
    /// `ack`, since it could be satisfied by an event already in flight.
    #[test]
    fn does_not_match_the_accepted_event_for_a_different_order_id() {
        let env = envelope(Payload::Order(OrderUpdate::Accepted {
            order_id: 41,
            symbol: "BTC_USDT".to_string(),
            side: Side::Buy,
            order_type: OrderType::Market,
            price: None,
            qty: QTY,
        }));

        assert!(!is_order_accepted(&env, 42));
    }

    #[test]
    fn does_not_match_other_order_events_for_the_same_id() {
        let updated = envelope(Payload::Order(OrderUpdate::Updated {
            order_id: 42,
            filled_qty: QTY,
            qty: QTY,
            status: OrderStatus::Filled,
        }));
        let cancelled = envelope(Payload::Order(OrderUpdate::Cancelled {
            order_id: 42,
            symbol: "BTC_USDT".to_string(),
            unfilled_qty: 0,
        }));

        assert!(!is_order_accepted(&updated, 42));
        assert!(!is_order_accepted(&cancelled, 42));
    }

    #[test]
    fn does_not_match_a_public_depth_payload() {
        let env = envelope(Payload::Depth(DepthUpdate {
            symbol: "BTC_USDT".to_string(),
            depth_seq: 1,
            deltas: Vec::new(),
        }));

        assert!(!is_order_accepted(&env, 42));
    }
}
