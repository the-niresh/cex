//! The users table.
//!
//! The only place the API reads Postgres on a request path. Everything else the
//! database holds is history, written behind the engine by the persister.
//!
//! Note the shape of the schema handling below. sqlx 0.9 refuses to build a
//! query from a runtime-constructed string unless you wrap it in
//! [`AssertSqlSafe`], which is a compile-time nudge to audit for injection.
//! Only one statement here needs it — `CREATE SCHEMA`, because SQL cannot
//! parameterise an identifier — and the name is validated first. Everything
//! else is a static string with bound parameters.

use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgRow};
use sqlx::{AssertSqlSafe, PgPool, Row};
use std::str::FromStr;
use uuid::Uuid;

use crate::auth::{hash_password, verify_password};

/// Minimum accepted password length. Deliberately modest — length is the only
/// composition rule worth enforcing, and complexity rules push people toward
/// predictable substitutions that are easier to guess, not harder.
const MIN_PASSWORD_LEN: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum UsersError {
    #[error("username is already taken")]
    UsernameTaken,
    #[error("username cannot be empty")]
    InvalidUsername,
    #[error("password must be at least {MIN_PASSWORD_LEN} characters")]
    WeakPassword,
    /// Deliberately one variant for both "no such user" and "wrong password".
    /// Two distinct errors would tell an attacker which usernames exist.
    #[error("invalid username or password")]
    BadCredentials,
    #[error("database: {0}")]
    Db(String),
    #[error("could not hash password: {0}")]
    Hash(String),
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
}

fn row_to_user(row: PgRow) -> User {
    User {
        id: row.get("id"),
        username: row.get("username"),
        password_hash: row.get("password_hash"),
    }
}

const SCHEMA_SQL: &str = include_str!("../migrations/0001_users.sql");

#[derive(Clone)]
pub struct UserStore {
    pool: PgPool,
}

impl UserStore {
    /// Connect and bring the schema up to date.
    pub async fn connect(database_url: &str) -> Result<Self, UsersError> {
        Self::connect_to_schema(database_url, "public").await
    }

    /// Connect against a named schema. Tests use a fresh schema per run so they
    /// neither collide nor need tearing down.
    pub async fn connect_to_schema(database_url: &str, schema: &str) -> Result<Self, UsersError> {
        // Audited: the identifier cannot contain a quote, a semicolon, or
        // whitespace, so it cannot terminate the statement it is spliced into.
        if schema.is_empty()
            || !schema
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(UsersError::Db(format!("unsafe schema name {schema:?}")));
        }

        // The search path is a *connection parameter*, not SQL — so it applies
        // to every pooled connection without a statement, and cannot be injected.
        let opts = PgConnectOptions::from_str(database_url)
            .map_err(|e| UsersError::Db(e.to_string()))?
            .options([("search_path", format!("{schema},public"))]);

        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .map_err(|e| UsersError::Db(e.to_string()))?;

        sqlx::raw_sql(AssertSqlSafe(format!(
            "CREATE SCHEMA IF NOT EXISTS {schema}"
        )))
        .execute(&pool)
        .await
        .map_err(|e| UsersError::Db(e.to_string()))?;

        // Static SQL, unqualified names — the search path puts them in place.
        // `IF NOT EXISTS` throughout, so every boot can safely run it.
        sqlx::raw_sql(SCHEMA_SQL)
            .execute(&pool)
            .await
            .map_err(|e| UsersError::Db(e.to_string()))?;

        Ok(UserStore { pool })
    }

    pub async fn register(&self, username: &str, password: &str) -> Result<User, UsersError> {
        let username = username.trim();
        if username.is_empty() {
            return Err(UsersError::InvalidUsername);
        }
        if password.len() < MIN_PASSWORD_LEN {
            return Err(UsersError::WeakPassword);
        }

        let hash = hash_password(password).map_err(|e| UsersError::Hash(e.to_string()))?;
        let id = Uuid::new_v4();

        let result =
            sqlx::query("INSERT INTO users (id, username, password_hash) VALUES ($1, $2, $3)")
                .bind(id)
                .bind(username)
                .bind(&hash)
                .execute(&self.pool)
                .await;

        match result {
            Ok(_) => Ok(User {
                id,
                username: username.to_string(),
                password_hash: hash,
            }),
            Err(e) => {
                // 23505 is unique_violation — the case-insensitive index fired.
                if e.as_database_error().and_then(|db| db.code()).as_deref() == Some("23505") {
                    return Err(UsersError::UsernameTaken);
                }
                Err(UsersError::Db(e.to_string()))
            }
        }
    }

    pub async fn find_by_username(&self, username: &str) -> Result<Option<User>, UsersError> {
        sqlx::query(
            "SELECT id, username, password_hash FROM users WHERE lower(username) = lower($1)",
        )
        .bind(username.trim())
        .fetch_optional(&self.pool)
        .await
        .map(|opt| opt.map(row_to_user))
        .map_err(|e| UsersError::Db(e.to_string()))
    }

    /// Check credentials and return the authenticated user's id.
    pub async fn authenticate(&self, username: &str, password: &str) -> Result<Uuid, UsersError> {
        let Some(user) = self.find_by_username(username).await? else {
            return Err(UsersError::BadCredentials);
        };
        if !verify_password(password, &user.password_hash) {
            return Err(UsersError::BadCredentials);
        }
        Ok(user.id)
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}
