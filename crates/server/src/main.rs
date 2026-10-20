#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("failed to run server: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn run() -> std::io::Result<()> {
    let (address, config) = configuration(std::env::args().skip(1))?;
    // configuration validates the complete map and rules before opening the port.
    let listener = tokio::net::TcpListener::bind(&address).await?;
    eprintln!(
        "RTS server listening on ws://{} (use a reverse proxy for TLS)",
        listener.local_addr()?
    );
    rts_server::serve_with_config(listener, config).await
}

#[cfg(not(target_arch = "wasm32"))]
fn configuration(
    arguments: impl IntoIterator<Item = String>,
) -> std::io::Result<(String, rts_server::Config)> {
    use std::io::{Error, ErrorKind};

    const USAGE: &str = "usage: rts-server [bind-address] [--rules PATH] [--map PATH] [--seed u64]";
    let invalid = || Error::new(ErrorKind::InvalidInput, USAGE);
    let mut arguments = arguments.into_iter();
    let mut address = None;
    let mut rules = None;
    let mut map = None;
    let mut seed = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--rules" if rules.is_none() => rules = Some(arguments.next().ok_or_else(invalid)?),
            "--map" if map.is_none() => map = Some(arguments.next().ok_or_else(invalid)?),
            "--seed" if seed.is_none() => {
                seed = Some(
                    arguments
                        .next()
                        .ok_or_else(invalid)?
                        .parse::<u64>()
                        .map_err(|_| invalid())?,
                );
            }
            value if !value.starts_with('-') && address.is_none() => address = Some(argument),
            _ => return Err(invalid()),
        }
    }
    let mut config = rts_server::Config::default();
    if let Some(path) = rules {
        config.setup.rules = read_document(&path)?;
    }
    if let Some(path) = map {
        config.setup.map = read_document(&path)?;
    }
    if let Some(seed) = seed {
        config.setup.seed = seed;
    }
    let map = rts_base::MapData::from_ron(&config.setup.map)
        .map_err(|error| Error::new(ErrorKind::InvalidInput, error))?;
    config.setup.players = u8::try_from(map.players.len()).map_err(|_| invalid())?;
    config
        .setup
        .simulation()
        .map_err(|error| Error::new(ErrorKind::InvalidInput, error))?;
    let address = address
        .or_else(|| std::env::var("RTS_SERVER_ADDR").ok())
        .unwrap_or_else(|| "127.0.0.1:9001".to_owned());
    Ok((address, config))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_document(path: &str) -> std::io::Result<String> {
    use std::io::{Error, ErrorKind, Read};
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

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn config(args: &[&str]) -> std::io::Result<(String, rts_server::Config)> {
        configuration(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn defaults_and_each_document_override_work_independently() {
        let (address, default) = config(&["127.0.0.1:1234"]).unwrap();
        assert_eq!(address, "127.0.0.1:1234");
        assert_eq!(default.setup.players, 2);
        let (_, rules) = config(&[
            "--rules",
            "../../assets/data/core.rules.ron",
            "--seed",
            "42",
        ])
        .unwrap();
        assert_eq!(rules.setup.rules, default.setup.rules);
        assert_eq!(rules.setup.map, default.setup.map);
        assert_eq!(rules.setup.seed, 42);
        let (_, map) = config(&["--map", "../../assets/maps/proving_grounds.map.ron"]).unwrap();
        assert_eq!(map.setup, default.setup);
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
        let result = config(&[
            "--map",
            path.to_str().unwrap(),
            "127.0.0.1:0",
            "--seed",
            "99",
        ]);
        std::fs::write(&path, " ".repeat(rts_net::MAX_SETUP_BYTES + 1)).unwrap();
        let oversized = read_document(path.to_str().unwrap());
        std::fs::remove_file(path).unwrap();
        let (address, config) = result.unwrap();
        assert_eq!(address, "127.0.0.1:0");
        assert_eq!(config.setup.players, 1);
        assert_eq!(config.setup.seed, 99);
        assert_eq!(config.setup.simulation().unwrap().players().len(), 1);
        assert_eq!(
            oversized.unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
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
            vec!["addr", "other"],
            vec!["--rules", "../../Cargo.toml"],
            vec!["--map", "../../Cargo.toml"],
        ] {
            assert!(config(&args).is_err(), "accepted {args:?}");
        }
    }
}
