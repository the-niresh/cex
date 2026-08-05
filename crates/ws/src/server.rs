//! The WebSocket surface.
//!
//! One task per connection, holding its own [`Session`] and its own cursor into
//! the shared broadcast ring.
//!
//! ## What happens to a subscriber that cannot keep up
//!
//! It is disconnected, and told why. A broadcast receiver that falls behind the
//! ring reports [`RecvError::Lagged`] rather than blocking the sender, so a
//! stalled connection can never hold up the others — but it also means the
//! updates it missed are simply gone. Carrying on would hand that client a feed
//! with a silent hole in it, and an order book rebuilt from a feed with a hole
//! is wrong without ever looking wrong. Closing forces a reconnect and a fresh
//! snapshot, which is the only honest option.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use cex_auth::Tokens;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};

use crate::route::Update;
use crate::session::Session;
use crate::wire::{Channel, ClientMessage, ServerMessage};

#[derive(Clone)]
pub struct AppState {
    updates: broadcast::Sender<Arc<Update>>,
    tokens: Arc<Tokens>,
}

impl AppState {
    pub fn new(updates: broadcast::Sender<Arc<Update>>, tokens: Tokens) -> Self {
        AppState {
            updates,
            tokens: Arc::new(tokens),
        }
    }
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/ws", get(upgrade))
        .with_state(state)
}

async fn upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| serve(socket, state))
}

async fn serve(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();
    let mut updates = state.updates.subscribe();
    let mut session = Session::new();

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let Some(Ok(message)) = incoming else { break };
                match message {
                    Message::Text(text) => {
                        let reply = handle(&mut session, &state.tokens, &text);
                        let Ok(json) = serde_json::to_string(&reply) else { break };
                        if sink.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    // Ping/Pong are answered by axum; binary frames are not part
                    // of this protocol.
                    _ => {}
                }
            }

            update = updates.recv() => {
                match update {
                    Ok(update) => {
                        if session.wants(&update)
                            && sink
                                .send(Message::Text(update.payload.clone().into()))
                                .await
                                .is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(missed)) => {
                        warn!(missed, "subscriber fell behind, closing so it resyncs");
                        let reply = ServerMessage::Error {
                            error: format!(
                                "fell behind by {missed} updates; reconnect and resync"
                            ),
                        };
                        if let Ok(json) = serde_json::to_string(&reply) {
                            let _ = sink.send(Message::Text(json.into())).await;
                        }
                        break;
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }

    debug!("connection closed");
    let _ = sink.close().await;
}

/// Apply one client message and produce the reply.
fn handle(session: &mut Session, tokens: &Tokens, text: &str) -> ServerMessage {
    let message: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            return ServerMessage::Error {
                error: format!("could not decode: {e}"),
            }
        }
    };

    match message {
        ClientMessage::Auth { token } => match session.authenticate(tokens, &token) {
            Ok(_) => ServerMessage::Authenticated,
            Err(e) => ServerMessage::Error {
                error: e.to_string(),
            },
        },

        ClientMessage::Subscribe { channels } => {
            // All or nothing. A partial subscription would leave the client
            // believing it is watching something it is not.
            let mut parsed = Vec::with_capacity(channels.len());
            for name in &channels {
                match name.parse::<Channel>() {
                    Ok(c) => parsed.push(c),
                    Err(e) => {
                        return ServerMessage::Error {
                            error: e.to_string(),
                        }
                    }
                }
            }
            for channel in &parsed {
                if channel.is_private() && session.user().is_none() {
                    return ServerMessage::Error {
                        error: format!("channel {channel} requires an authenticated connection"),
                    };
                }
            }
            for channel in parsed {
                if let Err(e) = session.subscribe(channel) {
                    return ServerMessage::Error {
                        error: e.to_string(),
                    };
                }
            }
            ServerMessage::Subscribed {
                channels: session.subscriptions(),
            }
        }

        ClientMessage::Unsubscribe { channels } => {
            for name in &channels {
                match name.parse::<Channel>() {
                    Ok(c) => session.unsubscribe(&c),
                    Err(e) => {
                        return ServerMessage::Error {
                            error: e.to_string(),
                        }
                    }
                }
            }
            ServerMessage::Subscribed {
                channels: session.subscriptions(),
            }
        }
    }
}
