//! Drives real orders through a running exchange and reports what they cost.
//!
//! Two latencies per order. `ack` is the HTTP response returning. `visible` is
//! the order appearing on the private feed, which is when a real trader would
//! see it. They are different numbers and the gap between them is the event
//! path, so both are reported. Two more histograms come off the response
//! headers rather than this process's clock: `engine` is `x-cex-engine-us`,
//! the time spent waiting on the matching engine, and `server` is
//! `x-cex-server-us`, the whole request as the API measured it. Both are the
//! exchange's view of itself, independent of where this process happens to be
//! running. `server` minus `engine` is the API's own work — routing, auth,
//! JSON — and *not* the Redis hop, which sits inside `engine` and cannot be
//! split out without giving the engine a clock it deliberately does not have.
//! `server` is also what the trading screen displays as its `engine` figure,
//! so its p99 is the source of that screen's degraded threshold.
//!
//! Correlation is done on the private `orders` feed, which carries the
//! order's own id, not on the public depth feed: several orders can land
//! inside one depth delta, so a delta cannot be attributed to one order. See
//! `is_order_accepted` below. The `visible ≥ ack` ordering this produces is a
//! consequence of how the two are timed (both from one `Instant`, `ack`
//! recorded first), not evidence of correctness by itself — see
//! `docs/internals.md` for what actually establishes the correlation is
//! right.
//!
//! Before the timed loop starts, the private connection blocks until the
//! server confirms the `orders` subscription (`wait_for_subscribed`) so the
//! first order can't be placed — and its `Accepted` event lost — while the
//! subscription is still being registered. Each order's wait on the private
//! feed is bounded by a timeout that names the order it gave up on, rather
//! than hanging forever on a dropped event.
//!
//! Re-derive with:
//!   cargo run -p cex-loadgen -- --host http://localhost:8080 \
//!     --ws ws://localhost:8081 --count 2000 --out target/latency

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use cex_loadgen::report::Samples;
use clap::Parser;
use uuid::Uuid;

const MID: i64 = 50_000_000_000;
const TICK: i64 = 10_000;
const QTY: i64 = 100_000;

/// The part of a request spent waiting on the engine.
const ENGINE_US_HEADER: &str = "x-cex-engine-us";
/// The whole request, as the API measured it. Always at least `ENGINE_US_HEADER`.
const SERVER_US_HEADER: &str = "x-cex-server-us";

/// How long to wait for the server to confirm the `orders` subscription
/// before giving up. This is a single local round trip with no matching
/// engine work behind it, so it should resolve in milliseconds; generous
/// mainly so a slow CI runner doesn't make this flaky.
const SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to wait for one order's `Accepted` event on the private feed
/// before giving up. Generous enough not to fire on a healthy run over the
/// public internet (Task 8's target), where the HTTP round trip alone can
/// already run into the hundreds of milliseconds.
const ORDER_TIMEOUT: Duration = Duration::from_secs(30);

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
    let mut engine = HeaderSamples::new(ENGINE_US_HEADER);
    let mut server = HeaderSamples::new(SERVER_US_HEADER);

    for _ in 0..args.count {
        // A market buy, so it takes from the seeded asks rather than resting.
        let started = Instant::now();
        let (order_id, timings) =
            place(&http, &args.host, &taker, &args.symbol, "BUY", None).await?;
        ack.record(started.elapsed().as_micros() as u64);
        engine.record(order_id, timings.engine);
        server.record(order_id, timings.server);

        wait_for_order(&mut orders_feed, order_id).await?;
        visible.record(started.elapsed().as_micros() as u64);
    }

    std::fs::create_dir_all(&args.out)?;
    ack.write_csv(std::fs::File::create(format!("{}/ack.csv", args.out))?)?;
    visible.write_csv(std::fs::File::create(format!("{}/visible.csv", args.out))?)?;
    engine
        .samples
        .write_csv(std::fs::File::create(format!("{}/engine.csv", args.out))?)?;
    server
        .samples
        .write_csv(std::fs::File::create(format!("{}/server.csv", args.out))?)?;

    println!("host    {}", args.host);
    println!("orders  {}", args.count);
    println!("ack     {:?}", ack.summary());
    println!("visible {:?}", visible.summary());
    println!("engine  {:?}", engine.samples.summary());
    println!("server  {:?}", server.samples.summary());
    println!("csv     {}/", args.out);

    // A silent shortfall is worse than a loud one, and it is worse for each
    // header for its own reason: the screen's amber threshold comes from
    // `server`'s p99, and the budget's Redis-hop row is `server` minus
    // `engine`, so a partial drop in either skews a published number without
    // anyone noticing the run went wrong.
    let expected = args.count as u64;
    engine.check_complete(expected)?;
    server.check_complete(expected)?;

    Ok(())
}

