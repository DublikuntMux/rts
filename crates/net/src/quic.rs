//! Pure-Rust QUIC transport built on `quinn`, `rustls`, `glomar`, and `glomar-rustls`.
//!
//! No C or assembly cryptography is used: packet protection, header protection, TLS 1.3
//! handshakes, QUIC retry tags, and self-signed Ed25519 certificates all run in pure Rust.

use std::{
    any::Any,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket},
    sync::Arc,
    time::Duration,
};

use bytes::BytesMut;
use glomar::{
    aead, digest, hkdf, hmac,
    rand::{SecureRandom, SystemRandom},
    signature::{Ed25519KeyPair, KeyPair as _},
};
use quinn::{
    ClientConfig, Endpoint, EndpointConfig, RecvStream, SendStream, ServerConfig, TransportConfig,
    VarInt,
};
use quinn_proto::{
    ConnectError, ConnectionId, Side, TransportError, TransportErrorCode,
    crypto::{
        self, CryptoError, ExportKeyingMaterialError, HeaderKey, KeyPair, Keys, PacketKey,
        UnsupportedVersion,
    },
    transport_parameters::TransportParameters,
};
use rcgen::{CertificateParams, PublicKeyData, SignatureAlgorithm, SigningKey};
use rustls::{
    CipherSuite, DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    quic::{
        Connection as RustlsQuicConnection, HeaderProtectionKey, KeyChange,
        PacketKey as RustlsPacketKey, Secrets, Suite, Version,
    },
};

/// ALPN identifier for the RTS QUIC protocol.
pub const ALPN: &[u8] = b"rts-quic/10";
/// Default QUIC UDP port when omitted from a server address.
pub const DEFAULT_PORT: u16 = 9001;
/// Stream kind byte for the primary ordered control and gameplay stream.
pub const STREAM_GAME: u8 = 0;
/// Stream kind byte for background server-to-client asset transfers.
pub const STREAM_ASSETS: u8 = 1;
/// Highest priority for the ordered gameplay and control stream.
pub const PRIORITY_GAME: i32 = 10;
/// Lower priority for background asset downloads so they never stall gameplay.
pub const PRIORITY_ASSETS: i32 = -10;

pub use quinn;

/// Parsed `quic://host:port[#sha256]` server address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerUrl {
    pub canonical: String,
    pub host: String,
    pub port: u16,
    pub pinned_sha256: Option<[u8; 32]>,
}

impl ServerUrl {
    /// Returns `"host:port"` formatted for `tokio::net::lookup_host`.
    #[must_use]
    pub fn socket_host_port(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// Returns a TLS SNI server name suitable for `quinn::Endpoint::connect`.
    #[must_use]
    pub fn sni_name(&self) -> &str {
        if self.host.parse::<IpAddr>().is_ok() {
            "rts.local"
        } else {
            &self.host
        }
    }
}

/// Parses a `quic://host[:port][#sha256]` or bare `host[:port]` server address.
///
/// # Errors
/// Returns a human-readable error if the scheme, host, port, or optional SHA-256 pin is invalid.
pub fn parse_server_url(raw: &str) -> Result<ServerUrl, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 2048 {
        return Err("Enter a server address such as quic://127.0.0.1:9001.".into());
    }
    let without_scheme = if let Some(rest) = trimmed.strip_prefix("quic://") {
        rest
    } else if trimmed.contains("://") {
        return Err("Enter a server address beginning with quic://.".into());
    } else {
        trimmed
    };
    let (authority, pin_hex) = match without_scheme.split_once('#') {
        Some((auth, pin)) => (auth, Some(pin)),
        None => (without_scheme, None),
    };
    let authority = authority.trim_end_matches('/');
    if authority.is_empty() || authority.contains('/') || authority.chars().any(char::is_whitespace)
    {
        return Err("Invalid server address.".into());
    }
    let (host, port) = split_host_port(authority)?;
    let pinned_sha256 = match pin_hex {
        Some(hex) => Some(decode_sha256_hex(hex)?),
        None => None,
    };
    let canonical_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.clone()
    };
    let canonical = pin_hex.map_or_else(
        || format!("quic://{canonical_host}:{port}"),
        |pin| {
            format!(
                "quic://{canonical_host}:{port}#{}",
                pin.to_ascii_lowercase()
            )
        },
    );
    Ok(ServerUrl {
        canonical,
        host,
        port,
        pinned_sha256,
    })
}

