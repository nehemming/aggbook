use aggcommon::{nanos::nanos_pretty, proto::orderbook::Summary};
use std::fmt;

#[cfg(not(test))]
use aggcommon::nanos::nanos_now;

#[cfg(test)]
fn nanos_now() -> u64 {
    // Return a predictable value for testing
    1_000_000_000
}

pub struct DisplaySummary<'a>(pub &'a Summary);

impl fmt::Display for DisplaySummary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let summary = self.0;
        let now = nanos_now();

        writeln!(
            f,
            "Latency to now {}:",
            nanos_pretty(now - summary.arrival_time)
        )?;

        writeln!(f, "Spread: {:.8}", summary.spread)?;
        writeln!(f, "Bids:")?;
        for bid in &summary.bids {
            writeln!(
                f,
                "  {:>10.8} : {:>10.4} [{}]",
                bid.price, bid.amount, bid.exchange
            )?;
        }
        writeln!(f, "Asks:")?;
        for ask in &summary.asks {
            writeln!(
                f,
                "  {:>10.8} : {:>10.4} [{}]",
                ask.price, ask.amount, ask.exchange
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aggcommon::proto::orderbook::Level;

    #[test]
    fn test_display_summary_formatting() {
        let summary = Summary {
            spread: 1.23456789,
            bids: vec![
                Level {
                    price: 109.12345678,
                    amount: 1.0,
                    exchange: "exchangeA".into(),
                },
                Level {
                    price: 108.87654321,
                    amount: 2.0,
                    exchange: "exchangeB".into(),
                },
            ],
            asks: vec![
                Level {
                    price: 110.12345678,
                    amount: 1.5,
                    exchange: "exchangeA".into(),
                },
                Level {
                    price: 111.87654321,
                    amount: 2.5,
                    exchange: "exchangeB".into(),
                },
            ],
            arrival_time: nanos_now(),
        };

        let expected = "\
Latency to now 0s:
Spread: 1.23456789
Bids:
  109.12345678 :     1.0000 [exchangeA]
  108.87654321 :     2.0000 [exchangeB]
Asks:
  110.12345678 :     1.5000 [exchangeA]
  111.87654321 :     2.5000 [exchangeB]
";
        let output = format!("{}", DisplaySummary(&summary));

        assert_eq!(output, expected);
    }
}
