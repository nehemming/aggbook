//! Binance Order Book Feed with Summary + Watch
use crate::service::model::{ContributorSnapshot, OrderBookSummarySource};
use aggcommon::{
    join_observer::ObserveJoinHandle,
    nanos::{NanoTime, nanos_now},
    shutdown::ShutdownReceiver,
    sources::SourceId,
};
use anyhow::Result;
use arc_swap::ArcSwap;
use futures::SinkExt;
use futures_util::StreamExt;
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, time::interval};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info};

const BACKOFF_SECONDS: u64 = 5;
const PING_PERIOD_SECONDS: u64 = 30;

/// Represents a feed for Binance order book updates.
///
/// This struct maintains the latest order book snapshot and provides mechanisms
/// for subscribing to updates.
///
/// # Type Parameters
/// - `N`: Maximum depth of the order book.
pub struct BinanceFeed<const N: usize> {
    latest: ArcSwap<Option<Result<ContributorSnapshot<N>>>>,
    watch_tx: watch::Sender<()>,
}

impl<const N: usize> BinanceFeed<N> {
    /// Subscribes to the Binance order book feed for a given symbol.
    ///
    /// # Arguments
    /// - `symbol`: The trading pair symbol (e.g., "BTCUSDT").
    /// - `shutdown`: Receiver for shutdown signals.
    ///
    /// # Returns
    /// - `Result<Arc<Self>>`: An instance of `BinanceFeed` wrapped in an `Arc`.
    pub fn subscribe(symbol: &str, mut shutdown: ShutdownReceiver) -> Result<Arc<Self>> {
        let latest = ArcSwap::from_pointee(None);
        let (watch_tx, _watch_rx) = watch::channel(());
        let instance = Arc::new(Self { latest, watch_tx });

        let symbol = symbol.to_lowercase();
        let instance_clone = Arc::clone(&instance);

        tokio::spawn(async move {
            loop {
                match run_feed_loop::<N>(&symbol, &instance_clone, &mut shutdown).await {
                    Ok(_) => break, // graceful shutdown from inside run_feed_loop
                    Err(e) => {
                        error!("Connection issue {} {symbol}: {e}", SourceId::Binance);
                        instance_clone.latest.store(Arc::new(Some(Err(e))));
                        let _ = instance_clone.watch_tx.send(());

                        // Sleep or shutdown, whichever comes first
                        let interrupted = tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(BACKOFF_SECONDS)) => false,
                            _ = shutdown.recv() => true,
                        };

                        if interrupted {
                            break;
                        }
                    }
                }
            }
            info!("Feed {} {symbol}: closed", SourceId::Binance);
        })
        .observe("binance");

        Ok(instance)
    }
}

impl<const N: usize> OrderBookSummarySource<N> for BinanceFeed<N> {
    /// Returns the source ID for Binance.
    fn source_id(&self) -> SourceId {
        SourceId::Binance
    }

    /// Provides a receiver for the latest updates.
    fn watch_latest_change(&self) -> watch::Receiver<()> {
        self.watch_tx.subscribe()
    }

    /// Retrieves the latest order book snapshot.
    fn latest_snapshot(&self) -> Arc<Option<Result<ContributorSnapshot<N>>>> {
        self.latest.load_full()
    }
}

/// Runs the feed loop for consuming Binance order book updates.
///
/// # Arguments
/// - `symbol`: The trading pair symbol.
/// - `feed`: Reference to the `BinanceFeed` instance.
/// - `shutdown`: Receiver for shutdown signals.
///
/// # Returns
/// - `Result<()>`: Indicates success or failure.
#[tracing::instrument(skip_all, fields(symbol = %symbol))]
async fn run_feed_loop<const N: usize>(
    symbol: &str,
    feed: &Arc<BinanceFeed<N>>,
    shutdown: &mut ShutdownReceiver,
) -> Result<()> {
    // lets assert N < 20!
    assert!(N <= 20, "N must be less than or equal to 20");

    let url = format!("wss://stream.binance.com:9443/ws/{}@depth20@100ms", symbol);
    let (ws_stream, _) = connect_async(&url).await?;
    let (mut write, mut read) = ws_stream.split();
    let mut ping_interval = interval(Duration::from_secs(PING_PERIOD_SECONDS));

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        let arrival_time = nanos_now();
                        let snapshot = parse_orderbook_update::<N>(&txt, arrival_time)?;
                        feed.latest.store(Arc::new(Some(Ok(snapshot))));
                        feed.watch_tx.send(()).ok();
                    }
                    Some(Ok(Message::Ping(_))) => {
                        // Auto-reply with pong
                        write.send(Message::Pong(vec![].into())).await.ok();
                    }
                    Some(Ok(_)) => { /* ignore other frames */ }
                    Some(Err(e)) => {
                        return Err(anyhow::anyhow!("WebSocket error: {}", e));
                    }
                    None => {
                        return Err(anyhow::anyhow!("WebSocket disconnected."));
                    }
                }
            }

            _ = ping_interval.tick() => {
                if let Err(e) = write.send(Message::Ping(vec![].into())).await {
                    return Err(anyhow::anyhow!("WebSocket ping error: {}", e));
                }
            }

            _ = shutdown.recv() => {
                return Ok(());
            }
        }
    }
}

