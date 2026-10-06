//! src/shutdown.rs
#[cfg(unix)]
pub async fn terminate_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm =
        signal(SignalKind::terminate()).expect("Failed to register the SIGTERM handler");

    tokio::select! {
        _ = sigterm.recv() => tracing::info!("Received SIGTERM, starting graceful shutdown"),
        r = tokio::signal::ctrl_c() => {
            r.expect("Failed to listen for Ctrl-C");
            tracing::info!("Received Ctrl-C, starting graceful shutdown");
        }
    }
}

#[cfg(not(unix))]
pub async fn terminate_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for Ctrl-C");
    tracing::info!("Received Ctrl-C, starting graceful shutdown");
}
