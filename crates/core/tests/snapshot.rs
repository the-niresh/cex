//! Snapshot and replay.
//!
//! The engine keeps everything in memory, so these tests are what make that
//! safe. They assert the two properties recovery depends on:
//!
//! 1. A snapshot round-trips to state that behaves identically.
//! 2. Loading a snapshot and replaying the commands after it reaches exactly the
//!    same place as never having restarted at all.
//!
//! Serialisation must also be *deterministic* — the same state must always
//! produce the same bytes, or two snapshots can never be compared and a replay
//! can never be verified.

use cex_core::state::{Snapshot, State, SNAPSHOT_VERSION};
use cex_core::MarketRegistry;
use cex_proto::{Command, OrderType, Query, ResponseBody, Side, TimeInForce};
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const USDT: &str = "USDT";
const BTC: &str = "BTC";

const P49K: i64 = 49_000_000_000;
const P50K: i64 = 50_000_000_000;
const P51K: i64 = 51_000_000_000;
const Q1: i64 = 100_000;

fn state() -> State {
    State::new(MarketRegistry::with_defaults())
}

fn rid() -> Uuid {
    Uuid::new_v4()
}

fn deposit(who: Uuid, asset: &str, amount: i64) -> Command {
    Command::Deposit {
        request_id: rid(),
        user_id: who,
        asset: asset.to_string(),
        amount,
    }
}

fn limit(who: Uuid, side: Side, price: i64, qty: i64) -> Command {
    Command::PlaceOrder {
        request_id: rid(),
        user_id: who,
        symbol: SYM.to_string(),
        side,
        order_type: OrderType::Limit,
        time_in_force: Some(TimeInForce::Gtc),
        price: Some(price),
        qty,
    }
}

/// A representative sequence: funded accounts, resting orders on both sides,
/// a trade, and a cancel. Enough that a lazy snapshot implementation cannot
/// pass by accident.
fn busy_session() -> (State, Uuid, Uuid) {
    let mut s = state();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    s.apply(deposit(alice, USDT, 1_000_000_000)).unwrap();
    s.apply(deposit(bob, BTC, 100_000_000)).unwrap();
    s.apply(deposit(bob, USDT, 1_000_000_000)).unwrap();

    s.apply(limit(alice, Side::Buy, P49K, Q1)).unwrap();   // rests
    s.apply(limit(bob, Side::Sell, P51K, Q1)).unwrap();    // rests
    s.apply(limit(bob, Side::Sell, P50K, Q1 * 2)).unwrap(); // rests
    s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();   // trades

    (s, alice, bob)
}

fn balances_of(s: &State, who: Uuid) -> Vec<(String, i64, i64)> {
    match s
        .query(&Query::Balances {
            request_id: rid(),
            user_id: who,
        })
        .unwrap()
    {
        ResponseBody::Balances(v) => v
            .into_iter()
            .map(|b| (b.asset, b.available, b.locked))
            .collect(),
        other => panic!("expected Balances, got {other:?}"),
    }
}

fn depth_of(s: &State) -> (Vec<[i64; 2]>, Vec<[i64; 2]>) {
    match s
        .query(&Query::Depth {
            request_id: rid(),
            symbol: SYM.to_string(),
            limit: None,
        })
        .unwrap()
    {
        ResponseBody::Depth(d) => (d.bids, d.asks),
        other => panic!("expected Depth, got {other:?}"),
    }
}

// ───────────────────────── determinism of encoding ─────────────────────────

#[test]
fn encoding_the_same_state_twice_produces_identical_bytes() {
    // If this fails, some collection iterates in a random order and no snapshot
    // can ever be compared against another.
    let (s, _, _) = busy_session();

    let a = Snapshot::of(&s, "8000-0").encode().unwrap();
    let b = Snapshot::of(&s, "8000-0").encode().unwrap();

    assert_eq!(a, b, "serialisation is not deterministic");
}

