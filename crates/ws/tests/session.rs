//! Subscriptions and identity, without a socket in sight.
//!
//! `Session::wants` is the single decision point for whether a connection sees
//! an update. Every leak this crate could have goes through it, so it is worth
//! testing directly rather than only through a WebSocket.

use cex_auth::Tokens;
use cex_proto::Seq;
use cex_ws::route::Update;
use cex_ws::wire::Channel;
use cex_ws::{Session, SessionError};
use std::time::Duration;
use uuid::Uuid;

const SYM: &str = "BTC_USDT";

fn tokens() -> Tokens {
    Tokens::new(
        b"a test secret that is long enough",
        Duration::from_secs(3600),
    )
}

fn update(channel: Channel, audience: Option<Uuid>, seq: Seq) -> Update {
    Update {
        channel,
        audience,
        seq,
        payload: "{}".into(),
    }
}

// ───────────────────────── subscribing ─────────────────────────

#[tokio::test]
async fn a_new_session_is_subscribed_to_nothing() {
    let s = Session::new();
    assert!(s.subscriptions().is_empty());
    assert_eq!(s.user(), None);
    assert!(!s.wants(&update(Channel::Depth(SYM.into()), None, 1)));
}

#[tokio::test]
async fn a_subscriber_receives_only_what_it_subscribed_to() {
    let mut s = Session::new();
    s.subscribe(Channel::Depth(SYM.into())).unwrap();

    assert!(s.wants(&update(Channel::Depth(SYM.into()), None, 1)));
    assert!(!s.wants(&update(Channel::Trades(SYM.into()), None, 1)));
    assert!(!s.wants(&update(Channel::Orders, Some(Uuid::new_v4()), 1)));
}

#[tokio::test]
async fn a_subscription_is_per_symbol() {
    let mut s = Session::new();
    s.subscribe(Channel::Trades(SYM.into())).unwrap();

    assert!(s.wants(&update(Channel::Trades(SYM.into()), None, 1)));
    assert!(
        !s.wants(&update(Channel::Trades("ETH_USDT".into()), None, 1)),
        "subscribing to one market must not deliver another"
    );
}

#[tokio::test]
async fn unsubscribing_stops_delivery() {
    let mut s = Session::new();
    s.subscribe(Channel::Depth(SYM.into())).unwrap();
    s.unsubscribe(&Channel::Depth(SYM.into()));

    assert!(!s.wants(&update(Channel::Depth(SYM.into()), None, 1)));
    assert!(s.subscriptions().is_empty());
}

#[tokio::test]
async fn subscribing_twice_is_not_an_error_and_does_not_duplicate() {
    let mut s = Session::new();
    s.subscribe(Channel::Depth(SYM.into())).unwrap();
    s.subscribe(Channel::Depth(SYM.into())).unwrap();
    assert_eq!(s.subscriptions().len(), 1);
}

#[tokio::test]
async fn unsubscribing_from_something_never_subscribed_is_harmless() {
    let mut s = Session::new();
    s.unsubscribe(&Channel::Depth(SYM.into()));
    assert!(s.subscriptions().is_empty());
}

// ───────────────────────── the private channel ─────────────────────────

#[tokio::test]
async fn the_private_channel_cannot_be_subscribed_without_authenticating() {
    let mut s = Session::new();
    let err = s.subscribe(Channel::Orders).unwrap_err();

    assert!(matches!(err, SessionError::AuthRequired(_)));
    assert!(
        s.subscriptions().is_empty(),
        "a refused subscription must not be recorded"
    );
}

#[tokio::test]
async fn authenticating_allows_the_private_channel() {
    let t = tokens();
    let alice = Uuid::new_v4();
    let mut s = Session::new();

    assert_eq!(s.authenticate(&t, &t.issue(alice).unwrap()).unwrap(), alice);
    s.subscribe(Channel::Orders).unwrap();
    assert!(s.wants(&update(Channel::Orders, Some(alice), 1)));
}

#[tokio::test]
async fn a_forged_token_authenticates_nobody() {
    let attacker = Tokens::new(b"not the servers secret at all", Duration::from_secs(3600));
    let forged = attacker.issue(Uuid::new_v4()).unwrap();

    let mut s = Session::new();
    assert!(matches!(
        s.authenticate(&tokens(), &forged).unwrap_err(),
        SessionError::BadToken
    ));
    assert_eq!(s.user(), None);
    assert!(s.subscribe(Channel::Orders).is_err());
}

#[tokio::test]
async fn an_expired_token_authenticates_nobody() {
    let t = tokens();
    let expired = t.issue_expiring_at(Uuid::new_v4(), 1_000).unwrap();

    let mut s = Session::new();
    assert!(matches!(
        s.authenticate(&t, &expired).unwrap_err(),
        SessionError::BadToken
    ));
    assert_eq!(s.user(), None);
}

/// The rule this whole crate is riskiest around.
#[tokio::test]
async fn the_private_feed_never_delivers_another_users_update() {
    let t = tokens();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    let mut s = Session::new();
    s.authenticate(&t, &t.issue(alice).unwrap()).unwrap();
    s.subscribe(Channel::Orders).unwrap();

    assert!(s.wants(&update(Channel::Orders, Some(alice), 1)));
    assert!(
        !s.wants(&update(Channel::Orders, Some(bob), 1)),
        "alice was offered bob's private update"
    );
}

#[tokio::test]
async fn an_unauthenticated_session_is_offered_no_private_update_even_if_subscribed() {
    // Belt and braces: `subscribe` already refuses, but `wants` must refuse too,
    // so a future path that populates subscriptions some other way cannot leak.
    let mut s = Session::new();
    s.subscribe(Channel::Depth(SYM.into())).unwrap();

    assert!(!s.wants(&update(Channel::Orders, Some(Uuid::new_v4()), 1)));
}

#[tokio::test]
async fn a_public_update_reaches_an_authenticated_session_too() {
    let t = tokens();
    let mut s = Session::new();
    s.authenticate(&t, &t.issue(Uuid::new_v4()).unwrap())
        .unwrap();
    s.subscribe(Channel::Trades(SYM.into())).unwrap();

    assert!(s.wants(&update(Channel::Trades(SYM.into()), None, 1)));
}

#[tokio::test]
async fn re_authenticating_as_someone_else_switches_the_private_feed() {
    let t = tokens();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    let mut s = Session::new();
    s.authenticate(&t, &t.issue(alice).unwrap()).unwrap();
    s.subscribe(Channel::Orders).unwrap();
    s.authenticate(&t, &t.issue(bob).unwrap()).unwrap();

    assert!(
        !s.wants(&update(Channel::Orders, Some(alice), 1)),
        "the previous identity must stop receiving immediately"
    );
    assert!(s.wants(&update(Channel::Orders, Some(bob), 1)));
}

#[tokio::test]
async fn a_failed_re_authentication_does_not_keep_the_old_identity() {
    let t = tokens();
    let alice = Uuid::new_v4();
    let attacker = Tokens::new(b"wrong secret entirely, truly", Duration::from_secs(3600));

    let mut s = Session::new();
    s.authenticate(&t, &t.issue(alice).unwrap()).unwrap();
    s.subscribe(Channel::Orders).unwrap();

    let forged = attacker.issue(Uuid::new_v4()).unwrap();
    assert!(s.authenticate(&t, &forged).is_err());

    // Rejecting a bad token must drop the connection's identity rather than
    // leave the previous one attached — otherwise a failed takeover attempt
    // quietly leaves the attacker on someone else's feed.
    assert_eq!(s.user(), None);
    assert!(!s.wants(&update(Channel::Orders, Some(alice), 1)));
}
