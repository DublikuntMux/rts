//! Command-line argument definition (powered by `clap`) shared by `rts-server` and `rts server`.

use std::{
    io::{self, Error, ErrorKind, Read},
    path::{Path, PathBuf},
};

use clap::{Args, Parser};

use crate::{Config, load_assets_dir, random_setup, serve_with_config};

/// Command-line options for running the authoritative QUIC server.
#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub struct ServerArgs {
    /// UDP address to bind, such as `0.0.0.0:9001`.
    #[arg(
        value_name = "BIND_ADDRESS",
        env = "RTS_SERVER_ADDR",
        default_value = "127.0.0.1:9001"
    )]
    pub bind: String,

    /// Custom rules RON file sent to every client.
    #[arg(long, value_name = "PATH")]
    pub rules: Option<PathBuf>,

    /// Fixed map RON file played in every room instead of private random layouts.
    #[arg(long, value_name = "PATH")]
    pub map: Option<PathBuf>,

    /// Unsigned 64-bit match seed for fixed maps.
    #[arg(long, value_name = "U64")]
    pub seed: Option<u64>,

    /// Reveal the map and its units to every player.
    #[arg(long)]
    pub no_fog: bool,

    /// Directory containing optional `textures/**/*.png` and `locales/*.lang.ron` files
    /// synced to connecting clients.
    #[arg(long, value_name = "DIR")]
    pub assets: Option<PathBuf>,
}

/// Standalone CLI for the `rts-server` binary.
#[derive(Parser, Debug)]
#[command(
    name = "rts-server",
    version,
    about = "Authoritative QUIC multiplayer server for RTS"
)]
pub struct ServerCli {
    #[command(flatten)]
    pub args: ServerArgs,
}

/// Validates CLI options, reads any external rules/map/assets files, and builds the server config.
///
/// # Errors
/// Returns an error if any file cannot be read, exceeds size bounds, or fails simulation/asset
/// validation.
pub fn build_config(args: &ServerArgs) -> io::Result<(String, Config)> {
    let mut config = Config {
        fog_of_war: !args.no_fog,
        ..Config::default()
    };
    if let Some(path) = &args.rules {
        config.setup.rules = read_document(path)?;
    }
    if let Some(path) = &args.map {
        config.setup.map = read_document(path)?;
        config.random_layout = false;
    }
    if let Some(seed) = args.seed {
        config.setup.seed = seed;
    }
    if let Some(dir) = &args.assets {
        config.assets = load_assets_dir(dir)?;
    }
    let map = rts_base::MapData::from_ron(&config.setup.map)
        .map_err(|error| Error::new(ErrorKind::InvalidInput, error))?;
    config.setup.players = u8::try_from(map.players.len())
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "too many players in map"))?;
    config
        .setup
        .simulation()
        .map_err(|error| Error::new(ErrorKind::InvalidInput, error))?;
    if config.random_layout {
        random_setup(&config.setup.rules, config.setup.seed)
            .map_err(|error| Error::new(ErrorKind::InvalidInput, error))?;
    }
    Ok((args.bind.clone(), config))
}

/// Starts and runs the QUIC server asynchronously until cancelled or terminated.
///
/// # Errors
/// Returns an error if configuration validation, UDP binding, TLS setup, or serving fails.
pub async fn run_server(args: ServerArgs) -> io::Result<()> {
    let (address, config) = build_config(&args)?;
    let socket = std::net::UdpSocket::bind(&address)?;
    let local_addr = socket.local_addr()?;
    let cert = rts_net::quic::ServerCertificate::generate_self_signed()?;
    let endpoint = rts_net::quic::server_endpoint(socket, &cert, config.lobby_timeout)?;
    eprintln!("RTS QUIC server listening on quic://{local_addr}");
    eprintln!("Certificate SHA-256: {}", cert.fingerprint);
    if !config.fog_of_war {
        eprintln!("Fog of war is disabled");
    }
    if !config.assets.is_empty() {
        eprintln!("Serving {} client asset(s)", config.assets.len());
    }
    serve_with_config(endpoint, config).await
}

