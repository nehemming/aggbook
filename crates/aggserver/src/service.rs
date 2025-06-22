pub mod manager;
pub mod model;
mod publisher;
pub mod sources;

use crate::service::manager::Manager;
use crate::stream::into_grpc_stream;
use aggcommon::join_observer::ObserveJoinHandle;
use aggcommon::proto::orderbook::SummaryRequest;
use aggcommon::proto::orderbook::{
    Empty, Summary,
    orderbook_aggregator_server::{OrderbookAggregator, OrderbookAggregatorServer},
};
use std::{pin::Pin, sync::Arc};
use tokio::sync::mpsc;
use tonic::{Request, Response, Status};
use tracing::info;

const CLIENT_BACKLOG_SIZE: usize = 1; // Only send one pending message at at time to the client

/// Represents the gRPC service for the order book aggregator.
///
/// This service handles client requests for order book summaries and manages
/// the underlying data streams.
///
/// # Type Parameters
/// - `N`: Maximum depth of the order book.
pub struct AggregatorService<const N: usize> {
    manager: Arc<Manager<N>>, // Manages the lifecycle of feeds and subscriptions.
    default_symbol: String,   // Default trading symbol for the service.
}

/// Type alias for the gRPC service implementation.
///
/// This alias simplifies the usage of the `OrderbookAggregatorServer` type.
pub type GrpcService<const N: usize> = OrderbookAggregatorServer<AggregatorService<N>>;

impl<const N: usize> AggregatorService<N> {
    /// Creates a new instance of the `AggregatorService`.
    ///
    /// # Arguments
    /// - `manager`: The manager responsible for handling feeds and subscriptions.
    /// - `default_symbol`: The default trading symbol for the service.
    ///
    /// # Returns
    /// - A new instance of `AggregatorService`.
    fn new(manager: Arc<Manager<N>>, default_symbol: String) -> Self {
        AggregatorService {
            manager,
            default_symbol,
        }
    }

    /// Creates a gRPC server instance for the aggregator service.
    ///
    /// # Arguments
    /// - `manager`: The manager responsible for handling feeds and subscriptions.
    /// - `default_symbol`: The default trading symbol for the service.
    ///
    /// # Returns
    /// - A gRPC server instance.
    pub fn create_server(manager: Arc<Manager<N>>, default_symbol: String) -> GrpcService<N> {
        OrderbookAggregatorServer::new(Self::new(manager, default_symbol))
    }

    /// Opens a summary stream for a given trading symbol.
    ///
    /// # Arguments
    /// - `symbol`: The trading symbol for which to open the stream.
    ///
    /// # Returns
    /// - A receiver channel for the summary stream.
    async fn open_summary_stream(
        &self,
        symbol: &str,
    ) -> Result<mpsc::Receiver<Result<Summary, Status>>, Status> {
        let (tx, rx) = mpsc::channel(CLIENT_BACKLOG_SIZE); // Create a channel with a fixed backlog size.

        // Attempt to open a feed for the given symbol using the manager.
        let feed = self
            .manager
            .open(symbol)
            .await
            .map_err(|e| Status::internal(format!("Failed to open stream: {}", e)))?;

        let manager = Arc::clone(&self.manager); // Clone the manager for use in the spawned task.

        // Spawn a task to publish updates from the feed to the channel.
        tokio::spawn(feed.clone().publish(manager, tx)).observe("summary stream publisher");

        Ok(rx) // Return the receiver end of the channel.
    }
}

#[tonic::async_trait]
impl<const N: usize> OrderbookAggregator for AggregatorService<N> {
    /// Type alias for the default book summary stream.
    type BookSummaryStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<Summary, Status>> + Send + 'static>>;

    /// Type alias for the book summary stream for a specific symbol.
    type BookSummaryForSymbolStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<Summary, Status>> + Send + 'static>>;

