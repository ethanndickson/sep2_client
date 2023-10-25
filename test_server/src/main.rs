use anyhow::Result;
use clap::Parser;
use sep2_test_server::TestServer;
use simple_logger::SimpleLogger;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port that the server should listen on
    port: u16,
    /// Path to a Server SSL Certificate
    cert: String,
    /// Path to the Server's SSL Private Key
    key: String,
    /// Path to the rootCA
    ca: String,
}
#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new()
        .with_level(log::LevelFilter::Debug)
        .init()
        .unwrap();
    let args = Args::parse();
    let server =
        TestServer::new(("127.0.0.1", *&args.port), &args.cert, &args.key, &args.ca).unwrap();
    server.run(tokio::signal::ctrl_c()).await
}
