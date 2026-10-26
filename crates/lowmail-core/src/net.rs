//! TCP and TLS connections shared by IMAP and ManageSieve.

use std::io;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::error::{Error, ErrorKind, Result};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// A connection that may or may not be encrypted.
pub enum MailStream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl std::fmt::Debug for MailStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MailStream::Plain(_) => "MailStream::Plain",
            MailStream::Tls(_) => "MailStream::Tls",
        })
    }
}

impl AsyncRead for MailStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MailStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            MailStream::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MailStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            MailStream::Plain(s) => Pin::new(s).poll_write(cx, data),
            MailStream::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, data),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MailStream::Plain(s) => Pin::new(s).poll_flush(cx),
            MailStream::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MailStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            MailStream::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

pub async fn connect_tcp(host: &str, port: u16) -> Result<TcpStream> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| Error::new(ErrorKind::Network, format!("{host}:{port} did not answer in time.")))?
        .map_err(|e| {
            Error::new(ErrorKind::Network, format!("Could not connect to {host}:{port}.")).with_detail(e.to_string())
        })?;
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

/// Wraps a connected socket in TLS.
pub async fn upgrade(stream: TcpStream, host: &str, accept_invalid_certs: bool) -> Result<MailStream> {
    let name = ServerName::try_from(host.to_string())
        .map_err(|_| Error::invalid(format!("Invalid server name: {host}")))?;
    let connector = TlsConnector::from(client_config(accept_invalid_certs));
    let tls = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(name, stream))
        .await
        .map_err(|_| Error::new(ErrorKind::Tls, "The secure connection timed out."))?
        .map_err(tls_error)?;
    Ok(MailStream::Tls(Box::new(tls)))
}

fn tls_error(e: io::Error) -> Error {
    let detail = e.to_string();
    let message = if detail.contains("certificate") || detail.contains("UnknownIssuer") || detail.contains("NotValidForName") {
        "The server certificate is not trusted. If this is your own server with a self-signed certificate, allow invalid certificates in the account settings."
    } else {
        "The secure connection to the server failed."
    };
    Error::new(ErrorKind::Tls, message).with_detail(detail)
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

pub fn client_config(accept_invalid_certs: bool) -> Arc<ClientConfig> {
    static STRICT: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    static LENIENT: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    if accept_invalid_certs {
        LENIENT
            .get_or_init(|| {
                let provider = provider();
                let config = ClientConfig::builder_with_provider(provider.clone())
                    .with_safe_default_protocol_versions()
                    .expect("TLS versions")
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(AcceptAnyCert(provider)))
                    .with_no_client_auth();
                Arc::new(config)
            })
            .clone()
    } else {
        STRICT
            .get_or_init(|| {
                let config = ClientConfig::builder_with_provider(provider())
                    .with_safe_default_protocol_versions()
                    .expect("TLS versions")
                    .with_root_certificates(roots())
                    .with_no_client_auth();
                Arc::new(config)
            })
            .clone()
    }
}

/// The system's trust store plus Mozilla's, so a stale or minimal system
/// store never blocks a well-known certificate authority.
fn roots() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for cert in rustls_native_certs::load_native_certs().certs {
        let _ = roots.add(cert);
    }
    roots
}

/// Accepts any certificate but still checks the handshake signatures.
#[derive(Debug)]
struct AcceptAnyCert(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
