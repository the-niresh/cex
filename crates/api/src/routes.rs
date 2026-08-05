//! The HTTP surface.
//!
//! Every handler is thin on purpose. The API decides nothing about trading — it
//! authenticates, validates shape, forwards to the engine, and translates the
//! answer into a status code. All the judgement lives in `cex-core`.
//!
//! Because it holds no exchange state, any number of copies can run behind a
//! load balancer.

use std::sync::Arc;

use axum::extract::{Path, Query as UrlQuery, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use cex_proto::{Command, OrderType, Query, ResponseBody, Side, TimeInForce};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::Tokens;
use crate::loopback::{Loopback, LoopbackError};
use crate::users::{UserStore, UsersError};
use cex_persist::HistoryStore;
use std::collections::HashMap;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    loopback: Loopback,
    users: UserStore,
    tokens: Tokens,
    /// Read-only here. Only `persist` ever writes these tables.
    history: HistoryStore,
}

impl AppState {
    pub fn new(
        loopback: Loopback,
        users: UserStore,
        tokens: Tokens,
        history: HistoryStore,
    ) -> Self {
        AppState {
            inner: Arc::new(Inner {
                loopback,
                users,
                tokens,
                history,
            }),
        }
    }

    /// Whether the exchange lists this market.
    ///
    /// Only worth asking when a history query came back empty, to tell a market
    /// that has not traded yet from a symbol that does not exist.
    async fn market_exists(&self, symbol: &str) -> Result<bool, ApiError> {
        let body = self
            .inner
            .loopback
            .query(Query::Markets {
                request_id: Uuid::nil(),
            })
            .await?;
        match body {
            ResponseBody::Markets(markets) => Ok(markets.iter().any(|m| m.symbol == symbol)),
            other => Err(unexpected(other)),
        }
    }
}

/// The authenticated user, attached by the middleware and read by handlers.
#[derive(Clone, Copy)]
struct CallerId(Uuid);

// ───────────────────────── errors ─────────────────────────

/// One error type for every handler, so status codes are decided in one place
/// rather than scattered across twelve `match` arms.
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        ApiError {
            status,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::BAD_REQUEST, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<LoopbackError> for ApiError {
    fn from(e: LoopbackError) -> Self {
        match e {
            // The engine considered it and said no. That is the caller's fault.
            LoopbackError::Rejected(msg) => ApiError::new(StatusCode::BAD_REQUEST, msg),
            // A timeout is genuinely "don't know": the command is on the durable
            // log and may yet apply. 504 says so more honestly than 500.
            LoopbackError::Timeout => ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "the exchange did not respond in time; the request may still be applied",
            ),
            LoopbackError::Transport(msg) => ApiError::new(StatusCode::BAD_GATEWAY, msg),
        }
    }
}

impl From<UsersError> for ApiError {
    fn from(e: UsersError) -> Self {
        let status = match e {
            UsersError::UsernameTaken => StatusCode::CONFLICT,
            UsersError::InvalidUsername | UsersError::WeakPassword => StatusCode::BAD_REQUEST,
            UsersError::BadCredentials => StatusCode::UNAUTHORIZED,
            UsersError::Db(_) | UsersError::Hash(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        // Never leak a database error to a caller.
        let message = match e {
            UsersError::Db(_) | UsersError::Hash(_) => "internal error".to_string(),
            other => other.to_string(),
        };
        ApiError::new(status, message)
    }
}

type ApiResult<T> = Result<T, ApiError>;

// ───────────────────────── router ─────────────────────────

pub fn build_router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/deposit", post(deposit))
        .route("/balances", get(balances))
        .route("/orders", post(place_order))
        .route("/orders/open", get(open_orders))
        .route("/orders/{id}", delete(cancel_order))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .route("/health", get(health))
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/markets", get(markets))
        .route("/depth/{symbol}", get(depth))
        .route("/trades/{symbol}", get(trades))
        .merge(protected)
        .with_state(state)
}

/// Turn a bearer token into a caller id, or refuse.
async fn require_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "missing bearer token"))?;

    let user_id = state
        .inner
        .tokens
        .verify(token)
        .map_err(|_| ApiError::new(StatusCode::UNAUTHORIZED, "invalid or expired token"))?;

    req.extensions_mut().insert(CallerId(user_id));
    Ok(next.run(req).await)
}

fn caller(req: &Request) -> ApiResult<Uuid> {
    req.extensions()
        .get::<CallerId>()
        .map(|c| c.0)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "not authenticated"))
}

// ───────────────────────── open handlers ─────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct Session {
    user_id: Uuid,
    token: String,
}

async fn register(
    State(state): State<AppState>,
    Json(body): Json<Credentials>,
) -> ApiResult<(StatusCode, Json<Session>)> {
    let user = state
        .inner
        .users
        .register(&body.username, &body.password)
        .await?;
    let token = state
        .inner
        .tokens
        .issue(user.id)
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;

    Ok((
        StatusCode::CREATED,
        Json(Session {
            user_id: user.id,
            token,
        }),
    ))
}

