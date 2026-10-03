//! How the API listens on a TCP port.

use std::net::SocketAddr;

use axum::Router;
use tokio::net::TcpListener;

/// Serve `app` the way `main` does: with `ConnectInfo` wired in.
pub async fn serve(listener: TcpListener, app: Router) -> std::io::Result<()> {
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}
