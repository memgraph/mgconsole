//! Bolt transport: a plain TCP stream, optionally wrapped in rustls TLS.
//!
//! `bolt-client` is generic over any async byte stream, so TLS is added by
//! swapping the transport rather than touching protocol code. [`MaybeTlsStream`]
//! is the single concrete stream type the Session stores whether or not TLS is
//! on, keeping the `Client` monomorphic.
//!
//! TLS uses rustls with the ring crypto provider (ADR 0007). The server
//! certificate is **not** verified — mirroring mgclient's `REQUIRE` sslmode that
//! today's mgconsole uses with `--use-ssl`: encrypt the channel without
//! authenticating the peer, so self-signed Memgraph certs connect unconfigured.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio_rustls::TlsConnector;

use crate::error::Error;
use crate::session::Endpoint;

/// A Bolt byte transport: plaintext TCP, or the same TCP wrapped in TLS. Both
/// inner streams are `Unpin`, so this type is `Unpin` and its async-IO impls
/// delegate without pin projection (no `unsafe`).
pub(crate) enum MaybeTlsStream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for MaybeTlsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTlsStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            MaybeTlsStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MaybeTlsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            MaybeTlsStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            MaybeTlsStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTlsStream::Plain(s) => Pin::new(s).poll_flush(cx),
            MaybeTlsStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTlsStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            MaybeTlsStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Open a TCP connection to the endpoint, wrapping it in TLS when `use_tls`.
/// A TLS handshake failure surfaces as [`Error::Connection`], never a panic.
pub(crate) async fn connect_stream(
    endpoint: &Endpoint,
    use_tls: bool,
) -> Result<MaybeTlsStream, Error> {
    let tcp = TcpStream::connect((endpoint.host(), endpoint.port()))
        .await
        .map_err(|e| Error::Connection(e.to_string()))?;
    if !use_tls {
        return Ok(MaybeTlsStream::Plain(tcp));
    }

    let host = endpoint.host();
    let connector = TlsConnector::from(Arc::new(tls_config()));
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|e| Error::Connection(format!("invalid TLS server name '{host}': {e}")))?;
    let tls = connector
        .connect(server_name, tcp)
        .await
        .map_err(|e| Error::Connection(format!("TLS handshake failed: {e}")))?;
    Ok(MaybeTlsStream::Tls(Box::new(tls)))
}

/// A rustls client config (ring provider) that encrypts but does not verify the
/// server certificate — see the module docs and ADR 0007.
fn tls_config() -> ClientConfig {
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoCertVerification))
        .with_no_client_auth()
}

/// Accepts any server certificate. See ADR 0007: parity with mgclient `REQUIRE`.
#[derive(Debug)]
struct NoCertVerification;

impl ServerCertVerifier for NoCertVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        tokio_rustls::rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
