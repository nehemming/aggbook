use super::model::{CombinedSummary, ContributorSnapshot};
use aggcommon::{join_observer::ObserveJoinHandle, proto::orderbook::Summary};
use anyhow::Result;
use arc_swap::ArcSwap;
use std::sync::Arc;
use tokio::{
    sync::{Mutex as TokioMutex, broadcast, mpsc, watch},
    task::JoinHandle,
};

/// Default backlog size for the publisher's message channel.
const BACKLOG_SIZE: usize = 100; //100ms x 10 sec

/// Represents errors related to the summary.
#[derive(Debug, Clone, Copy)]
pub enum SummaryError {
    Shutdown,
    // Other error variants if needed in the future
}

/// Type alias for the result of a summary operation.
type SummaryResult = Result<Option<Summary>, SummaryError>;

/// Represents a publisher for order book summaries.
///
/// This struct handles updates, broadcasting changes, and managing shutdown logic.
///
/// # Type Parameters
/// - `N`: Maximum depth of the order book.
#[derive(Debug)]
pub struct Publisher<const N: usize> {
    latest: ArcSwap<SummaryResult>,
    latest_channel: watch::Sender<()>,
    source_tx: mpsc::Sender<ContributorSnapshot<N>>,
    handle: TokioMutex<Option<JoinHandle<()>>>,
    shutdown_tx: broadcast::Sender<()>,
}

impl<const N: usize> Publisher<N> {
    /// Returns the channel for sending updates.
    pub fn source_tx(&self) -> mpsc::Sender<ContributorSnapshot<N>> {
        self.source_tx.clone()
    }

    /// Provides a receiver for the latest updates.
    pub fn watch_latest(&self) -> watch::Receiver<()> {
        self.latest_channel.subscribe()
    }

    /// Retrieves the latest summary.
    pub fn latest_summary(&self) -> Arc<SummaryResult> {
        self.latest.load_full()
    }

    /// Starts the publisher and returns an `Arc` instance.
    ///
    /// This function initializes the publisher, sets up the necessary channels,
    /// and spawns tasks to handle updates and shutdown logic.
    ///
    /// # Returns
    /// - An `Arc`-wrapped instance of the `Publisher`.
    pub fn start() -> Arc<Publisher<N>> {
        let (source_tx, mut source_rx) = mpsc::channel::<ContributorSnapshot<N>>(BACKLOG_SIZE);
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let (latest_channel, _) = watch::channel(());

        // Create a new publisher instance wrapped in an `Arc`.
        let publisher = Arc::new(Publisher {
            latest: ArcSwap::from_pointee(Result::Ok(None)),
            latest_channel,
            source_tx: source_tx.clone(),
            handle: TokioMutex::new(None),
            shutdown_tx,
        });

        let publisher_clone = Arc::clone(&publisher);
        let mut shutdown_rx = publisher.shutdown_tx.subscribe();

        // Spawn a task to run the publisher's main loop.
        let handle = tokio::spawn(async move {
            let _ = Publisher::local_run(publisher_clone, &mut source_rx, &mut shutdown_rx).await;
            source_rx.close(); // Close the source channel when the task completes.
        });

        // Store the handle for the spawned task in the publisher.
        let publisher_handle = Arc::clone(&publisher);
        tokio::spawn(async move {
            let mut guard = publisher_handle.handle.lock().await;
            *guard = Some(handle);
        })
        .observe("start guard");

        publisher
    }

    /// Closes the publisher and stops receiving updates.
    ///
    /// This function sends a shutdown signal to the publisher, which stops
    /// the processing of updates and gracefully shuts down the associated tasks.
    pub fn close(&self) {
        // Close the source channel to stop receiving updates.
        let _ = self.shutdown_tx.send(());
    }

    /// Waits for the publisher task to complete.
    ///
    /// This function blocks until the publisher's main task has finished execution.
    ///
    /// # Returns
    /// - `Result<()>`: Indicates success or failure of the task.
    pub async fn wait_complete(&self) -> Result<()> {
        let mut guard = self.handle.lock().await;

        if let Some(handle) = guard.take() {
            // Await the completion of the publisher's main task.
            handle
                .await
                .map_err(|e| anyhow::anyhow!("Publisher task failed: {}", e))?;
        }

        Ok(())
    }

    /// Runs the publisher task locally.
    ///
    /// This function processes updates from the source channel and handles
    /// shutdown signals. It updates the latest summary and notifies subscribers
    /// of changes.
    ///
    /// # Arguments
    /// - `self`: Reference to the publisher.
    /// - `source_rx`: Receiver for updates.
    /// - `shutdown_rx`: Receiver for shutdown signals.
    ///
    /// # Returns
    /// - `Result<()>`: Indicates success or failure of the task.
    async fn local_run(
        self: Arc<Self>,
        source_rx: &mut mpsc::Receiver<ContributorSnapshot<N>>,
        shutdown_rx: &mut broadcast::Receiver<()>,
    ) -> Result<()> {
        let mut combined: CombinedSummary<N> = CombinedSummary::<N>::new();
        loop {
            tokio::select! {
                    Some(snapshot) = source_rx.recv() => {
                    // Update the combined summary with the received snapshot.
                    combined.update_snapshot(snapshot);
                    let summary = combined.to_proto();

                    // Store the latest summary and notify subscribers.
                    self.latest.store(Arc::new(Ok(Some(summary))));
                    self.latest_channel.send(()).ok();
                }

                _ = shutdown_rx.recv() => {
                    // Break the loop if a shutdown signal is received.
                    break;
                }
            }
        }

        // Gracefully close the publisher by setting the latest summary to an error state.
        self.latest.store(Arc::new(Err(SummaryError::Shutdown)));
        self.latest_channel.send(()).ok();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::model::ContributorSnapshot;
    use aggcommon::sources::SourceId;
    use std::time::Duration;

    fn dummy_snapshot<const N: usize>(
        source: SourceId,
        price: f64,
        amount: f64,
    ) -> ContributorSnapshot<N> {
        let mut snap = ContributorSnapshot::new(source, 123456);
        snap.push_bid(price, amount);
        snap
    }

    #[tokio::test]
    async fn publisher_receives_and_updates_summary() {
        let publisher = Publisher::<5>::start();
        let tx = publisher.source_tx();

        let update = dummy_snapshot(SourceId::Binance, 100.0, 1.0);

        tx.send(update).await.unwrap();

        let mut rx = publisher.watch_latest();

        // Wait for the update to propagate
        for _ in 0..5 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;

            if rx.changed().await.is_ok() {
                let summary = publisher.latest_summary();
                if let Ok(Some(s)) = summary.as_ref() {
                    if !s.bids.is_empty() {
                        assert_eq!(s.bids[0].price, 100.0);
                        assert_eq!(s.bids[0].amount, 1.0);
                        return;
                    }
                }
            }
        }

        panic!("No update received or summary was empty");
    }

    #[tokio::test]
    async fn publisher_shutdown_triggers_summary_error() {
        let publisher = Publisher::<5>::start();
        let mut rx = publisher.watch_latest();

        publisher.close();

        // wait for shutdown and notification
        assert!(rx.changed().await.is_ok());

        let result = publisher.latest_summary();
        assert!(result.is_err(), "Expected summary to be shutdown");
    }

    #[tokio::test]
    async fn publisher_wait_complete_joins_successfully() {
        let publisher = Publisher::<5>::start();
        publisher.close();

        // Allow time for shutdown and join
        tokio::time::sleep(Duration::from_millis(50)).await;

        let result = publisher.wait_complete().await;
        assert!(result.is_ok());
    }
}