fn split_host_port(authority: &str) -> Result<(String, u16), String> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .ok_or_else(|| "Invalid IPv6 address.".to_owned())?;
        if host.is_empty() || host.parse::<Ipv6Addr>().is_err() {
            return Err("Invalid IPv6 address.".into());
        }
        let port = if after.is_empty() {
            DEFAULT_PORT
        } else {
            let port_str = after
                .strip_prefix(':')
                .ok_or_else(|| "Invalid server port.".to_owned())?;
            parse_port(port_str)?
        };
        return Ok((host.to_owned(), port));
    }
    if let Some((host, port_str)) = authority.rsplit_once(':') {
        if host.is_empty() || host.contains(':') {
            return Err("Wrap IPv6 addresses in brackets, e.g. quic://[::1]:9001.".into());
        }
        Ok((host.to_owned(), parse_port(port_str)?))
    } else {
        Ok((authority.to_owned(), DEFAULT_PORT))
    }
}

fn parse_port(text: &str) -> Result<u16, String> {
    let port: u16 = text
        .parse()
        .map_err(|_| "Invalid server port.".to_owned())?;
    if port == 0 {
        return Err("Server port must be nonzero.".into());
    }
    Ok(port)
}

fn decode_sha256_hex(hex: &str) -> Result<[u8; 32], String> {
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Certificate fingerprint must be 64 hex characters.".into());
    }
    let mut out = [0_u8; 32];
    for (index, chunk) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let hi = hex_nibble(chunk[0]);
        let lo = hex_nibble(chunk[1]);
        out[index] = (hi << 4) | lo;
    }
    Ok(out)
}

const fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}

/// Formats a 32-byte SHA-256 digest as lowercase hexadecimal.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let hash = sha256(bytes);
    let mut out = String::with_capacity(64);
    for byte in hash {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Computes the SHA-256 digest of `bytes` in pure Rust.
#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out.copy_from_slice(digest::digest(&digest::SHA256, bytes).as_ref());
    out
}

/// An ephemeral or loaded Ed25519 TLS certificate for a QUIC server.
#[derive(Debug)]
pub struct ServerCertificate {
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivateKeyDer<'static>,
    pub fingerprint: String,
}

impl Clone for ServerCertificate {
    fn clone(&self) -> Self {
        Self {
            cert_der: self.cert_der.clone(),
            key_der: self.key_der.clone_key(),
            fingerprint: self.fingerprint.clone(),
        }
    }
}

impl ServerCertificate {
    /// Generates a fresh self-signed Ed25519 certificate using pure-Rust cryptography.
    ///
    /// # Errors
    /// Returns an I/O error if OS entropy or certificate serialization fails.
    pub fn generate_self_signed() -> io::Result<Self> {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)
            .map_err(|_| io::Error::other("failed to generate Ed25519 key"))?;
        let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
            .map_err(|_| io::Error::other("failed to parse generated Ed25519 key"))?;
        let signer = Ed25519RcgenSigner { key_pair };
        let mut params =
            CertificateParams::new(vec!["rts.local".to_owned(), "localhost".to_owned()])
                .map_err(|error| io::Error::other(format!("certificate params: {error}")))?;
        let mut serial = [0_u8; 16];
        rng.fill(&mut serial)
            .map_err(|_| io::Error::other("failed to generate certificate serial number"))?;
        serial[0] &= 0x7f;
        params.serial_number = Some(rcgen::SerialNumber::from_slice(&serial));
        let cert = params
            .self_signed(&signer)
            .map_err(|error| io::Error::other(format!("self-sign certificate: {error}")))?;
        let cert_der = cert.der().clone();
        let fingerprint = sha256_hex(cert_der.as_ref());
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8.as_ref().to_vec()));
        Ok(Self {
            cert_der,
            key_der,
            fingerprint,
        })
    }
}

struct Ed25519RcgenSigner {
    key_pair: Ed25519KeyPair,
}

impl PublicKeyData for Ed25519RcgenSigner {
    fn der_bytes(&self) -> &[u8] {
        self.key_pair.public_key().as_ref()
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &rcgen::PKCS_ED25519
    }
}

impl SigningKey for Ed25519RcgenSigner {
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        Ok(self.key_pair.sign(msg).as_ref().to_vec())
    }
}

