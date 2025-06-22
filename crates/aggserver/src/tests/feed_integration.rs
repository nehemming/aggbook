use crate::service::sources::{binance::BinanceFeed, bitstamp::BitstampFeed};
use aggcommon::shutdown::shutdown_channel;
use rustls::crypto::ring::default_provider;
use std::{sync::Arc, time::Duration};
use tokio::sync::OnceCell;
use tokio::time::timeout;
static INIT: OnceCell<()> = OnceCell::const_new();

async fn setup_tls() {
    INIT.get_or_init(|| async {
        // Runs once before tests to set up the tls.
        rustls::crypto::CryptoProvider::install_default(default_provider())
            .expect("install rustls CryptoProvider");
    })
    .await;
}

/// Helper to await snapshot availability with timeout.
async fn wait_for_snapshot<F, const N: usize>(feed: Arc<F>, venue: &str)
where
    F: crate::service::model::OrderBookSummarySource<N> + 'static,
{
    let mut rx = feed.watch_latest_change();

    let ok = timeout(Duration::from_secs(120), async {
        loop {
            let snapshot = feed.latest_snapshot();
            if let Some(Ok(snapshot)) = snapshot.as_ref() {
                if !snapshot.is_empty() {
                    break;
                }
            }

            rx.changed().await.ok();
        }
    })
    .await;

    assert!(ok.is_ok(), "Did not receive snapshot from {venue} in time");
}

#[tokio::test]
async fn test_binance_feed_live() {
    setup_tls().await; // Ensure TLS is set up for Binance

    const N: usize = 5;
    let symbol = "ethbtc";
    let (shutdown, _) = shutdown_channel();

    let feed = BinanceFeed::<N>::subscribe(symbol, shutdown.subscribe())
        .expect("Failed to subscribe to Binance feed");

    wait_for_snapshot(feed, "Binance").await;
}

#[tokio::test]
async fn test_bitstamp_feed_live() {
    setup_tls().await; // Ensure TLS is set up for Bitstamp

    const N: usize = 5;
    let symbol = "ethbtc";
    let (shutdown, _) = shutdown_channel();

    let feed = BitstampFeed::<N>::subscribe(symbol, shutdown.subscribe())
        .expect("Failed to subscribe to Bitstamp feed");

    wait_for_snapshot(feed, "Bitstamp").await;
}
