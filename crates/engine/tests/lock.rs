//! The engine boot lock.
//!
//! Design rule 6 says exactly one engine per command stream. Until now nothing
//! enforced it: a second instance would read the same commands with plain
//! `XREAD` and apply every one of them a second time. That is the only failure
//! mode left in this system that corrupts state rather than degrading service,
//! and it has actually happened — two engines started by accident printed two
//! trades where there should have been one.
//!
//! These need Redis up (`docker compose up -d`).

use cex_engine::config::Config;
use cex_engine::lock::{lock_key, EngineLock, LockError};
use cex_engine::runner::Runner;
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use std::time::Duration;
use uuid::Uuid;

fn redis_url() -> String {
    std::env::var("CEX_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into())
}

async fn conn() -> MultiplexedConnection {
    redis::Client::open(redis_url())
        .expect("redis client")
        .get_multiplexed_async_connection()
        .await
        .expect("redis — is `docker compose up -d` running?")
}

/// A stream name nothing else is using.
fn stream() -> String {
    format!("test:{}:commands", Uuid::new_v4().simple())
}

async fn acquire(stream: &str, ttl: Duration) -> Result<EngineLock, LockError> {
    EngineLock::acquire(conn().await, stream, ttl).await
}

const TTL: Duration = Duration::from_secs(30);

// ───────────────────────── acquiring ─────────────────────────

#[tokio::test]
async fn a_free_stream_can_be_locked() {
    let s = stream();
    let lock = acquire(&s, TTL).await.expect("a free stream");
    assert_eq!(lock.key(), lock_key(&s));
    assert!(!lock.id().is_empty());
}

#[tokio::test]
async fn a_second_engine_is_refused_while_the_first_holds_it() {
    let s = stream();
    let first = acquire(&s, TTL).await.expect("the first engine");

    match acquire(&s, TTL).await {
        Err(LockError::Held { holder, .. }) => assert_eq!(
            holder,
            first.id(),
            "the refusal must name the engine actually holding it, so whoever \
             is looking at the logs knows which process to go and find"
        ),
        other => panic!("the second engine was allowed to start: {other:?}"),
    }
}

#[tokio::test]
async fn engines_on_different_command_streams_do_not_collide() {
    // The lock guards one command stream, not the whole exchange. Two engines
    // on two streams is a legitimate deployment, not a mistake.
    let a = acquire(&stream(), TTL).await.expect("first stream");
    let b = acquire(&stream(), TTL).await.expect("second stream");
    assert_ne!(a.key(), b.key());
}

#[tokio::test]
async fn a_released_lock_can_be_taken_by_the_next_engine() {
    let s = stream();
    let mut first = acquire(&s, TTL).await.unwrap();
    assert!(first.release().await.unwrap(), "we held it, so we released it");

    acquire(&s, TTL).await.expect("the stream is free again");
}

#[tokio::test]
async fn a_lock_left_by_a_dead_engine_expires_and_the_next_one_boots() {
    // Nothing releases the lock when an engine is killed, so the lease is what
    // stops one crash from locking the exchange out permanently.
    let s = stream();
    let dead = acquire(&s, Duration::from_millis(300)).await.unwrap();
    drop(dead);

    assert!(
        acquire(&s, TTL).await.is_err(),
        "the lease has not expired yet, so the stream is still guarded"
    );

    tokio::time::sleep(Duration::from_millis(500)).await;
    acquire(&s, TTL).await.expect("the lease expired");
}

// ───────────────────────── holding on to it ─────────────────────────

#[tokio::test]
async fn refreshing_extends_the_hold() {
    let s = stream();
    let mut lock = acquire(&s, Duration::from_millis(600)).await.unwrap();

    // Past the original expiry, but refreshed along the way.
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        lock.refresh().await.expect("still ours");
    }

    assert!(
        acquire(&s, TTL).await.is_err(),
        "the lock lapsed while its owner was still running"
    );
}

#[tokio::test]
async fn refresh_is_only_due_once_a_third_of_the_lease_has_gone() {
    let s = stream();
    let mut lock = acquire(&s, Duration::from_millis(900)).await.unwrap();

    assert!(!lock.refresh_if_due().await.unwrap(), "nothing has elapsed yet");
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(lock.refresh_if_due().await.unwrap(), "a third of the lease has gone");
}

/// The one that matters most.
#[tokio::test]
async fn refresh_fails_rather_than_extending_a_lock_someone_else_now_holds() {
    let s = stream();
    let mut lock = acquire(&s, Duration::from_millis(300)).await.unwrap();

    // Our lease lapsed and another engine took the stream — the exact situation
    // a paused or descheduled process wakes up into.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let usurper = acquire(&s, TTL).await.expect("the stream was free");

    match lock.refresh().await {
        Err(LockError::Lost(_)) => {}
        other => panic!("blindly extended someone else's lock: {other:?}"),
    }

    // And the new owner's hold is untouched by our attempt.
    let holder: Option<String> = conn().await.get(lock_key(&s)).await.unwrap();
    assert_eq!(holder.as_deref(), Some(usurper.id()));
}

#[tokio::test]
async fn releasing_does_not_delete_a_lock_someone_else_now_holds() {
    let s = stream();
    let mut lock = acquire(&s, Duration::from_millis(300)).await.unwrap();

    tokio::time::sleep(Duration::from_millis(400)).await;
    let usurper = acquire(&s, TTL).await.expect("the stream was free");

    assert!(
        !lock.release().await.unwrap(),
        "a stale owner reported releasing a lock it did not hold"
    );

    // Unlocking the exchange out from under the engine that legitimately owns
    // it would be worse than never releasing at all.
    let holder: Option<String> = conn().await.get(lock_key(&s)).await.unwrap();
    assert_eq!(
        holder.as_deref(),
        Some(usurper.id()),
        "the stale owner unlocked the running engine's stream"
    );
}

