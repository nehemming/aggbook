use super::publisher::Publisher;
use crate::service::{model::SummarySourceFactoryMap, publisher::SummaryError};
use aggcommon::{
    instrument::{Instrument, InstrumentMap},
    join_observer::ObserveJoinHandle,
    proto::orderbook::Summary,
    shutdown::{ShutdownReceiver, ShutdownSignaller, shutdown_channel},
};
use anyhow::Result;
use std::sync::Arc;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::{
    select,
    sync::{Mutex as TokioMutex, Notify, mpsc},
    task::JoinHandle,
    time::{Duration, sleep},
};
use tonic::Status;
use tracing::{debug, error, info, warn};

#[cfg(test)]
const GRACE_PERIOD_SECS: u64 = 1;

#[cfg(not(test))]
const GRACE_PERIOD_SECS: u64 = 10;

/// Represents a feed for managing order book updates.
///
/// This struct handles subscriptions, publishing updates, and shutdown logic.
///
/// # Type Parameters
/// - `N`: Maximum depth of the order book.
pub struct Feed<const N: usize> {
    instrument: Instrument,
    publisher: Arc<Publisher<N>>,
    shutdown_guard: TokioMutex<()>,
    subscriptions: AtomicUsize,
    cancel_notify: Arc<Notify>,
    source_tasks: TokioMutex<Vec<JoinHandle<()>>>,
    feed_shutdown_tx: ShutdownSignaller,
}

impl<const N: usize> Feed<N> {
    /// Creates a new feed instance.
    ///
    /// # Arguments
    /// - `instrument`: The trading instrument.
    /// - `publisher`: Publisher for updates.
    /// - `source_tasks`: Tasks for summary sources.
    fn new(
        instrument: Instrument,
        publisher: Arc<Publisher<N>>,
        source_tasks: Vec<JoinHandle<()>>,
        feed_shutdown_tx: ShutdownSignaller,
    ) -> Self {
        Self {
            instrument,
            publisher,
            shutdown_guard: TokioMutex::new(()),
            subscriptions: AtomicUsize::new(0),
            cancel_notify: Arc::new(Notify::new()),
            source_tasks: TokioMutex::new(source_tasks),
            feed_shutdown_tx,
        }
    }