/// Creates a QUIC server endpoint bound to `socket` with pure-Rust TLS 1.3 and tuned transport
/// parameters.
///
/// # Errors
/// Returns an I/O error if the socket or TLS configuration cannot be created.
pub fn server_endpoint(
    socket: UdpSocket,
    cert: &ServerCertificate,
    idle_timeout: Duration,
) -> io::Result<Endpoint> {
    let mut tls = rustls::ServerConfig::builder_with_provider(pure_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| io::Error::other(format!("TLS 1.3 config: {error}")))?
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert_der.clone()], cert.key_der.clone_key())
        .map_err(|error| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("TLS cert: {error}"))
        })?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    tls.max_early_data_size = u32::MAX;

    let crypto = QuicServerCrypto::new(Arc::new(tls))
        .map_err(|error| io::Error::other(format!("QUIC crypto: {error}")))?;
    let token_key = random_token_key()?;
    let mut server_config = ServerConfig::new(Arc::new(crypto), Arc::new(token_key));
    server_config.transport_config(Arc::new(game_transport_config(idle_timeout)));

    let endpoint_config = EndpointConfig::new(Arc::new(random_reset_key()?));
    Endpoint::new(
        endpoint_config,
        Some(server_config),
        socket,
        Arc::new(quinn::TokioRuntime),
    )
}

/// Creates a QUIC client endpoint configured with pure-Rust TLS 1.3 and optional SHA-256
/// certificate pinning.
///
/// # Errors
/// Returns an I/O error if the local UDP socket or TLS configuration cannot be created.
pub fn client_endpoint(
    bind_ipv6: bool,
    pinned_sha256: Option<[u8; 32]>,
    idle_timeout: Duration,
) -> io::Result<Endpoint> {
    let bind_addr = if bind_ipv6 {
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)
    } else {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
    };
    let socket = UdpSocket::bind(bind_addr)?;
    let verifier = Arc::new(GameServerVerifier { pinned_sha256 });
    let mut tls = rustls::ClientConfig::builder_with_provider(pure_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| io::Error::other(format!("TLS 1.3 config: {error}")))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    tls.enable_early_data = true;

    let crypto = QuicClientCrypto::new(Arc::new(tls))
        .map_err(|error| io::Error::other(format!("QUIC crypto: {error}")))?;
    let mut client_config = ClientConfig::new(Arc::new(crypto));
    client_config.transport_config(Arc::new(game_transport_config(idle_timeout)));

    let endpoint_config = EndpointConfig::new(Arc::new(random_reset_key()?));
    let mut endpoint = Endpoint::new(endpoint_config, None, socket, Arc::new(quinn::TokioRuntime))?;
    endpoint.set_default_client_config(client_config);
    Ok(endpoint)
}

fn game_transport_config(idle_timeout: Duration) -> TransportConfig {
    let mut transport = TransportConfig::default();
    let keep_alive = (idle_timeout / 3).clamp(Duration::from_millis(50), Duration::from_secs(3));
    transport.keep_alive_interval(Some(keep_alive));
    if let Ok(timeout) = idle_timeout.try_into() {
        transport.max_idle_timeout(Some(timeout));
    }
    transport.initial_rtt(Duration::from_millis(80));
    transport.max_concurrent_bidi_streams(VarInt::from_u32(16));
    transport.max_concurrent_uni_streams(VarInt::from_u32(16));
    transport
}

/// Writes one length-prefixed binary frame to a QUIC send stream.
///
/// The length prefix uses unsigned LEB128 varint encoding (1 byte for frames under 128 bytes).
///
/// # Errors
/// Returns an I/O error if the stream fails or closes.
pub async fn write_frame(send: &mut SendStream, payload: &[u8]) -> io::Result<()> {
    let mut header = [0_u8; 10];
    let header_len = encode_varint(payload.len() as u64, &mut header);
    send.write_all(&header[..header_len])
        .await
        .map_err(io::Error::other)?;
    send.write_all(payload).await.map_err(io::Error::other)?;
    Ok(())
}

/// Reads one length-prefixed binary frame from a QUIC receive stream, enforcing `max_bytes`
/// before allocating. Returns `Ok(None)` on a clean EOF before the next frame starts.
///
/// # Errors
/// Returns an I/O error for truncation, invalid varint, oversized frame, or stream error.
pub async fn read_frame(recv: &mut RecvStream, max_bytes: usize) -> io::Result<Option<Vec<u8>>> {
    let mut first = [0_u8; 1];
    match recv.read(&mut first).await.map_err(io::Error::other)? {
        None | Some(0) => return Ok(None),
        Some(_) => {}
    }
    let mut len = u64::from(first[0] & 0x7f);
    let mut shift = 7_u32;
    let mut byte = first[0];
    while byte & 0x80 != 0 {
        if shift >= 64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame length varint overflow",
            ));
        }
        let mut next = [0_u8; 1];
        recv.read_exact(&mut next)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::UnexpectedEof, "truncated frame length"))?;
        byte = next[0];
        let part = u64::from(byte & 0x7f);
        if shift == 63 && part > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame length varint overflow",
            ));
        }
        len |= part << shift;
        shift += 7;
    }
    let len = usize::try_from(len)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame length overflow"))?;
    if len > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame exceeds {max_bytes} bytes"),
        ));
    }
    let mut buf = vec![0_u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::UnexpectedEof, "truncated frame payload"))?;
    Ok(Some(buf))
}

