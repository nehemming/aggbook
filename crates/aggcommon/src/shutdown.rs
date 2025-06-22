use crate::join_observer::ObserveJoinHandle;

use tokio::sync::broadcast;
use tracing::{error, info};

/// Type alias for the shutdown signal sender.
///
/// `broadcast::Sender<()>` allows multiple receivers to listen for shutdown notifications.
/// All receivers get notified when `send(())` is called.
pub type ShutdownSignaller = broadcast::Sender<()>;

/// Type alias for a shutdown signal receiver.
///
/// Each task that wants to listen for shutdown gets its own receiver via `shutdown_tx.subscribe()`.
pub type ShutdownReceiver = broadcast::Receiver<()>;

/// Creates a shutdown signal broadcaster that listens for a Ctrl+C interrupt (SIGINT).
///
/// This function sets up a Tokio task that listens for a Ctrl+C signal using
/// `tokio::signal::ctrl_c`. When the signal is received, it broadcasts a shutdown
/// message (`()`) to all subscribed receivers via a `broadcast::Sender`.
///
/// This is typically used to trigger graceful shutdown across multiple running tasks.
///
/// # Returns
///
/// A `ShutdownSignaller` (a `broadcast::Sender<()>`) that can be cloned or used
/// to create receivers with `.subscribe()`.
///
/// # Example
///
/// ```no_run
/// use aggcommon::shutdown::ctrl_c_shutdown_signal;
///
/// let shutdown_tx = ctrl_c_shutdown_signal();
/// let mut shutdown_rx = shutdown_tx.subscribe();
///
/// tokio::spawn(async move {
///     shutdown_rx.recv().await.ok();
///     println!("Shutting down...");
/// });
/// ```
pub fn ctrl_c_shutdown_signal() -> ShutdownSignaller {
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    // spawn the ctrl-c handler
    let shutdown_tx_spawned = shutdown_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = tokio::signal::ctrl_c().await {
            error!("Failed to listen for shutdown signal: {}", e);
        } else {
            info!("Shutdown signal received");
        }

        let _ = shutdown_tx_spawned.send(()); // Notify all subscribers
    })
    .observe("ctrl-c handler");

    shutdown_tx
}

/// Creates a shutdown channel for manual shutdown signals.
pub fn shutdown_channel() -> (ShutdownSignaller, ShutdownReceiver) {
    let (tx, rx) = broadcast::channel::<()>(1);
    (tx, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ctrl_c_shutdown_signal_manual() {
        let shutdown_tx = ctrl_c_shutdown_signal();
        let mut shutdown_rx = shutdown_tx.subscribe();
        let _ = shutdown_tx.send(()); // simulate ctrl-c

        let result = shutdown_rx.recv().await;
        assert!(result.is_ok());
    }
}