async fn login(
    State(state): State<AppState>,
    Json(body): Json<Credentials>,
) -> ApiResult<Json<Session>> {
    let user_id = state
        .inner
        .users
        .authenticate(&body.username, &body.password)
        .await?;
    let token = state
        .inner
        .tokens
        .issue(user_id)
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;

    Ok(Json(Session { user_id, token }))
}

async fn markets(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let body = state
        .inner
        .loopback
        .query(Query::Markets {
            request_id: Uuid::nil(),
        })
        .await?;
    match body {
        ResponseBody::Markets(m) => Ok(Json(json!({ "markets": m }))),
        other => Err(unexpected(other)),
    }
}


/// Namespace for deriving a request id from a caller's idempotency key.
///
/// Fixed forever. Changing it would make every key a client has already used
/// stop matching, which would silently turn retries back into duplicates.
const IDEMPOTENCY_NAMESPACE: Uuid = Uuid::from_u128(0x9f2b_1c44_6a7d_4e18_9c3a_5d0e_7b61_28af);

/// Longest idempotency key accepted. Long enough for a UUID or a ULID with room
/// to spare, short enough that the header cannot be used as free storage.
const MAX_IDEMPOTENCY_KEY: usize = 200;

/// The id this command should travel under.
///
/// With an `Idempotency-Key` header the id is derived from it, so a retry
/// carries the same id and the engine recognises it as one it already applied.
/// Without the header a fresh id is used and nothing is deduplicated —
/// idempotency is opt-in, because two deliberate identical orders are a normal
/// thing to want.
///
/// The key is scoped to the caller. Two users picking `"1"` must not have one
/// of them handed the other's answer.
fn request_id_for(user_id: Uuid, req: &Request) -> ApiResult<Uuid> {
    let Some(raw) = req.headers().get("idempotency-key") else {
        return Ok(Uuid::new_v4());
    };

    let key = raw
        .to_str()
        .map_err(|_| ApiError::bad_request("Idempotency-Key must be ASCII"))?
        .trim();

    // Rejected rather than ignored: a caller who sent a key believes they are
    // protected, and silently dropping it is the one outcome they cannot see.
    if key.is_empty() || key.len() > MAX_IDEMPOTENCY_KEY {
        return Err(ApiError::bad_request(format!(
            "Idempotency-Key must be 1 to {MAX_IDEMPOTENCY_KEY} characters"
        )));
    }

    Ok(Uuid::new_v5(
        &IDEMPOTENCY_NAMESPACE,
        format!("{user_id}:{key}").as_bytes(),
    ))
}

/// The most trades one request may ask for.
///
/// Without a ceiling, `?limit=100000000` is a way to make one HTTP request drag
/// the exchange's entire trade history through Postgres and into memory.
const MAX_TRADES: i64 = 500;
const DEFAULT_TRADES: i64 = 50;

/// One trade, as the public is entitled to see it.
///
/// Built field by field from a `FillRow` rather than by serialising it, so the
/// counterparty columns that row carries — `maker_user_id`, `taker_user_id`,
/// and the order ids that would link trades to a trader across requests — have
/// no route to the wire. Same rule the websocket feed keeps.
#[derive(Debug, Serialize)]
struct PublicTrade {
    seq: u64,
    price: i64,
    qty: i64,
    /// Side of the aggressing order.
    taker_side: String,
    timestamp_ms: i64,
}

async fn trades(
    State(state): State<AppState>,
    Path(symbol): Path<String>,
    UrlQuery(params): UrlQuery<HashMap<String, String>>,
) -> ApiResult<Json<serde_json::Value>> {
    let limit = match params.get("limit") {
        None => DEFAULT_TRADES,
        Some(raw) => raw
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| ApiError::bad_request("limit must be a positive integer"))?
            .min(MAX_TRADES),
    };

    let rows = state
        .inner
        .history
        .fills_for_symbol(&symbol, limit)
        .await
        .map_err(|e| {
            // The caller gets nothing useful from a database error, and the
            // detail belongs in the log rather than in a public response.
            tracing::error!(error = %e, symbol, "reading trade history failed");
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "could not read trade history")
        })?;

    // An empty result is ambiguous: a market that has not traded yet looks
    // exactly like a typo. Only then is it worth a round trip to find out
    // which, so the common path stays a single query.
    if rows.is_empty() && !state.market_exists(&symbol).await? {
        return Err(ApiError::bad_request("unknown market"));
    }

    let trades: Vec<PublicTrade> = rows
        .into_iter()
        .map(|f| PublicTrade {
            seq: f.seq,
            price: f.price,
            qty: f.qty,
            taker_side: f.taker_side,
            timestamp_ms: f.created_at_ms,
        })
        .collect();

    Ok(Json(json!({
        "symbol": symbol,
        "limit": limit,
        "trades": trades,
    })))
}

