#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("failed to run relay: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn run() -> std::io::Result<()> {
    let address = bind_address()?;
    let listener = tokio::net::TcpListener::bind(&address).await?;
    eprintln!(
        "RTS relay listening on ws://{} (use a reverse proxy for TLS)",
        listener.local_addr()?
    );
    rts_server::serve(listener).await
}

#[cfg(not(target_arch = "wasm32"))]
fn bind_address() -> std::io::Result<String> {
    let mut arguments = std::env::args().skip(1);
    let address = arguments
        .next()
        .or_else(|| std::env::var("RTS_SERVER_ADDR").ok())
        .unwrap_or_else(|| "127.0.0.1:9001".to_owned());
    if arguments.next().is_some() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "usage: rts-server [bind-address]",
        ));
    }
    Ok(address)
}

#[cfg(target_arch = "wasm32")]
fn main() {}