struct User {
    token: String,
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

/// What the `x-cex-engine-us` header told us about one response.
///
/// Kept distinct from `Option<u64>` on purpose: a header that never showed up
/// and a header that showed up mangled are different failures with different
/// causes, and collapsing them into one silent `None` would give a build that
/// stopped emitting the header, and a build that emits it in the wrong
/// format, the exact same (silent) symptom. Task 10 sources its amber
/// threshold from this histogram's p99, so a drop here is not a footnote.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TimingUs {
    Present(u64),
    Absent,
    Malformed(String),
}

/// Both timing headers off one response.
struct Timings {
    engine: TimingUs,
    server: TimingUs,
}

/// Reads both timing headers, each from its own name. They are not
/// interchangeable: `server` covers the whole request and `engine` only the
/// part of it spent waiting on the engine, so the difference between them is
/// the API's own overhead — a budget row nothing else measures.
fn read_timings(headers: &reqwest::header::HeaderMap) -> Timings {
    Timings {
        engine: parse_timing_us(headers.get(ENGINE_US_HEADER)),
        server: parse_timing_us(headers.get(SERVER_US_HEADER)),
    }
}

/// One histogram plus the accounting for every response that failed to supply
/// the header behind it.
struct HeaderSamples {
    header: &'static str,
    samples: Samples,
    missing: u64,
    malformed: u64,
}

impl HeaderSamples {
    fn new(header: &'static str) -> Self {
        HeaderSamples {
            header,
            samples: Samples::new(),
            missing: 0,
            malformed: 0,
        }
    }

    fn record(&mut self, order_id: u64, value: TimingUs) {
        match value {
            TimingUs::Present(us) => self.samples.record(us),
            TimingUs::Absent => self.missing += 1,
            TimingUs::Malformed(raw) => {
                self.malformed += 1;
                eprintln!(
                    "warning: order {order_id}: {} header present but unparsable: {raw:?}",
                    self.header
                );
            }
        }
    }

    /// Fails the whole run if any sample went unrecorded. A run that quietly
    /// measured 1,900 of 2,000 orders publishes a percentile drawn from a set
    /// nobody knows is incomplete.
    fn check_complete(&self, expected: u64) -> Result<()> {
        let recorded = self.samples.summary().count;
        if recorded != expected {
            anyhow::bail!(
                "the {} histogram recorded only {recorded} of {expected} samples \
                 ({} responses had no header at all, {} had one that failed to parse \
                 as a u64) — published numbers are derived from this histogram, so treat \
                 this run as unreliable until the cause is fixed",
                self.header,
                self.missing,
                self.malformed
            );
        }
        Ok(())
    }
}

/// Reads one timing header out of a response header value, distinguishing
/// "not sent" from "sent but not a valid u64" rather than merging both into
/// `None`. Takes the raw `HeaderValue` rather than an already-decoded `&str`
/// so this is exercised directly by unit tests without a live response.
fn parse_timing_us(header: Option<&reqwest::header::HeaderValue>) -> TimingUs {
    let Some(value) = header else {
        return TimingUs::Absent;
    };
    match value.to_str() {
        Ok(s) => match s.parse::<u64>() {
            Ok(n) => TimingUs::Present(n),
            Err(_) => TimingUs::Malformed(s.to_string()),
        },
        Err(_) => TimingUs::Malformed("<non-utf8 header value>".to_string()),
    }
}

/// Returns the new order's id and what the server said about its own timing.
/// A fresh `idempotency-key` every time.
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
) -> Result<(u64, Timings)> {
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

    let timings = read_timings(raw.headers());

    let response: serde_json::Value = raw.json().await?;
    let order_id = response["order_id"]
        .as_u64()
        .context("no order_id in the order response")?;

    Ok((order_id, timings))
}

type Feed =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Connects, authenticates, subscribes to the private `orders` channel, and
/// blocks until the server has confirmed that subscription.
///
/// That last part matters: without it, the caller could place the first
/// order before the ws server finished registering this connection for
/// `orders`, in which case its `Accepted` event is published to nobody and
/// `wait_for_order` would wait out its full timeout for an event that
/// already came and went.
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

    tokio::time::timeout(SUBSCRIBE_TIMEOUT, wait_for_subscribed(&mut socket))
        .await
        .context("timed out waiting for the server to confirm the orders subscription")??;

    Ok(socket)
}

