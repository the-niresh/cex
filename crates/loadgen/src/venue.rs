//! The handful of REST calls a dev tool makes against a running exchange.
//!
//! Extracted so the load driver and the demo maker cannot drift apart on what
//! "register an account" or "cancel an order" means. Order *placement* stays
//! split: the driver's version reads the timing headers and is the source of
//! published numbers, so it keeps its own copy rather than growing a parameter
//! it would have to ignore.

use std::time::Duration;

use anyhow::{Context, Result};
use cex_proto::deposit::deposit_chunks;
use reqwest::StatusCode;
use uuid::Uuid;

/// A registered account and the token that speaks for it.
pub struct User {
    pub token: String,
}

/// Credentials the demo maker can use to log back in after a token expires.
#[derive(Clone)]
pub struct Account {
    pub token: String,
    username: String,
    password: String,
}

impl Account {
    pub fn token(&self) -> &str {
        &self.token
    }
}

/// Register a throwaway account. The username is random, so runs never collide.
pub async fn register(http: &reqwest::Client, host: &str, prefix: &str) -> Result<Account> {
    let username = format!("{prefix}{}", Uuid::new_v4().simple());
    let password = "a-good-password";
    let token = register_named(http, host, &username, password)
        .await
        .context("register")?;
    Ok(Account {
        token,
        username,
        password: password.to_string(),
    })
}