#[test]
fn two_states_built_from_the_same_commands_encode_identically() {
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    // Built once and replayed, because the command ids are part of the state now
    // — the engine remembers them to deduplicate retries. Regenerating them per
    // build would be two different command logs, which is not what this asserts.
    let commands = vec![
        deposit(alice, USDT, 1_000_000_000),
        deposit(bob, BTC, 100_000_000),
        limit(alice, Side::Buy, P49K, Q1),
        limit(bob, Side::Sell, P49K, Q1),
    ];

    let build = || {
        let mut s = state();
        for cmd in &commands {
            s.apply(cmd.clone()).unwrap();
        }
        s
    };

    let one = Snapshot::of(&build(), "1-0").encode().unwrap();
    let two = Snapshot::of(&build(), "1-0").encode().unwrap();

    assert_eq!(one, two, "same commands must produce the same bytes");
}

// ───────────────────────── round trip ─────────────────────────

#[test]
fn a_snapshot_round_trips_to_an_identical_state() {
    let (before, alice, bob) = busy_session();

    let bytes = Snapshot::of(&before, "8000-0").encode().unwrap();
    let restored = Snapshot::decode(&bytes).unwrap();

    assert_eq!(restored.last_stream_id, "8000-0");
    assert_eq!(restored.version, SNAPSHOT_VERSION);

    let after = restored.state;
    assert_eq!(balances_of(&after, alice), balances_of(&before, alice));
    assert_eq!(balances_of(&after, bob), balances_of(&before, bob));
    assert_eq!(depth_of(&after), depth_of(&before));
    assert_eq!(after.seq(), before.seq());
    after.check_invariants().expect("restored state is coherent");
}

#[test]
fn a_restored_state_re_encodes_to_the_same_bytes() {
    // The strongest form of the round-trip claim: nothing was lost, including
    // fields no query happens to expose.
    let (before, _, _) = busy_session();

    let bytes = Snapshot::of(&before, "8000-0").encode().unwrap();
    let restored = Snapshot::decode(&bytes).unwrap();
    let again = Snapshot::of(&restored.state, "8000-0").encode().unwrap();

    assert_eq!(bytes, again, "a field was dropped or reordered by the round trip");
}

#[test]
fn resting_orders_survive_a_round_trip_and_can_still_be_cancelled() {
    let (before, alice, _) = busy_session();
    let open = before.open_order_ids();
    assert!(!open.is_empty(), "the fixture should leave orders resting");

    let bytes = Snapshot::of(&before, "1-0").encode().unwrap();
    let mut after = Snapshot::decode(&bytes).unwrap().state;

    assert_eq!(after.open_order_ids(), open);

    // A restored order is a real order: it can be cancelled and its reservation
    // comes back. This is what proves the arena and the price levels reconnected.
    let alices: Vec<u64> = open
        .into_iter()
        .filter(|id| after.order_owner(*id) == Some(alice))
        .collect();
    assert!(!alices.is_empty());

    for id in alices {
        after
            .apply(Command::CancelOrder {
                request_id: rid(),
                user_id: alice,
                order_id: id,
            })
            .expect("a restored order must be cancellable");
    }
    after.check_invariants().unwrap();
}

#[test]
fn a_restored_book_still_matches_new_orders_in_the_right_order() {
    let (before, alice, bob) = busy_session();
    let bytes = Snapshot::of(&before, "1-0").encode().unwrap();
    let mut after = Snapshot::decode(&bytes).unwrap().state;

    // Bob's 50k ask has qty left after the fixture's trade; a new buy must hit it.
    let applied = after.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();
    let traded = applied.events.iter().any(|e| matches!(e, cex_proto::Event::Trades { .. }));

    assert!(traded, "the restored book did not match");
    let _ = bob;
    after.check_invariants().unwrap();
}

// ───────────────────────── replay ─────────────────────────

