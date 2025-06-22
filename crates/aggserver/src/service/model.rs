use aggcommon::instrument::Instrument;
use aggcommon::nanos::NanoTime;
use aggcommon::proto::orderbook::{Level, Summary};
use aggcommon::shutdown::{ShutdownReceiver, ShutdownSignaller};
use aggcommon::sources::{NUM_SOURCES, SourceId};
use anyhow::{Context, Result};
use arrayvec::ArrayVec;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::watch;

/// Trait for order book summary sources.
///
/// Provides methods for retrieving source ID, watching for changes, and accessing the latest snapshot.
///
/// # Type Parameters
/// - `N`: Maximum depth of the order book.
pub trait OrderBookSummarySource<const N: usize>: Send + Sync {
    fn source_id(&self) -> SourceId;
    fn watch_latest_change(&self) -> watch::Receiver<()>;
    fn latest_snapshot(&self) -> Arc<Option<Result<ContributorSnapshot<N>>>>;
}

/// Type alias for a summary source.
pub type SummarySource<const N: usize> = Arc<dyn OrderBookSummarySource<N> + Send + Sync>;

/// Type alias for a summary source factory.
pub type SummarySourceFactory<const N: usize> =
    Box<dyn Fn(&str, ShutdownReceiver) -> Result<SummarySource<N>> + Send + Sync>;

/// Represents a map of summary source factories.
///
/// This struct allows for the creation of summary sources based on source-specific symbols.
///
/// # Type Parameters
/// - `N`: Maximum depth of the order book.
pub struct SummarySourceFactoryMap<const N: usize> {
    factories: HashMap<SourceId, SummarySourceFactory<N>>,
}

impl<const N: usize> SummarySourceFactoryMap<N> {
    /// Creates a factory map from a list of (SourceId, Factory) pairs.
    pub fn from_vec(pairs: Vec<(SourceId, SummarySourceFactory<N>)>) -> Self {
        let factories = pairs.into_iter().collect();
        Self { factories }
    }

    /// Instantiates feeds for all source-specific symbols in the instrument.
    pub fn create_summary_sources(
        &self,
        instrument: &Instrument,
        shutdown_tx: &ShutdownSignaller,
    ) -> Result<Vec<SummarySource<N>>> {
        let mut feeds = Vec::new();

        for (source_id, source_symbol) in instrument.source_mapping() {
            let factory = self
                .factories
                .get(source_id)
                .with_context(|| format!("Missing factory for source {}", source_id))?;

            let shutdown = shutdown_tx.subscribe();
            let feed = factory(source_symbol, shutdown).with_context(|| {
                format!(
                    "Factory failed for source {} ({})",
                    source_id, source_symbol
                )
            })?;

            feeds.push(feed);
        }

        Ok(feeds)
    }
}

#[cfg(test)]
impl<const N: usize> SummarySourceFactoryMap<N> {
    fn contains(&self, source_id: SourceId) -> bool {
        self.factories.contains_key(&source_id)
    }
}

/// Represents the side of the order book (Bid or Ask).
/// Alias for `f64` to improve code readability.
type Price = f64;

/// Represents the amount of an asset at a given price level.
/// Alias for `f64` to improve code readability.
type Amount = f64;

/// Represents a price level in the order book.
/// Contains the price and amount of an asset.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PriceLevel {
    pub price: Price,
    pub amount: Amount,
}
use std::cmp::Ordering;

/// Represents a price used for ordering ask levels in an order book.
///
/// This struct guarantees a total ordering over `f64` by sanitizing any `NaN` input
/// to `0.0` at construction. `AskKeyedPrice` is sorted in ascending order, which is
/// typical for asks (lowest price first).
#[derive(Debug, Clone, Copy)]
struct AskKeyedPrice(pub f64);

impl AskKeyedPrice {
    /// Constructs a new `AskKeyedPrice`, converting `NaN` to `0.0`.
    pub fn new(value: f64) -> Self {
        if value.is_nan() {
            Self(0.0)
        } else {
            Self(value)
        }
    }
}

