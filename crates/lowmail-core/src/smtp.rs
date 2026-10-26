//! Sending through the account's SMTP (submission) server.

use std::time::Duration;

use lettre::address::Envelope;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::error::{Error, Result};
use crate::model::{AccountConfig, Security};

fn transport(account: &AccountConfig, password: &str) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let ep = &account.smtp;
    let params = || {
        TlsParameters::builder(ep.host.clone())
            .dangerous_accept_invalid_certs(ep.accept_invalid_certs)
            .dangerous_accept_invalid_hostnames(ep.accept_invalid_certs)
            .build_rustls()
            .map_err(Error::from)
    };
    let tls = match ep.security {
        Security::Tls => Tls::Wrapper(params()?),
        Security::StartTls => Tls::Required(params()?),
        Security::None => Tls::None,
    };
    let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(ep.host.clone())
        .port(ep.port)
        .tls(tls)
        .timeout(Some(Duration::from_secs(60)));
    if account.smtp_auth {
        builder = builder.credentials(Credentials::new(account.username.clone(), password.to_string()));
    }
    Ok(builder.build())
}

/// Sends an already built message to `recipients`.
pub async fn send(account: &AccountConfig, password: &str, from: &str, recipients: &[String], raw: &[u8]) -> Result<()> {
    if recipients.is_empty() {
        return Err(Error::invalid("Add at least one recipient."));
    }
    let from = from.parse()?;
    let to = recipients.iter().map(|r| r.parse()).collect::<std::result::Result<Vec<_>, _>>()?;
    let envelope = Envelope::new(Some(from), to).map_err(|e| Error::invalid("Invalid recipients.").with_detail(e.to_string()))?;
    let transport = transport(account, password)?;
    transport.send_raw(&envelope, raw).await?;
    Ok(())
}

/// Checks that the server accepts the connection and the credentials.
pub async fn test(account: &AccountConfig, password: &str) -> Result<()> {
    let transport = transport(account, password)?;
    match transport.test_connection().await {
        Ok(true) => Ok(()),
        Ok(false) => Err(Error::server("The outgoing server did not accept the connection.")),
        Err(e) => Err(e.into()),
    }
}
