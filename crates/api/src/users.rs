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
use std::time::Duration;
use uuid::Uuid;

use crate::auth::{hash_password, verify_password};

/// Minimum accepted password length. Deliberately modest — length is the only
/// composition rule worth enforcing, and complexity rules push people toward
/// predictable substitutions that are easier to guess, not harder.
const MIN_PASSWORD_LEN: usize = 8;

/// Upper bound on a display name. It is only ever shown, never matched on, so
/// the limit exists to bound what one row can cost rather than to shape input.
const MAX_DISPLAY_NAME_LEN: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum UsersError {
    #[error("username is already taken")]
    UsernameTaken,
    #[error("username cannot be empty")]
    InvalidUsername,
    #[error("name cannot be blank")]
    BlankDisplayName,
    #[error("name must be at most {MAX_DISPLAY_NAME_LEN} characters")]
    DisplayNameTooLong,
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
    /// What to call this person on screen. `None` for accounts registered
    /// before the column existed — a name is a label, not a requirement.
    pub display_name: Option<String>,
    pub password_hash: String,
}

/// One registration's worth of input.
///
/// A struct rather than three `&str` parameters: `username` and `display_name`
/// are both free text of the same type, and positional arguments that can be
/// swapped without the compiler noticing are how someone ends up logging in
/// with their own display name.
#[derive(Debug, Clone, Copy)]
pub struct NewUser<'a> {
    pub username: &'a str,
    pub display_name: Option<&'a str>,
    pub password: &'a str,
}

fn row_to_user(row: PgRow) -> User {
    User {
        id: row.get("id"),
        username: row.get("username"),
        display_name: row.get("display_name"),
        password_hash: row.get("password_hash"),
    }
}

const SCHEMA_SQL: &str = include_str!("../migrations/0001_users.sql");
const DISPLAY_NAME_SQL: &str = include_str!("../migrations/0002_user_display_name.sql");

/// How long a caller waits for a pooled connection before being told no. Same
/// reasoning as the history pool — see `ACQUIRE_TIMEOUT` in `cex-persist`.
///
/// A sign-in that fails in five seconds can be retried. One that hangs for
/// sqlx's default thirty looks like a broken site.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

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
            .acquire_timeout(ACQUIRE_TIMEOUT)
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
        // `IF NOT EXISTS` throughout, so every boot can safely run it. Later
        // migrations run in order after it, and are equally re-runnable.
        for statements in [SCHEMA_SQL, DISPLAY_NAME_SQL] {
            sqlx::raw_sql(statements)
                .execute(&pool)
                .await
                .map_err(|e| UsersError::Db(e.to_string()))?;
        }

        Ok(UserStore { pool })
    }

    pub async fn register(&self, new: NewUser<'_>) -> Result<User, UsersError> {
        let username = new.username.trim();
        if username.is_empty() {
            return Err(UsersError::InvalidUsername);
        }
        let display_name = validate_display_name(new.display_name)?;
        if new.password.len() < MIN_PASSWORD_LEN {
            return Err(UsersError::WeakPassword);
        }

        let hash = hash_password(new.password).map_err(|e| UsersError::Hash(e.to_string()))?;
        let id = Uuid::new_v4();

        let result = sqlx::query(
            "INSERT INTO users (id, username, display_name, password_hash) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(username)
        .bind(display_name.as_deref())
        .bind(&hash)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(User {
                id,
                username: username.to_string(),
                display_name,
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
            "SELECT id, username, display_name, password_hash FROM users \
             WHERE lower(username) = lower($1)",
        )
        .bind(username.trim())
        .fetch_optional(&self.pool)
        .await
        .map(|opt| opt.map(row_to_user))
        .map_err(|e| UsersError::Db(e.to_string()))
    }

    /// Check credentials and return the user they belong to.
    pub async fn authenticate(&self, username: &str, password: &str) -> Result<User, UsersError> {
        let Some(user) = self.find_by_username(username).await? else {
            return Err(UsersError::BadCredentials);
        };
        if !verify_password(password, &user.password_hash) {
            return Err(UsersError::BadCredentials);
        }
        Ok(user)
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

/// Trim a supplied name, refusing one that is blank or absurdly long.
///
/// Absent and blank are deliberately *not* the same thing. Absent is an older
/// client that does not know about names; blank is a client that asked for one
/// and sent whitespace, which is a mistake worth reporting rather than storing.
fn validate_display_name(supplied: Option<&str>) -> Result<Option<String>, UsersError> {
    let Some(raw) = supplied else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(UsersError::BlankDisplayName);
    }
    if trimmed.chars().count() > MAX_DISPLAY_NAME_LEN {
        return Err(UsersError::DisplayNameTooLong);
    }
    Ok(Some(trimmed.to_string()))
}