/// Reads frames until the server confirms the `orders` subscription, or
/// reports an outright rejection (bad token, unknown channel, ...).
/// Everything else on the wire before that point — `Authenticated`, or a
/// `Subscribed` ack for some other channel — is expected and skipped.
async fn wait_for_subscribed(feed: &mut Feed) -> Result<()> {
    use cex_ws::wire::ServerMessage;
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;

    while let Some(frame) = feed.next().await {
        let Message::Text(text) = frame? else {
            continue;
        };
        let Ok(msg) = serde_json::from_str::<ServerMessage>(&text) else {
            // Envelopes (market data / order events) are not ServerMessages;
            // none are expected this early, but skipping is still correct.
            continue;
        };
        match msg {
            ServerMessage::Subscribed { channels } if channels.iter().any(|c| c == "orders") => {
                return Ok(());
            }
            ServerMessage::Error { error } => {
                anyhow::bail!("server rejected the orders subscription: {error}")
            }
            _ => continue,
        }
    }
    anyhow::bail!("the private feed closed before the orders subscription was confirmed")
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

/// Blocks until this exact order is acknowledged on the private feed, or
/// gives up loudly after `ORDER_TIMEOUT` — naming the order it gave up on —
/// rather than hanging forever on a dropped event. A caller must not treat a
/// timeout as "no sample this time" and carry on; it is propagated as an
/// error so the run stops rather than silently biasing the percentiles.
async fn wait_for_order(feed: &mut Feed, order_id: u64) -> Result<()> {
    match tokio::time::timeout(ORDER_TIMEOUT, wait_for_order_inner(feed, order_id)).await {
        Ok(result) => result,
        Err(_) => anyhow::bail!(
            "timed out after {ORDER_TIMEOUT:?} waiting for order {order_id} to be \
             acknowledged on the private feed"
        ),
    }
}

async fn wait_for_order_inner(feed: &mut Feed, order_id: u64) -> Result<()> {
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
    use reqwest::header::HeaderValue;

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

    #[test]
    fn parse_timing_us_reads_a_present_and_valid_header() {
        let value = reqwest::header::HeaderValue::from_static("1234");
        assert_eq!(parse_timing_us(Some(&value)), TimingUs::Present(1234));
    }

    #[test]
    fn parse_timing_us_reports_absent_when_there_is_no_header() {
        assert_eq!(parse_timing_us(None), TimingUs::Absent);
    }

    /// The failure Important 3 is about: a header that showed up but is not a
    /// valid u64 must be reported, not folded into the same "nothing to see
    /// here" bucket as a header that never showed up at all.
    #[test]
    fn parse_timing_us_reports_malformed_rather_than_dropping_a_present_but_unparsable_header() {
        let value = reqwest::header::HeaderValue::from_static("not-a-number");
        assert_eq!(
            parse_timing_us(Some(&value)),
            TimingUs::Malformed("not-a-number".to_string())
        );
    }

    #[test]
    fn parse_timing_us_reports_malformed_for_a_non_utf8_header_value() {
        let value = reqwest::header::HeaderValue::from_bytes(&[0xFF, 0xFE]).unwrap();
        assert!(matches!(
            parse_timing_us(Some(&value)),
            TimingUs::Malformed(_)
        ));
    }

    /// The two headers measure different things and both are load-bearing:
    /// the budget's Redis-hop row is `server minus engine`, and the screen's
    /// amber threshold comes from `server`'s p99. Reading one header name
    /// twice would make the hop exactly zero and the threshold too tight,
    /// with both numbers still looking entirely plausible.
    #[test]
    fn reads_the_server_and_engine_timings_from_their_own_header_names() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(ENGINE_US_HEADER, HeaderValue::from_static("100"));
        headers.insert(SERVER_US_HEADER, HeaderValue::from_static("250"));

        let timings = read_timings(&headers);

        assert_eq!(timings.engine, TimingUs::Present(100));
        assert_eq!(timings.server, TimingUs::Present(250));
    }

    /// `server` covers the whole request and `engine` only the part of it
    /// spent waiting on the engine, so a response carrying one and not the
    /// other is a half-measured sample, not a usable one. Each has to be
    /// accounted for on its own.
    #[test]
    fn a_response_missing_only_one_of_the_two_headers_is_counted_against_that_one() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(ENGINE_US_HEADER, HeaderValue::from_static("100"));

        let timings = read_timings(&headers);

        assert_eq!(timings.engine, TimingUs::Present(100));
        assert_eq!(timings.server, TimingUs::Absent);
    }
}
