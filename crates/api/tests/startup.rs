//! API boot must not block on history migrations or table locks.

use cex_api::UserStore;
use cex_persist::{HistoryStore, TestResources};
use sqlx::postgres::PgPoolOptions;
use sqlx::AssertSqlSafe;
use std::time::{Duration, Instant};

fn database_url() -> String {
    std::env::var("CEX_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cex:cex@127.0.0.1:5442/cex".into())
}

#[tokio::test]
async fn api_history_connect_skips_migration_while_orders_is_locked() {
    let resources = TestResources::new();
    let url = database_url();
    let schema = &resources.schema;

    // Persist owns the schema; run migrations once so tables exist.
    let _persist = HistoryStore::connect_to_schema(&url, schema)
        .await
        .expect("persist migrations");

    // Hold `orders` locked, as persist does during its own migration.
    let locker = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await
        .expect("locker pool");
    let mut tx = locker.begin().await.expect("begin");
    sqlx::raw_sql(AssertSqlSafe(format!(
        "SET search_path TO {schema}, public"
    )))
    .execute(&mut *tx)
    .await
    .expect("search_path");
    sqlx::query("LOCK TABLE orders IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await
        .expect("lock orders");

    let start = Instant::now();
    let history = HistoryStore::connect_read_only_to_schema(&url, schema)
        .await
        .expect("read-only history connect must not wait on migration locks");
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(2),
        "read-only connect waited {:?} while orders was locked",
        elapsed
    );
}

#[tokio::test]
async fn user_migration_fails_fast_when_users_is_locked() {
    let resources = TestResources::new();
    let url = database_url();
    let schema = &resources.schema;

    let locker = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await
        .expect("locker pool");
    sqlx::raw_sql(AssertSqlSafe(format!(
        "CREATE SCHEMA IF NOT EXISTS {schema}"
    )))
    .execute(&locker)
    .await
    .expect("schema");
    let mut tx = locker.begin().await.expect("begin");
    sqlx::raw_sql(AssertSqlSafe(format!(
        "SET search_path TO {schema}, public"
    )))
    .execute(&mut *tx)
    .await
    .expect("search_path");
    sqlx::query("CREATE TABLE IF NOT EXISTS users (id UUID PRIMARY KEY)")
        .execute(&mut *tx)
        .await
        .expect("stub users");
    sqlx::query("LOCK TABLE users IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await
        .expect("lock users");

    let start = Instant::now();
    let result = UserStore::connect_to_schema(&url, schema).await;
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(10),
        "migration must not hang on lock, waited {:?}",
        elapsed
    );
    let msg = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("migration must fail when users is locked"),
    };
    assert!(
        msg.contains("lock") || msg.contains("timeout") || msg.contains("Timeout"),
        "error should mention the lock, got: {msg}"
    );
}
