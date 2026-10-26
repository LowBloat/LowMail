//! `lowmail-testserver`: runs the in-memory mail server until Ctrl+C.

use std::net::IpAddr;

use anyhow::{Context as _, bail};
use lowmail_testserver::Config;

const USAGE: &str = "Usage: lowmail-testserver [--bind ADDR] [--imap-port N] [--smtp-port N] \
[--sieve-port N] [--no-seed] [--user EMAIL:PASSWORD]...

In-memory IMAP/SMTP/ManageSieve server for LowMail development (plain TCP, no TLS).
Ports set to 0 pick a free port. --user replaces the default accounts.";

fn parse_args() -> anyhow::Result<Config> {
    let mut config = Config::default();
    let mut users = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().with_context(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--bind" => config.bind = value("--bind")?.parse::<IpAddr>()?,
            "--imap-port" => config.imap_port = value("--imap-port")?.parse()?,
            "--smtp-port" => config.smtp_port = value("--smtp-port")?.parse()?,
            "--sieve-port" => config.sieve_port = value("--sieve-port")?.parse()?,
            "--no-seed" => config.seed_demo_data = false,
            "--user" => {
                let spec = value("--user")?;
                let (email, password) = spec.split_once(':').context("--user expects EMAIL:PASSWORD")?;
                users.push((email.to_owned(), password.to_owned()));
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown argument {other}\n\n{USAGE}"),
        }
    }
    if !users.is_empty() {
        config.users = users;
    }
    Ok(config)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "lowmail_testserver=info".into()))
        .init();

    let config = parse_args()?;
    let users = config.users.clone();
    let seeded = config.seed_demo_data;
    let server = lowmail_testserver::start(config).await?;

    println!("LowMail test server (plain TCP, no TLS)");
    println!("  IMAP        {}", server.imap_addr());
    println!("  SMTP        {}", server.smtp_addr());
    println!("  ManageSieve {}", server.sieve_addr());
    println!("Accounts (login = full email address):");
    for (email, password) in &users {
        println!("  {email} / {password}");
    }
    if seeded {
        println!("Demo data seeded for {}", users.first().map_or("-", |u| u.0.as_str()));
    }
    println!("Press Ctrl+C to stop.");

    tokio::signal::ctrl_c().await?;
    println!("Shutting down...");
    server.shutdown().await;
    Ok(())
}