impl PartialEq for AskKeyedPrice {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for AskKeyedPrice {}

impl Ord for AskKeyedPrice {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.partial_cmp(&other.0).unwrap()
    }
}

impl PartialOrd for AskKeyedPrice {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Represents a price used for ordering bid levels in an order book.
///
/// This struct also guarantees a total ordering over `f64` by converting `NaN`
/// to `0.0`. Unlike `AskKeyedPrice`, bids are ordered in **descending** order,
/// meaning the highest bid price comes first.
#[derive(Debug, Clone, Copy)]
struct BidKeyedPrice(pub f64);

impl BidKeyedPrice {
    /// Constructs a new `BidKeyedPrice`, converting `NaN` to `0.0`.
    pub fn new(value: f64) -> Self {
        if value.is_nan() {
            Self(0.0)
        } else {
            Self(value)
        }
    }
}

impl PartialEq for BidKeyedPrice {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for BidKeyedPrice {}

impl Ord for BidKeyedPrice {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse for descending sort (highest bid first)
        self.0.partial_cmp(&other.0).unwrap().reverse()
    }
}

impl PartialOrd for BidKeyedPrice {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Represents top-N levels from a single contributor.
/// Assumes input is already sorted correctly (descending for bids, ascending for asks).
#[derive(Debug, Clone, Copy)]
pub struct ContributorLevels<const N: usize> {
    levels: [PriceLevel; N],
    depth: usize,
}

impl<const N: usize> ContributorLevels<N> {
    fn new() -> Self {
        Self {
            levels: [PriceLevel {
                price: 0.0,
                amount: 0.0,
            }; N],
            depth: 0,
        }
    }

    fn push(&mut self, level: PriceLevel) {
        if self.depth < N {
            self.levels[self.depth] = level;
            self.depth += 1;
        }
    }

    fn as_slice(&self) -> &[PriceLevel] {
        &self.levels[..self.depth]
    }

    pub fn depth(&self) -> usize {
        self.depth
    }
}

impl<const N: usize> Default for ContributorLevels<N> {
    fn default() -> Self {
        Self {
            levels: [PriceLevel {
                price: 0.0,
                amount: 0.0,
            }; N],
            depth: 0,
        }
    }
}

/// Represents a single order book snapshot from a contributor.
/// Contains top-N bids and asks, along with metadata like source and arrival time.
#[derive(Debug, Clone, Copy)]
pub struct ContributorSnapshot<const N: usize> {
    bids: ContributorLevels<N>,
    asks: ContributorLevels<N>,
    source_id: SourceId,
    arrival_time: NanoTime,
}

impl<const N: usize> ContributorSnapshot<N> {
    /// Creates a new contributor snapshot.
    pub fn new(source: SourceId, arrival_time: NanoTime) -> Self {
        Self {
            bids: ContributorLevels::<N>::new(),
            asks: ContributorLevels::<N>::new(),
            source_id: source,
            arrival_time,
        }
    }

    /// Pushes a new bid level into the snapshot.
    pub fn push_bid(&mut self, price: Price, amount: Amount) {
        self.bids.push(PriceLevel { price, amount });
    }

    /// Pushes a new ask level into the snapshot.
    pub fn push_ask(&mut self, price: Price, amount: Amount) {
        self.asks.push(PriceLevel { price, amount });
    }

    /// Checks if the snapshot is empty (no bids or asks).
    pub fn is_empty(&self) -> bool {
        self.bids.depth() == 0 && self.asks.depth() == 0
    }
}

#[cfg(test)]
impl<const N: usize> ContributorSnapshot<N> {
    pub fn source_id(&self) -> SourceId {
        self.source_id
    }

    /// Returns a slice of the bid levels up to the current depth.
    ///
    /// This function provides a view of the bid levels stored in the `ContributorLevels` struct,
    /// limited to the number of levels currently populated (i.e., up to `depth`).
    ///
    /// # Returns
    /// - A slice of `PriceLevel` representing the bid levels.
    pub fn bids(&self) -> &[PriceLevel] {
        &self.bids.levels[..self.bids.depth]
    }

