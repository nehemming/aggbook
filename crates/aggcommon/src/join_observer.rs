use tokio::task::JoinHandle;
use tracing::error;

/// Extension trait for observing join handles.
pub trait ObserveJoinHandle {
    fn observe(self, name: &'static str);
}

impl<T> ObserveJoinHandle for JoinHandle<T>
where
    T: Send + 'static,
{
    /// Spawns a task that observes the join handle and logs errors if the task panics or is aborted.
    ///
    /// Use to ensure a spawn task is monitored and any errors are logged.
    fn observe(self, name: &'static str) {
        tokio::spawn(async move {
            if let Err(e) = self.await {
                error!("Task '{}' panicked or was aborted: {}", name, e);
            }
        });
    }
}