    /// Publishes updates to subscribers.
    ///
    /// This function listens for changes in the publisher's latest summary and sends
    /// updates to the provided channel. It also handles shutdown signals and
    /// unsubscribes when necessary.
    ///
    /// # Arguments
    /// - `self`: Reference to the feed.
    /// - `manager`: Reference to the manager.
    /// - `sender`: Channel for sending updates.
    ///
    /// # Behavior
    /// - Sends the initial summary if available.
    /// - Listens for changes in the publisher's latest summary and sends updates.
    /// - Handles shutdown signals and unsubscribes when necessary.
    /// - Drops the sender when unsubscribed or shutdown is triggered.
    pub async fn publish(
        self: Arc<Self>,
        manager: Arc<Manager<N>>,
        sender: mpsc::Sender<Result<Summary, Status>>,
    ) {
        self.on_subscribe();

        let mut shutdown = self.feed_shutdown_tx.subscribe();

        // Send initial summary if already available
        if let Ok(Some(summary)) = self.publisher.latest_summary().as_ref() {
            if !send_with_shutdown(&mut sender.clone(), &mut shutdown, Ok(summary.clone()))
                .await
                .unwrap_or_else(|e| {
                    error!("Unable to publish initial: {}", e);
                    false
                })
            {
                self.on_unsubscribe(manager);
                drop(sender);
                return;
            }
        }

        let mut rx = self.publisher.watch_latest();
        loop {
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        break;
                    }

                    match self.publisher.latest_summary().as_ref() {
                        Ok(Some(summary)) => {
                            let summary = summary.clone();
                            if !send_with_shutdown(&mut sender.clone(), &mut shutdown, Ok(summary)).await.unwrap_or_else(|e| {
                                error!("Unable to publish: {}", e);
                                false
                            }) {
                                break;
                            }
                        }
                        Ok(None) => continue,
                        Err(SummaryError::Shutdown) => {
                            break;
                        }
                    }
                }
                _ = shutdown.recv() => {
                    break;
                }
            }
        }
        self.on_unsubscribe(manager);
        drop(sender);
    }

    /// Shuts down the feed and its associated tasks.
    ///
    /// This function ensures a graceful shutdown of the feed by:
    /// - Acquiring a lock to prevent concurrent shutdowns.
    /// - Sending a shutdown signal to the publisher and sources.
    /// - Waiting for all associated tasks to complete.
    /// - Logging the shutdown process.
    pub async fn shutdown(&self) {
        // Acquire lock to prevent concurrent shutdown
        let _guard = self.shutdown_guard.lock().await;

        let _ = self.feed_shutdown_tx.send(());
        info!("Feeds closing for {}", self.instrument.symbol());
        self.publisher.close();

        let mut tasks = self.source_tasks.lock().await;
        let handles = std::mem::take(&mut *tasks);

        for handle in handles {
            match handle.await {
                Ok(_) => debug!("Source task exited cleanly"),
                Err(e) => warn!("Source task panicked: {:?}", e),
            }
        }

        // Then optionally wait for publisher, if owner
        if let Err(e) = self.publisher.wait_complete().await {
            warn!("Publisher wait_complete failed: {:?}", e);
        }
    }

    /// Handles subscription logic.
    pub fn on_subscribe(&self) {
        let count = self.subscriptions.fetch_add(1, Ordering::SeqCst);
        if count == 0 {
            // cancel pending shutdown if any
            self.cancel_notify.notify_waiters();
        }
    }

    /// Handles unsubscription logic.
    pub fn on_unsubscribe(self: Arc<Self>, manager: Arc<Manager<N>>) {
        let count = self.subscriptions.fetch_sub(1, Ordering::SeqCst);

        if count == 1 {
            // Reached zero subscribers
            let cancel_notify = self.cancel_notify.clone();
            let feed = Arc::clone(&self);
            let mut shutdown_rx = manager.shutdown_subscriber(); // manager-wide shutdown signal

            tokio::spawn(async move {
                let notified = tokio::select! {
                    _ = sleep(Duration::from_secs(GRACE_PERIOD_SECS)) => false,
                    _ = cancel_notify.notified() => true,
                    _ = shutdown_rx.recv() => false, // global shutdown overrides grace
                };

                if !notified {
                    info!(
                        "No subscribers left for feed {}, requesting shutdown",
                        self.instrument.symbol()
                    );
                    manager.remove_feed_if_unused(&feed).await;
                }
            })
            .observe("unsubscribe handler");
        }
    }
}

/// Represents a manager for handling multiple feeds.
///
/// This struct manages feed lifecycle, subscriptions, and shutdown logic.
///
/// # Type Parameters
/// - `N`: Maximum depth of the order book.
pub struct Manager<const N: usize> {
    feeds: TokioMutex<HashMap<String, Arc<Feed<N>>>>,
    factory_map: SummarySourceFactoryMap<N>,
    instrument_map: InstrumentMap,
    is_shutting_down: AtomicBool,
    shutdown_tx: ShutdownSignaller,
}

impl<const N: usize> Manager<N> {
    /// Creates a new manager instance.
    ///
    /// # Arguments
    /// - `shutdown_tx`: Shutdown signal transmitter.
    /// - `instrument_map`: Map of trading instruments.
    /// - `factory_map`: Factory map for summary sources.
    pub fn new(
        shutdown_tx: ShutdownSignaller,
        instrument_map: InstrumentMap,
        factory_map: SummarySourceFactoryMap<N>,
    ) -> Arc<Self> {
        let mut shutdown_rx: ShutdownReceiver = shutdown_tx.subscribe();
        let manager = Arc::new(Self {
            feeds: TokioMutex::new(HashMap::new()),
            instrument_map,
            factory_map,
            is_shutting_down: AtomicBool::new(false),
            shutdown_tx,
        });

        tokio::spawn({
            let manager = Arc::clone(&manager);
            async move {
                if shutdown_rx.recv().await.is_ok() {
                    manager.shutdown().await;
                }
            }
        });

        manager
    }

