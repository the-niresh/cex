//! Password hashing and tokens.
//!
//! Two different jobs that are easy to conflate:
//!
//! * **Argon2** proves *who you are*, once, at login. Deliberately slow.
//! * **JWT** proves *you already proved it*, on every later request. Fast.
//!
//! Both are places where a mistake is silent — a hash that always verifies, or a
//! token whose signature is not actually checked, looks exactly like working
//! code until someone notices they can log in as anyone.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cex_api::auth::{hash_password, verify_password, Tokens};
use uuid::Uuid;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// ───────────────────────── password hashing ─────────────────────────

#[test]
fn a_password_verifies_against_its_own_hash() {
    let hash = hash_password("correct horse battery staple").unwrap();
    assert!(verify_password("correct horse battery staple", &hash));
}

#[test]
fn a_wrong_password_does_not_verify() {
    let hash = hash_password("correct horse battery staple").unwrap();

    assert!(!verify_password("Correct horse battery staple", &hash), "case");
    assert!(!verify_password("correct horse battery stapl", &hash), "truncated");
    assert!(!verify_password("", &hash), "empty");
    assert!(!verify_password("something else entirely", &hash), "different");
}

#[test]
fn the_stored_hash_does_not_contain_the_password() {
    let hash = hash_password("hunter2").unwrap();
    assert!(
        !hash.contains("hunter2"),
        "the password leaked into the stored hash"
    );
}

#[test]
fn the_same_password_hashes_differently_every_time() {
    // Per-user salt. Without it, two people with the same password share a hash,
    // and one precomputed table cracks both.
    let a = hash_password("same password").unwrap();
    let b = hash_password("same password").unwrap();

    assert_ne!(a, b, "hashes are not salted");
    assert!(verify_password("same password", &a));
    assert!(verify_password("same password", &b));
}

#[test]
fn the_hash_records_the_algorithm_that_produced_it() {
    // Self-describing, so the parameters can be raised later without
    // invalidating every existing hash.
    let hash = hash_password("whatever").unwrap();
    assert!(hash.starts_with("$argon2"), "unexpected hash format: {hash}");
}

#[test]
fn a_malformed_stored_hash_fails_verification_rather_than_panicking() {
    // Corrupt data in the users table must log a user out, not crash the process.
    for junk in ["", "not-a-hash", "$argon2id$broken", "$2y$10$bcryptshaped"] {
        assert!(
            !verify_password("anything", junk),
            "junk hash {junk:?} should not verify"
        );
    }
}

#[test]
fn a_long_password_is_accepted() {
    let long = "x".repeat(500);
    let hash = hash_password(&long).unwrap();
    assert!(verify_password(&long, &hash));
    assert!(!verify_password(&"x".repeat(499), &hash));
}

// ───────────────────────── tokens ─────────────────────────

fn tokens() -> Tokens {
    Tokens::new(b"a test secret that is long enough", Duration::from_secs(3600))
}

#[test]
fn a_token_round_trips_to_the_user_it_was_issued_for() {
    let t = tokens();
    let user = Uuid::new_v4();

    let token = t.issue(user).unwrap();
    assert_eq!(t.verify(&token).unwrap(), user);
}

#[test]
fn a_token_signed_with_a_different_secret_is_rejected() {
    // The whole point: without the secret you cannot mint a valid token.
    let issuer = tokens();
    let attacker = Tokens::new(b"a different secret entirely!!", Duration::from_secs(3600));

    let token = attacker.issue(Uuid::new_v4()).unwrap();

    assert!(
        issuer.verify(&token).is_err(),
        "a token from an unknown signer was accepted"
    );
}

#[test]
fn a_tampered_payload_is_rejected() {
    // Flipping a byte in the claims invalidates the signature.
    let t = tokens();
    let token = t.issue(Uuid::new_v4()).unwrap();

    let mut parts: Vec<String> = token.split('.').map(str::to_string).collect();
    assert_eq!(parts.len(), 3, "a JWT has three parts");

    let payload = &mut parts[1];
    let last = payload.pop().unwrap();
    payload.push(if last == 'A' { 'B' } else { 'A' });

    assert!(t.verify(&parts.join(".")).is_err(), "tampering was not detected");
}

#[test]
fn an_expired_token_is_rejected() {
    let t = tokens();
    let expired = t.issue_expiring_at(Uuid::new_v4(), now() - 3_600).unwrap();

    assert!(t.verify(&expired).is_err(), "an expired token was accepted");
}

#[test]
fn a_token_expiring_shortly_is_still_valid() {
    let t = tokens();
    let user = Uuid::new_v4();
    let token = t.issue_expiring_at(user, now() + 120).unwrap();

    assert_eq!(t.verify(&token).unwrap(), user);
}

#[test]
fn garbage_is_rejected_rather_than_panicking() {
    let t = tokens();
    for junk in ["", "not.a.token", "a.b.c", "...", "eyJhbGciOiJIUzI1NiJ9"] {
        assert!(t.verify(junk).is_err(), "junk token {junk:?} was accepted");
    }
}

#[test]
fn an_unsigned_token_is_rejected() {
    // The classic JWT attack: claim the algorithm is "none" and drop the
    // signature. The verifier must insist on the algorithm it issued with.
    let t = tokens();
    use base64::Engine;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = b64.encode(br#"{"alg":"none","typ":"JWT"}"#);
    let claims = b64.encode(
        format!(r#"{{"sub":"{}","exp":{}}}"#, Uuid::new_v4(), now() + 3600).as_bytes(),
    );
    let forged = format!("{header}.{claims}.");

    assert!(t.verify(&forged).is_err(), "an alg=none token was accepted");
}

#[test]
fn two_tokens_for_different_users_do_not_verify_to_the_same_id() {
    let t = tokens();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();

    assert_eq!(t.verify(&t.issue(a).unwrap()).unwrap(), a);
    assert_eq!(t.verify(&t.issue(b).unwrap()).unwrap(), b);
}
