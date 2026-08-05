//! The WebSocket surface, over a real socket.
//!
//! These speak the actual protocol against a real listener rather than calling
//! handlers directly, because the three things worth proving here — you get
//! only what you asked for, a slow client is dropped instead of stalling the
//! others, and the private feed never leaks — are all properties of the
//! connection loop, not of any one function.
//!
//! No Redis: the tests own the broadcast sender, so they can decide exactly
//! what is published and when.

use cex_auth::Tokens;
use cex_proto::{Seq, Side, UserId};
use cex_ws::route::Update;
use cex_ws::wire::{Channel, Envelope, Payload, PublicTrade};
use cex_ws::{build_router, AppState};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const P50K: i64 = 50_000_000_000;

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

const SECRET: &[u8] = b"a test secret that is long enough";

fn tokens() -> Tokens {
    Tokens::new(SECRET, Duration::from_secs(3600))
}

/// A listening server, plus the sender its connections read from.
struct Harness {
    url: String,
    tx: broadcast::Sender<Arc<Update>>,
    _server: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn start_with_capacity(capacity: usize) -> Harness {
        let (tx, _) = broadcast::channel(capacity);
        let state = AppState::new(tx.clone(), tokens());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, build_router(state)).await;
        });

        Harness {
            url: format!("ws://{addr}/ws"),
            tx,
            _server: server,
        }
    }

    async fn start() -> Harness {
        Harness::start_with_capacity(1024).await
    }

    async fn connect(&self) -> Client {
        connect_async(&self.url).await.expect("websocket").0
    }

    fn publish(&self, update: Update) {
        let _ = self.tx.send(Arc::new(update));
    }

    fn publish_trade(&self, symbol: &str, price: i64) {
        self.publish(trade(symbol, price));
    }
}

fn trade(symbol: &str, price: i64) -> Update {
    update_with(
        Channel::Trades(symbol.into()),
        None,
        1,
        Payload::Trade(PublicTrade {
            symbol: symbol.into(),
            price,
            qty: 100_000,
            taker_side: Side::Buy,
        }),
    )
}

fn update_with(channel: Channel, audience: Option<UserId>, seq: Seq, data: Payload) -> Update {
    let payload = serde_json::to_string(&Envelope {
        channel: channel.to_string(),
        seq,
        data,
    })
    .unwrap();
    Update {
        channel,
        audience,
        seq,
        payload,
    }
}

/// A private order update addressed to one user.
fn private_fill(user: UserId, order_id: u64) -> Update {
    update_with(
        Channel::Orders,
        Some(user),
        7,
        Payload::Order(cex_ws::wire::OrderUpdate::Fill {
            order_id,
            symbol: SYM.into(),
            price: P50K,
            qty: 100_000,
            side: Side::Buy,
            fee: 50,
            role: cex_ws::wire::Role::Taker,
        }),
    )
}

async fn send(client: &mut Client, json: serde_json::Value) {
    client
        .send(Message::Text(json.to_string()))
        .await
        .expect("send");
}

/// Next text frame, or `None` if the connection closed or went quiet.
async fn next_text(client: &mut Client) -> Option<String> {
    loop {
        let frame = tokio::time::timeout(Duration::from_millis(750), client.next()).await;
        match frame {
            Ok(Some(Ok(Message::Text(t)))) => return Some(t),
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) => return None,
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(_))) => return None,
            Err(_) => return None, // quiet
        }
    }
}

/// Subscribe and consume the acknowledgement.
async fn subscribe(client: &mut Client, channels: &[&str]) -> String {
    send(
        client,
        serde_json::json!({"op": "subscribe", "channels": channels}),
    )
    .await;
    next_text(client).await.expect("a subscribe reply")
}

async fn authenticate(client: &mut Client, user: UserId) -> String {
    let token = tokens().issue(user).unwrap();
    send(client, serde_json::json!({"op": "auth", "token": token})).await;
    next_text(client).await.expect("an auth reply")
}

// ───────────────────────── subscribing ─────────────────────────

#[tokio::test]
async fn a_subscriber_receives_what_it_subscribed_to() {
    let h = Harness::start().await;
    let mut c = h.connect().await;

    let reply = subscribe(&mut c, &["trades@BTC_USDT"]).await;
    assert!(reply.contains("subscribed"), "{reply}");
    assert!(reply.contains("trades@BTC_USDT"), "{reply}");

    h.publish_trade(SYM, P50K);

    let got = next_text(&mut c).await.expect("the trade");
    let env: Envelope = serde_json::from_str(&got).unwrap();
    assert_eq!(env.channel, "trades@BTC_USDT");
    match env.data {
        Payload::Trade(t) => assert_eq!(t.price, P50K),
        other => panic!("expected a trade, got {other:?}"),
    }
}