    /// Returns a slice of the ask levels up to the current depth.
    ///
    /// This function provides a view of the ask levels stored in the `ContributorLevels` struct,
    /// limited to the number of levels currently populated (i.e., up to `depth`).
    ///
    /// # Returns
    /// - A slice of `PriceLevel` representing the ask levels.
    pub fn asks(&self) -> &[PriceLevel] {
        &self.asks.levels[..self.asks.depth]
    }
}

impl<const N: usize> Default for ContributorSnapshot<N> {
    fn default() -> Self {
        Self {
            source_id: SourceId::default(),
            arrival_time: 0,
            bids: ContributorLevels::<N>::default(),
            asks: ContributorLevels::<N>::default(),
        }
    }
}

/// Represents a combined summary of order book data from multiple contributors.
#[derive(Debug, Clone)]
pub struct CombinedSummary<const N: usize> {
    sources: [ContributorSnapshot<N>; NUM_SOURCES],
}

impl<const N: usize> CombinedSummary<N> {
    /// Creates a new combined summary.
    pub fn new() -> Self {
        Self {
            sources: [ContributorSnapshot::<N>::default(); NUM_SOURCES],
        }
    }

    /// Updates or inserts a source snapshot into the combined summary.
    pub fn update_snapshot(&mut self, snapshot: ContributorSnapshot<N>) {
        let index = usize::from(snapshot.source_id);
        self.sources[index] = snapshot;
    }

