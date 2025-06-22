use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
pub struct Cli {
    #[arg(
        long,
        default_value = "http://localhost:50051",
        help = "Server address to connect to"
    )]
    pub server_addr: String,
}

impl Cli {
    pub fn try_parse_args() -> Result<Self, clap::Error> {
        Self::try_parse()
    }
}