/// Parses a Binance order book update from JSON payload.
///
/// # Arguments
/// - `payload`: JSON string containing order book data.
///
/// # Returns
/// - `Result<ContributorSnapshot<N>>`: Parsed snapshot of the order book.
fn parse_orderbook_update<const N: usize>(
    payload: &str,
    arrival_time: NanoTime,
) -> Result<ContributorSnapshot<N>> {
    let json: serde_json::Value = serde_json::from_str(payload)?;

    let mut snap: ContributorSnapshot<N> =
        ContributorSnapshot::new(SourceId::Binance, arrival_time);

    if let Some(bids) = json.get("bids").and_then(|b| b.as_array()) {
        for entry in bids.iter().take(N) {
            let price = entry[0].as_str().unwrap().parse::<f64>()?;
            let qty = entry[1].as_str().unwrap().parse::<f64>()?;
            if qty > 0.0 {
                snap.push_bid(price, qty);
            }
        }
    }

    if let Some(asks) = json.get("asks").and_then(|a| a.as_array()) {
        for entry in asks.iter().take(N) {
            let price = entry[0].as_str().unwrap().parse::<f64>()?;
            let qty = entry[1].as_str().unwrap().parse::<f64>()?;
            if qty > 0.0 {
                snap.push_ask(price, qty);
            }
        }
    }

    Ok(snap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aggcommon::sources::SourceId;

    const SAMPLE_JSON: &str = r#"
    {
      "bids": [["0.062", "7.5"], ["0.0619", "2.3"]],
      "asks": [["0.063", "1.7"], ["0.064", "0.4"]]
    }
    "#;

    #[test]
    fn test_parse_binance_update_valid() {
        let snapshot = parse_orderbook_update::<2>(SAMPLE_JSON, 0).expect("Failed to parse");

        assert_eq!(snapshot.source_id(), SourceId::Binance);

        let bids = snapshot.bids();
        assert_eq!(bids.len(), 2);
        assert_eq!(bids[0].price, 0.062);
        assert_eq!(bids[0].amount, 7.5);
        assert_eq!(bids[1].price, 0.0619);
        assert_eq!(bids[1].amount, 2.3);

        let asks = snapshot.asks();
        assert_eq!(asks.len(), 2);
        assert_eq!(asks[0].price, 0.063);
        assert_eq!(asks[0].amount, 1.7);
        assert_eq!(asks[1].price, 0.064);
        assert_eq!(asks[1].amount, 0.4);
    }

    #[test]
    fn test_parse_binance_update_empty() {
        let empty_json = r#"{"b": [], "a": []}"#;
        let snapshot = parse_orderbook_update::<5>(empty_json, 0).expect("Failed to parse");
        assert!(snapshot.bids().is_empty());
        assert!(snapshot.asks().is_empty());
    }

    #[test]
    fn test_parse_binance_update_partial_depth() {
        //lastUpdateId is also present
        let small_json = r#"
        {
          "bids": [["0.05", "3.0"]],
          "asks": [["0.06", "4.0"]]
        }
        "#;
        let snapshot = parse_orderbook_update::<5>(small_json, 0).expect("Failed to parse");

        assert_eq!(snapshot.bids().len(), 1);
        assert_eq!(snapshot.asks().len(), 1);
        assert_eq!(snapshot.bids()[0].price, 0.05);
        assert_eq!(snapshot.bids()[0].amount, 3.0);
        assert_eq!(snapshot.asks()[0].price, 0.06);
        assert_eq!(snapshot.asks()[0].amount, 4.0);
    }
}

/*
"{"lastUpdateId":8190853218,"bids":[["0.02394000","68.00070000"],["0.02393000","100.14040000"],["0.02392000","112.34620000"],["0.02391000","87.02560000"],["0.02390000","92.51030000"],["0.02389000","81.40200000"],["0.02388000","51.58650000"],["0.02387000","13.98130000"],["0.02386000","59.12360000"],["0.02385000","85.42080000"],["0.02384000","6.06730000"],["0.02383000","45.99620000"],["0.02382000","33.11670000"],["0.02381000","58.48670000"],["0.02380000","103.83900000"],["0.02379000","3.89390000"],["0.02378000","117.72960000"],["0.02377000","3.61950000"],["0.02376000","3.38350000"],["0.02375000","16.81400000"]],"asks":[["0.02395000","59.67610000"],["0.02396000","37.73950000"],["0.02397000","52.65750000"],["0.02398000","60.39340000"],["0.02399000","56.30370000"],["0.02400000","57.26160000"],["0.02401000","28.59600000"],["0.02402000","57.92410000"],["0.02403000","9.66360000"],["0.02404000","10.13550000"],["0.02405000","6.70940000"],["0.02406000","46.54610000"],["0.02407000","42.74680000"],["0.02408000","70.84800000"],["0.02409000","2.59840000"],["0.02410000","255.30140000"],["0.02411000","104.23000000"],["0.02412000","6.11980000"],["0.02413000","13.55320000"],["0.02414000","101.83020000"]]}"
*/
