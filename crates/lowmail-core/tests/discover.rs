//! Discovery against real domains; needs the internet, so it runs only with
//! `cargo test -p lowmail-core --test discover -- --ignored`.

use lowmail_core::discover;
use lowmail_core::model::Security;

/// batatinhas.net.br is hosted on MXRoute: `imap.` answers with a
/// certificate for another name and must be skipped; `mail.` is MXRoute with
/// a valid certificate, and so is the MX host.
#[tokio::test]
#[ignore]
async fn skips_hosts_with_a_certificate_for_another_name() {
    let mx = discover::mx_hosts("batatinhas.net.br").await;
    assert_eq!(mx.first().map(String::as_str), Some("fusion.mxrouting.net"), "{mx:?}");

    let account = discover::discover("teste@batatinhas.net.br").await.unwrap();
    let good = ["mail.batatinhas.net.br", "fusion.mxrouting.net"];
    assert!(good.contains(&account.imap.host.as_str()), "{:?}", account.imap);
    assert_eq!((account.imap.port, account.imap.security), (993, Security::Tls));
    assert!(good.contains(&account.smtp.host.as_str()), "{:?}", account.smtp);
    assert!(account.sieve.is_some());
}