const fn encode_varint(mut value: u64, out: &mut [u8; 10]) -> usize {
    let mut idx = 0;
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out[idx] = byte;
        idx += 1;
        if value == 0 {
            return idx;
        }
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust certificate verifier for game servers
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct GameServerVerifier {
    pinned_sha256: Option<[u8; 32]>,
}

impl ServerCertVerifier for GameServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if let Some(expected) = self.pinned_sha256 {
            let actual = sha256(end_entity.as_ref());
            if actual != expected {
                return Err(rustls::Error::General(
                    "server certificate fingerprint mismatch".into(),
                ));
            }
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.2 is not supported".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &pure_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        pure_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust quinn_proto::crypto adapter backed by rustls + glomar-rustls
// ---------------------------------------------------------------------------

fn pure_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(glomar_rustls::default_provider())
}

fn random_reset_key() -> io::Result<GlomarHmacKey> {
    let mut key = [0_u8; 64];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| io::Error::other("failed to draw stateless reset key"))?;
    Ok(GlomarHmacKey(hmac::Key::new(hmac::HMAC_SHA256, &key)))
}

fn random_token_key() -> io::Result<GlomarTokenKey> {
    let mut key = [0_u8; 64];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| io::Error::other("failed to draw handshake token key"))?;
    Ok(GlomarTokenKey(hkdf::Prk::new_less_safe(
        hkdf::HKDF_SHA256,
        &key,
    )))
}

struct GlomarHmacKey(hmac::Key);

impl crypto::HmacKey for GlomarHmacKey {
    fn sign(&self, data: &[u8], out: &mut [u8]) {
        out.copy_from_slice(hmac::sign(&self.0, data).as_ref());
    }

    fn signature_len(&self) -> usize {
        32
    }

    fn verify(&self, data: &[u8], signature: &[u8]) -> Result<(), CryptoError> {
        hmac::verify(&self.0, data, signature).map_err(|_| CryptoError)
    }
}

struct GlomarTokenKey(hkdf::Prk);

impl crypto::HandshakeTokenKey for GlomarTokenKey {
    fn aead_from_hkdf(&self, random_bytes: &[u8]) -> Box<dyn crypto::AeadKey> {
        let mut key_buffer = [0_u8; 32];
        let info = [random_bytes];
        let okm = self
            .0
            .expand(&info, hkdf::HKDF_SHA256)
            .expect("valid HKDF expand");
        okm.fill(&mut key_buffer).expect("valid HKDF fill");
        let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key_buffer).expect("valid key");
        Box::new(GlomarAeadKey(aead::LessSafeKey::new(unbound)))
    }
}

struct GlomarAeadKey(aead::LessSafeKey);

impl crypto::AeadKey for GlomarAeadKey {
    fn seal(&self, data: &mut Vec<u8>, additional_data: &[u8]) -> Result<(), CryptoError> {
        let aad = aead::Aad::from(additional_data);
        let nonce = aead::Nonce::assume_unique_for_key([0_u8; 12]);
        self.0
            .seal_in_place_append_tag(nonce, aad, data)
            .map_err(|_| CryptoError)
    }

    fn open<'a>(
        &self,
        data: &'a mut [u8],
        additional_data: &[u8],
    ) -> Result<&'a mut [u8], CryptoError> {
        let aad = aead::Aad::from(additional_data);
        let nonce = aead::Nonce::assume_unique_for_key([0_u8; 12]);
        self.0
            .open_in_place(nonce, aad, data)
            .map_err(|_| CryptoError)
    }
}

struct HeaderKeyAdapter(Box<dyn HeaderProtectionKey>);