    /// Handles client requests for the default book summary stream.
    ///
    /// # Arguments
    /// - `_request`: The client request.
    ///
    /// # Returns
    /// - A response containing the default book summary stream.
    async fn book_summary(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<Self::BookSummaryStream>, Status> {
        info!("New client connected to BookSummary default stream");

        // Open the summary stream for the default symbol.
        let symbol = &self.default_symbol;
        let rx = self.open_summary_stream(symbol).await?;
        Ok(Response::new(into_grpc_stream(rx)))
    }

    /// Handles client requests for the book summary stream of a specific symbol.
    ///
    /// # Arguments
    /// - `request`: The client request containing the symbol.
    ///
    /// # Returns
    /// - A response containing the book summary stream for the requested symbol.
    async fn book_summary_for_symbol(
        &self,
        request: tonic::Request<SummaryRequest>,
    ) -> Result<Response<Self::BookSummaryForSymbolStream>, Status> {
        let symbol = request.into_inner().symbol;
        if symbol.is_empty() {
            return Err(Status::invalid_argument("Symbol cannot be empty"));
        }

        info!(
            "New client connected to BookSummary stream for symbol: {}",
            symbol
        );

        // Open the summary stream for the requested symbol.
        let rx = self.open_summary_stream(&symbol).await?;
        Ok(Response::new(into_grpc_stream(rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::model::{
        ContributorSnapshot, SummarySource, SummarySourceFactory, SummarySourceFactoryMap,
    };
    use aggcommon::instrument::{Instrument, InstrumentMap, SourceSymbol};
    use aggcommon::shutdown::shutdown_channel;
    use aggcommon::{proto::orderbook::Empty, sources::SourceId};
    use anyhow::Result;
    use std::sync::{Arc, Mutex};
    use tokio::sync::watch;
    use tokio_stream::StreamExt;

    fn build_single_instrument_map() -> InstrumentMap {
        let source_symbols = vec![SourceSymbol {
            source: SourceId::Binance,
            symbol: "ETHBTC".to_string(), // uppercase
        }];

        let instrument = Instrument::new("ETH-BTC".to_string(), source_symbols);
        InstrumentMap::new(vec![instrument])
    }

    fn build_instrument_map() -> InstrumentMap {
        let source_symbols = vec![
            SourceSymbol {
                source: SourceId::Bitstamp,
                symbol: "ethbtc".to_string(), // lowercase
            },
            SourceSymbol {
                source: SourceId::Binance,
                symbol: "ETHBTC".to_string(), // uppercase
            },
        ];

        let instrument = Instrument::new("ETH-BTC".to_string(), source_symbols);
        InstrumentMap::new(vec![instrument])
    }

    #[derive(Debug)]
    struct MockSource<const N: usize> {
        source_id: SourceId,
        snapshot: Arc<Mutex<Option<ContributorSnapshot<N>>>>,
        notifier: watch::Sender<()>,
    }

    impl<const N: usize> MockSource<N> {
        fn new(source_id: SourceId) -> Self {
            let (tx, _) = watch::channel(());
            Self {
                source_id,
                snapshot: Arc::new(Mutex::new(None)),
                notifier: tx,
            }
        }

        fn set_snapshot(&self, snapshot: ContributorSnapshot<N>) {
            let mut guard = self.snapshot.lock().unwrap();
            *guard = Some(snapshot);

            // Notify any watchers; ignore if no receivers exist
            let _ = self.notifier.send(());
        }
    }

    impl<const N: usize> crate::service::model::OrderBookSummarySource<N> for MockSource<N> {
        fn source_id(&self) -> SourceId {
            self.source_id
        }

        fn watch_latest_change(&self) -> watch::Receiver<()> {
            self.notifier.subscribe()
        }

        fn latest_snapshot(&self) -> Arc<Option<Result<ContributorSnapshot<N>>>> {
            let inner = self.snapshot.lock().unwrap();
            Arc::new(inner.map(Ok))
        }
    }

    #[tokio::test]
    async fn aggregator_service_emits_summary_from_mock_source() {
        const N: usize = 5;

        let mock = Arc::new(MockSource::<N>::new(SourceId::Binance));
        let mock_clone: Arc<MockSource<N>> = Arc::clone(&mock);
        let factory: SummarySourceFactory<N> =
            Box::new(move |_symbol, _shutdown| Ok(mock.clone() as SummarySource<N>));

        let factory_map = SummarySourceFactoryMap::from_vec(vec![(SourceId::Binance, factory)]);
        // Build instrument map
        let instrument_map = build_single_instrument_map();

        // Create shutdown
        let (shutdown_tx, _) = shutdown_channel();

        // Create the manager (note the updated argument order)
        let manager = Manager::new(shutdown_tx, instrument_map, factory_map);

        // Create the gRPC service using the manager
        let service = AggregatorService::new(manager.clone(), "ETH-BTC".to_string());
        // Create stream
        let response = service.book_summary(Request::new(Empty {})).await.unwrap();
        let mut stream = response.into_inner();

        // Send a snapshot from the mock source
        let mut snapshot = ContributorSnapshot::<N>::new(SourceId::Binance, 42);
        snapshot.push_bid(101.0, 1.5);
        mock_clone.set_snapshot(snapshot);

        // Receive from stream
        let summary = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("timeout")
            .expect("no summary")
            .expect("stream error");

        assert_eq!(summary.bids.len(), 1);
        assert_eq!(summary.bids[0].price, 101.0);
        assert_eq!(summary.bids[0].amount, 1.5);
    }

    #[tokio::test]
    async fn aggregator_service_fails_on_missing_factory() {
        const N: usize = 5;

        // Only provide a factory for Binance
        let mock = Arc::new(MockSource::<N>::new(SourceId::Binance));
        let factory: SummarySourceFactory<N> =
            Box::new(move |_symbol, _shutdown| Ok(mock.clone() as SummarySource<N>));
        let factory_map = SummarySourceFactoryMap::from_vec(vec![(SourceId::Binance, factory)]);

        // This map includes both Binance and Bitstamp, triggering the error
        let instrument_map = build_instrument_map();

        let (shutdown_tx, _) = shutdown_channel();
        let manager = Manager::new(shutdown_tx, instrument_map, factory_map);

        let service = AggregatorService::new(manager, "ETH-BTC".to_string());

        let response = service.book_summary(Request::new(Empty {})).await;

        match response {
            Ok(_) => panic!("Expected failure due to missing factory, but got success"),
            Err(status) => {
                assert_eq!(status.code(), tonic::Code::Internal);
                assert!(
                    status.message().contains("Missing factory"),
                    "Unexpected error message: {}",
                    status.message()
                );
            }
        }
    }
}
