//! Passwords and tokens.
//!
//! Two different jobs. Argon2 proves *who you are*, once, at login, and is
//! deliberately slow. A JWT proves *you already proved it*, on every later
//! request, and must be fast.
//!
//! The engine never sees either. It receives a `user_id` that this module has
//! already authenticated.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

// ───────────────────────── passwords ─────────────────────────

/// Hash a password for storage.
///
/// The salt is generated per call and embedded in the returned string, so two
/// users with the same password get different hashes and one precomputed table
/// cannot crack both.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AuthError::Hash(e.to_string()))
}

/// Check a password against a stored hash.
///
/// Returns `false` for anything that is not a match, including a corrupt or
/// unparseable stored hash. Bad data in the users table logs someone out; it
/// must not take the process down.
pub fn verify_password(password: &str, stored: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

// ───────────────────────── tokens ─────────────────────────

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