impl HeaderKey for HeaderKeyAdapter {
    fn decrypt(&self, pn_offset: usize, packet: &mut [u8]) {
        let (header, sample) = packet.split_at_mut(pn_offset + 4);
        let (first, rest) = header.split_at_mut(1);
        let pn_end = (pn_offset + 3).min(rest.len());
        self.0
            .decrypt_in_place(
                &sample[..self.0.sample_len()],
                &mut first[0],
                &mut rest[pn_offset - 1..pn_end],
            )
            .expect("valid QUIC header protection");
    }

    fn encrypt(&self, pn_offset: usize, packet: &mut [u8]) {
        let (header, sample) = packet.split_at_mut(pn_offset + 4);
        let (first, rest) = header.split_at_mut(1);
        let pn_end = (pn_offset + 3).min(rest.len());
        self.0
            .encrypt_in_place(
                &sample[..self.0.sample_len()],
                &mut first[0],
                &mut rest[pn_offset - 1..pn_end],
            )
            .expect("valid QUIC header protection");
    }

    fn sample_size(&self) -> usize {
        self.0.sample_len()
    }
}

struct PacketKeyAdapter(Box<dyn RustlsPacketKey>);

impl PacketKey for PacketKeyAdapter {
    fn encrypt(&self, packet: u64, buf: &mut [u8], header_len: usize) {
        let (header, payload_tag) = buf.split_at_mut(header_len);
        let tag_len = self.0.tag_len();
        let (payload, tag_storage) = payload_tag.split_at_mut(payload_tag.len() - tag_len);
        let tag = self
            .0
            .encrypt_in_place(packet, header, payload)
            .expect("valid QUIC packet encryption");
        tag_storage.copy_from_slice(tag.as_ref());
    }

    fn decrypt(
        &self,
        packet: u64,
        header: &[u8],
        payload: &mut BytesMut,
    ) -> Result<(), CryptoError> {
        let plain = self
            .0
            .decrypt_in_place(packet, header, payload.as_mut())
            .map_err(|_| CryptoError)?;
        let len = plain.len();
        payload.truncate(len);
        Ok(())
    }

    fn tag_len(&self) -> usize {
        self.0.tag_len()
    }

    fn confidentiality_limit(&self) -> u64 {
        self.0.confidentiality_limit()
    }

    fn integrity_limit(&self) -> u64 {
        self.0.integrity_limit()
    }
}

/// Negotiated TLS 1.3 handshake metadata for a QUIC connection.
pub struct QuicHandshakeData {
    pub protocol: Option<Vec<u8>>,
    pub server_name: Option<String>,
}

struct TlsSession {
    version: Version,
    got_handshake_data: bool,
    next_secrets: Option<Secrets>,
    inner: RustlsQuicConnection,
    suite: Suite,
}

impl TlsSession {
    const fn side(&self) -> Side {
        match self.inner {
            RustlsQuicConnection::Client(_) => Side::Client,
            RustlsQuicConnection::Server(_) => Side::Server,
        }
    }
}

const RETRY_INTEGRITY_KEY_DRAFT: [u8; 16] = [
    0xcc, 0xce, 0x18, 0x7e, 0xd0, 0x9a, 0x09, 0xd0, 0x57, 0x28, 0x15, 0x5a, 0x6c, 0xb9, 0x6b, 0xe1,
];
const RETRY_INTEGRITY_NONCE_DRAFT: [u8; 12] = [
    0xe5, 0x49, 0x30, 0xf9, 0x7f, 0x21, 0x36, 0xf0, 0x53, 0x0a, 0x8c, 0x1c,
];
const RETRY_INTEGRITY_KEY_V1: [u8; 16] = [
    0xbe, 0x0c, 0x69, 0x0b, 0x9f, 0x66, 0x57, 0x5a, 0x1d, 0x76, 0x6b, 0x54, 0xe3, 0x68, 0xc8, 0x4e,
];
const RETRY_INTEGRITY_NONCE_V1: [u8; 12] = [
    0x46, 0x15, 0x99, 0xd3, 0x5d, 0x63, 0x2b, 0xf2, 0x23, 0x98, 0x25, 0xbb,
];

impl crypto::Session for TlsSession {
    fn initial_keys(&self, dst_cid: &ConnectionId, side: Side) -> Keys {
        derive_initial_keys(self.version, *dst_cid, side, &self.suite)
    }

    fn handshake_data(&self) -> Option<Box<dyn Any>> {
        if !self.got_handshake_data {
            return None;
        }
        Some(Box::new(QuicHandshakeData {
            protocol: self.inner.alpn_protocol().map(<[u8]>::to_vec),
            server_name: match &self.inner {
                RustlsQuicConnection::Client(_) => None,
                RustlsQuicConnection::Server(session) => session.server_name().map(str::to_owned),
            },
        }))
    }

