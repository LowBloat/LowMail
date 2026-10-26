//! Discovery against a real domain; needs the internet and a domain to try,
//! so it runs only when asked:
//!
//! ```sh
//! LOWMAIL_DISCOVER_EMAIL=user@example.com \
//! LOWMAIL_DISCOVER_HOSTS=mail.example.com,mx.provider.net \
//! cargo test -p lowmail-core --test discover -- --ignored
//! ```
//!
//! `LOWMAIL_DISCOVER_HOSTS` lists the acceptable IMAP/SMTP hosts. Pick a
//! domain whose `imap.` host answers with a certificate for another name: the
//! test checks that discovery skips it.

use lowmail_core::discover;
use lowmail_core::model::Security;

#[tokio::test]
#[ignore]
async fn skips_hosts_with_a_certificate_for_another_name() {
    let (Ok(email), Ok(hosts)) = (std::env::var("LOWMAIL_DISCOVER_EMAIL"), std::env::var("LOWMAIL_DISCOVER_HOSTS")) else {
        panic!("set LOWMAIL_DISCOVER_EMAIL and LOWMAIL_DISCOVER_HOSTS");
    };
    let good: Vec<&str> = hosts.split(',').map(str::trim).collect();
    let domain = email.rsplit_once('@').map(|(_, d)| d).unwrap();
    let mx = discover::mx_hosts(domain).await;
    assert!(!mx.is_empty(), "no MX hosts for {domain}");

    let account = discover::discover(&email).await.unwrap();
    assert!(good.contains(&account.imap.host.as_str()), "{:?}", account.imap);
    assert_eq!((account.imap.port, account.imap.security), (993, Security::Tls));
    assert!(good.contains(&account.smtp.host.as_str()), "{:?}", account.smtp);
    assert!(account.sieve.is_some());
}
