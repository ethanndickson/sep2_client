use clap::Parser;
use log::info;
use server::Server;
use std::error::Error;

mod server;
mod tls;

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
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args = Args::parse();
    let addr = format!("127.0.0.1:{}", args.port);
    let server = Server::new(&addr, &args.cert, &args.key).unwrap();
    info!("Server listening on {addr}");
    server.run().await
}
