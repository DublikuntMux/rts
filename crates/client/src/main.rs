use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "rts", version, about = "Top-down 2D real-time strategy game")]
struct GameCli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run a dedicated authoritative QUIC multiplayer server.
    Server(rts_server::ServerArgs),
}

fn main() {
    let cli = GameCli::parse();
    match cli.command {
        None => rts_client::run(),
        Some(Command::Server(args)) => {
            if let Err(error) = rts_server::run_blocking(args) {
                eprintln!("failed to run server: {error}");
                std::process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gui_default_and_server_subcommand() {
        let gui = GameCli::try_parse_from(["rts"]).unwrap();
        assert!(gui.command.is_none());

        let srv = GameCli::try_parse_from([
            "rts",
            "server",
            "0.0.0.0:9001",
            "--seed",
            "7",
            "--assets",
            "assets",
        ])
        .unwrap();
        let Some(Command::Server(args)) = srv.command else {
            panic!("expected server subcommand");
        };
        assert_eq!(args.bind, "0.0.0.0:9001");
        assert_eq!(args.seed, Some(7));
        assert_eq!(args.assets.as_deref(), Some(std::path::Path::new("assets")));
    }
}
