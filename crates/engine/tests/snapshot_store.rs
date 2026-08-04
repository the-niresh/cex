//! Snapshots on disk.
//!
//! Two failure modes this file exists to prevent:
//!
//! * **Loading a half-written file.** A crash during a write must leave the
//!   previous snapshot intact and usable, never a truncated one.
//! * **Picking the wrong "newest".** Redis stream ids are `<millis>-<seq>`.
//!   Sorted as text, `"9-0"` comes after `"10-0"`, so a naive implementation
//!   silently recovers from an *older* snapshot and replays commands that were
//!   already applied.

use std::fs;

use cex_core::state::{Snapshot, State};
use cex_core::MarketRegistry;
use cex_engine::snapshot_store::SnapshotStore;
use cex_engine::stream_id::StreamId;
use cex_proto::Command;
use uuid::Uuid;

fn state_with_deposit(amount: i64) -> State {
    let mut s = State::new(MarketRegistry::with_defaults());
    s.apply(Command::Deposit {
        request_id: Uuid::nil(),
        user_id: Uuid::from_u128(7),
        asset: "USDT".into(),
        amount,
    })
    .unwrap();
    s
}

fn deposited(s: &State) -> i64 {
    s.balances().get(Uuid::from_u128(7), "USDT").available
}

// ───────────────────────── stream id ordering ─────────────────────────

#[test]
fn stream_ids_compare_numerically_not_as_text() {
    let nine = StreamId::parse("9-0").unwrap();
    let ten = StreamId::parse("10-0").unwrap();

    assert!(ten > nine, "10-0 must be newer than 9-0");
    assert!("10-0" < "9-0", "text comparison really is backwards here");
}

#[test]
fn the_sequence_part_breaks_ties_within_a_millisecond() {
    let a = StreamId::parse("1700000000000-1").unwrap();
    let b = StreamId::parse("1700000000000-2").unwrap();
    assert!(b > a);
}

#[test]
fn a_malformed_stream_id_is_rejected() {
    assert!(StreamId::parse("").is_none());
    assert!(StreamId::parse("nonsense").is_none());
    assert!(StreamId::parse("12").is_none());
    assert!(StreamId::parse("12-").is_none());
    assert!(StreamId::parse("-3").is_none());
    assert!(StreamId::parse("a-b").is_none());
}

#[test]
fn a_stream_id_round_trips_through_its_text_form() {
    let id = StreamId::parse("1699999999999-42").unwrap();
    assert_eq!(id.to_string(), "1699999999999-42");
}

#[test]
fn the_zero_id_is_the_oldest_possible_position() {
    let zero = StreamId::parse("0-0").unwrap();
    assert_eq!(StreamId::ZERO, zero);
    assert!(StreamId::parse("1-0").unwrap() > StreamId::ZERO);
}

// ───────────────────────── save and load ─────────────────────────

#[test]
fn saving_then_loading_returns_an_equivalent_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = SnapshotStore::new(dir.path(), 3);
    let state = state_with_deposit(500);

    store.save(&Snapshot::of(&state, "100-0")).unwrap();
    let loaded = store.load_latest().unwrap().expect("a snapshot should exist");

    assert_eq!(loaded.last_stream_id, "100-0");
    assert_eq!(deposited(&loaded.state), 500);
}

#[test]
fn an_empty_directory_yields_nothing_rather_than_an_error() {
    // A first boot is not a failure.
    let dir = tempfile::tempdir().unwrap();
    let store = SnapshotStore::new(dir.path(), 3);

    assert!(store.load_latest().unwrap().is_none());
}

#[test]
fn a_missing_directory_is_created_on_first_save() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("data").join("snapshots");
    let store = SnapshotStore::new(&nested, 3);

    store.save(&Snapshot::of(&state_with_deposit(1), "1-0")).unwrap();

    assert!(nested.exists());
    assert!(store.load_latest().unwrap().is_some());
}

#[test]
fn the_newest_snapshot_wins_even_when_text_ordering_disagrees() {
    // The regression this whole file is named for.
    let dir = tempfile::tempdir().unwrap();
    let store = SnapshotStore::new(dir.path(), 10);

    store.save(&Snapshot::of(&state_with_deposit(900), "9-0")).unwrap();
    store.save(&Snapshot::of(&state_with_deposit(1000), "10-0")).unwrap();

    let loaded = store.load_latest().unwrap().unwrap();

    assert_eq!(loaded.last_stream_id, "10-0", "loaded the older snapshot");
    assert_eq!(deposited(&loaded.state), 1000);
}

