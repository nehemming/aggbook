use crate::service::model::{SummarySource, SummarySourceFactoryMap};
use crate::service::sources::{binance::BinanceFeed, bitstamp::BitstampFeed};
use aggcommon::shutdown::ShutdownReceiver;
use aggcommon::sources::SourceId;
use anyhow::Result;

/// Creates a new factory map for summary sources.
///
/// # Type Parameters
/// - `N`: Maximum depth of the order book.
///
/// # Returns
/// - `SummarySourceFactoryMap<N>`: The factory map for summary sources.
pub fn new_factory_map<const N: usize>() -> SummarySourceFactoryMap<N> {
    SummarySourceFactoryMap::from_vec(vec![
        (SourceId::Binance, Box::new(binance_factory::<N>)),
        (SourceId::Bitstamp, Box::new(bitstamp_factory::<N>)),
    ])
}

/// Factory function for creating Binance summary sources.
///
/// # Arguments
/// - `symbol`: The trading pair symbol.
/// - `shutdown`: Receiver for shutdown signals.
///
/// # Returns
/// - `Result<SummarySource<N>>`: The Binance summary source or an error.
fn binance_factory<const N: usize>(
    symbol: &str,
    shutdown: ShutdownReceiver,
) -> Result<SummarySource<N>> {
    BinanceFeed::<N>::subscribe(symbol, shutdown).map(|arc| arc as SummarySource<N>)
}

/// Factory function for creating Bitstamp summary sources.
///
/// # Arguments
/// - `symbol`: The trading pair symbol.
/// - `shutdown`: Receiver for shutdown signals.
///
/// # Returns
/// - `Result<SummarySource<N>>`: The Bitstamp summary source or an error.
fn bitstamp_factory<const N: usize>(
    symbol: &str,
    shutdown: ShutdownReceiver,
) -> Result<SummarySource<N>> {
    BitstampFeed::<N>::subscribe(symbol, shutdown).map(|arc| arc as SummarySource<N>)
}