    fn peer_identity(&self) -> Option<Box<dyn Any>> {
        let _ = (&self.inner, std::any::TypeId::of::<QuicHandshakeData>());
        self.inner.peer_certificates().map(|certs| -> Box<dyn Any> {
            Box::new(
                certs
                    .iter()
                    .map(|cert| cert.clone().into_owned())
                    .collect::<Vec<CertificateDer<'static>>>(),
            )
        })
    }

    fn early_crypto(&self) -> Option<(Box<dyn HeaderKey>, Box<dyn PacketKey>)> {
        let keys = self.inner.zero_rtt_keys()?;
        Some((
            Box::new(HeaderKeyAdapter(keys.header)),
            Box::new(PacketKeyAdapter(keys.packet)),
        ))
    }

    fn early_data_accepted(&self) -> Option<bool> {
        match &self.inner {
            RustlsQuicConnection::Client(session) => Some(session.is_early_data_accepted()),
            RustlsQuicConnection::Server(_) => None,
        }
    }

    fn is_handshaking(&self) -> bool {
        self.inner.is_handshaking()
    }

    fn read_handshake(&mut self, buf: &[u8]) -> Result<bool, TransportError> {
        self.inner.read_hs(buf).map_err(|error| {
            self.inner.alert().map_or_else(
                || TransportError {
                    code: TransportErrorCode::PROTOCOL_VIOLATION,
                    frame: None,
                    reason: format!("TLS error: {error}"),
                },
                |alert| TransportError {
                    code: TransportErrorCode::crypto(alert.into()),
                    frame: None,
                    reason: error.to_string(),
                },
            )
        })?;
        if !self.got_handshake_data {
            let have_server_name = match &self.inner {
                RustlsQuicConnection::Client(_) => false,
                RustlsQuicConnection::Server(session) => session.server_name().is_some(),
            };
            if self.inner.alpn_protocol().is_some() || have_server_name || !self.is_handshaking() {
                self.got_handshake_data = true;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn transport_parameters(&self) -> Result<Option<TransportParameters>, TransportError> {
        self.inner
            .quic_transport_parameters()
            .map_or(Ok(None), |buf| {
                TransportParameters::read(self.side(), &mut io::Cursor::new(buf))
                    .map(Some)
                    .map_err(Into::into)
            })
    }

    fn write_handshake(&mut self, buf: &mut Vec<u8>) -> Option<Keys> {
        let keys = match self.inner.write_hs(buf)? {
            KeyChange::Handshake { keys } => keys,
            KeyChange::OneRtt { keys, next } => {
                self.next_secrets = Some(next);
                keys
            }
        };
        Some(Keys {
            header: KeyPair {
                local: Box::new(HeaderKeyAdapter(keys.local.header)),
                remote: Box::new(HeaderKeyAdapter(keys.remote.header)),
            },
            packet: KeyPair {
                local: Box::new(PacketKeyAdapter(keys.local.packet)),
                remote: Box::new(PacketKeyAdapter(keys.remote.packet)),
            },
        })
    }

    fn next_1rtt_keys(&mut self) -> Option<KeyPair<Box<dyn PacketKey>>> {
        let keys = self.next_secrets.as_mut()?.next_packet_keys();
        Some(KeyPair {
            local: Box::new(PacketKeyAdapter(keys.local)),
            remote: Box::new(PacketKeyAdapter(keys.remote)),
        })
    }

    fn is_valid_retry(&self, orig_dst_cid: &ConnectionId, header: &[u8], payload: &[u8]) -> bool {
        let Some(tag_start) = payload.len().checked_sub(16) else {
            return false;
        };
        let Ok(cid_len) = u8::try_from(orig_dst_cid.len()) else {
            return false;
        };
        let mut pseudo_packet =
            Vec::with_capacity(header.len() + payload.len() + orig_dst_cid.len() + 1);
        pseudo_packet.push(cid_len);
        pseudo_packet.extend_from_slice(orig_dst_cid);
        pseudo_packet.extend_from_slice(header);
        let tag_start = tag_start + pseudo_packet.len();
        pseudo_packet.extend_from_slice(payload);

        let (nonce, key) = match self.version {
            Version::V1 => (RETRY_INTEGRITY_NONCE_V1, RETRY_INTEGRITY_KEY_V1),
            Version::V1Draft => (RETRY_INTEGRITY_NONCE_DRAFT, RETRY_INTEGRITY_KEY_DRAFT),
            _ => return false,
        };
        let nonce = aead::Nonce::assume_unique_for_key(nonce);
        let key = aead::LessSafeKey::new(
            aead::UnboundKey::new(&aead::AES_128_GCM, &key).expect("valid retry key"),
        );
        let (aad, tag) = pseudo_packet.split_at_mut(tag_start);
        key.open_in_place(nonce, aead::Aad::from(aad), tag).is_ok()
    }

    fn export_keying_material(
        &self,
        output: &mut [u8],
        label: &[u8],
        context: &[u8],
    ) -> Result<(), ExportKeyingMaterialError> {
        self.inner
            .export_keying_material(output, label, Some(context))
            .map_err(|_| ExportKeyingMaterialError)?;
        Ok(())
    }
}

struct QuicClientCrypto {
    inner: Arc<rustls::ClientConfig>,
    initial: Suite,
}

impl QuicClientCrypto {
    fn new(inner: Arc<rustls::ClientConfig>) -> Result<Self, &'static str> {
        let initial =
            initial_suite(inner.crypto_provider()).ok_or("no TLS13_AES_128_GCM_SHA256 suite")?;
        Ok(Self { inner, initial })
    }
}

impl crypto::ClientConfig for QuicClientCrypto {
    fn start_session(
        self: Arc<Self>,
        version: u32,
        server_name: &str,
        params: &TransportParameters,
    ) -> Result<Box<dyn crypto::Session>, ConnectError> {
        let version = interpret_version(version)?;
        let sni = ServerName::try_from(server_name)
            .map_err(|_| ConnectError::InvalidServerName(server_name.to_owned()))?
            .to_owned();
        let connection =
            rustls::quic::ClientConnection::new(self.inner.clone(), version, sni, to_vec(params))
                .map_err(|_| ConnectError::InvalidServerName(server_name.to_owned()))?;
        Ok(Box::new(TlsSession {
            version,
            got_handshake_data: false,
            next_secrets: None,
            inner: RustlsQuicConnection::Client(connection),
            suite: self.initial,
        }))
    }
}

struct QuicServerCrypto {
    inner: Arc<rustls::ServerConfig>,
    initial: Suite,
}

impl QuicServerCrypto {
    fn new(inner: Arc<rustls::ServerConfig>) -> Result<Self, &'static str> {
        let initial =
            initial_suite(inner.crypto_provider()).ok_or("no TLS13_AES_128_GCM_SHA256 suite")?;
        Ok(Self { inner, initial })
    }
}

impl crypto::ServerConfig for QuicServerCrypto {
    fn initial_keys(
        &self,
        version: u32,
        dst_cid: &ConnectionId,
    ) -> Result<Keys, UnsupportedVersion> {
        let version = interpret_version(version)?;
        Ok(derive_initial_keys(
            version,
            *dst_cid,
            Side::Server,
            &self.initial,
        ))
    }

