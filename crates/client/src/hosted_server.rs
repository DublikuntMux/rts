//! Embedded authoritative QUIC server managed directly from the game UI.

use std::{
    net::{SocketAddr, UdpSocket},
    path::Path,
    sync::mpsc,
    thread::JoinHandle,
};

use bevy::prelude::*;
use rts_net::{
    MatchSetup,
    quic::{ServerCertificate, server_endpoint},
};
use rts_server::{Config, RANDOM_MAP_NAME, load_assets_dir, serve_with_config};
use tokio::sync::oneshot;

/// Default UDP bind address for a GUI-hosted server.
pub const DEFAULT_HOST_BIND: &str = "0.0.0.0:9001";

/// Connection details for an active embedded QUIC server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostedServerInfo {
    pub bind_addr: SocketAddr,
    /// Loopback URL with pinned SHA-256 fingerprint for the local client.
    pub local_url: String,
    /// Shareable URL (with bound host/port and pinned SHA-256 fingerprint).
    pub share_url: String,
    pub fingerprint: String,
    pub map_name: String,
    pub fog_of_war: bool,
    pub asset_count: usize,
}

struct RunningServer {
    info: HostedServerInfo,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

/// Manages an embedded `rts_server` instance started from the Multiplayer menu.
#[derive(Resource, Default)]
pub struct HostedServer {
    running: Option<RunningServer>,
}

impl HostedServer {
    /// Returns details of the currently running embedded server, if any.
    #[must_use]
    pub fn info(&self) -> Option<&HostedServerInfo> {
        self.running.as_ref().map(|server| &server.info)
    }

    /// Whether an embedded server is currently listening.
    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Starts an embedded QUIC server on `bind_addr`.
    ///
    /// Pass `setup = None` to generate private *Random frontier* layouts, or `Some((name, setup))`
    /// to host a specific fixed map. If `assets_dir` is non-empty, textures and locales from that
    /// folder are validated and served to connecting clients.
    ///
    /// # Errors
    /// Returns a user-facing error if the address, map/rules setup, assets folder, or QUIC
    /// endpoint fails validation or startup.
    pub fn start(
        &mut self,
        bind_addr: &str,
        setup: Option<(String, MatchSetup)>,
        assets_dir: Option<&str>,
        fog_of_war: bool,
    ) -> Result<HostedServerInfo, String> {
        let bind_addr = bind_addr.trim();
        if bind_addr.is_empty() {
            return Err("Enter a UDP bind address such as 0.0.0.0:9001.".into());
        }
        let (config, map_name) = build_hosted_config(setup, assets_dir, fog_of_war)?;
        let asset_count = config.assets.len();

        // Stop any previous embedded server before binding so reusing the same port works.
        self.stop();

        let socket = UdpSocket::bind(bind_addr)
            .map_err(|error| format!("Cannot bind UDP `{bind_addr}`: {error}"))?;
        let local_addr = socket
            .local_addr()
            .map_err(|error| format!("Cannot inspect bound socket: {error}"))?;
        let cert = ServerCertificate::generate_self_signed()
            .map_err(|error| format!("Cannot generate server certificate: {error}"))?;
        let fingerprint = cert.fingerprint.clone();

        let local_host = if local_addr.is_ipv6() {
            "[::1]"
        } else {
            "127.0.0.1"
        };
        let local_url = format!("quic://{local_host}:{}#{fingerprint}", local_addr.port());
        let share_url = format!("quic://{local_addr}#{fingerprint}");

        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let thread = std::thread::Builder::new()
            .name("rts-hosted-server".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!("Cannot start server runtime: {error}")));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let endpoint = match server_endpoint(socket, &cert, config.lobby_timeout) {
                        Ok(endpoint) => endpoint,
                        Err(error) => {
                            let _ =
                                ready_tx.send(Err(format!("Cannot start QUIC endpoint: {error}")));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(()));
                    tokio::select! {
                        result = serve_with_config(endpoint.clone(), config) => {
                            if let Err(error) = result {
                                warn!("hosted server stopped: {error}");
                            }
                        }
                        _ = shutdown_rx => {
                            endpoint.close(0_u32.into(), b"server stopped");
                        }
                    }
                });
            })
            .map_err(|error| format!("Cannot spawn server thread: {error}"))?;

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(error);
            }
            Err(_) => {
                let _ = thread.join();
                return Err("Server thread exited before listening.".into());
            }
        }

        let info = HostedServerInfo {
            bind_addr: local_addr,
            local_url,
            share_url,
            fingerprint,
            map_name,
            fog_of_war,
            asset_count,
        };
        self.running = Some(RunningServer {
            info: info.clone(),
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        });
        Ok(info)
    }

    /// Stops the embedded server if running and releases its UDP port.
    pub fn stop(&mut self) {
        self.running = None;
    }
}

fn build_hosted_config(
    setup: Option<(String, MatchSetup)>,
    assets_dir: Option<&str>,
    fog_of_war: bool,
) -> Result<(Config, String), String> {
    let mut config = Config {
        fog_of_war,
        ..Config::default()
    };
    let map_name = match setup {
        Some((name, custom)) => {
            config.setup = custom;
            config.random_layout = false;
            name
        }
        None => RANDOM_MAP_NAME.to_owned(),
    };
    if let Some(dir) = assets_dir.map(str::trim).filter(|dir| !dir.is_empty()) {
        config.assets = load_assets_dir(Path::new(dir))
            .map_err(|error| format!("Cannot load assets folder: {error}"))?;
    }
    config
        .setup
        .simulation()
        .map_err(|error| format!("Invalid match setup: {error}"))?;
    if config.random_layout {
        rts_server::random_setup(&config.setup.rules, config.setup.seed)
            .map_err(|error| format!("Invalid random layout setup: {error}"))?;
    }
    Ok((config, map_name))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rts_net::{
        ClientMessage, MAX_SERVER_MESSAGE_BYTES, PROTOCOL_VERSION, ServerMessage, decode_server,
        encode_client,
        quic::{
            PRIORITY_GAME, STREAM_GAME, client_endpoint, parse_server_url, read_frame, write_frame,
        },
    };

    use super::*;

    #[tokio::test]
    async fn embedded_server_starts_serves_quic_clients_and_stops_cleanly() {
        let mut hosted = HostedServer::default();
        let info = hosted.start("127.0.0.1:0", None, None, true).unwrap();
        assert!(hosted.is_running());
        assert_eq!(info.map_name, RANDOM_MAP_NAME);
        assert_eq!(info.fingerprint.len(), 64);

        let parsed = parse_server_url(&info.local_url).unwrap();
        let endpoint =
            client_endpoint(false, parsed.pinned_sha256, Duration::from_secs(5)).unwrap();
        let conn = endpoint
            .connect(info.bind_addr, "rts.local")
            .unwrap()
            .await
            .unwrap();
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        let _ = send.set_priority(PRIORITY_GAME);
        send.write_all(&[STREAM_GAME]).await.unwrap();
        let rooms_req = encode_client(&ClientMessage::Rooms {
            version: PROTOCOL_VERSION,
        })
        .unwrap();
        write_frame(&mut send, &rooms_req).await.unwrap();
        let raw = read_frame(&mut recv, MAX_SERVER_MESSAGE_BYTES)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            decode_server(&raw).unwrap(),
            ServerMessage::Rooms { rooms: Vec::new() }
        );

        let addr = info.bind_addr;
        hosted.stop();
        assert!(!hosted.is_running());
        // Port should be free immediately after shutdown.
        let rebound = UdpSocket::bind(addr);
        assert!(rebound.is_ok(), "UDP port was not released after stop");
    }
}
