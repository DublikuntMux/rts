use clap::Parser;
use rts_server::{ServerCli, run_server};

#[tokio::main]
async fn main() {
    let cli = ServerCli::parse();
    if let Err(error) = run_server(cli.args).await {
        eprintln!("failed to run server: {error}");
        std::process::exit(1);
    }
}