    fn retry_tag(&self, version: u32, orig_dst_cid: &ConnectionId, packet: &[u8]) -> [u8; 16] {
        let version = interpret_version(version).expect("validated QUIC version");
        let (nonce, key) = match version {
            Version::V1 => (RETRY_INTEGRITY_NONCE_V1, RETRY_INTEGRITY_KEY_V1),
            Version::V1Draft => (RETRY_INTEGRITY_NONCE_DRAFT, RETRY_INTEGRITY_KEY_DRAFT),
            _ => unreachable!(),
        };
        let mut pseudo_packet = Vec::with_capacity(packet.len() + orig_dst_cid.len() + 1);
        pseudo_packet.push(u8::try_from(orig_dst_cid.len()).expect("CID <= 20 bytes"));
        pseudo_packet.extend_from_slice(orig_dst_cid);
        pseudo_packet.extend_from_slice(packet);

        let nonce = aead::Nonce::assume_unique_for_key(nonce);
        let key = aead::LessSafeKey::new(
            aead::UnboundKey::new(&aead::AES_128_GCM, &key).expect("valid retry key"),
        );
        let tag = key
            .seal_in_place_separate_tag(nonce, aead::Aad::from(pseudo_packet), &mut [])
            .expect("valid retry seal");
        let mut result = [0_u8; 16];
        result.copy_from_slice(tag.as_ref());
        result
    }

