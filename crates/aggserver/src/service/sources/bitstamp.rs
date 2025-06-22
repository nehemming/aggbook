//! Bitstamp Order Book Feed with Summary + Watch

use crate::service::model::{ContributorSnapshot, OrderBookSummarySource};
use aggcommon::{
    join_observer::ObserveJoinHandle,
    nanos::{NanoTime, nanos_now},
    shutdown::ShutdownReceiver,
    sources::SourceId,
};
use anyhow::Result;
use arc_swap::ArcSwap;
use futures_util::{SinkExt, StreamExt};
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, time::interval};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info};

const BACKOFF_SECONDS: u64 = 5;
const PING_PERIOD_SECONDS: u64 = 30;

/// Represents a feed for Bitstamp order book updates.
///
/// This struct maintains the latest order book snapshot and provides mechanisms
/// for subscribing to updates.
///
/// # Type Parameters
/// - `N`: Maximum depth of the order book.
pub struct BitstampFeed<const N: usize> {
    latest: ArcSwap<Option<Result<ContributorSnapshot<N>>>>,
    watch_tx: watch::Sender<()>,
}

impl<const N: usize> BitstampFeed<N> {
    /// Subscribes to the Bitstamp order book feed for a given symbol.
    ///
    /// # Arguments
    /// - `symbol`: The trading pair symbol (e.g., "ETHBTC").
    /// - `shutdown`: Receiver for shutdown signals.
    ///
    /// # Returns
    /// - `Result<Arc<Self>>`: An instance of `BitstampFeed` wrapped in an `Arc`.
    pub fn subscribe(symbol: &str, mut shutdown: ShutdownReceiver) -> Result<Arc<Self>> {
        assert!(N <= 10, "N must be <= 10 for Bitstamp feed");
        let latest = ArcSwap::from_pointee(None);
        let (watch_tx, _watch_rx) = watch::channel(());
        let instance = Arc::new(Self { latest, watch_tx });

        let symbol = symbol.to_lowercase();
        let channel = format!("order_book_{}", symbol);
        let ws_url = "wss://ws.bitstamp.net";

        let instance_clone = Arc::clone(&instance);
        tokio::spawn(async move {
            loop {
                match run_feed_loop::<N>(ws_url, &channel, &instance_clone, &mut shutdown).await {
                    Ok(_) => break,
                    Err(e) => {
                        error!("Connection issue {} {symbol}: {e}", SourceId::Bitstamp);
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
            info!("Feed {} {symbol}: closed", SourceId::Bitstamp);
        })
        .observe("bitstamp");

        Ok(instance)
    }
}

impl<const N: usize> OrderBookSummarySource<N> for BitstampFeed<N> {
    /// Returns the source ID for Bitstamp.
    fn source_id(&self) -> SourceId {
        SourceId::Bitstamp
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

/// Runs the feed loop for consuming Bitstamp order book updates.
///
/// # Arguments
/// - `ws_url`: WebSocket URL for Bitstamp.
/// - `channel`: Subscription channel name.
/// - `feed`: Reference to the `BitstampFeed` instance.
/// - `shutdown`: Receiver for shutdown signals.
///
/// # Returns
/// - `Result<()>`: Indicates success or failure.
async fn run_feed_loop<const N: usize>(
    ws_url: &str,
    channel: &str,
    feed: &Arc<BitstampFeed<N>>,
    shutdown: &mut ShutdownReceiver,
) -> Result<()> {
    let (ws_stream, _) = connect_async(ws_url).await?;
    let (mut write, mut read) = ws_stream.split();

    // Send subscription message
    let sub_msg = serde_json::json!({
        "event": "bts:subscribe",
        "data": {
            "channel": channel
        }
    });
    write
        .send(Message::Text(sub_msg.to_string().into()))
        .await?;

    let mut ping_interval = interval(Duration::from_secs(PING_PERIOD_SECONDS));

    loop {
        tokio::select! {
            msg = read.next() => {

                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        let arrival_time = nanos_now();
                        if let Some(snapshot) = parse_bitstamp_update::<N>(&txt,channel, arrival_time)? {
                            feed.latest.store(Arc::new(Some(Ok(snapshot))));
                            feed.watch_tx.send(()).ok();
                        }
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

/// Parses a Bitstamp order book update, returns None if not relevant.
///
/// # Arguments
/// - `txt`: JSON string containing order book data.
/// - `subscribed_channel`: The channel name to filter updates.
///
/// # Returns
/// - `Result<Option<ContributorSnapshot<N>>>`: Parsed snapshot or None if irrelevant.
fn parse_bitstamp_update<const N: usize>(
    txt: &str,
    subscribed_channel: &str,
    arrival_time: NanoTime,
) -> Result<Option<ContributorSnapshot<N>>> {
    let vp: serde_json::Value = serde_json::from_str(txt)?;
    let event = vp.get("event").and_then(|v| v.as_str()).unwrap_or("");
    if event != "data" {
        return Ok(None);
    }

    let data = &vp["data"];
    let channel = vp["channel"].as_str().unwrap_or("");
    if channel != subscribed_channel {
        return Ok(None);
    }

    // Extract bids and asks arrays
    static EMPTY: Vec<serde_json::Value> = Vec::new();

    let bids = data["bids"].as_array().unwrap_or(&EMPTY);
    let asks = data["asks"].as_array().unwrap_or(&EMPTY);

    // Merge into fixed snapshot
    let mut snap = ContributorSnapshot::new(
        SourceId::Bitstamp,
        arrival_time,
        // data["timestamp"]
        //     .as_str()
        //     .and_then(|s| s.parse::<u64>().ok())
        //     .unwrap_or(0),
    );

    for entry in bids.iter().take(N) {
        if let (Some(price_s), Some(qty_s)) = (entry[0].as_str(), entry[1].as_str()) {
            let price = price_s.parse::<f64>()?;
            let qty = qty_s.parse::<f64>()?;
            if qty > 0.0 {
                snap.push_bid(price, qty);
            }
        }
    }

    // asks in ascending but we want lowest asks, so same ordering
    for entry in asks.iter().take(N) {
        if let (Some(price_s), Some(qty_s)) = (entry[0].as_str(), entry[1].as_str()) {
            let price = price_s.parse::<f64>()?;
            let qty = qty_s.parse::<f64>()?;
            if qty > 0.0 {
                snap.push_ask(price, qty);
            }
        }
    }

    Ok(Some(snap))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aggcommon::sources::SourceId;

    const SAMPLE_JSON: &str = r#"
    {
      "event": "data",
      "channel": "order_book_ethbtc",
      "data": {
        "timestamp": "1629384756",
        "bids": [["0.032", "5.0"], ["0.0319", "1.2"]],
        "asks": [["0.033", "3.1"], ["0.034", "0.7"]]
      }
    }"#;

    #[test]
    fn test_parse_bitstamp_update_valid() {
        let result = parse_bitstamp_update::<2>(SAMPLE_JSON, "order_book_ethbtc", 0).unwrap();
        let snapshot = result.expect("Expected snapshot");

        assert_eq!(snapshot.source_id(), SourceId::Bitstamp);

        let bids = snapshot.bids();
        assert_eq!(bids.len(), 2);
        assert_eq!(bids[0].price, 0.032);
        assert_eq!(bids[0].amount, 5.0);
        assert_eq!(bids[1].price, 0.0319);
        assert_eq!(bids[1].amount, 1.2);

        let asks = snapshot.asks();
        assert_eq!(asks.len(), 2);
        assert_eq!(asks[0].price, 0.033);
        assert_eq!(asks[0].amount, 3.1);
        assert_eq!(asks[1].price, 0.034);
        assert_eq!(asks[1].amount, 0.7);
    }

    #[test]
    fn test_parse_bitstamp_update_wrong_channel() {
        let result = parse_bitstamp_update::<2>(SAMPLE_JSON, "order_book_btcusd", 0).unwrap();
        assert!(result.is_none(), "Should return None for unrelated channel");
    }

    #[test]
    fn test_parse_bitstamp_update_wrong_event() {
        let wrong_event = SAMPLE_JSON.replace("\"event\": \"data\"", "\"event\": \"other\"");
        let result = parse_bitstamp_update::<2>(&wrong_event, "order_book_ethbtc", 0).unwrap();
        assert!(result.is_none(), "Should return None for wrong event type");
    }
}