#[test]
fn saving_the_same_position_twice_overwrites_rather_than_accumulating() {
    let dir = tempfile::tempdir().unwrap();
    let store = SnapshotStore::new(dir.path(), 10);

    store.save(&Snapshot::of(&state_with_deposit(1), "5-0")).unwrap();
    store.save(&Snapshot::of(&state_with_deposit(2), "5-0")).unwrap();

    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(deposited(&store.load_latest().unwrap().unwrap().state), 2);
}

// ───────────────────────── surviving bad files ─────────────────────────

#[test]
fn a_corrupt_newest_snapshot_falls_back_to_the_previous_one() {
    // Losing the newest snapshot costs a longer replay. Refusing to boot costs
    // an outage, so falling back is the correct behaviour.
    let dir = tempfile::tempdir().unwrap();
    let store = SnapshotStore::new(dir.path(), 10);

    store.save(&Snapshot::of(&state_with_deposit(100), "100-0")).unwrap();
    let newest = store.save(&Snapshot::of(&state_with_deposit(200), "200-0")).unwrap();

    fs::write(&newest, b"{ this is not a snapshot").unwrap();

    let loaded = store.load_latest().unwrap().expect("should fall back");
    assert_eq!(loaded.last_stream_id, "100-0");
    assert_eq!(deposited(&loaded.state), 100);
}

#[test]
fn a_directory_of_only_corrupt_snapshots_yields_nothing() {
    // Replaying the whole log is still correct — just slow.
    let dir = tempfile::tempdir().unwrap();
    let store = SnapshotStore::new(dir.path(), 10);

    let path = store.save(&Snapshot::of(&state_with_deposit(1), "1-0")).unwrap();
    fs::write(&path, b"garbage").unwrap();

    assert!(store.load_latest().unwrap().is_none());
}

#[test]
fn a_leftover_temp_file_is_never_loaded() {
    // A crash mid-write leaves a .tmp behind. It must not be mistaken for a
    // finished snapshot, or a truncated state gets loaded as real.
    let dir = tempfile::tempdir().unwrap();
    let store = SnapshotStore::new(dir.path(), 10);

    store.save(&Snapshot::of(&state_with_deposit(50), "50-0")).unwrap();
    fs::write(dir.path().join("999-0.snapshot.tmp"), b"half written").unwrap();

    let loaded = store.load_latest().unwrap().unwrap();
    assert_eq!(loaded.last_stream_id, "50-0");
}

#[test]
fn unrelated_files_in_the_directory_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let store = SnapshotStore::new(dir.path(), 10);

    store.save(&Snapshot::of(&state_with_deposit(1), "1-0")).unwrap();
    fs::write(dir.path().join("README.md"), b"notes").unwrap();
    fs::write(dir.path().join("not-an-id.snapshot"), b"{}").unwrap();

    assert_eq!(store.list().unwrap().len(), 1);
    assert!(store.load_latest().unwrap().is_some());
}

// ───────────────────────── pruning ─────────────────────────

#[test]
fn pruning_keeps_the_newest_n_and_deletes_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let store = SnapshotStore::new(dir.path(), 2);

    for id in ["1-0", "2-0", "9-0", "10-0", "11-0"] {
        store.save(&Snapshot::of(&state_with_deposit(1), id)).unwrap();
    }

    let removed = store.prune().unwrap();
    assert_eq!(removed, 3);

    let kept: Vec<String> = store
        .list()
        .unwrap()
        .into_iter()
        .map(|(id, _)| id.to_string())
        .collect();
    assert_eq!(kept, vec!["11-0", "10-0"], "newest first, numerically");
}

#[test]
fn pruning_a_directory_with_fewer_than_the_limit_removes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = SnapshotStore::new(dir.path(), 5);
    store.save(&Snapshot::of(&state_with_deposit(1), "1-0")).unwrap();

    assert_eq!(store.prune().unwrap(), 0);
    assert_eq!(store.list().unwrap().len(), 1);
}

// ───────────────────────── the recovery contract ─────────────────────────

#[test]
fn a_saved_snapshot_reports_where_replay_should_resume() {
    // The one thing the engine actually asks the store for on boot.
    let dir = tempfile::tempdir().unwrap();
    let store = SnapshotStore::new(dir.path(), 3);

    assert_eq!(store.resume_position().unwrap(), StreamId::ZERO);

    store.save(&Snapshot::of(&state_with_deposit(1), "4242-7")).unwrap();

    assert_eq!(
        store.resume_position().unwrap(),
        StreamId::parse("4242-7").unwrap()
    );
}