    fn start_session(
        self: Arc<Self>,
        version: u32,
        params: &TransportParameters,
    ) -> Box<dyn crypto::Session> {
        let version = interpret_version(version).expect("validated QUIC version");
        let connection =
            rustls::quic::ServerConnection::new(self.inner.clone(), version, to_vec(params))
                .expect("valid server TLS config");
        Box::new(TlsSession {
            version,
            got_handshake_data: false,
            next_secrets: None,
            inner: RustlsQuicConnection::Server(connection),
            suite: self.initial,
        })
    }
}

fn initial_suite(provider: &Arc<rustls::crypto::CryptoProvider>) -> Option<Suite> {
    provider
        .cipher_suites
        .iter()
        .find_map(|cs| match (cs.suite(), cs.tls13()) {
            (CipherSuite::TLS13_AES_128_GCM_SHA256, Some(suite)) => suite.quic_suite(),
            _ => None,
        })
}

fn derive_initial_keys(version: Version, dst_cid: ConnectionId, side: Side, suite: &Suite) -> Keys {
    let rustls_side = match side {
        Side::Client => rustls::Side::Client,
        Side::Server => rustls::Side::Server,
    };
    let keys = suite.keys(&dst_cid, rustls_side, version);
    Keys {
        header: KeyPair {
            local: Box::new(HeaderKeyAdapter(keys.local.header)),
            remote: Box::new(HeaderKeyAdapter(keys.remote.header)),
        },
        packet: KeyPair {
            local: Box::new(PacketKeyAdapter(keys.local.packet)),
            remote: Box::new(PacketKeyAdapter(keys.remote.packet)),
        },
    }
}

fn to_vec(params: &TransportParameters) -> Vec<u8> {
    let mut bytes = Vec::new();
    params.write(&mut bytes);
    bytes
}

const fn interpret_version(version: u32) -> Result<Version, UnsupportedVersion> {
    match version {
        0xff00_001d..=0xff00_0020 => Ok(Version::V1Draft),
        0x0000_0001 | 0xff00_0021..=0xff00_0022 => Ok(Version::V1),
        _ => Err(UnsupportedVersion),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quic_urls_with_optional_ports_ipv6_and_pins() {
        let default = parse_server_url("quic://127.0.0.1:9001").unwrap();
        assert_eq!(default.host, "127.0.0.1");
        assert_eq!(default.port, 9001);
        assert_eq!(default.pinned_sha256, None);
        assert_eq!(default.sni_name(), "rts.local");

        let bare = parse_server_url("play.example.com").unwrap();
        assert_eq!(bare.canonical, "quic://play.example.com:9001");
        assert_eq!(bare.sni_name(), "play.example.com");

        let ipv6 = parse_server_url("quic://[::1]:4433").unwrap();
        assert_eq!(ipv6.socket_host_port(), "[::1]:4433");
        assert_eq!(ipv6.sni_name(), "rts.local");

        let pin = "ab".repeat(32);
        let pinned = parse_server_url(&format!("quic://10.0.0.2:9001#{pin}")).unwrap();
        assert_eq!(pinned.pinned_sha256, Some([0xab; 32]));

        for bad in [
            "",
            "ws://127.0.0.1:9001",
            "http://example.com",
            "quic://",
            "quic://127.0.0.1:0",
            "quic://127.0.0.1:9001#deadbeef",
        ] {
            assert!(parse_server_url(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[tokio::test]
    async fn pure_rust_quic_handshake_and_framing_roundtrip() {
        let cert = ServerCertificate::generate_self_signed().unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        let server = server_endpoint(socket, &cert, Duration::from_secs(5)).unwrap();

        let expected_pin = decode_sha256_hex(&cert.fingerprint).unwrap();
        let client = client_endpoint(false, Some(expected_pin), Duration::from_secs(5)).unwrap();

        let server_task = tokio::spawn(async move {
            let incoming = server.accept().await.unwrap();
            let conn = incoming.await.unwrap();
            let (mut send, mut recv) = conn.accept_bi().await.unwrap();
            let msg = read_frame(&mut recv, 1024).await.unwrap().unwrap();
            write_frame(&mut send, &msg).await.unwrap();
            send.finish().unwrap();
            conn.closed().await;
        });

        let conn = client.connect(addr, "rts.local").unwrap().await.unwrap();
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        write_frame(&mut send, b"hello quic").await.unwrap();
        let echoed = read_frame(&mut recv, 1024).await.unwrap().unwrap();
        assert_eq!(echoed, b"hello quic");
        conn.close(0_u32.into(), b"done");
        server_task.await.unwrap();
    }
}
