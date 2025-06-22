mod config;
mod format;

use aggcommon::proto::orderbook::{Empty, orderbook_aggregator_client::OrderbookAggregatorClient};
use anyhow::{Context, Result};
use format::DisplaySummary;
use tokio_stream::StreamExt;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // Parse the command line arguments.
    let cli = config::Cli::try_parse_args().map_err(|e| {
        e.print().expect("Failed to write clap error");
        std::process::exit(e.exit_code());
    })?;

    let shutdown = aggcommon::shutdown::ctrl_c_shutdown_signal();
    let mut shutdown_rx = shutdown.subscribe();

    // Initialize tracing
    tracing_subscriber::fmt().with_env_filter("info").init();
    info!("Connecting to server at {}", cli.server_addr);

    // Connect to server
    let mut client = OrderbookAggregatorClient::connect(cli.server_addr.clone())
        .await
        .context("Failed to connect to server")?;

    // Create request (Empty message)
    let request = tonic::Request::new(Empty {});
    let mut stream = client
        .book_summary(request)
        .await
        .context("Failed to initiate BookSummary stream")?
        .into_inner();

    // Main streaming loop
    loop {
        tokio::select! {
            maybe_item = stream.next() => {
                match maybe_item {
                    Some(Ok(summary)) => {
                        println!("{}", DisplaySummary(&summary));
                    }
                    Some(Err(e)) => {
                        error!("Stream error: {:?}", e);
                        return Err(e.into());
                    }
                    None => {
                        info!("Server closed stream cleanly");
                        break;
                    }
                }
            }

            _ = shutdown_rx.recv() => {
                info!("Shutdown signal received. Closing client...");
                break;
            }
        }
    }

    Ok(())
}
