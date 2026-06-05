use anyhow::Result;
use clap::Parser;
use jmcp_speechd::{serve, SpeechConfig};
use std::net::SocketAddr;

#[derive(Parser)]
#[command(author, version, about = "JMCP deterministic local speech daemon")]
struct Args {
    #[arg(long, env = "JMCP_SPEECHD_BIND", default_value = "127.0.0.1:18878")]
    bind: SocketAddr,
    #[arg(long, env = "JMCP_SPEECHD_ROLE", default_value = "both")]
    role: String,
    #[arg(long, env = "JMCP_SPEECHD_TRANSCRIPT", default_value = "")]
    transcript: String,
    #[arg(long, env = "JMCP_SPEECHD_FAIL_CLOSED", default_value_t = false)]
    fail_closed: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    serve(
        args.bind,
        SpeechConfig::new(args.role, args.transcript, args.fail_closed),
    )
    .await
}
