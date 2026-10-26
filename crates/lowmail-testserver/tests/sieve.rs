//! Raw-socket ManageSieve tests.

mod common;

use common::*;

const SCRIPT: &str = "require [\"fileinto\"];\r\nif header :contains \"subject\" \"[rust-br]\" {\r\n  fileinto \"Listas\";\r\n}\r\n";

#[tokio::test]
async fn manage_scripts() {
    let server = server().await;
    let mut c = Raw::connect(server.sieve_addr()).await;
    let greeting = c.until("OK").await;
    assert!(greeting.contains(&"\"IMPLEMENTATION\" \"LowMail Test\"".to_owned()));
    assert!(greeting.contains(&"\"SASL\" \"PLAIN\"".to_owned()));
    assert!(greeting.contains(&"\"VERSION\" \"1.0\"".to_owned()));
    assert!(
        greeting
            .iter()
            .any(|l| l.starts_with("\"SIEVE\" \"fileinto reject envelope vacation"))
    );

    c.send("LISTSCRIPTS\r\n").await;
    assert!(c.line().await.starts_with("NO"));

    c.send(&format!(
        "AUTHENTICATE \"PLAIN\" \"{}\"\r\n",
        plain_auth(ALICE, "alice")
    ))
    .await;
    assert!(c.line().await.starts_with("OK"));

    c.send(&format!(
        "PUTSCRIPT \"lists\" {{{}+}}\r\n{SCRIPT}\r\n",
        SCRIPT.len()
    ))
    .await;
    assert!(c.line().await.starts_with("OK"));
    c.send("PUTSCRIPT \"other\" \"keep;\"\r\n").await;
    assert!(c.line().await.starts_with("OK"));
    c.send("PUTSCRIPT \"empty\" \"\"\r\n").await;
    assert!(c.line().await.starts_with("NO"));
    c.send("CHECKSCRIPT \"keep;\"\r\n").await;
    assert!(c.line().await.starts_with("OK"));

    c.send("SETACTIVE \"lists\"\r\n").await;
    assert!(c.line().await.starts_with("OK"));
    c.send("LISTSCRIPTS\r\n").await;
    let lines = c.until("OK").await;
    assert!(lines.contains(&"\"lists\" ACTIVE".to_owned()), "{lines:?}");
    assert!(lines.contains(&"\"other\"".to_owned()));

    c.send("GETSCRIPT \"lists\"\r\n").await;
    assert_eq!(c.line().await, format!("{{{}}}", SCRIPT.len()));
    assert_eq!(c.bytes(SCRIPT.len()).await, SCRIPT.as_bytes());
    assert_eq!(c.line().await, "");
    assert!(c.line().await.starts_with("OK"));

    c.send("DELETESCRIPT \"lists\"\r\n").await;
    assert!(c.line().await.starts_with("NO (ACTIVE)"));
    c.send("GETSCRIPT \"missing\"\r\n").await;
    assert!(c.line().await.starts_with("NO (NONEXISTENT)"));
    c.send("RENAMESCRIPT \"other\" \"renamed\"\r\n").await;
    assert!(c.line().await.starts_with("OK"));
    c.send("HAVESPACE \"x\" 1000\r\n").await;
    assert!(c.line().await.starts_with("OK"));
    c.send("NOOP \"t1\"\r\n").await;
    assert_eq!(c.line().await, "OK (TAG \"t1\") \"Done\"");
    c.send("CAPABILITY\r\n").await;
    assert!(c.until("OK").await.len() > 4);

    assert_eq!(
        server.sieve_scripts(ALICE),
        vec![
            ("lists".to_owned(), SCRIPT.to_owned(), true),
            ("renamed".to_owned(), "keep;".to_owned(), false),
        ]
    );

    c.send("SETACTIVE \"\"\r\n").await;
    assert!(c.line().await.starts_with("OK"));
    c.send("DELETESCRIPT \"lists\"\r\n").await;
    assert!(c.line().await.starts_with("OK"));
    c.send("LOGOUT\r\n").await;
    assert!(c.line().await.starts_with("OK"));

    // AUTHENTICATE with a continuation and a literal response.
    let mut c = Raw::connect(server.sieve_addr()).await;
    c.until("OK").await;
    c.send("AUTHENTICATE \"PLAIN\"\r\n").await;
    assert_eq!(c.line().await, "\"\"");
    let auth = plain_auth(BOB, "bob");
    c.send(&format!("{{{}+}}\r\n{auth}\r\n", auth.len())).await;
    assert!(c.line().await.starts_with("OK"));
    c.send("LISTSCRIPTS\r\n").await;
    assert_eq!(c.until("OK").await.len(), 1);
}