    /// Opens a feed for a given symbol.
    ///
    /// # Arguments
    /// - `symbol`: The trading pair symbol.
    ///
    /// # Returns
    /// - `Result<Arc<Feed<N>>>`: The feed instance.
    pub async fn open(&self, symbol: &str) -> Result<Arc<Feed<N>>> {
        let mut feeds = self.feeds.lock().await;

        if self.is_shutting_down.load(Ordering::SeqCst) {
            return Err(anyhow::anyhow!("Manager is shutting down"));
        }

        if let Some(feed) = feeds.get(symbol) {
            info!("Reusing existing feed for symbol: {}", symbol);
            return Ok(feed.clone());
        }

        let instrument = self
            .instrument_map
            .get(symbol)
            .ok_or_else(|| anyhow::anyhow!("Instrument not found: {}", symbol))?
            .clone();

        let symbol: String = symbol.to_string();

        info!("Opening new feed for symbol: {}", symbol);
        let publisher = Publisher::<N>::start();
        let (feed_shutdown_tx, _) = shutdown_channel();
        let source_tasks: Vec<JoinHandle<()>> = self
            .start_summary_sources(
                &instrument,
                Arc::clone(&publisher),
                feed_shutdown_tx.clone(),
            )
            .await?;

        let feed = Arc::new(Feed::new(
            instrument,
            publisher,
            source_tasks,
            feed_shutdown_tx,
        ));

        feeds.insert(symbol.clone(), feed.clone());

        Ok(feed)
    }

    /// Returns a shutdown subscriber.
    pub fn shutdown_subscriber(&self) -> ShutdownReceiver {
        self.shutdown_tx.subscribe()
    }

    /// Shuts down the manager and all feeds.
    pub async fn shutdown(&self) {
        self.is_shutting_down.store(true, Ordering::SeqCst);

        let mut feeds = self.feeds.lock().await;
        let drained: Vec<Arc<Feed<N>>> = feeds.drain().map(|(_, f)| f).collect();
        drop(feeds);

        for feed in drained {
            feed.shutdown().await;
        }
    }

    /// Removes a feed if it is unused.
    ///
    /// # Arguments
    /// - `feed`: Reference to the feed.
    pub async fn remove_feed_if_unused(&self, feed: &Arc<Feed<N>>) {
        let symbol = feed.instrument.symbol(); // adjust based on your actual getter

        let mut feeds = self.feeds.lock().await;

        if let Some(current) = feeds.get(symbol) {
            if Arc::ptr_eq(current, feed) && feed.subscriptions.load(Ordering::SeqCst) == 0 {
                feeds.remove(symbol);
                drop(feeds); // release lock before shutdown
                feed.shutdown().await;
            }
        }
    }