/// Runs the QUIC server on a fresh Tokio runtime (used by `rts server`).
///
/// # Errors
/// Returns an error if the runtime cannot start or [`run_server`] fails.
pub fn run_blocking(args: ServerArgs) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_server(args))
}

fn read_document(path: &Path) -> io::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take((rts_net::MAX_SETUP_BYTES + 1) as u64)
        .read_to_string(&mut text)?;
    if text.len() > rts_net::MAX_SETUP_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "setup document exceeds 4 MiB",
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_build(args: &[&str]) -> io::Result<(String, Config)> {
        let tokens = std::iter::once("rts-server").chain(args.iter().copied());
        let cli = ServerCli::try_parse_from(tokens)
            .map_err(|error| Error::new(ErrorKind::InvalidInput, error.to_string()))?;
        build_config(&cli.args)
    }

    #[test]
    fn defaults_and_each_document_override_work_independently() {
        let (address, default) = parse_and_build(&["127.0.0.1:1234"]).unwrap();
        assert_eq!(address, "127.0.0.1:1234");
        assert_eq!(default.setup.players, 2);
        assert!(default.random_layout);
        assert!(default.fog_of_war);
        assert!(!parse_and_build(&["--no-fog"]).unwrap().1.fog_of_war);
        let (_, rules) = parse_and_build(&[
            "--rules",
            "../../assets/data/core.rules.ron",
            "--seed",
            "42",
        ])
        .unwrap();
        assert_eq!(rules.setup.rules, default.setup.rules);
        assert_eq!(rules.setup.map, default.setup.map);
        assert_eq!(rules.setup.seed, 42);
        assert!(rules.random_layout);
        let (_, map) =
            parse_and_build(&["--map", "../../assets/maps/proving_grounds.map.ron"]).unwrap();
        assert!(!map.random_layout);
        assert_eq!(
            map.setup.map,
            include_str!("../../../assets/maps/proving_grounds.map.ron")
        );
        let (_, with_assets) = parse_and_build(&["--assets", "../../assets"]).unwrap();
        assert!(with_assets.assets.contains_key("locales/en.lang.ron"));
        assert!(with_assets.assets.contains_key("textures/terrain/wood.png"));
    }

    #[test]
    fn custom_map_derives_player_count_and_bounded_reads_reject_excess_data() {
        let path =
            std::env::temp_dir().join(format!("rts-server-cli-{}.map.ron", std::process::id()));
        std::fs::write(
            &path,
            r#"(name:"solo",players:[(start:(0.5,0.5))],tiles:[".."],units:[])"#,
        )
        .unwrap();
        let result = parse_and_build(&[
            "--map",
            path.to_str().unwrap(),
            "127.0.0.1:0",
            "--seed",
            "99",
        ]);
        std::fs::write(&path, " ".repeat(rts_net::MAX_SETUP_BYTES + 1)).unwrap();
        let oversized = read_document(&path);
        std::fs::remove_file(path).unwrap();
        let (address, config) = result.unwrap();
        assert_eq!(address, "127.0.0.1:0");
        assert!(!config.random_layout);
        assert_eq!(config.setup.players, 1);
        assert_eq!(config.setup.seed, 99);
        assert_eq!(config.setup.simulation().unwrap().players().len(), 1);
        assert_eq!(oversized.unwrap_err().kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn cli_rejects_unknown_duplicate_missing_and_invalid_arguments_or_data() {
        for args in [
            vec!["--unknown"],
            vec!["--rules"],
            vec!["--map"],
            vec!["--seed"],
            vec!["--seed", "invalid"],
            vec!["--seed", "-1"],
            vec!["--seed", "0", "--seed", "1"],
            vec!["--assets"],
            vec!["--assets", "../../Cargo.toml"],
            vec!["addr", "other"],
            vec!["--rules", "../../Cargo.toml"],
            vec!["--map", "../../Cargo.toml"],
        ] {
            assert!(parse_and_build(&args).is_err(), "accepted {args:?}");
        }
    }
}
