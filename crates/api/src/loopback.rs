//! Request/response over Redis.
//!
//! The API holds no exchange state, so every write and most reads are a round
//! trip to the engine. The mechanism:
//!
//! 1. Stamp the request with a fresh `request_id`.
//! 2. Park a [`oneshot::Sender`] in a map under that id.
//! 3. Publish the request — a command goes on the durable stream, a query on the
//!    ephemeral list.
//! 4. A background task subscribed to the response channel looks up each reply's
//!    id and wakes the matching waiter.
//! 5. The caller awaits with a timeout, and removes its own entry either way.
//!
//! Every reply travels on one shared channel, so step 4 is the part that must
//! not be wrong: a routing mistake would deliver one user's balances to another.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cex_proto::{Command, Query, RequestId, Response, ResponseBody, ResponseResult, FIELD_PAYLOAD};
use futures_util::StreamExt;
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use tracing::{debug, error, warn};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum LoopbackError {
    /// The engine answered, and the answer was "no".
    #[error("{0}")]
    Rejected(String),
    /// Nobody answered in time. The request may still have been applied — it is
    /// on the durable log — so this is not proof that nothing happened.
    #[error("engine did not respond in time")]
    Timeout,
    #[error("transport: {0}")]
    Transport(String),
}

#[derive(Debug, Clone)]
pub struct LoopbackConfig {
    pub redis_url: String,
    pub commands_stream: String,
    pub queries_queue: String,
    pub responses_channel: String,
    pub timeout: Duration,
}

impl Default for LoopbackConfig {
    fn default() -> Self {
        LoopbackConfig {
            redis_url: "redis://127.0.0.1:6390".into(),
            commands_stream: cex_proto::STREAM_COMMANDS.into(),
            queries_queue: cex_proto::QUEUE_QUERIES.into(),
            responses_channel: cex_proto::CHANNEL_RESPONSES.into(),
            timeout: Duration::from_secs(5),
        }
    }
}

type Pending = Arc<Mutex<HashMap<RequestId, tokio::sync::oneshot::Sender<Response>>>>;

#[derive(Clone)]
pub struct Loopback {
    cfg: Arc<LoopbackConfig>,
    conn: MultiplexedConnection,
    pending: Pending,
}

impl Loopback {
    /// Connect and start listening for replies.
    pub async fn connect(cfg: LoopbackConfig) -> Result<Self, LoopbackError> {
        let client = redis::Client::open(cfg.redis_url.as_str())
            .map_err(|e| LoopbackError::Transport(e.to_string()))?;
        let conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| LoopbackError::Transport(e.to_string()))?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

        // Subscribe before returning, so a request sent immediately after
        // `connect` cannot have its reply published before we are listening.
        let mut sub = client
            .get_async_pubsub()
            .await
            .map_err(|e| LoopbackError::Transport(e.to_string()))?;
        sub.subscribe(&cfg.responses_channel)
            .await
            .map_err(|e| LoopbackError::Transport(e.to_string()))?;

