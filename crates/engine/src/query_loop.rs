//! Concurrent query serving.
//!
//! Runs on its own Redis connection and its own tokio task, entirely independent of the command
//! loop's `XREAD BLOCK` — that separation is the whole fix for the read-latency defect (see the
//! "Known gaps" entry this removed from `README.md`). `state` is shared with the command loop
//! behind a `Mutex` so a query can never be answered from a command that is only half-applied;
//! `Runner::handle` takes the same lock for the full `apply()` + `check_invariants()` of one
//! command, so the two never interleave mid-command.

use std::sync::{Arc, Mutex};

use cex_core::state::State;
use cex_proto::{Query, Response};
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use tracing::{error, warn};

pub type SharedState = Arc<Mutex<State>>;

#[derive(Debug, Clone)]
pub struct QueryLoopConfig {
    pub queries_queue: String,
    pub responses_channel: String,
}

/// Decode and answer one query payload against `state`. `None` means the payload was
/// undecodable — logged and dropped, same as any other malformed message in this system.
///
/// Shared between [`run`] (the concurrent `BRPOP` loop) and
/// [`crate::runner::Runner::poll_queries`] (the non-blocking `RPOP` loop tests drive directly),
/// so both answer identically.
pub fn answer(state: &SharedState, payload: &str) -> Option<Response> {
    let query: Query = match serde_json::from_str(payload) {
        Ok(q) => q,
        Err(e) => {
            error!(error = %e, "undecodable query, dropping");
            return None;
        }
    };

    let request_id = query.request_id();
    let guard = state.lock().expect("state mutex poisoned");
    let response = match guard.query(&query) {
        Ok(body) => Response::ok(request_id, body),
        Err(e) => Response::err(request_id, e.to_string()),
    };
    Some(response)
}

/// Pop queries with `BRPOP` and answer them concurrently with the command loop.
///
/// Blocks indefinitely on an empty queue (timeout `0.0`) — there is nothing else for this task to
/// do, and it is stopped from outside (see `Runner::run` and `Drop for Runner`) rather than
/// exiting on its own.
pub async fn run(mut conn: MultiplexedConnection, cfg: QueryLoopConfig, state: SharedState) {
    loop {
        let popped: Option<(String, String)> = match conn.brpop(&cfg.queries_queue, 0.0).await {
            Ok(v) => v,
            Err(e) => {
                error!(error = %e, "query BRPOP failed, retrying");
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                continue;
            }
        };
        let Some((_key, payload)) = popped else {
            continue;
        };

        let Some(response) = answer(&state, &payload) else {
            continue;
        };
        let json = match serde_json::to_string(&response) {
            Ok(j) => j,
            Err(_) => {
                error!("could not encode query response");
                continue;
            }
        };
        let published: redis::RedisResult<i64> =
            conn.publish(&cfg.responses_channel, json.as_str()).await;
        if let Err(e) = published {
            warn!(error = %e, "failed to publish query response");
        }
    }
}
