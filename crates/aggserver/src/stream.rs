use async_stream::stream;
use std::pin::Pin;
use tokio::sync::mpsc::Receiver;
use tokio_stream::Stream;
use tonic::Status;

/// Converts a `tokio::sync::mpsc::Receiver` into a gRPC-compatible stream.
///
/// This function takes a receiver that yields `Result<T, Status>` items and
/// wraps it into a `Stream` that can be consumed by gRPC services.
///
/// # Type Parameters
/// - `T`: The type of the successful result in the `Result`.
///
/// # Arguments
/// - `rx`: The receiver channel that provides `Result<T, Status>` items.
///
/// # Returns
/// - A pinned, boxed stream of `Result<T, Status>` items.
pub fn into_grpc_stream<T>(
    mut rx: Receiver<Result<T, Status>>,
) -> Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>
where
    T: Send + 'static,
{
    // Wrap the receiver into a pinned, boxed stream using the `async_stream` crate.
    Box::pin(stream! {
        // Loop to continuously receive messages from the channel.
        while let Some(msg) = rx.recv().await {
            // Yield each message received from the channel.
            yield msg;
        }
        // If the sender is dropped and the channel is closed, yield an error indicating the stream ended.
        yield Err(Status::unavailable("stream ended"));
        // The stream ends when `recv()` returns `None`.
    })
}