        let routes = Arc::clone(&pending);
        tokio::spawn(async move {
            let mut stream = sub.on_message();
            while let Some(msg) = stream.next().await {
                let payload: String = match msg.get_payload() {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(error = %e, "unreadable reply");
                        continue;
                    }
                };
                let response: Response = match serde_json::from_str(&payload) {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(error = %e, "undecodable reply");
                        continue;
                    }
                };

                // Take the waiter out of the map before waking it, so a late
                // duplicate cannot wake the same caller twice.
                let waiter = routes
                    .lock()
                    .expect("pending map poisoned")
                    .remove(&response.request_id);

                match waiter {
                    Some(tx) => {
                        // An error here means the caller already gave up. Fine.
                        let _ = tx.send(response);
                    }
                    None => debug!(
                        request_id = %response.request_id,
                        "reply for a request nobody is waiting on, dropping"
                    ),
                }
            }
            error!("response subscription ended; the loopback is now deaf");
        });

        Ok(Loopback {
            cfg: Arc::new(cfg),
            conn,
            pending,
        })
    }

    /// How many requests are currently in flight. Exposed so tests can prove the
    /// map does not leak.
    pub fn pending_count(&self) -> usize {
        self.pending.lock().expect("pending map poisoned").len()
    }

    /// Send a state-changing request. It is appended to the durable log, so it
    /// survives an engine restart even if the reply never arrives.
    /// Send a command under a fresh id. Nothing deduplicates it.
    pub async fn command(&self, cmd: Command) -> Result<ResponseBody, LoopbackError> {
        self.command_with_id(cmd, Uuid::new_v4()).await
    }

    /// Send a command under a caller-chosen id.
    ///
    /// The engine remembers ids it has applied, so sending the same one twice
    /// applies it once and returns the original answer. That is what makes a
    /// retry after a timeout safe.
    pub async fn command_with_id(
        &self,
        mut cmd: Command,
        id: Uuid,
    ) -> Result<ResponseBody, LoopbackError> {
        set_command_request_id(&mut cmd, id);

        let json =
            serde_json::to_string(&cmd).map_err(|e| LoopbackError::Transport(e.to_string()))?;

        let rx = self.park(id);
        let mut conn = self.conn.clone();
        let sent: Result<String, _> = conn
            .xadd(
                &self.cfg.commands_stream,
                "*",
                &[(FIELD_PAYLOAD, json.as_str())],
            )
            .await;
        self.finish_send(id, sent.map(|_| ()))?;

        self.await_reply(id, rx).await
    }

    /// Send a read-only request. Never logged.
    pub async fn query(&self, mut query: Query) -> Result<ResponseBody, LoopbackError> {
        let id = Uuid::new_v4();
        set_query_request_id(&mut query, id);

        let json =
            serde_json::to_string(&query).map_err(|e| LoopbackError::Transport(e.to_string()))?;

        let rx = self.park(id);
        let mut conn = self.conn.clone();
        let sent: Result<i64, _> = conn.lpush(&self.cfg.queries_queue, json).await;
        self.finish_send(id, sent.map(|_| ()))?;

        self.await_reply(id, rx).await
    }

    fn park(&self, id: RequestId) -> tokio::sync::oneshot::Receiver<Response> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending
            .lock()
            .expect("pending map poisoned")
            .insert(id, tx);
        rx
    }

    fn forget(&self, id: RequestId) {
        self.pending
            .lock()
            .expect("pending map poisoned")
            .remove(&id);
    }

    /// Drop the parked waiter if the request never made it out.
    fn finish_send(
        &self,
        id: RequestId,
        sent: Result<(), redis::RedisError>,
    ) -> Result<(), LoopbackError> {
        if let Err(e) = sent {
            self.forget(id);
            return Err(LoopbackError::Transport(e.to_string()));
        }
        Ok(())
    }

    async fn await_reply(
        &self,
        id: RequestId,
        rx: tokio::sync::oneshot::Receiver<Response>,
    ) -> Result<ResponseBody, LoopbackError> {
        let outcome = tokio::time::timeout(self.cfg.timeout, rx).await;

        // Whatever happened, this request is no longer pending. Without this the
        // map grows by one entry for every request that times out.
        self.forget(id);

        match outcome {
            Ok(Ok(response)) => match response.result {
                ResponseResult::Ok { data } => Ok(data),
                ResponseResult::Err { error } => Err(LoopbackError::Rejected(error)),
            },
            // The subscriber task dropped the sender: it died.
            Ok(Err(_)) => Err(LoopbackError::Transport("response listener stopped".into())),
            Err(_) => Err(LoopbackError::Timeout),
        }
    }
}

/// Stamp our own id over whatever the caller supplied.
///
/// The caller must not be able to cause a collision by reusing an id, because a
/// collision means two requests share one waiter and one caller gets the other's
/// answer.
fn set_command_request_id(cmd: &mut Command, id: RequestId) {
    match cmd {
        Command::Deposit { request_id, .. }
        | Command::Withdraw { request_id, .. }
        | Command::PlaceOrder { request_id, .. }
        | Command::CancelOrder { request_id, .. } => *request_id = id,
    }
}

fn set_query_request_id(query: &mut Query, id: RequestId) {
    match query {
        Query::Depth { request_id, .. }
        | Query::Balances { request_id, .. }
        | Query::Order { request_id, .. }
        | Query::OpenOrders { request_id, .. }
        | Query::Markets { request_id } => *request_id = id,
    }
}
