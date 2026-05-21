use std::path::PathBuf;

use autowc_mcp::AutoWcMcpServer;
use clap::Parser;
use rmcp::{transport::stdio, ServiceExt};

#[derive(Debug, Parser)]
#[command(name = "autowc-mcp", about = "MCP server for AutoWC automation")]
struct Cli {
    /// Path to the AutoWC compositor binary.
    #[arg(long, default_value = "autowc")]
    autowc_binary: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let server = AutoWcMcpServer::new(cli.autowc_binary).await?;
    let shutdown = server.clone();
    let service = server.serve(stdio()).await?;
    let result = service.waiting().await;
    shutdown.shutdown().await;
    result?;
    Ok(())
}
