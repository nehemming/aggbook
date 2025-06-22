mod config;
mod server;
mod service;
mod stream;

// Include your internal tests
#[cfg(test)]
mod tests;

use crate::service::{model::SummarySourceFactoryMap, sources::factories::new_factory_map};
use aggcommon::{
    instrument::{Instrument, InstrumentMap, SourceSymbol},
    shutdown::{ShutdownReceiver, ctrl_c_shutdown_signal},
    sources::SourceId,
};
use anyhow::Result;
use rustls::crypto::ring::default_provider;
use service::manager::Manager;
use std::sync::Arc;
use tracing::{error, info};

/// Represents the trading pair symbol for ETH/BTC.
pub const SYMBOL_ETH_BTC: &str = "ETH-BTC";

/// Entry point for the application.
///
/// Parses command-line arguments, initializes logging, and starts the server.
#[tokio::main]
async fn main() -> Result<()> {
    // Parse the command line arguments using the CLI parser.
    let cli = config::Cli::try_parse_args().map_err(|e| {
        // Print the error and exit with the appropriate code if parsing fails.
        e.print().expect("Failed to write clap error");
        std::process::exit(e.exit_code());
    })?;

    // Install the default crypto provider for Rustls.
    rustls::crypto::CryptoProvider::install_default(default_provider())
        .expect("install rustls CryptoProvider");

    // Configure the logging system with an environment-based filter.
    tracing_subscriber::fmt().with_env_filter("info").init();

    // Convert the parsed CLI arguments into the application configuration.
    let config = config::AppConfig::try_from(cli).map_err(|e| {
        // Log an error if configuration parsing fails.
        error!("Failed to parse config: {}", e);
        e
    })?;

    // Run the server with the loaded configuration.
    if let Err(e) = run(config).await {
        // Log any errors encountered during server execution.
        error!("Server error: {}", e);
        return Err(anyhow::anyhow!("service failure"));
    }

    // Log a message indicating the server has shut down successfully.
    info!("Server shutdown complete");
    Ok(())
}

/// Runs the server with the given configuration.
///
/// This function sets up the necessary components for the gRPC server,
/// including the manager, factory map, and shutdown handling.
///
/// # Arguments
/// - `config`: The application configuration.
///
/// # Returns
/// - `Result<()>`: Indicates success or failure.
async fn run(config: config::AppConfig) -> Result<()> {
    const N: usize = 10; // Number of order book levels to track.

    // Set up graceful shutdown support using a Ctrl+C signal.
    let shutdown = ctrl_c_shutdown_signal();

    // Create the factories for the feeds.
    let factory_map: SummarySourceFactoryMap<N> = new_factory_map::<N>();

    // Create a manager for the aggregator service.
    let instruments_map: InstrumentMap = build_instrument_map();
    let manager: Arc<Manager<N>> =
        Manager::<N>::new(shutdown.clone(), instruments_map, factory_map);

    // Create the gRPC service instance with the default trading symbol.
    let grpc_service =
        service::AggregatorService::create_server(manager, SYMBOL_ETH_BTC.to_string());

    // Get the bind address from the configuration.
    let bind_addr = config.network.bind_addr;

    // Subscribe to the shutdown signal to handle graceful termination.
    let shutdown_rx: ShutdownReceiver = shutdown.subscribe();

    // Log a message indicating the server is starting on the specified address.
    info!("Starting gRPC server on {}", bind_addr);

    // Run the gRPC server and handle any errors that occur.
    let result: Result<()> = server::run_grpc_server(bind_addr, shutdown_rx, grpc_service).await;
    if result.is_err() {
        // Send a shutdown signal if the server encounters an error.
        let _ = shutdown.send(());
        return result;
    }

    Ok(())
}

/// Builds the instrument map for the aggregator service.
///
/// This function defines the trading instruments and their associated source symbols.
///
/// # Returns
/// - `InstrumentMap`: The map of trading instruments.
fn build_instrument_map() -> InstrumentMap {
    // Define the source symbols for the trading instruments.
    let source_symbols = vec![
        SourceSymbol {
            source: SourceId::Bitstamp,   // Source ID for Bitstamp.
            symbol: "ethbtc".to_string(), // Lowercase symbol for Bitstamp.
        },
        SourceSymbol {
            source: SourceId::Binance,    // Source ID for Binance.
            symbol: "ETHBTC".to_string(), // Uppercase symbol for Binance.
        },
    ];

    // Create an instrument for ETH-BTC with the defined source symbols.
    let instrument = Instrument::new("ETH-BTC".to_string(), source_symbols);

    // Return the instrument map containing the defined instrument.
    InstrumentMap::new(vec![instrument])
}