    /// Converts the combined summary to a protobuf-compatible format.
    pub fn to_proto(&self) -> Summary {
        type SourceVec<const N: usize> = ArrayVec<Level, N>;
        let mut bid_map: BTreeMap<BidKeyedPrice, SourceVec<N>> = BTreeMap::new();
        let mut ask_map: BTreeMap<AskKeyedPrice, SourceVec<N>> = BTreeMap::new();

        let mut latest_time: NanoTime = 0;

        for snapshot in &self.sources {
            if snapshot.is_empty() {
                continue;
            }

            latest_time = latest_time.max(snapshot.arrival_time);

            for level in snapshot.bids.as_slice() {
                let level_proto = Level {
                    exchange: snapshot.source_id.to_string(),
                    price: level.price,
                    amount: level.amount,
                };
                bid_map
                    .entry(BidKeyedPrice::new(level.price))
                    .or_default()
                    .push(level_proto);
            }

            for level in snapshot.asks.as_slice() {
                let level_proto = Level {
                    exchange: snapshot.source_id.to_string(),
                    price: level.price,
                    amount: level.amount,
                };
                ask_map
                    .entry(AskKeyedPrice::new(level.price))
                    .or_default()
                    .push(level_proto);
            }
        }

        let bids: Vec<Level> = bid_map
            .into_iter()
            .take(N)
            .flat_map(|(_, mut levels)| {
                levels.sort_unstable_by(|a, b| b.amount.partial_cmp(&a.amount).unwrap());
                levels
            })
            .collect();

        let asks: Vec<Level> = ask_map
            .into_iter()
            .take(N)
            .flat_map(|(_, mut levels)| {
                levels.sort_unstable_by(|a, b| b.amount.partial_cmp(&a.amount).unwrap());
                levels
            })
            .collect();

        let spread = spread_from(bids.first(), asks.first());

        Summary {
            spread,
            bids,
            asks,
            arrival_time: latest_time,
        }
    }
}

/// Calculates the spread from the top bid and ask levels.
#[inline]
fn spread_from(bid: Option<&Level>, ask: Option<&Level>) -> f64 {
    match (bid, ask) {
        (Some(b), Some(a)) if b.price > 0.0 && a.price > 0.0 => a.price - b.price,
        _ => f64::NAN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aggcommon::sources::SourceId;

    // Add MockSource for testing purposes
    #[derive(Debug, Clone)]
    struct MockSource<const N: usize> {
        source_id: SourceId,
    }

    impl<const N: usize> MockSource<N> {
        fn new(source_id: SourceId) -> Self {
            Self { source_id }
        }
    }

    impl<const N: usize> OrderBookSummarySource<N> for MockSource<N> {
        fn source_id(&self) -> SourceId {
            self.source_id
        }

        fn watch_latest_change(&self) -> watch::Receiver<()> {
            let (tx, rx) = watch::channel(());
            tx.send(()).unwrap();
            rx
        }

        fn latest_snapshot(&self) -> Arc<Option<Result<ContributorSnapshot<N>>>> {
            Arc::new(None)
        }
    }

    fn snapshot_with_bid<const N: usize>(
        source: SourceId,
        time: u64,
        price: f64,
        amount: f64,
    ) -> ContributorSnapshot<N> {
        let mut s = ContributorSnapshot::new(source, time);
        s.push_bid(price, amount);
        s
    }

    fn snapshot_with_ask<const N: usize>(
        source: SourceId,
        time: u64,
        price: f64,
        amount: f64,
    ) -> ContributorSnapshot<N> {
        let mut s = ContributorSnapshot::new(source, time);
        s.push_ask(price, amount);
        s
    }

    fn snapshot_with_levels<const N: usize>(
        source: SourceId,
        time: u64,
        bids: &[(f64, f64)],
        asks: &[(f64, f64)],
    ) -> ContributorSnapshot<N> {
        let mut s = ContributorSnapshot::new(source, time);
        for &(p, a) in bids {
            s.push_bid(p, a);
        }
        for &(p, a) in asks {
            s.push_ask(p, a);
        }
        s
    }

    #[test]
    fn test_to_proto_n1() {
        let mut combined = CombinedSummary::<5>::new();

        combined.update_snapshot(snapshot_with_bid(SourceId::Binance, 1, 100.0, 1.0));
        combined.update_snapshot(snapshot_with_ask(SourceId::Bitstamp, 1, 101.0, 1.0));

        let summary = combined.to_proto();

        assert_eq!(summary.bids.len(), 1);
        assert_eq!(summary.asks.len(), 1);
        assert_eq!(summary.bids[0].price, 100.0);
        assert_eq!(summary.asks[0].price, 101.0);
        assert_eq!(summary.spread, 1.0);
        assert_eq!(summary.arrival_time, 1);
    }

    #[test]
    fn test_depth_limit_respected() {
        let mut combined = CombinedSummary::<3>::new();

        let bids = (0..10).map(|i| (100.0 - i as f64, 1.0)).collect::<Vec<_>>();
        let asks = (0..10).map(|i| (100.0 + i as f64, 1.0)).collect::<Vec<_>>();

        combined.update_snapshot(snapshot_with_levels(SourceId::Binance, 1, &bids, &asks));

        let summary: Summary = combined.to_proto();
        assert_eq!(summary.bids.len(), 3);
        assert_eq!(summary.asks.len(), 3);
    }

    #[test]
    fn test_latest_time_reflects_max() {
        let mut combined = CombinedSummary::<5>::new();

        combined.update_snapshot(snapshot_with_bid(SourceId::Binance, 5, 100.0, 1.0));
        combined.update_snapshot(snapshot_with_ask(SourceId::Bitstamp, 10, 101.0, 1.0));

        let summary: Summary = combined.to_proto();
        assert_eq!(summary.arrival_time, 10);
    }

    #[test]
    fn test_levels_sorted_by_amount_within_price() {
        let mut combined = CombinedSummary::<5>::new();

        let mut snap1 = ContributorSnapshot::new(SourceId::Binance, 1);
        snap1.push_bid(100.0, 1.0);
        combined.update_snapshot(snap1);

        let mut snap2 = ContributorSnapshot::new(SourceId::Bitstamp, 2);
        snap2.push_bid(100.0, 2.0);
        combined.update_snapshot(snap2);

        let summary: Summary = combined.to_proto();
        assert_eq!(summary.bids[0].amount, 2.0);
        assert_eq!(summary.bids[1].amount, 1.0);
    }

    #[test]
    fn test_empty_combined_summary_yields_nan_spread() {
        let combined = CombinedSummary::<5>::new();

        let summary: Summary = combined.to_proto();

        assert!(summary.bids.is_empty());
        assert!(summary.asks.is_empty());
        assert!(summary.spread.is_nan());
        assert_eq!(summary.arrival_time, 0);
    }

    #[test]
    fn test_spread_only_bids_or_asks() {
        let mut combined = CombinedSummary::<5>::new();

        combined.update_snapshot(snapshot_with_bid(SourceId::Binance, 1, 100.0, 1.0));
        let summary: Summary = combined.to_proto();
        assert!(summary.spread.is_nan());

        let mut combined = CombinedSummary::<5>::new();
        combined.update_snapshot(snapshot_with_ask(SourceId::Binance, 1, 105.0, 1.0));
        let summary: Summary = combined.to_proto();
        assert!(summary.spread.is_nan());
    }

    #[test]
    fn test_overwrites_existing_snapshot() {
        let mut combined = CombinedSummary::<5>::new();

        combined.update_snapshot(snapshot_with_bid(SourceId::Binance, 1, 100.0, 1.0));
        combined.update_snapshot(snapshot_with_ask(SourceId::Binance, 2, 105.0, 1.0));

        let summary: Summary = combined.to_proto();
        assert!(summary.bids.is_empty()); // previous bid is overwritten
        assert_eq!(summary.asks.len(), 1);
        assert_eq!(summary.arrival_time, 2);
    }

    #[test]
    fn test_factorymap_contains() {
        let factory_map = SummarySourceFactoryMap::<5>::from_vec(vec![
            (
                SourceId::Binance,
                Box::new(|_, _| {
                    Ok(Arc::new(MockSource::<5>::new(SourceId::Binance)) as SummarySource<5>)
                }),
            ),
            (
                SourceId::Bitstamp,
                Box::new(|_, _| {
                    Ok(Arc::new(MockSource::<5>::new(SourceId::Bitstamp)) as SummarySource<5>)
                }),
            ),
        ]);

        assert!(factory_map.contains(SourceId::Binance));
        assert!(factory_map.contains(SourceId::Bitstamp));
    }

    #[test]
    fn spread_from_valid_bid_and_ask() {
        let bid = Level {
            price: 100.0,
            amount: 1.0,
            exchange: "Binance".into(),
        };
        let ask = Level {
            price: 105.0,
            amount: 1.0,
            exchange: "Bitstamp".into(),
        };
        let spread = spread_from(Some(&bid), Some(&ask));
        assert_eq!(spread, 5.0);
    }

    #[test]
    fn spread_from_zero_bid_price() {
        let bid = Level {
            price: 0.0,
            amount: 1.0,
            exchange: "Binance".into(),
        };
        let ask = Level {
            price: 105.0,
            amount: 1.0,
            exchange: "Bitstamp".into(),
        };
        let spread = spread_from(Some(&bid), Some(&ask));
        assert!(spread.is_nan());
    }

    #[test]
    fn spread_from_zero_ask_price() {
        let bid = Level {
            price: 100.0,
            amount: 1.0,
            exchange: "Binance".into(),
        };
        let ask = Level {
            price: 0.0,
            amount: 1.0,
            exchange: "Bitstamp".into(),
        };
        let spread = spread_from(Some(&bid), Some(&ask));
        assert!(spread.is_nan());
    }

    #[test]
    fn spread_from_missing_bid() {
        let ask = Level {
            price: 105.0,
            amount: 1.0,
            exchange: "Bitstamp".into(),
        };
        let spread = spread_from(None, Some(&ask));
        assert!(spread.is_nan());
    }

    #[test]
    fn spread_from_missing_ask() {
        let bid = Level {
            price: 100.0,
            amount: 1.0,
            exchange: "Binance".into(),
        };
        let spread = spread_from(Some(&bid), None);
        assert!(spread.is_nan());
    }

    #[test]
    fn spread_from_missing_both() {
        let spread = spread_from(None, None);
        assert!(spread.is_nan());
    }
}
