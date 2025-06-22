use crate::sources::SourceId;
use dashmap::DashMap;
use std::fmt;
use std::{collections::HashMap, fmt::Formatter};

/// Represents a source-specific symbol for a trading instrument.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceSymbol {
    pub source: SourceId,
    pub symbol: String,
}

/// Represents a trading instrument.
///
/// Contains the symbol and a mapping of source-specific symbols.
#[derive(Debug, Clone, Default)]
pub struct Instrument {
    pub symbol: String,
    pub source_mapping: HashMap<SourceId, String>,
}

impl Instrument {
    /// Creates a new trading instrument.
    ///
    /// # Arguments
    /// - `symbol`: The trading pair symbol.
    /// - `source_symbols`: A vector of source-specific symbols.
    ///
    /// # Returns
    /// - `Self`: The constructed `Instrument` instance.
    pub fn new(symbol: String, source_symbols: Vec<SourceSymbol>) -> Self {
        let source_mapping: HashMap<SourceId, String> = source_symbols
            .into_iter()
            .map(|ss| (ss.source, ss.symbol))
            .collect();
        Instrument {
            symbol,
            source_mapping,
        }
    }

    /// Returns the symbol of the trading instrument.
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Returns the source mapping of the trading instrument.
    pub fn source_mapping(&self) -> &HashMap<SourceId, String> {
        &self.source_mapping
    }
}

impl PartialEq for Instrument {
    fn eq(&self, other: &Self) -> bool {
        self.symbol == other.symbol
    }
}

impl fmt::Display for Instrument {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.symbol)
    }
}

/// Represents a map of trading instruments.
#[derive(Debug)]
pub struct InstrumentMap {
    map: DashMap<String, Instrument>,
}

impl InstrumentMap {
    /// Constructs a new `InstrumentMap` from a vector of instruments.
    ///
    /// # Arguments
    /// - `instruments`: A vector of trading instruments.
    ///
    /// # Returns
    /// - `Self`: The constructed `InstrumentMap` instance.
    pub fn new(instruments: Vec<Instrument>) -> Self {
        let map = DashMap::new();
        for instrument in instruments {
            map.insert(instrument.symbol.clone(), instrument);
        }
        Self { map }
    }

    /// Looks up an instrument by symbol and returns a cloned copy, if found.
    ///
    /// # Arguments
    /// - `symbol`: The trading pair symbol.
    ///
    /// # Returns
    /// - `Option<Instrument>`: The found instrument or `None` if not found.
    pub fn get(&self, symbol: &str) -> Option<Instrument> {
        self.map.get(symbol).map(|entry| entry.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instrument_map_find_success() {
        let btc_sources = vec![
            SourceSymbol {
                source: SourceId::Bitstamp,
                symbol: "BTC/USD".to_string(),
            },
            SourceSymbol {
                source: SourceId::Binance,
                symbol: "XBT/USDT".to_string(),
            },
        ];

        let eth_sources = vec![
            SourceSymbol {
                source: SourceId::Bitstamp,
                symbol: "ETH/USD".to_string(),
            },
            SourceSymbol {
                source: SourceId::Binance,
                symbol: "ETH/USDT".to_string(),
            },
        ];

        let instruments = vec![
            Instrument::new("BTC".to_string(), btc_sources),
            Instrument::new("ETH".to_string(), eth_sources),
        ];

        let instrument_map = InstrumentMap::new(instruments);

        let btc = instrument_map.get("BTC");
        assert!(btc.is_some());
        let btc = btc.unwrap();
        assert_eq!(btc.symbol, "BTC");
        assert_eq!(
            btc.source_mapping.get(&SourceId::Binance),
            Some(&"XBT/USDT".to_string())
        );

        let eth = instrument_map.get("ETH");
        assert!(eth.is_some());
        let eth = eth.unwrap();
        assert_eq!(eth.symbol, "ETH");
        assert_eq!(
            eth.source_mapping.get(&SourceId::Bitstamp),
            Some(&"ETH/USD".to_string())
        );
    }

    #[test]
    fn test_instrument_map_find_not_found() {
        let instrument_map = InstrumentMap::new(vec![]);
        assert!(instrument_map.get("DOGE").is_none());
    }
}