#[tokio::test]
async fn every_engine_gets_a_distinct_identity() {
    let a = acquire(&stream(), TTL).await.unwrap();
    let b = acquire(&stream(), TTL).await.unwrap();
    assert_ne!(
        a.id(),
        b.id(),
        "two engines sharing an id could release each other's locks"
    );
}

// ───────────────────────── the engine itself ─────────────────────────

fn test_config(dir: &std::path::Path, tag: &str) -> Config {
    Config {
        redis_url: redis_url(),
        commands_stream: format!("test:{tag}:commands"),
        events_stream: format!("test:{tag}:events"),
        responses_channel: format!("test:{tag}:responses"),
        queries_queue: format!("test:{tag}:queries"),
        snapshot_dir: dir.to_path_buf(),
        snapshot_every: 1_000_000,
        snapshot_keep: 3,
        block_ms: 50,
        lock_ttl_ms: 900,
    }
}

#[tokio::test]
async fn a_second_engine_refuses_to_boot_on_the_same_command_stream() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path(), &Uuid::new_v4().simple().to_string());

    let _first = Runner::boot(cfg.clone()).await.expect("the first engine");

    let second = Runner::boot(cfg.clone()).await;
    assert!(
        second.is_err(),
        "a second engine booted on a stream that was already owned; it would \
         have applied every command a second time"
    );
    let message = second.err().unwrap().to_string();
    assert!(
        message.contains(&cfg.commands_stream),
        "the error should name the contested stream, got: {message}"
    );
}

#[tokio::test]
async fn engines_on_separate_streams_both_boot() {
    let dir = tempfile::tempdir().unwrap();
    let a = test_config(dir.path(), &Uuid::new_v4().simple().to_string());
    let b = test_config(dir.path(), &Uuid::new_v4().simple().to_string());

    let _a = Runner::boot(a).await.expect("first engine");
    let _b = Runner::boot(b).await.expect("second engine");
}

#[tokio::test]
async fn an_engine_that_loses_its_lock_stops_instead_of_carrying_on() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path(), &Uuid::new_v4().simple().to_string());
    let mut runner = Runner::boot(cfg.clone()).await.expect("engine boot");

    // Another engine has taken the stream. Continuing to apply commands now
    // would mean two engines writing the same state — the failure this whole
    // mechanism exists to prevent. Stopping is the only safe response.
    let _: () = conn()
        .await
        .set(lock_key(&cfg.commands_stream), "some-other-engine")
        .await
        .unwrap();

    let outcome = tokio::time::timeout(Duration::from_secs(5), runner.run()).await;
    match outcome {
        Ok(Err(e)) => assert!(
            e.to_string().contains("lost"),
            "stopped, but not for the right reason: {e}"
        ),
        Ok(Ok(())) => panic!("the engine returned successfully instead of failing"),
        Err(_) => panic!("the engine carried on running without its lock"),
    }
}

#[tokio::test]
async fn a_blocking_read_longer_than_the_lease_is_refused_at_boot() {
    // If the loop can sit in `XREAD` for longer than the refresh interval, the
    // lease lapses under a perfectly healthy engine and it stops for no reason.
    // That is a misconfiguration, and it is knowable at startup.
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = test_config(dir.path(), &Uuid::new_v4().simple().to_string());
    cfg.block_ms = 5_000;
    cfg.lock_ttl_ms = 900;

    let booted = Runner::boot(cfg).await;
    assert!(booted.is_err(), "a self-defeating configuration was accepted");
}

#[tokio::test]
async fn a_gracefully_stopped_engine_hands_the_stream_straight_over() {
    // The deploy path. Waiting out a 30-second lease on every restart would
    // make an ordinary rolling deploy an outage, so a clean stop has to release.
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path(), &Uuid::new_v4().simple().to_string());

    let mut leaving = Runner::boot(cfg.clone()).await.expect("engine boot");
    assert!(leaving.shutdown().await.unwrap(), "we still held it");

    Runner::boot(cfg).await.expect("the replacement, with no wait at all");
}

#[tokio::test]
async fn releasing_after_the_lock_was_already_lost_takes_nothing_from_the_new_owner() {
    // The engine calls `shutdown` on its way out even when it stopped *because*
    // it lost the lock. That must not unlock the stream under whoever has it.
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path(), &Uuid::new_v4().simple().to_string());
    let mut runner = Runner::boot(cfg.clone()).await.expect("engine boot");

    let key = lock_key(&cfg.commands_stream);
    let _: () = conn().await.set(&key, "the-new-engine").await.unwrap();

    assert!(!runner.shutdown().await.unwrap(), "we no longer held it");
    let holder: Option<String> = conn().await.get(&key).await.unwrap();
    assert_eq!(
        holder.as_deref(),
        Some("the-new-engine"),
        "a departing engine unlocked the stream under its replacement"
    );
}

#[tokio::test]
async fn a_restarted_engine_reclaims_its_own_stream_once_the_lease_lapses() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path(), &Uuid::new_v4().simple().to_string());

    let killed = Runner::boot(cfg.clone()).await.expect("engine boot");
    drop(killed); // No graceful release: this is a kill -9.

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    Runner::boot(cfg).await.expect("the replacement engine");
}
