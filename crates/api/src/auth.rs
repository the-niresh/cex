//! Passwords and tokens.
//!
//! Two different jobs. Argon2 proves *who you are*, once, at login, and is
//! deliberately slow. A JWT proves *you already proved it*, on every later
//! request, and must be fast.
//!
//! The engine never sees either. It receives a `user_id` that this module has
//! already authenticated.
//!
//! Tokens themselves live in [`cex_auth`] and are re-exported here, because
//! `ws` has to verify the same tokens this crate issues and two definitions of
//! a claim format is one too many. Hashing stays here: it is only ever needed
//! next to the users table, and a market-data process should not be linking a
//! password hasher.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

pub use cex_auth::{AuthError, Tokens};

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