#[tokio::test]
async fn a_subscriber_receives_only_what_it_subscribed_to() {
    let h = Harness::start().await;
    let mut c = h.connect().await;
    subscribe(&mut c, &["trades@BTC_USDT"]).await;

    // Another market, and another channel on the same market.
    h.publish_trade("ETH_USDT", 1);
    h.publish(update_with(
        Channel::Depth(SYM.into()),
        None,
        2,
        Payload::Depth(cex_ws::wire::DepthUpdate {
            symbol: SYM.into(),
            depth_seq: 1,
            deltas: vec![],
        }),
    ));
    // Then something it did ask for, so the test cannot pass by silence alone.
    h.publish_trade(SYM, P50K);

    let got = next_text(&mut c).await.expect("the subscribed trade");
    let env: Envelope = serde_json::from_str(&got).unwrap();
    assert_eq!(
        env.channel, "trades@BTC_USDT",
        "the first frame delivered was one it never subscribed to"
    );
    assert!(
        next_text(&mut c).await.is_none(),
        "nothing else should have been delivered"
    );
}

#[tokio::test]
async fn subscribing_to_nothing_delivers_nothing() {
    let h = Harness::start().await;
    let mut c = h.connect().await;

    h.publish_trade(SYM, P50K);
    assert!(next_text(&mut c).await.is_none());
}

#[tokio::test]
async fn unsubscribing_stops_delivery() {
    let h = Harness::start().await;
    let mut c = h.connect().await;
    subscribe(&mut c, &["trades@BTC_USDT"]).await;

    send(
        &mut c,
        serde_json::json!({"op": "unsubscribe", "channels": ["trades@BTC_USDT"]}),
    )
    .await;
    next_text(&mut c).await.expect("an unsubscribe reply");

    h.publish_trade(SYM, P50K);
    assert!(next_text(&mut c).await.is_none());
}

#[tokio::test]
async fn an_unknown_channel_is_refused_and_subscribes_to_nothing() {
    let h = Harness::start().await;
    let mut c = h.connect().await;

    let reply = subscribe(&mut c, &["trades@BTC_USDT", "book@BTC_USDT"]).await;
    assert!(reply.contains("error"), "{reply}");

    // All or nothing: the valid channel in the same request must not have
    // taken effect, or the client is watching something it was told it is not.
    h.publish_trade(SYM, P50K);
    assert!(next_text(&mut c).await.is_none());
}

#[tokio::test]
async fn a_malformed_message_is_answered_with_an_error_not_a_disconnect() {
    let h = Harness::start().await;
    let mut c = h.connect().await;

    send(&mut c, serde_json::json!({"op": "nonsense"})).await;
    let reply = next_text(&mut c).await.expect("an error reply");
    assert!(reply.contains("error"), "{reply}");

    // Still usable.
    let reply = subscribe(&mut c, &["trades@BTC_USDT"]).await;
    assert!(reply.contains("subscribed"), "{reply}");
}

// ───────────────────────── the private feed ─────────────────────────

#[tokio::test]
async fn the_private_channel_is_refused_without_a_token() {
    let h = Harness::start().await;
    let mut c = h.connect().await;

    let reply = subscribe(&mut c, &["orders"]).await;
    assert!(reply.contains("error"), "{reply}");

    h.publish(private_fill(Uuid::new_v4(), 1));
    assert!(next_text(&mut c).await.is_none());
}

#[tokio::test]
async fn a_forged_token_is_refused() {
    let h = Harness::start().await;
    let mut c = h.connect().await;

    let attacker = Tokens::new(b"not the servers secret at all", Duration::from_secs(3600));
    let forged = attacker.issue(Uuid::new_v4()).unwrap();
    send(&mut c, serde_json::json!({"op": "auth", "token": forged})).await;

    let reply = next_text(&mut c).await.expect("an auth reply");
    assert!(reply.contains("error"), "{reply}");
    assert!(subscribe(&mut c, &["orders"]).await.contains("error"));
}

#[tokio::test]
async fn an_authenticated_client_receives_its_own_private_updates() {
    let h = Harness::start().await;
    let alice = Uuid::new_v4();
    let mut c = h.connect().await;

    assert!(authenticate(&mut c, alice).await.contains("authenticated"));
    assert!(subscribe(&mut c, &["orders"]).await.contains("orders"));

    h.publish(private_fill(alice, 11));
    let got = next_text(&mut c).await.expect("alice's fill");
    assert!(got.contains("\"order_id\":11"), "{got}");
}

