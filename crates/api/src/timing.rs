//! How long a request spent inside the engine, kept separate from how long the
//! whole request took.
//!
//! The engine number is recorded around the whole `Loopback` round trip, which
//! means it already contains both Redis hops — the `XADD` out and the reply
//! coming back — as well as the apply itself. The gap between the two headers
//! is therefore **this process's own work**: routing, auth, deserialising the
//! request and serialising the response. It is not the Redis hop, and labelling
//! it as such in the published budget would be wrong; the hop cannot be
//! separated out from here, because the engine holds no clock by design.
//!
//! `Loopback` is shared across every request and cannot tell which one it is
//! serving, so the accumulator rides the task instead of a handler argument.
//! Nothing in a handler signature changes.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

tokio::task_local! {
    static ENGINE_MICROS: Arc<AtomicU64>;
}

/// Run `f` with a fresh accumulator. Returns its output and the microseconds
/// recorded inside the engine while it ran.
pub async fn measure_engine<F, T>(f: F) -> (T, u64)
where
    F: Future<Output = T>,
{
    let cell = Arc::new(AtomicU64::new(0));
    let out = ENGINE_MICROS.scope(cell.clone(), f).await;
    (out, cell.load(Ordering::Relaxed))
}

/// Add to the current request's engine total. Outside a [`measure_engine`]
/// scope this does nothing.
pub fn record_engine(micros: u64) {
    let _ = ENGINE_MICROS.try_with(|cell| {
        cell.fetch_add(micros, Ordering::Relaxed);
    });
}

/// The whole request, in microseconds.
pub const SERVER_US: HeaderName = HeaderName::from_static("x-cex-server-us");
/// The part of it spent waiting on the engine, both Redis hops included. The
/// difference between the two is this process's own work, not the Redis hop —
/// see the module docs.
pub const ENGINE_US: HeaderName = HeaderName::from_static("x-cex-engine-us");

pub async fn timing_middleware(req: Request, next: Next) -> Response {
    let started = std::time::Instant::now();
    let (mut response, engine_micros) = measure_engine(next.run(req)).await;
    let total_micros = started.elapsed().as_micros() as u64;

    let headers = response.headers_mut();
    // Both always set, even at zero. A missing header and a fast one must not
    // look the same to the reader.
    if let Ok(value) = HeaderValue::from_str(&total_micros.to_string()) {
        headers.insert(SERVER_US, value);
    }
    if let Ok(value) = HeaderValue::from_str(&engine_micros.to_string()) {
        headers.insert(ENGINE_US, value);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sums_every_record_inside_the_scope() {
        let (out, micros) = measure_engine(async {
            record_engine(40);
            record_engine(2);
            "done"
        })
        .await;

        assert_eq!(out, "done");
        assert_eq!(micros, 42);
    }

    #[tokio::test]
    async fn recording_outside_a_scope_is_a_no_op() {
        // A background task or a test calling the loopback directly has no
        // request to attribute time to. That must be silent, not a panic.
        record_engine(99);
    }

    #[tokio::test]
    async fn one_scope_cannot_see_another() {
        let (_, first) = measure_engine(async { record_engine(10) }).await;
        let (_, second) = measure_engine(async { record_engine(1) }).await;

        assert_eq!((first, second), (10, 1));
    }
}
