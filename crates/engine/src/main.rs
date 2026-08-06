//! The engine process.

use anyhow::Result;
use cex_engine::{Config, Runner};
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = Config::from_env();
    let mut runner = Runner::boot(cfg).await?;

    // Run until the engine stops itself — losing the stream lock is the main
    // way that happens — or until the operator asks it to stop.
    let outcome = tokio::select! {
        result = runner.run() => result,
        _ = stop_requested() => {
            info!("stop requested, shutting down");
            Ok(())
        }
    };

    // Snapshot before letting go, so the replacement resumes from here instead
    // of replaying the log from the last periodic snapshot.
    if let Err(e) = runner.snapshot().await {
        error!(error = %e, "final snapshot failed; recovery will replay further back");
    }
    // Always attempt this, including after `run` returned an error. If the lock
    // was lost the release is a no-op by design — it will not take the stream
    // away from whichever engine legitimately holds it now.
    if let Err(e) = runner.shutdown().await {
        error!(error = %e, "could not release the lock; it will lapse on its own");
    }

    outcome
}

/// SIGTERM from an orchestrator, or Ctrl-C from a terminal.
async fn stop_requested() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            // Nothing to wait for, so never resolve and let Ctrl-C win the race.
            Err(e) => {
                error!(error = %e, "could not listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