    /// Starts summary sources for a given instrument.
    ///
    /// # Arguments
    /// - `instrument`: The trading instrument.
    /// - `publisher`: Publisher for updates.
    ///
    /// # Returns
    /// - `Result<Vec<JoinHandle<()>>>`: Tasks for summary sources.
    async fn start_summary_sources(
        &self,
        instrument: &Instrument,
        publisher: Arc<Publisher<N>>,
        feed_shutdown_tx: ShutdownSignaller,
    ) -> Result<Vec<JoinHandle<()>>> {
        let summary_sources = self
            .factory_map
            .create_summary_sources(instrument, &feed_shutdown_tx)?;

        let mut source_tasks = Vec::new();

        for source in &summary_sources {
            let mut rx = source.watch_latest_change();
            let source_id = source.source_id();
            let source_tx = publisher.source_tx();
            let source = Arc::clone(source);
            let mut shutdown: ShutdownReceiver = feed_shutdown_tx.subscribe();

            let handle = tokio::spawn(async move {
                loop {
                    match source.latest_snapshot().as_ref() {
                        Some(Ok(snapshot)) => {
                            if !send_with_shutdown(&mut source_tx.clone(), &mut shutdown, *snapshot)
                                .await
                                .unwrap_or_else(|e| {
                                    error!("Failed to send update from {}: {}", source_id, e);
                                    false
                                })
                            {
                                break;
                            }
                        }
                        Some(Err(e)) => {
                            error!("Error in source {}: {}", source_id, e);
                        }
                        None => {
                            // Not ready yet
                        }
                    }

                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            });

            source_tasks.push(handle);
        }

        Ok(source_tasks)
    }
}

/// Attempts to send a message to the client,
/// or returns `Ok(false)` if shutdown is signaled.
async fn send_with_shutdown<T>(
    sender: &mut mpsc::Sender<T>,
    shutdown: &mut ShutdownReceiver,
    item: T,
) -> Result<bool>
where
    T: Sync + Send + 'static,
{
    select! {
        result = sender.send(item) => {
            result.map(|_| true).map_err(|e| e.into())
        }
        _ = shutdown.recv() => {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::model::{ContributorSnapshot, OrderBookSummarySource, SummarySource};
    use aggcommon::instrument::{Instrument, InstrumentMap, SourceSymbol};
    use aggcommon::shutdown::{ShutdownReceiver, shutdown_channel};
    use aggcommon::sources::SourceId;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use tokio::sync::watch;

    #[derive(Debug)]
    struct MockSource<const N: usize> {
        source_id: SourceId,
        latest: Arc<Option<Result<ContributorSnapshot<N>>>>,
        tx: watch::Sender<()>,
    }

    impl<const N: usize> MockSource<N> {
        fn new_shared(source_id: SourceId) -> SummarySource<N> {
            let (tx, _rx) = watch::channel(());
            Arc::new(Self {
                source_id,
                latest: Arc::new(None),
                tx,
            })
        }
    }

    impl<const N: usize> OrderBookSummarySource<N> for MockSource<N> {
        fn source_id(&self) -> SourceId {
            self.source_id
        }

        fn watch_latest_change(&self) -> watch::Receiver<()> {
            self.tx.subscribe()
        }

        fn latest_snapshot(&self) -> Arc<Option<Result<ContributorSnapshot<N>>>> {
            self.latest.clone()
        }
    }

    fn mock_factory_map<const N: usize>() -> SummarySourceFactoryMap<N> {
        SummarySourceFactoryMap::from_vec(vec![(
            SourceId::Binance,
            Box::new(|_symbol: &str, _shutdown: ShutdownReceiver| {
                Ok(MockSource::<N>::new_shared(SourceId::Binance) as SummarySource<N>)
            }),
        )])
    }

    fn instrument_with_symbol(symbol: &str) -> InstrumentMap {
        let source_symbols = vec![SourceSymbol {
            source: SourceId::Binance,
            symbol: symbol.to_string(),
        }];
        let instrument = Instrument::new(symbol.to_string(), source_symbols);
        InstrumentMap::new(vec![instrument])
    }

    #[tokio::test]
    async fn manager_creates_and_reuses_feed() {
        let (shutdown, _) = shutdown_channel();
        let factory_map = mock_factory_map::<10>();
        let instrument_map = instrument_with_symbol("ethbtc-test-1");

        let manager = Manager::new(shutdown, instrument_map, factory_map);
        let feed1 = manager.open("ethbtc-test-1").await.unwrap();
        let feed2 = manager.open("ethbtc-test-1").await.unwrap();

        assert!(Arc::ptr_eq(&feed1, &feed2));
    }

    #[tokio::test]
    async fn feed_unsubscribe_triggers_removal() {
        let (shutdown, _) = shutdown_channel();
        let factory_map = mock_factory_map::<10>();
        let instrument_map = instrument_with_symbol("ethbtc-test-2");

        let manager = Manager::new(shutdown, instrument_map, factory_map);
        let feed = manager.open("ethbtc-test-2").await.unwrap();

        feed.on_subscribe();
        assert_eq!(feed.subscriptions.load(Ordering::SeqCst), 1);

        feed.on_unsubscribe(manager.clone());

        // Wait longer than GRACE_PERIOD_SECS (10s) is overkill in tests
        tokio::time::sleep(Duration::from_secs(2)).await;

        let feeds = manager.feeds.lock().await;
        assert!(feeds.get("ethbtc-test-2").is_none());
    }

    #[tokio::test]
    async fn manager_shutdown_drains_all_feeds() {
        let (shutdown, shutdown_rx) = shutdown_channel();
        let factory_map = mock_factory_map::<10>();
        let instrument_map = instrument_with_symbol("ethbtc-test-3");

        let manager = Manager::new(shutdown.clone(), instrument_map, factory_map);
        let _feed = manager.open("ethbtc-test-3").await.unwrap();

        let _ = shutdown.send(()); // ignore send result
        let _ = shutdown_rx;

        tokio::time::sleep(Duration::from_millis(100)).await;

        let feeds = manager.feeds.lock().await;
        assert!(feeds.is_empty(), "Feeds should be drained on shutdown");
    }
}
