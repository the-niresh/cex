//! Behavioural tests for the balance ledger.
//!
//! The ledger has one job: never create or destroy value. Every test here is
//! ultimately a statement about conservation, because that is the property the
//! settlement code will be checked against.

use cex_core::balances::Balances;
use uuid::Uuid;

const USDT: &str = "USDT";
const BTC: &str = "BTC";

fn user() -> Uuid {
    Uuid::new_v4()
}

// ───────────────────────── reads ─────────────────────────

#[test]
fn an_unknown_account_reads_as_zero_not_as_free_money() {
    // `cex` minted 100,000 USD and 10 BTC for any unseen user, from inside a
    // getter. Reading a balance must never be a mutation.
    let b = Balances::new();
    let who = user();

    let bal = b.get(who, USDT);

    assert_eq!(bal.available, 0);
    assert_eq!(bal.locked, 0);
    assert_eq!(b.total_supply(USDT), 0, "a read must not create supply");
}

#[test]
fn balances_are_tracked_per_asset_independently() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 500).unwrap();

    assert_eq!(b.get(who, USDT).available, 500);
    assert_eq!(b.get(who, BTC).available, 0);
}

#[test]
fn balances_are_tracked_per_user_independently() {
    let mut b = Balances::new();
    let alice = user();
    let bob = user();
    b.credit(alice, USDT, 500).unwrap();

    assert_eq!(b.get(alice, USDT).available, 500);
    assert_eq!(b.get(bob, USDT).available, 0);
}

// ───────────────────────── credit and debit ─────────────────────────

#[test]
fn credit_increases_available() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();
    b.credit(who, USDT, 50).unwrap();

    assert_eq!(b.get(who, USDT).available, 150);
}

#[test]
fn debit_reduces_available() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();
    b.debit(who, USDT, 40).unwrap();

    assert_eq!(b.get(who, USDT).available, 60);
}

#[test]
fn debit_beyond_available_is_rejected_and_changes_nothing() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();

    assert!(b.debit(who, USDT, 101).is_err());
    assert_eq!(
        b.get(who, USDT).available,
        100,
        "a rejected debit must leave the account untouched"
    );
}

#[test]
fn debit_cannot_reach_into_locked_funds() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();
    b.lock(who, USDT, 80).unwrap();

    assert!(
        b.debit(who, USDT, 50).is_err(),
        "only the available 20 is spendable"
    );
    assert!(b.debit(who, USDT, 20).is_ok());
}

// ───────────────────────── lock and unlock ─────────────────────────

#[test]
fn lock_moves_funds_from_available_to_locked() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();
    b.lock(who, USDT, 30).unwrap();

    let bal = b.get(who, USDT);
    assert_eq!(bal.available, 70);
    assert_eq!(bal.locked, 30);
}

#[test]
fn lock_beyond_available_is_rejected_and_changes_nothing() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();

    assert!(b.lock(who, USDT, 101).is_err());
    let bal = b.get(who, USDT);
    assert_eq!(bal.available, 100);
    assert_eq!(bal.locked, 0);
}

#[test]
fn unlock_moves_funds_back_from_locked_to_available() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();
    b.lock(who, USDT, 30).unwrap();
    b.unlock(who, USDT, 30).unwrap();

    let bal = b.get(who, USDT);
    assert_eq!(bal.available, 100);
    assert_eq!(bal.locked, 0);
}

#[test]
fn unlock_beyond_locked_is_rejected() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();
    b.lock(who, USDT, 30).unwrap();

    assert!(
        b.unlock(who, USDT, 31).is_err(),
        "unlocking more than was locked would mint money"
    );
}

// ───────────────────────── settlement ─────────────────────────

#[test]
fn settle_removes_from_locked_without_crediting_available() {
    // Settling is a payment: the funds leave this account for another one.
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();
    b.lock(who, USDT, 30).unwrap();
    b.settle_locked(who, USDT, 30).unwrap();

    let bal = b.get(who, USDT);
    assert_eq!(bal.available, 70);
    assert_eq!(bal.locked, 0);
    assert_eq!(b.total_supply(USDT), 70, "the paid funds left this account");
}