/// The one that would matter most if it were wrong.
#[tokio::test]
async fn the_private_feed_never_leaks_another_users_fills() {
    let h = Harness::start().await;
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    let mut a = h.connect().await;
    authenticate(&mut a, alice).await;
    subscribe(&mut a, &["orders"]).await;

    let mut b = h.connect().await;
    authenticate(&mut b, bob).await;
    subscribe(&mut b, &["orders"]).await;

    // Bob's fill first, then Alice's. If Alice's connection leaked, the first
    // frame she reads would be bob's order id.
    h.publish(private_fill(bob, 99));
    h.publish(private_fill(alice, 11));

    let to_alice = next_text(&mut a).await.expect("alice receives something");
    assert!(
        to_alice.contains("\"order_id\":11"),
        "alice was handed bob's fill: {to_alice}"
    );
    assert!(
        next_text(&mut a).await.is_none(),
        "alice received a second frame that was not hers"
    );

    let to_bob = next_text(&mut b).await.expect("bob receives something");
    assert!(to_bob.contains("\"order_id\":99"), "{to_bob}");
    assert!(next_text(&mut b).await.is_none());
}

#[tokio::test]
async fn a_public_subscription_still_works_alongside_a_private_one() {
    let h = Harness::start().await;
    let alice = Uuid::new_v4();
    let mut c = h.connect().await;

    authenticate(&mut c, alice).await;
    subscribe(&mut c, &["orders", "trades@BTC_USDT"]).await;

    h.publish_trade(SYM, P50K);
    let got = next_text(&mut c).await.expect("the trade");
    assert!(got.contains("trades@BTC_USDT"), "{got}");
}

// ───────────────────────── keeping up ─────────────────────────

/// A subscriber that stops reading must be disconnected, and must not hold up
/// anybody else while it is being disconnected.
#[tokio::test]
async fn a_slow_subscriber_is_dropped_and_does_not_stall_the_others() {
    // A small ring, so falling behind takes a realistic amount of traffic
    // rather than a synthetic flood.
    let h = Harness::start_with_capacity(8).await;

    let mut slow = h.connect().await;
    subscribe(&mut slow, &["trades@BTC_USDT"]).await;

    let mut healthy = h.connect().await;
    subscribe(&mut healthy, &["trades@BTC_USDT"]).await;

    // `healthy` keeps draining throughout; `slow` never reads.
    let drain = tokio::spawn(async move {
        let mut seen = 0usize;
        while (next_text(&mut healthy).await).is_some() {
            seen += 1;
        }
        seen
    });

    for i in 0..20_000 {
        h.publish_trade(SYM, P50K + i);
    }

    // The slow one is closed, not merely warned. Being told it fell behind and
    // then left connected is the failure this asserts against: the client would
    // carry on with a silent hole in its feed. Only a real close counts, so
    // this must not use `next_text` — that cannot tell a closed connection from
    // a quiet one.
    let closed = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match slow.next().await {
                None => return true,
                Some(Ok(Message::Close(_))) => return true,
                // A reset is a drop too, just a blunter one.
                Some(Err(_)) => return true,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert_eq!(closed, Ok(true), "the slow subscriber was never dropped");

    let seen = tokio::time::timeout(Duration::from_secs(20), drain)
        .await
        .expect("the healthy subscriber's reader finished")
        .unwrap();
    assert!(
        seen > 0,
        "the healthy subscriber received nothing while the slow one misbehaved"
    );
}

#[tokio::test]
async fn a_disconnecting_subscriber_does_not_affect_the_others() {
    let h = Harness::start().await;

    let mut leaving = h.connect().await;
    subscribe(&mut leaving, &["trades@BTC_USDT"]).await;

    let mut staying = h.connect().await;
    subscribe(&mut staying, &["trades@BTC_USDT"]).await;

    leaving.close(None).await.unwrap();
    drop(leaving);
    tokio::time::sleep(Duration::from_millis(100)).await;

    h.publish_trade(SYM, P50K);
    assert!(
        next_text(&mut staying).await.is_some(),
        "the remaining subscriber stopped receiving when another left"
    );
}

#[tokio::test]
async fn a_subscriber_does_not_receive_updates_published_before_it_connected() {
    let h = Harness::start().await;

    h.publish_trade(SYM, P50K);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut c = h.connect().await;
    subscribe(&mut c, &["trades@BTC_USDT"]).await;

    assert!(
        next_text(&mut c).await.is_none(),
        "a fresh connection was handed a stale trade"
    );
}
