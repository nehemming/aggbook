use clap::Parser;
use std::convert::TryFrom;

/// Represents the command-line interface for the application.
///
/// Provides options for configuring the server, such as the port to bind to.
#[derive(Parser, Debug, Clone)]
pub struct Cli {
    #[arg(long, default_value_t = 50051, help = "Port to bind to")]
    port: u16,

    #[arg(long, help = "Interface to listen on (default:127.0.0.1)")]
    listen_interface: Option<String>,
}

/// Represents the application configuration.
///
/// Contains network-related settings.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub network: NetworkConfig,
}

/// Represents the network configuration.
///
/// Contains the address to bind the server to.
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub bind_addr: std::net::SocketAddr,
}

/// Represents errors related to application configuration.
#[derive(Debug)]
pub enum AppConfigError {
    InvalidBindAddr(String),
}

impl std::error::Error for AppConfigError {}

impl std::fmt::Display for AppConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppConfigError::InvalidBindAddr(s) => write!(f, "Invalid bind address: {}", s),
        }
    }
}

impl TryFrom<Cli> for AppConfig {
    type Error = AppConfigError;

    /// Converts a `Cli` instance into an `AppConfig`.
    ///
    /// # Arguments
    /// - `cli`: The command-line interface instance.
    ///
    /// # Returns
    /// - `Result<Self, Self::Error>`: The application configuration or an error.
    fn try_from(cli: Cli) -> Result<Self, Self::Error> {
        let addr_str = format!(
            "{}:{}",
            cli.listen_interface
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            cli.port
        );

        let bind_addr = addr_str
            .parse()
            .map_err(|_| AppConfigError::InvalidBindAddr(addr_str))?;

        Ok(AppConfig {
            network: NetworkConfig { bind_addr },
        })
    }
}

impl Cli {
    /// Parses command-line arguments into a `Cli` instance.
    ///
    /// # Returns
    /// - `Result<Self, clap::Error>`: The parsed `Cli` instance or an error.
    pub fn try_parse_args() -> Result<Self, clap::Error> {
        Self::try_parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::TryFrom;

    #[test]
    fn test_valid_cli_conversion() {
        let cli = Cli {
            port: 8080,
            listen_interface: None,
        };
        let config = AppConfig::try_from(cli).expect("Should convert successfully");
        assert_eq!(config.network.bind_addr.to_string(), "127.0.0.1:8080");
    }

    #[test]
    fn test_valid_cli_conversion_with_interface() {
        let cli = Cli {
            port: 8080,
            listen_interface: Some("0.0.0.0".to_string()),
        };
        let config = AppConfig::try_from(cli).expect("Should convert successfully");
        assert_eq!(config.network.bind_addr.to_string(), "0.0.0.0:8080");
    }

    #[test]
    fn test_app_config_error_display() {
        let err = AppConfigError::InvalidBindAddr("bad_addr".to_string());
        let msg = format!("{}", err);
        assert_eq!(msg, "Invalid bind address: bad_addr");
    }
}
