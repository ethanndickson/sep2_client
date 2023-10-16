use anyhow::Result;
use clap::Parser;
use sep2_test_server::TestServer;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port that the server should listen on
    port: i32,
    /// Path to a Server SSL Certificate
    cert: String,
    /// Path to the Server's SSL Private Key
    key: String,
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let addr = format!("127.0.0.1:{}", args.port);
    log::info!("Server listening on {addr}");
    let server = TestServer::new(&addr, &args.cert, &args.key).unwrap();
    server.run(tokio::signal::ctrl_c()).await
}