#[test]
fn settle_beyond_locked_is_rejected() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();
    b.lock(who, USDT, 30).unwrap();

    assert!(b.settle_locked(who, USDT, 31).is_err());
}

// ───────────────────────── conservation ─────────────────────────

#[test]
fn total_supply_counts_both_available_and_locked() {
    let mut b = Balances::new();
    let alice = user();
    let bob = user();
    b.credit(alice, USDT, 100).unwrap();
    b.credit(bob, USDT, 250).unwrap();
    b.lock(alice, USDT, 40).unwrap();

    assert_eq!(b.total_supply(USDT), 350, "locking does not destroy supply");
}

#[test]
fn locking_and_unlocking_never_changes_total_supply() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();
    let before = b.total_supply(USDT);

    b.lock(who, USDT, 60).unwrap();
    assert_eq!(b.total_supply(USDT), before);

    b.unlock(who, USDT, 60).unwrap();
    assert_eq!(b.total_supply(USDT), before);
}

#[test]
fn a_settled_transfer_between_two_users_conserves_total_supply() {
    // This is the shape of every fill: one side settles out of locked, the other
    // is credited. The pair must be neutral.
    let mut b = Balances::new();
    let alice = user();
    let bob = user();
    b.credit(alice, USDT, 100).unwrap();
    let before = b.total_supply(USDT);

    b.lock(alice, USDT, 60).unwrap();
    b.settle_locked(alice, USDT, 60).unwrap();
    b.credit(bob, USDT, 60).unwrap();

    assert_eq!(b.total_supply(USDT), before);
    assert_eq!(b.get(alice, USDT).available, 40);
    assert_eq!(b.get(bob, USDT).available, 60);
}

// ───────────────────────── input validation ─────────────────────────

#[test]
fn negative_amounts_are_rejected_everywhere() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();

    assert!(b.credit(who, USDT, -1).is_err(), "credit");
    assert!(b.debit(who, USDT, -1).is_err(), "debit");
    assert!(b.lock(who, USDT, -1).is_err(), "lock");
    assert!(b.unlock(who, USDT, -1).is_err(), "unlock");
    assert!(b.settle_locked(who, USDT, -1).is_err(), "settle");

    assert_eq!(b.get(who, USDT).available, 100, "nothing changed");
}

#[test]
fn a_balance_can_never_go_negative_through_any_sequence() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 10).unwrap();

    let _ = b.debit(who, USDT, 999);
    let _ = b.lock(who, USDT, 999);
    let _ = b.unlock(who, USDT, 999);
    let _ = b.settle_locked(who, USDT, 999);

    let bal = b.get(who, USDT);
    assert!(bal.available >= 0, "available went negative");
    assert!(bal.locked >= 0, "locked went negative");
    assert_eq!(bal.available, 10);
}

// ───────────────────────── enumeration ─────────────────────────

#[test]
fn all_balances_for_a_user_are_listed_with_their_locked_portion() {
    let mut b = Balances::new();
    let who = user();
    b.credit(who, USDT, 100).unwrap();
    b.credit(who, BTC, 5).unwrap();
    b.lock(who, BTC, 2).unwrap();

    let mut views = b.for_user(who);
    views.sort_by(|a, c| a.asset.cmp(&c.asset));

    assert_eq!(views.len(), 2);
    assert_eq!(views[0].asset, BTC);
    assert_eq!(views[0].available, 3);
    assert_eq!(views[0].locked, 2);
    assert_eq!(views[1].asset, USDT);
    assert_eq!(views[1].available, 100);
}

#[test]
fn a_user_with_no_balances_lists_nothing() {
    let b = Balances::new();
    assert!(b.for_user(user()).is_empty());
}
