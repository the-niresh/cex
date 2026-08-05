//! Tail a market data feed from the command line.
//!
//! ```text
//! cargo run -p cex-ws --example tail -- trades@BTC_USDT depth@BTC_USDT
//! cargo run -p cex-ws --example tail -- orders --token "$TOKEN"
//! ```
//!
//! Every frame the server sends is printed as it arrives. Exits when the
//! connection closes, or after `--seconds` if given.

use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut url = std::env::var("CEX_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:8081/ws".into());
    let mut token: Option<String> = None;
    let mut seconds: Option<u64> = None;
    let mut channels: Vec<String> = Vec::new();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--token" => token = args.next(),
            "--url" => url = args.next().unwrap_or(url),
            "--seconds" => seconds = args.next().and_then(|v| v.parse().ok()),
            other => channels.push(other.to_string()),
        }
    }
    if channels.is_empty() {
        eprintln!("usage: tail [--url URL] [--token T] [--seconds N] CHANNEL...");
        std::process::exit(2);
    }

    let (mut socket, _) = tokio_tungstenite::connect_async(&url).await?;
    eprintln!("connected to {url}");

    if let Some(token) = token {
        let auth = serde_json::json!({"op": "auth", "token": token});
        socket.send(Message::Text(auth.to_string())).await?;
    }
    let sub = serde_json::json!({"op": "subscribe", "channels": channels});
    socket.send(Message::Text(sub.to_string())).await?;

    let pump = async {
        while let Some(frame) = socket.next().await {
            match frame {
                Ok(Message::Text(text)) => println!("{text}"),
                Ok(Message::Close(reason)) => {
                    eprintln!("closed by server: {reason:?}");
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("connection error: {e}");
                    break;
                }
            }
        }
    };

    match seconds {
        Some(n) => {
            let _ = tokio::time::timeout(Duration::from_secs(n), pump).await;
        }
        None => pump.await,
    }
    Ok(())
}
