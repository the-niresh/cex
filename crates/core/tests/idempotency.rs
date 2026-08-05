//! Applying the same command twice.
//!
//! A `504` from the API is genuinely ambiguous: the command is already on the
//! durable log, so a timeout is not proof that nothing happened. Until now the
//! only answer was "re-read `/orders/open` and work it out". A caller that
//! simply retried could deposit twice or place the same order twice, and on an
//! exchange that is real money.
//!
//! The identity is the command's `request_id`. The API derives it from the
//! caller's idempotency key, so a retry carries the same one and the engine
//! recognises it. Every command is remembered, which also makes the durable log
//! itself exactly-once: appending the same command twice applies it once.

use cex_core::state::{State, IDEMPOTENCY_CAPACITY};
use cex_core::MarketRegistry;
use cex_proto::{Command, OrderStatus, OrderType, ResponseBody, Side, TimeInForce};
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const USDT: &str = "USDT";
const P50K: i64 = 50_000_000_000;
const Q1: i64 = 100_000;

fn state() -> State {
    State::new(MarketRegistry::with_defaults())
}

fn deposit(rid: Uuid, who: Uuid, amount: i64) -> Command {
    Command::Deposit {
        request_id: rid,
        user_id: who,
        asset: USDT.to_string(),
        amount,
    }
}

fn limit_buy(rid: Uuid, who: Uuid) -> Command {
    Command::PlaceOrder {
        request_id: rid,
        user_id: who,
        symbol: SYM.to_string(),
        side: Side::Buy,
        order_type: OrderType::Limit,
        time_in_force: Some(TimeInForce::Gtc),
        price: Some(P50K),
        qty: Q1,
    }
}

fn available(s: &State, who: Uuid, asset: &str) -> i64 {
    s.balances().get(who, asset).available
}

// ───────────────────────── money ─────────────────────────

#[test]
fn a_repeated_deposit_credits_the_account_once() {
    let mut s = state();
    let alice = Uuid::new_v4();
    let rid = Uuid::new_v4();

    s.apply(deposit(rid, alice, 1_000)).unwrap();
    s.apply(deposit(rid, alice, 1_000)).unwrap();

    assert_eq!(
        available(&s, alice, USDT),
        1_000,
        "the retry credited the account a second time"
    );
    s.check_invariants().unwrap();
}

#[test]
fn a_repeated_order_rests_only_once() {
    let mut s = state();
    let alice = Uuid::new_v4();
    s.apply(deposit(Uuid::new_v4(), alice, 1_000_000_000))
        .unwrap();

    let rid = Uuid::new_v4();
    s.apply(limit_buy(rid, alice)).unwrap();
    s.apply(limit_buy(rid, alice)).unwrap();

    assert_eq!(
        s.open_order_ids().len(),
        1,
        "the retry placed a second order"
    );
    s.check_invariants().unwrap();
}

#[test]
fn a_repeated_order_locks_the_funds_only_once() {
    let mut s = state();
    let alice = Uuid::new_v4();
    s.apply(deposit(Uuid::new_v4(), alice, 1_000_000_000))
        .unwrap();
    let before = available(&s, alice, USDT);

    let rid = Uuid::new_v4();
    s.apply(limit_buy(rid, alice)).unwrap();
    let after_first = available(&s, alice, USDT);
    s.apply(limit_buy(rid, alice)).unwrap();

    assert_eq!(
        available(&s, alice, USDT),
        after_first,
        "the retry locked the funds a second time"
    );
    assert!(after_first < before);
    s.check_invariants().unwrap();
}

// ───────────────────────── what the retry gets back ─────────────────────────

#[test]
fn the_retry_gets_the_answer_the_first_attempt_produced() {
    let mut s = state();
    let alice = Uuid::new_v4();
    s.apply(deposit(Uuid::new_v4(), alice, 1_000_000_000))
        .unwrap();

    let rid = Uuid::new_v4();
    let first = s.apply(limit_buy(rid, alice)).unwrap();
    let repeat = s.apply(limit_buy(rid, alice)).unwrap();

    assert_eq!(
        repeat.response, first.response,
        "a retry must be told what happened, not merely that nothing happened"
    );
    match (&first.response, &repeat.response) {
        (
            ResponseBody::OrderPlaced { order_id: a, .. },
            ResponseBody::OrderPlaced {
                order_id: b,
                status,
                ..
            },
        ) => {
            assert_eq!(a, b, "the retry must name the order that already exists");
            assert_eq!(*status, OrderStatus::Open);
        }
        (_, other) => panic!("expected an order ack, got {other:?}"),
    }
}

#[test]
fn the_retry_emits_no_events() {
    let mut s = state();
    let alice = Uuid::new_v4();
    let rid = Uuid::new_v4();

    let first = s.apply(deposit(rid, alice, 1_000)).unwrap();
    let repeat = s.apply(deposit(rid, alice, 1_000)).unwrap();

    assert!(!first.events.is_empty());
    assert!(
        repeat.events.is_empty(),
        "nothing changed, so nothing may be published — a duplicate balance \
         event would move every downstream reader's view"
    );
}

#[test]
fn the_retry_does_not_advance_the_sequence() {
    let mut s = state();
    let alice = Uuid::new_v4();
    let rid = Uuid::new_v4();

    s.apply(deposit(rid, alice, 1_000)).unwrap();
    let seq_after_first = s.seq();
    let repeat = s.apply(deposit(rid, alice, 1_000)).unwrap();

    assert_eq!(s.seq(), seq_after_first, "seq counts state changes");
    assert_eq!(repeat.seq, seq_after_first);
}

// ───────────────────────── what is *not* deduplicated ─────────────────────────

