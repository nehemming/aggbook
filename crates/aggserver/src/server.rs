use crate::service::GrpcService;
use std::net::SocketAddr;
use tokio::sync::broadcast;
use tonic::transport::Server;

/// Runs the gRPC server for the order book aggregator service.
///
/// # Arguments
/// - `addr`: The socket address to bind the server to.
/// - `shutdown`: Receiver for shutdown signals.
/// - `orderbook_service`: The gRPC service instance.
///
/// # Returns
/// - `anyhow::Result<()>`: Indicates success or failure.
pub async fn run_grpc_server<const N: usize>(
    addr: SocketAddr,
    mut shutdown: broadcast::Receiver<()>,
    orderbook_service: GrpcService<N>,
) -> anyhow::Result<()> {
    Server::builder()
        .add_service(orderbook_service)
        .serve_with_shutdown(addr, async move {
            let _ = shutdown.recv().await;
        })
        .await?;
    Ok(())
}
