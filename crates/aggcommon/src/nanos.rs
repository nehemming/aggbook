use humantime::format_duration;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

/// Represents the time in nanoseconds since the epoch.
/// Alias for `u64` to improve code readability.
pub type NanoTime = u64;

/// Returns the current time in nanoseconds since the Unix epoch.
pub fn nanos_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_nanos() as u64
}

/// Converts a `NanoTime` value to a `String`.
pub fn nanos_pretty(nanos: NanoTime) -> String {
    let duration = Duration::from_nanos(nanos);
    format_duration(duration).to_string()
}