#[test]
fn replaying_from_a_snapshot_reaches_the_same_state_as_never_restarting() {
    // The whole recovery argument in one test.
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    let setup: Vec<Command> = vec![
        deposit(alice, USDT, 1_000_000_000),
        deposit(bob, BTC, 100_000_000),
        limit(alice, Side::Buy, P49K, Q1),
    ];
    let after_snapshot: Vec<Command> = vec![
        limit(bob, Side::Sell, P51K, Q1),
        limit(bob, Side::Sell, P49K, Q1),
        limit(alice, Side::Buy, P50K, Q1),
    ];

    // Path A: one continuous run.
    let mut uninterrupted = state();
    for c in setup.iter().chain(after_snapshot.iter()) {
        let _ = uninterrupted.apply(c.clone());
    }

    // Path B: run the setup, snapshot, "crash", restore, replay the rest.
    let mut crashed = state();
    for c in &setup {
        let _ = crashed.apply(c.clone());
    }
    let bytes = Snapshot::of(&crashed, "3-0").encode().unwrap();
    drop(crashed);

    let mut recovered = Snapshot::decode(&bytes).unwrap().state;
    for c in &after_snapshot {
        let _ = recovered.apply(c.clone());
    }

    assert_eq!(
        Snapshot::of(&recovered, "6-0").encode().unwrap(),
        Snapshot::of(&uninterrupted, "6-0").encode().unwrap(),
        "recovery diverged from the uninterrupted run"
    );
    recovered.check_invariants().unwrap();
}

#[test]
fn order_ids_do_not_restart_after_recovery() {
    // A reused order id would silently corrupt every downstream consumer.
    let (before, alice, _) = busy_session();
    let bytes = Snapshot::of(&before, "7-0").encode().unwrap();
    let mut after = Snapshot::decode(&bytes).unwrap().state;

    let highest = before.open_order_ids().into_iter().max().unwrap_or(0);
    let applied = after.apply(limit(alice, Side::Buy, P49K, Q1)).unwrap();

    match applied.response {
        ResponseBody::OrderPlaced { order_id, .. } => {
            assert!(order_id > highest, "order id {order_id} was reused");
        }
        other => panic!("expected OrderPlaced, got {other:?}"),
    }
}

// ───────────────────────── the stream position ─────────────────────────

#[test]
fn a_snapshot_carries_the_stream_position_it_was_taken_at() {
    // Without this the snapshot is useless: there is no way to know which
    // commands still need replaying.
    let (s, _, _) = busy_session();
    let snap = Snapshot::of(&s, "1699999999999-4");

    let restored = Snapshot::decode(&snap.encode().unwrap()).unwrap();
    assert_eq!(restored.last_stream_id, "1699999999999-4");
}

// ───────────────────────── rejecting bad input ─────────────────────────

#[test]
fn corrupt_bytes_are_rejected_rather_than_panicking() {
    let (s, _, _) = busy_session();
    let mut bytes = Snapshot::of(&s, "1-0").encode().unwrap();
    let mid = bytes.len() / 2;
    bytes.truncate(mid);

    assert!(
        Snapshot::decode(&bytes).is_err(),
        "a truncated snapshot must be an error, never a partial load"
    );
}

#[test]
fn empty_input_is_rejected() {
    assert!(Snapshot::decode(&[]).is_err());
}

#[test]
fn a_snapshot_from_a_future_version_is_rejected() {
    // Loading a snapshot written by a newer build would silently misinterpret
    // fields. Refuse and replay from the log instead.
    let (s, _, _) = busy_session();
    let bytes = Snapshot::of(&s, "1-0").encode().unwrap();

    let text = String::from_utf8(bytes).expect("snapshots are utf-8 for now");
    let bumped = text.replacen(
        &format!("\"version\":{SNAPSHOT_VERSION}"),
        &format!("\"version\":{}", SNAPSHOT_VERSION + 1),
        1,
    );
    assert_ne!(bumped, text, "version field not found in the encoding");

    assert!(
        Snapshot::decode(bumped.as_bytes()).is_err(),
        "a newer snapshot version must be refused"
    );
}
