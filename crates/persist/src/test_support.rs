//! Helpers for integration tests. Not used in production binaries.

use std::sync::Arc;

use redis::AsyncCommands;
use sqlx::postgres::PgConnectOptions;
use sqlx::{AssertSqlSafe, PgPool};
use std::str::FromStr;
use uuid::Uuid;

/// Refuse to run against a remote database such as Neon.
pub fn require_safe_database_url() {
    let url = std::env::var("CEX_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cex:cex@127.0.0.1:5442/cex".into());
    if let Err(msg) = assert_safe_database_url(&url) {
        panic!("{msg}");
    }
}

pub fn assert_safe_database_url(url: &str) -> Result<(), String> {
    let host = db_host(url).ok_or_else(|| "CEX_DATABASE_URL has no host".to_string())?;

    if host == "localhost" || host == "127.0.0.1" || host == "[::1]" {
        return Ok(());
    }

    // Docker Compose service names have no dots.
    if !host.contains('.') {
        return Ok(());
    }

    Err(format!(
        "CEX_DATABASE_URL points at {host}, which is not localhost or a docker service name. \
         Tests must not touch Neon or other remote databases."
    ))
}

/// Drops a throwaway Postgres schema when the test finishes, pass or fail.
pub struct SchemaGuard {
    database_url: String,
    schema: String,
}

impl SchemaGuard {
    pub fn new(database_url: &str, schema: &str) -> Self {
        require_safe_database_url();
        SchemaGuard {
            database_url: database_url.to_string(),
            schema: schema.to_string(),
        }
    }
}

impl Drop for SchemaGuard {
    fn drop(&mut self) {
        let url = self.database_url.clone();
        let schema = self.schema.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            if let Ok(rt) = rt {
                let _ = rt.block_on(drop_schema(&url, &schema));
            }
        });
    }
}

/// Age every batch row for retention tests.
pub async fn age_all_batches(
    database_url: &str,
    schema: &str,
    days: i32,
) -> Result<(), sqlx::Error> {
    let opts = PgConnectOptions::from_str(database_url)?
        .options([("search_path", format!("{schema},public"))]);
    let pool = PgPool::connect_with(opts).await?;
    sqlx::query("UPDATE event_batches SET written_at = now() - make_interval(days => $1)")
        .bind(days)
        .execute(&pool)
        .await?;
    pool.close().await;
    Ok(())
}

async fn drop_schema(database_url: &str, schema: &str) -> Result<(), sqlx::Error> {
    if schema.is_empty()
        || !schema
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Ok(());
    }
    let opts = PgConnectOptions::from_str(database_url)?;
    let pool = PgPool::connect_with(opts).await?;
    sqlx::raw_sql(AssertSqlSafe(format!(
        "DROP SCHEMA IF EXISTS {schema} CASCADE"
    )))
    .execute(&pool)
    .await?;
    pool.close().await;
    Ok(())
}

/// Deletes `test:{tag}:*` Redis keys when the test finishes, pass or fail.
pub struct RedisTagGuard {
    redis_url: String,
    tag: Arc<String>,
}

impl RedisTagGuard {
    pub fn new(redis_url: &str, tag: &str) -> Self {
        RedisTagGuard {
            redis_url: redis_url.to_string(),
            tag: Arc::new(tag.to_string()),
        }
    }
}

impl Drop for RedisTagGuard {
    fn drop(&mut self) {
        let url = self.redis_url.clone();
        let tag = Arc::clone(&self.tag);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            if let Ok(rt) = rt {
                let _ = rt.block_on(delete_test_keys(&url, &tag));
            }
        });
    }
}

fn db_host(url: &str) -> Option<&str> {
    let after_scheme = url.split("://").nth(1)?;
    let authority = after_scheme.split('/').next()?;
    let host_port = authority.rsplit('@').next()?;
    host_port.split(':').next()
}

/// Throwaway Redis and Postgres resources for one integration test.
pub struct TestResources {
    pub tag: String,
    pub schema: String,
    _redis: RedisTagGuard,
    _schema: SchemaGuard,
}

impl Default for TestResources {
    fn default() -> Self {
        Self::new()
    }
}

impl TestResources {
    pub fn new() -> Self {
        require_safe_database_url();
        let tag = Uuid::new_v4().simple().to_string();
        let schema = format!("t{tag}");
        let redis_url =
            std::env::var("CEX_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
        let database_url = std::env::var("CEX_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://cex:cex@127.0.0.1:5442/cex".into());
        let _redis = RedisTagGuard::new(&redis_url, &tag);
        let _schema = SchemaGuard::new(&database_url, &schema);
        TestResources {
            tag,
            schema,
            _redis,
            _schema,
        }
    }
}

async fn delete_test_keys(redis_url: &str, tag: &str) -> Result<(), redis::RedisError> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let pattern = format!("test:{tag}:*");
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg(&pattern)
        .query_async(&mut conn)
        .await?;
    for key in keys {
        let _: i64 = conn.del(key).await?;
    }
    Ok(())
}