async fn register_named(
    http: &reqwest::Client,
    host: &str,
    username: &str,
    password: &str,
) -> Result<String> {
    let body: serde_json::Value = http
        .post(format!("{host}/register"))
        .json(&serde_json::json!({ "username": username, "password": password }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(body["token"]
        .as_str()
        .context("no token in the register response")?
        .to_string())
}

async fn login_named(
    http: &reqwest::Client,
    host: &str,
    username: &str,
    password: &str,
) -> Result<String> {
    let response = http
        .post(format!("{host}/login"))
        .json(&serde_json::json!({ "username": username, "password": password }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("login failed: {status} {body}");
    }

    let body: serde_json::Value = response.json().await?;
    Ok(body["token"]
        .as_str()
        .context("no token in the login response")?
        .to_string())
}

/// Log in with stored credentials, or register again when the user is gone.
pub async fn reauth(http: &reqwest::Client, host: &str, account: &mut Account) -> Result<()> {
    if let Ok(token) = login_named(http, host, &account.username, &account.password).await {
        account.token = token;
        return Ok(());
    }

    let prefix = account
        .username
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>();
    let prefix = if prefix.is_empty() {
        "demo"
    } else {
        prefix.as_str()
    };
    *account = register(http, host, prefix).await?;
    Ok(())
}

/// Credit an account, splitting each asset into deposits within the API limit.
pub async fn fund(
    http: &reqwest::Client,
    host: &str,
    who: &Account,
    usdt: i64,
    btc: i64,
) -> Result<()> {
    for chunk in deposit_chunks("USDT", usdt) {
        deposit_one(http, host, who, "USDT", chunk).await?;
    }
    for chunk in deposit_chunks("BTC", btc) {
        deposit_one(http, host, who, "BTC", chunk).await?;
    }
    Ok(())
}

async fn deposit_one(
    http: &reqwest::Client,
    host: &str,
    who: &Account,
    asset: &str,
    amount: i64,
) -> Result<()> {
    let response = http
        .post(format!("{host}/deposit"))
        .bearer_auth(who.token())
        .header("idempotency-key", Uuid::new_v4().to_string())
        .json(&serde_json::json!({ "asset": asset, "amount": amount }))
        .send()
        .await?;

    if response.status() == StatusCode::UNAUTHORIZED {
        anyhow::bail!("unauthorized");
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("deposit {asset} {amount}: {status} {body}");
    }
    Ok(())
}

/// Limits the demo maker needs from the market list.
pub struct MarketLimits {
    pub min_notional: i64,
    pub base_decimals: u32,
}

/// Read `min_notional` and `base_decimals` for one symbol from `/markets`.
pub async fn market_limits(
    http: &reqwest::Client,
    host: &str,
    symbol: &str,
) -> Result<MarketLimits> {
    let body: serde_json::Value = http
        .get(format!("{host}/markets"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let market = body["markets"]
        .as_array()
        .context("no markets in the response")?
        .iter()
        .find(|m| m["symbol"].as_str() == Some(symbol))
        .with_context(|| format!("market {symbol} not listed"))?;

    Ok(MarketLimits {
        min_notional: market["min_notional"]
            .as_i64()
            .context("min_notional missing")?,
        base_decimals: market["base_decimals"]
            .as_u64()
            .context("base_decimals missing")? as u32,
    })
}

/// The most recent trade price, if the tape has any prints yet.
pub async fn last_trade_price(
    http: &reqwest::Client,
    host: &str,
    symbol: &str,
) -> Result<Option<i64>> {
    let body: serde_json::Value = http
        .get(format!("{host}/trades/{symbol}?limit=1"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let price = body["trades"]
        .as_array()
        .and_then(|trades| trades.first())
        .and_then(|t| t["price"].as_i64());
    Ok(price)
}

/// The best bid and best ask currently resting, either of which may be absent.
///
/// Read before quoting: a maker that does not know where the market is will
/// price its ladder somewhere else and trade with everything in between.
pub async fn touch(
    http: &reqwest::Client,
    host: &str,
    symbol: &str,
) -> Result<(Option<i64>, Option<i64>)> {
    let book: serde_json::Value = http
        .get(format!("{host}/depth/{symbol}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // `[[price, qty], ...]`, and the API does not promise an order, so take the
    // extreme of each side rather than the first element.
    let side = |key: &str, best: fn(&[i64]) -> Option<i64>| -> Option<i64> {
        let prices: Vec<i64> = book[key]
            .as_array()?
            .iter()
            .filter_map(|level| level.get(0)?.as_i64())
            .collect();
        best(&prices)
    };

    Ok((
        side("bids", |p| p.iter().copied().max()),
        side("asks", |p| p.iter().copied().min()),
    ))
}

/// Rest a limit order. Returns its id, so it can be cancelled later.
pub async fn place_limit(
    http: &reqwest::Client,
    host: &str,
    who: &Account,
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
    who: &Account,
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
    who: &Account,
    body: &serde_json::Value,
) -> Result<u64> {
    let response = http
        .post(format!("{host}/orders"))
        .bearer_auth(who.token())
        .header("idempotency-key", Uuid::new_v4().to_string())
        .json(body)
        .send()
        .await?;

    if response.status() == StatusCode::UNAUTHORIZED {
        anyhow::bail!("unauthorized");
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("order failed: {status} {body}");
    }

    let response: serde_json::Value = response.json().await?;
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
pub async fn cancel(
    http: &reqwest::Client,
    host: &str,
    who: &Account,
    order_id: u64,
) -> Result<()> {
    let response = http
        .delete(format!("{host}/orders/{order_id}"))
        .bearer_auth(who.token())
        .send()
        .await?;

    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        anyhow::bail!("unauthorized");
    }
    if status.is_success() || status == StatusCode::NOT_FOUND {
        return Ok(());
    }

    let body = response.text().await.unwrap_or_default();
    if status == StatusCode::BAD_REQUEST && already_gone(&body) {
        return Ok(());
    }

    anyhow::bail!("cancel {order_id} failed: {status} {body}")
}

/// Run `op` and on `401` log in again, fund if the account was re-registered,
/// then retry once. Persistent failures back off from 1 s to 30 s and log once
/// per change of state.
/// USDT and BTC atom targets used when an account must be re-funded.
#[derive(Clone, Copy)]
pub struct Funding {
    pub usdt: i64,
    pub btc: i64,
}

pub async fn with_reauth<T, F, Fut>(
    http: &reqwest::Client,
    host: &str,
    account: &mut Account,
    funding: Funding,
    logger: &mut StateLogger,
    backoff: &mut Backoff,
    op: F,
) -> Result<T>
where
    F: Fn(&Account) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    loop {
        match op(account).await {
            Ok(value) => {
                backoff.reset();
                logger.ok();
                return Ok(value);
            }
            Err(e) if is_unauthorized(&e) => {
                logger.reauthing();
                reauth(http, host, account).await?;
                fund(http, host, account, funding.usdt, funding.btc).await?;
                logger.reauthed();
                match op(account).await {
                    Ok(value) => {
                        backoff.reset();
                        logger.ok();
                        return Ok(value);
                    }
                    Err(retry_err) => {
                        let wait = backoff.wait_after_failure();
                        logger.backing_off(wait, &retry_err);
                        tokio::time::sleep(wait).await;
                    }
                }
            }
            Err(e) => {
                let wait = backoff.wait_after_failure();
                logger.backing_off(wait, &e);
                tokio::time::sleep(wait).await;
            }
        }
    }
}

fn is_unauthorized(err: &anyhow::Error) -> bool {
    err.to_string().contains("unauthorized")
}

/// Whether a rejected cancel means the order had already left the book.
///
/// Kept as its own function so the wording this depends on is testable without
/// a server, and visible when the engine's phrasing changes.
fn already_gone(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    body.contains("already closed") || body.contains("not found") || body.contains("unknown order")
}

/// Exponential backoff from one second up to thirty.
pub struct Backoff {
    current: Duration,
    max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            current: Duration::from_secs(1),
            max: Duration::from_secs(30),
        }
    }
}

impl Backoff {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn wait_after_failure(&mut self) -> Duration {
        let wait = self.current;
        self.current = (self.current * 2).min(self.max);
        wait
    }

    pub fn reset(&mut self) {
        self.current = Duration::from_secs(1);
    }
}

/// Logs a state change once, not on every failed order in the same state.
#[derive(Default)]
pub struct StateLogger {
    last: Option<&'static str>,
}

impl StateLogger {
    pub fn new() -> Self {
        Self::default()
    }

    fn note(&mut self, state: &'static str, detail: Option<&str>) {
        if self.last == Some(state) {
            return;
        }
        self.last = Some(state);
        if let Some(detail) = detail {
            eprintln!("demo-maker: {state}: {detail}");
        } else {
            eprintln!("demo-maker: {state}");
        }
    }

    pub fn ok(&mut self) {
        self.note("running", None);
    }

    pub fn reauthing(&mut self) {
        self.note("token expired, logging in again", None);
    }

    pub fn reauthed(&mut self) {
        self.note("logged in and re-funded", None);
    }

    pub fn backing_off(&mut self, wait: Duration, err: &anyhow::Error) {
        self.note(
            "backing off",
            Some(&format!("{}s after {err}", wait.as_secs())),
        );
    }

    pub fn ignored_stray_book(&mut self, reference: i64, touch: i64) {
        self.note(
            "ignoring stray book",
            Some(&format!("reference {reference}, touch {touch}")),
        );
    }

    pub fn below_min_notional(&mut self) {
        self.note("skipping quotes below minimum notional", None);
    }
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
