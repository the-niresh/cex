//! Tests must refuse to run against remote databases.

use cex_persist::assert_safe_database_url;

#[test]
fn localhost_is_allowed() {
    assert!(assert_safe_database_url("postgres://cex:cex@127.0.0.1:5442/cex").is_ok());
    assert!(assert_safe_database_url("postgres://cex:cex@localhost:5442/cex").is_ok());
}

#[test]
fn docker_service_names_are_allowed() {
    assert!(assert_safe_database_url("postgres://cex:cex@postgres:5432/cex").is_ok());
}

#[test]
fn neon_is_rejected() {
    let err = assert_safe_database_url(
        "postgres://user:pass@ep-cool-name-123456.us-east-2.aws.neon.tech/neondb",
    )
    .unwrap_err();
    assert!(err.contains("neon.tech") || err.contains("not localhost"));
}
