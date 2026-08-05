//! Tokens, shared by every process that has to answer "who is this?".
//!
//! `api` issues them at login. `ws` verifies them before it will attach a
//! private feed to a connection. There is exactly one definition of the claim
//! format and one verification path, because two would be two things to keep in
//! step and one place for them to silently drift apart.
//!
//! ## Why password hashing is not in here
//!
//! Argon2 is memory-hard by design and only `api` ever needs it — a market-data
//! process should not compile a password hasher, let alone link one. Hashing
//! stays next to the users table in `cex-api`; only the part that crosses a
//! process boundary lives here.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The auth domain's error type, shared by both halves of it. `Hash` is
/// produced by the password functions in `cex-api` rather than by anything
/// here; it lives in this enum so both halves report failures the same way.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("could not hash password: {0}")]
    Hash(String),
    #[error("invalid or expired token")]
    BadToken,
    #[error("could not issue token: {0}")]
    Issue(String),
    #[error("system clock is before the unix epoch")]
    Clock,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    /// Subject: the user this token authenticates.
    sub: Uuid,
    /// Expiry, seconds since the unix epoch.
    exp: u64,
    /// Issued at.
    iat: u64,
}

pub struct Tokens {
    encoding: EncodingKey,
    decoding: DecodingKey,
    ttl: Duration,
}

impl Tokens {
    pub fn new(secret: &[u8], ttl: Duration) -> Self {
        Tokens {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
            ttl,
        }
    }

    /// Issue a token valid for the configured lifetime.
    pub fn issue(&self, user_id: Uuid) -> Result<String, AuthError> {
        self.issue_expiring_at(user_id, now()? + self.ttl.as_secs())
    }

    /// Issue a token with an explicit expiry. Exposed so tests can produce an
    /// already-expired token without waiting or stubbing the clock.
    pub fn issue_expiring_at(&self, user_id: Uuid, exp: u64) -> Result<String, AuthError> {
        let claims = Claims {
            sub: user_id,
            exp,
            iat: now()?,
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .map_err(|e| AuthError::Issue(e.to_string()))
    }

    /// Verify a token and return the user it authenticates.
    pub fn verify(&self, token: &str) -> Result<Uuid, AuthError> {
        // Pinning the algorithm is what rejects the `alg: "none"` forgery, where
        // an attacker drops the signature and claims none was required.
        let mut validation = Validation::new(Algorithm::HS256);
        validation.leeway = 0;
        validation.validate_exp = true;

        decode::<Claims>(token, &self.decoding, &validation)
            .map(|data| data.claims.sub)
            .map_err(|_| AuthError::BadToken)
    }
}

fn now() -> Result<u64, AuthError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| AuthError::Clock)
}