async fn depth(
    State(state): State<AppState>,
    Path(symbol): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let body = state
        .inner
        .loopback
        .query(Query::Depth {
            request_id: Uuid::nil(),
            symbol,
            limit: None,
        })
        .await?;
    match body {
        ResponseBody::Depth(d) => Ok(Json(json!({
            "symbol": d.symbol,
            "depth_seq": d.depth_seq,
            "bids": d.bids,
            "asks": d.asks,
        }))),
        other => Err(unexpected(other)),
    }
}

// ───────────────────────── protected handlers ─────────────────────────

#[derive(Deserialize)]
struct DepositBody {
    asset: String,
    amount: i64,
}

async fn deposit(
    State(state): State<AppState>,
    req: Request,
) -> ApiResult<Json<serde_json::Value>> {
    let user_id = caller(&req)?;
    let request_id = request_id_for(user_id, &req)?;
    let body: DepositBody = read_json(req).await?;

    state
        .inner
        .loopback
        .command_with_id(
            Command::Deposit {
                request_id,
                user_id,
                asset: body.asset,
                amount: body.amount,
            },
            request_id,
        )
        .await?;

    Ok(Json(json!({ "status": "ok" })))
}

async fn balances(
    State(state): State<AppState>,
    req: Request,
) -> ApiResult<Json<serde_json::Value>> {
    let user_id = caller(&req)?;
    let body = state
        .inner
        .loopback
        .query(Query::Balances {
            request_id: Uuid::nil(),
            user_id,
        })
        .await?;
    match body {
        ResponseBody::Balances(b) => Ok(Json(json!({ "balances": b }))),
        other => Err(unexpected(other)),
    }
}

#[derive(Deserialize)]
struct PlaceOrderBody {
    symbol: String,
    side: Side,
    order_type: OrderType,
    #[serde(default)]
    time_in_force: Option<TimeInForce>,
    #[serde(default)]
    price: Option<i64>,
    qty: i64,
}

async fn place_order(
    State(state): State<AppState>,
    req: Request,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let user_id = caller(&req)?;
    let request_id = request_id_for(user_id, &req)?;
    let body: PlaceOrderBody = read_json(req).await?;

    let result = state
        .inner
        .loopback
        .command_with_id(
            Command::PlaceOrder {
                request_id,
                user_id,
                symbol: body.symbol,
                side: body.side,
                order_type: body.order_type,
                time_in_force: body.time_in_force,
                price: body.price,
                qty: body.qty,
            },
            request_id,
        )
        .await?;

    match result {
        ResponseBody::OrderPlaced {
            order_id,
            status,
            filled_qty,
            qty,
            avg_price,
        } => Ok((
            StatusCode::CREATED,
            Json(json!({
                "order_id": order_id,
                "status": status,
                "filled_qty": filled_qty,
                "qty": qty,
                "avg_price": avg_price,
            })),
        )),
        other => Err(unexpected(other)),
    }
}

async fn cancel_order(
    State(state): State<AppState>,
    Path(order_id): Path<u64>,
    req: Request,
) -> ApiResult<Json<serde_json::Value>> {
    let user_id = caller(&req)?;
    let request_id = request_id_for(user_id, &req)?;

    state
        .inner
        .loopback
        .command_with_id(
            Command::CancelOrder {
                request_id,
                user_id,
                order_id,
            },
            request_id,
        )
        .await?;

    Ok(Json(json!({ "status": "ok", "order_id": order_id })))
}

async fn open_orders(
    State(state): State<AppState>,
    req: Request,
) -> ApiResult<Json<serde_json::Value>> {
    let user_id = caller(&req)?;
    let body = state
        .inner
        .loopback
        .query(Query::OpenOrders {
            request_id: Uuid::nil(),
            user_id,
            symbol: None,
        })
        .await?;
    match body {
        ResponseBody::Orders(o) => Ok(Json(json!({ "orders": o }))),
        other => Err(unexpected(other)),
    }
}

// ───────────────────────── helpers ─────────────────────────

/// Read a JSON body from a request the handler took whole.
///
/// Handlers that need the caller id take `Request` rather than a typed body
/// extractor, because axum only allows one body-consuming extractor and it must
/// be last. This does the decode by hand instead.
async fn read_json<T: serde::de::DeserializeOwned>(req: Request) -> ApiResult<T> {
    use axum::body::to_bytes;
    const MAX_BODY: usize = 64 * 1024;

    let bytes = to_bytes(req.into_body(), MAX_BODY)
        .await
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "could not read request body"))?;

    serde_json::from_slice(&bytes)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, format!("invalid request body: {e}")))
}

/// The engine answered with a shape this endpoint does not expect. A bug on our
/// side, not the caller's.
fn unexpected(body: ResponseBody) -> ApiError {
    tracing::error!(?body, "engine returned an unexpected response shape");
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}
