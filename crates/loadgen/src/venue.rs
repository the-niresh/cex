//! The handful of REST calls a dev tool makes against a running exchange.
//!
//! Extracted so the load driver and the demo maker cannot drift apart on what
//! "register an account" or "cancel an order" means. Order *placement* stays
//! split: the driver's version reads the timing headers and is the source of
//! published numbers, so it keeps its own copy rather than growing a parameter
//! it would have to ignore.

use anyhow::{Context, Result};
use uuid::Uuid;

/// A registered account and the token that speaks for it.
pub struct User {
    pub token: String,
}

/// Register a throwaway account. The username is random, so runs never collide.
pub async fn register(http: &reqwest::Client, host: &str, prefix: &str) -> Result<User> {
    let username = format!("{prefix}{}", Uuid::new_v4().simple());
    let body: serde_json::Value = http
        .post(format!("{host}/register"))
        .json(&serde_json::json!({ "username": username, "password": "a-good-password" }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(User {
        token: body["token"]
            .as_str()
            .context("no token in the register response")?
            .to_string(),
    })
}

/// Credit an account with enough of both assets that a long run cannot run dry.
pub async fn fund(http: &reqwest::Client, host: &str, who: &User, amount: i64) -> Result<()> {
    for asset in ["USDT", "BTC"] {
        http.post(format!("{host}/deposit"))
            .bearer_auth(&who.token)
            .header("idempotency-key", Uuid::new_v4().to_string())
            .json(&serde_json::json!({ "asset": asset, "amount": amount }))
            .send()
            .await?
            .error_for_status()?;
    }
    Ok(())
}

/// Rest a limit order. Returns its id, so it can be cancelled later.
pub async fn place_limit(
    http: &reqwest::Client,
    host: &str,
    who: &User,
    symbol: &str,
    side: &str,
    price: i64,
    qty: i64,
) -> Result<u64> {
    let body = serde_json::json!({
        "symbol": symbol,
        "side": side,
        "order_type": "LIMIT",
        "time_in_force": "GTC",
        "price": price,
        "qty": qty,
    });
    send_order(http, host, who, &body).await
}

/// Cross the spread. IOC, so nothing is left resting if the book is thin.
pub async fn place_market(
    http: &reqwest::Client,
    host: &str,
    who: &User,
    symbol: &str,
    side: &str,
    qty: i64,
) -> Result<u64> {
    let body = serde_json::json!({
        "symbol": symbol,
        "side": side,
        "order_type": "MARKET",
        "qty": qty,
    });
    send_order(http, host, who, &body).await
}

async fn send_order(
    http: &reqwest::Client,
    host: &str,
    who: &User,
    body: &serde_json::Value,
) -> Result<u64> {
    let response: serde_json::Value = http
        .post(format!("{host}/orders"))
        .bearer_auth(&who.token)
        .header("idempotency-key", Uuid::new_v4().to_string())
        .json(body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    response["order_id"]
        .as_u64()
        .context("no order_id in the order response")
}

/// Cancel a resting order. An order that is already gone counts as cancelled.
///
/// ⚠️ Between quoting and cancelling, a quote may have been filled — by this
/// tool's own taker or by anyone else on the venue — and the engine answers a
/// cancel for a closed order with **400 and `order N is already closed`**, not
/// a 404. Checked against a running exchange rather than assumed; the obvious
/// `error_for_status()` here would kill a long run the first time one of its
/// own quotes traded, which is the one outcome the whole thing is trying to
/// produce.
pub async fn cancel(http: &reqwest::Client, host: &str, who: &User, order_id: u64) -> Result<()> {
    let response = http
        .delete(format!("{host}/orders/{order_id}"))
        .bearer_auth(&who.token)
        .send()
        .await?;

    let status = response.status();
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }

    let body = response.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::BAD_REQUEST && already_gone(&body) {
        return Ok(());
    }

    anyhow::bail!("cancel {order_id} failed: {status} {body}")
}

/// Whether a rejected cancel means the order had already left the book.
///
/// Kept as its own function so the wording this depends on is testable without
/// a server, and visible when the engine's phrasing changes.
fn already_gone(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    body.contains("already closed") || body.contains("not found") || body.contains("unknown order")
}

#[cfg(test)]
mod tests {
    use super::already_gone;

    #[test]
    fn recognises_the_engine_saying_the_order_is_closed() {
        // Verbatim from a running exchange: cancel an order twice and the
        // second attempt answers 400 with this body.
        assert!(already_gone(r#"{"error":"order 713 is already closed"}"#));
    }

    #[test]
    fn does_not_swallow_an_unrelated_rejection() {
        assert!(!already_gone(
            r#"{"error":"order 713 belongs to another user"}"#
        ));
        assert!(!already_gone(r#"{"error":"malformed request"}"#));
        assert!(!already_gone(""));
    }
}
