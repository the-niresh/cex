//! The users table.
//!
//! One table, and the only one on a request path. The rules it must enforce:
//! usernames are unique case-insensitively, passwords are never stored, and a
//! failed lookup is indistinguishable from a wrong password to the caller.

use cex_api::users::{UserStore, UsersError};
use uuid::Uuid;

fn database_url() -> String {
    std::env::var("CEX_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cex:cex@127.0.0.1:5442/cex".into())
}

/// A store on its own schema, so tests neither collide nor need cleaning up.
async fn store() -> UserStore {
    let schema = format!("t{}", Uuid::new_v4().simple());
    UserStore::connect_to_schema(&database_url(), &schema)
        .await
        .expect("postgres — is `docker compose up -d` running?")
}

fn name() -> String {
    format!("user_{}", Uuid::new_v4().simple())
}

#[tokio::test]
async fn a_registered_user_can_be_found_again() {
    let s = store().await;
    let username = name();

    let created = s.register(&username, "hunter2000").await.unwrap();
    let found = s.find_by_username(&username).await.unwrap().unwrap();

    assert_eq!(found.id, created.id);
    assert_eq!(found.username, username);
}

#[tokio::test]
async fn an_unknown_username_is_not_found() {
    let s = store().await;
    assert!(s.find_by_username("nobody-at-all").await.unwrap().is_none());
}

#[tokio::test]
async fn the_stored_row_holds_a_hash_and_never_the_password() {
    let s = store().await;
    let username = name();
    s.register(&username, "hunter2000").await.unwrap();

    let found = s.find_by_username(&username).await.unwrap().unwrap();
    assert!(found.password_hash.starts_with("$argon2"));
    assert!(!found.password_hash.contains("hunter2000"));
}

#[tokio::test]
async fn a_duplicate_username_is_rejected() {
    let s = store().await;
    let username = name();
    s.register(&username, "password-one").await.unwrap();

    let err = s.register(&username, "password-two").await.unwrap_err();
    assert!(matches!(err, UsersError::UsernameTaken));
}

#[tokio::test]
async fn usernames_collide_regardless_of_case() {
    // Without a case-insensitive unique index, "Alice" and "alice" are two
    // accounts that look like one to every human who sees them.
    let s = store().await;
    let username = name();
    s.register(&username, "password-one").await.unwrap();

    let shouty = username.to_uppercase();
    let err = s.register(&shouty, "password-two").await.unwrap_err();
    assert!(matches!(err, UsersError::UsernameTaken));
}

#[tokio::test]
async fn a_user_can_log_in_with_the_right_password() {
    let s = store().await;
    let username = name();
    let created = s.register(&username, "correct horse").await.unwrap();

    let id = s.authenticate(&username, "correct horse").await.unwrap();
    assert_eq!(id, created.id);
}

#[tokio::test]
async fn logging_in_is_case_insensitive_on_the_username() {
    let s = store().await;
    let username = name();
    s.register(&username, "password-one").await.unwrap();

    assert!(s
        .authenticate(&username.to_uppercase(), "password-one")
        .await
        .is_ok());
}

#[tokio::test]
async fn the_wrong_password_is_refused() {
    let s = store().await;
    let username = name();
    s.register(&username, "right-password").await.unwrap();

    let err = s
        .authenticate(&username, "wrong-password")
        .await
        .unwrap_err();
    assert!(matches!(err, UsersError::BadCredentials));
}

#[tokio::test]
async fn an_unknown_user_and_a_wrong_password_give_the_same_error() {
    // Distinguishing them tells an attacker which usernames exist.
    let s = store().await;
    let username = name();
    s.register(&username, "right-password").await.unwrap();

    let wrong_password = s
        .authenticate(&username, "wrong-password")
        .await
        .unwrap_err();
    let no_such_user = s
        .authenticate("nobody-at-all", "wrong-password")
        .await
        .unwrap_err();

    assert_eq!(
        std::mem::discriminant(&wrong_password),
        std::mem::discriminant(&no_such_user),
        "the two failures are distinguishable"
    );
}

#[tokio::test]
async fn two_users_get_distinct_ids() {
    let s = store().await;
    let a = s.register(&name(), "password-one").await.unwrap();
    let b = s.register(&name(), "password-one").await.unwrap();

    assert_ne!(a.id, b.id);
}

#[tokio::test]
async fn an_empty_username_is_rejected() {
    let s = store().await;
    assert!(matches!(
        s.register("", "password-one").await.unwrap_err(),
        UsersError::InvalidUsername
    ));
    assert!(matches!(
        s.register("   ", "password-one").await.unwrap_err(),
        UsersError::InvalidUsername
    ));
}

#[tokio::test]
async fn a_short_password_is_rejected() {
    let s = store().await;
    assert!(matches!(
        s.register(&name(), "short").await.unwrap_err(),
        UsersError::WeakPassword
    ));
}

#[tokio::test]
async fn migrations_are_safe_to_run_twice() {
    // Every boot runs them; the second boot must be a no-op, not a crash.
    let schema = format!("t{}", Uuid::new_v4().simple());
    let first = UserStore::connect_to_schema(&database_url(), &schema).await;
    assert!(first.is_ok());
    let second = UserStore::connect_to_schema(&database_url(), &schema).await;
    assert!(second.is_ok(), "running migrations again failed");
}
