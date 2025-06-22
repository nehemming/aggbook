use std::fmt;
use std::str::FromStr;

/// Represents the ID of a data source.
///
/// Used to identify different sources of order book data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SourceId {
    Bitstamp,
    Binance,
}

/// The number of supported data sources.
pub const NUM_SOURCES: usize = 2;

impl fmt::Display for SourceId {
    /// Converts a `SourceId` to its string representation.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            SourceId::Bitstamp => "bitstamp",
            SourceId::Binance => "binance",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for SourceId {
    type Err = SourceIdParseError;

    /// Parses a string into a `SourceId`.
    ///
    /// # Arguments
    /// - `s`: The string to parse.
    ///
    /// # Returns
    /// - `Result<Self, Self::Err>`: The parsed `SourceId` or an error.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bitstamp" => Ok(SourceId::Bitstamp),
            "binance" => Ok(SourceId::Binance),
            _ => Err(SourceIdParseError(s.to_string())),
        }
    }
}

impl From<SourceId> for usize {
    /// Converts a `SourceId` to its corresponding index.
    fn from(s: SourceId) -> usize {
        match s {
            SourceId::Bitstamp => 0,
            SourceId::Binance => 1,
        }
    }
}

impl Default for SourceId {
    /// Returns the default `SourceId`.
    fn default() -> Self {
        SourceId::Bitstamp
    }
}

/// Represents an error that occurs when parsing a `SourceId`.
#[derive(Debug, Clone)]
pub struct SourceIdParseError(pub String);

impl fmt::Display for SourceIdParseError {
    /// Converts a `SourceIdParseError` to its string representation.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid source ID: '{}'", self.0)
    }
}

impl std::error::Error for SourceIdParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display() {
        assert_eq!(SourceId::Bitstamp.to_string(), "bitstamp");
        assert_eq!(SourceId::Binance.to_string(), "binance");
    }

    #[test]
    fn test_from_str_success() {
        assert_eq!("bitstamp".parse::<SourceId>().unwrap(), SourceId::Bitstamp);
        assert_eq!("Bitstamp".parse::<SourceId>().unwrap(), SourceId::Bitstamp);
        assert_eq!("BINANCE".parse::<SourceId>().unwrap(), SourceId::Binance);
    }

    #[test]
    fn test_from_str_failure() {
        let result = "kraken".parse::<SourceId>();
        assert!(result.is_err());
        assert_eq!(
            format!("{}", result.unwrap_err()),
            "Invalid source ID: 'kraken'"
        );
    }

    #[test]
    fn test_from_source_id_to_usize() {
        let bitstamp_index: usize = SourceId::Bitstamp.into();
        let binance_index: usize = SourceId::Binance.into();
        assert_eq!(bitstamp_index, 0);
        assert_eq!(binance_index, 1);
    }

    #[test]
    fn test_default() {
        let default_source = SourceId::default();
        assert_eq!(default_source, SourceId::Bitstamp);
    }
}