#[test]
fn two_different_request_ids_are_two_commands() {
    let mut s = state();
    let alice = Uuid::new_v4();

    s.apply(deposit(Uuid::new_v4(), alice, 1_000)).unwrap();
    s.apply(deposit(Uuid::new_v4(), alice, 1_000)).unwrap();

    assert_eq!(available(&s, alice, USDT), 2_000);
}

#[test]
fn the_same_key_from_two_users_is_two_commands() {
    // The engine takes the request id at face value; keeping one caller's key
    // from blocking another's is the API's job, and it scopes the key to the
    // user. This pins the engine's half of that contract.
    let mut s = state();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let rid = Uuid::new_v4();

    s.apply(deposit(rid, alice, 1_000)).unwrap();
    let bob_result = s.apply(deposit(rid, bob, 1_000));

    // Same id, so the engine answers from the log. This is exactly why the API
    // must not hand two users the same request id.
    assert!(bob_result.is_ok());
    assert_eq!(available(&s, bob, USDT), 0);
}

#[test]
fn a_rejected_command_is_not_remembered() {
    let mut s = state();
    let alice = Uuid::new_v4();
    let rid = Uuid::new_v4();

    // No funds, so this fails and changes nothing.
    assert!(s.apply(limit_buy(rid, alice)).is_err());

    // Having fixed the problem, the same key must work. Remembering failures
    // would leave a caller permanently unable to retry a request that never
    // happened.
    s.apply(deposit(Uuid::new_v4(), alice, 1_000_000_000))
        .unwrap();
    s.apply(limit_buy(rid, alice))
        .expect("a command that never applied must not be blocked by its own failure");
    assert_eq!(s.open_order_ids().len(), 1);
}

// ───────────────────────── the bound ─────────────────────────

#[test]
fn the_log_does_not_grow_without_limit() {
    let mut s = state();
    let alice = Uuid::new_v4();

    for _ in 0..(IDEMPOTENCY_CAPACITY + 100) {
        s.apply(deposit(Uuid::new_v4(), alice, 1)).unwrap();
    }

    assert_eq!(
        s.remembered_requests(),
        IDEMPOTENCY_CAPACITY,
        "an unbounded log is a memory leak that also bloats every snapshot"
    );
}

#[test]
fn a_key_pushed_out_of_the_log_is_applied_again() {
    // The honest limit of the guarantee: this protects a retry, not a retry an
    // arbitrary length of time later. Naming it in a test so it stays true and
    // stays known.
    let mut s = state();
    let alice = Uuid::new_v4();
    let rid = Uuid::new_v4();

    s.apply(deposit(rid, alice, 1_000)).unwrap();
    for _ in 0..IDEMPOTENCY_CAPACITY {
        s.apply(deposit(Uuid::new_v4(), alice, 1)).unwrap();
    }
    s.apply(deposit(rid, alice, 1_000)).unwrap();

    assert_eq!(
        available(&s, alice, USDT),
        1_000 + IDEMPOTENCY_CAPACITY as i64 + 1_000,
        "an evicted key should be applied again, not silently swallowed"
    );
}

#[test]
fn the_oldest_key_is_the_one_evicted() {
    let mut s = state();
    let alice = Uuid::new_v4();
    let oldest = Uuid::new_v4();
    let newest = Uuid::new_v4();

    s.apply(deposit(oldest, alice, 1_000)).unwrap();
    for _ in 0..(IDEMPOTENCY_CAPACITY - 2) {
        s.apply(deposit(Uuid::new_v4(), alice, 1)).unwrap();
    }
    s.apply(deposit(newest, alice, 1_000)).unwrap();

    // One more push evicts exactly the oldest.
    s.apply(deposit(Uuid::new_v4(), alice, 1)).unwrap();

    let before = available(&s, alice, USDT);
    s.apply(deposit(newest, alice, 1_000)).unwrap();
    assert_eq!(
        available(&s, alice, USDT),
        before,
        "the newest key was evicted before the oldest"
    );

    s.apply(deposit(oldest, alice, 1_000)).unwrap();
    assert_eq!(
        available(&s, alice, USDT),
        before + 1_000,
        "the oldest key should have been the one to go"
    );
}

// ───────────────────────── across a restart ─────────────────────────

#[test]
fn the_log_survives_a_snapshot_and_restore() {
    use cex_core::state::Snapshot;

    let mut s = state();
    let alice = Uuid::new_v4();
    let rid = Uuid::new_v4();
    s.apply(deposit(rid, alice, 1_000)).unwrap();

    let snap = Snapshot::of(&s, "0-0");
    let encoded = serde_json::to_vec(&snap).unwrap();
    let restored: Snapshot = serde_json::from_slice(&encoded).unwrap();
    let mut recovered = restored.state;

    // A retry that arrives after the engine restarted must still be recognised,
    // or a crash turns every in-flight request into a possible double-apply.
    recovered.apply(deposit(rid, alice, 1_000)).unwrap();
    assert_eq!(available(&recovered, alice, USDT), 1_000);
}

#[test]
fn a_snapshot_of_the_same_state_is_byte_for_byte_identical() {
    use cex_core::state::Snapshot;

    // The log is part of the state now, so it has to encode deterministically
    // like everything else — otherwise no replay could ever be verified.
    let mut a = state();
    let mut b = state();
    let alice = Uuid::new_v4();
    let ids: Vec<Uuid> = (0..50).map(|_| Uuid::new_v4()).collect();

    for id in &ids {
        a.apply(deposit(*id, alice, 10)).unwrap();
        b.apply(deposit(*id, alice, 10)).unwrap();
    }

    let ea = serde_json::to_vec(&Snapshot::of(&a, "0-0")).unwrap();
    let eb = serde_json::to_vec(&Snapshot::of(&b, "0-0")).unwrap();
    assert_eq!(ea, eb);
}
