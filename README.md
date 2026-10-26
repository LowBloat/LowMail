<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" alt="LowMail logo" width="112">
</p>

<h1 align="center">LowMail</h1>

<p align="center">
  A light e-mail client for your own mail server.<br>
  On the desktop (Windows, Linux, macOS) and in the browser, as a Roundcube replacement.
</p>

<p align="center">
  <a href="#download">Download</a>
  · <a href="#run-the-webmail">Webmail</a>
  · <a href="#desktop-and-android-apps">Desktop and Android</a>
  · <a href="#features">Features</a>
  · <a href="#how-it-works">How it works</a>
  · <a href="#development">Development</a>
  · <a href="docs/roundcube.pt-BR.md">Substituir o Roundcube (pt-BR)</a>
</p>

---

LowMail talks to any standard mail server: **IMAP** to read, **SMTP** (submission) to send and **ManageSieve** for server-side filters and the vacation reply. There is no LowMail-specific server component and nothing to change on your mail server: point it at the same hosts and ports your users already use.

One Rust engine (`lowmail-core`) does all the mail work. The same Svelte interface runs in two shells:

| | Webmail (`lowmail`) | Desktop (`LowMail`) |
| --- | --- | --- |
| Shell | Rust web server (axum), one binary or one container | Tauri 2 app |
| Sign-in | E-mail and password of the mail server, per browser session | Several accounts, passwords in the OS credential store |
| Local data | Preferences, identities and contacts per user in SQLite | The same, on the computer |
| New mail | IMAP IDLE pushed to the browser (Server-Sent Events) | IMAP IDLE, native notifications |

## Download

Every [release](https://github.com/LowBloat/LowMail/releases/latest) has:

| System | Files |
| --- | --- |
| Windows | `windows-x64-setup.exe` (installer), `windows-x64.msi`, `windows-x64-portable.zip` |
| macOS (Apple Silicon and Intel) | `macos-universal.dmg` |
| Linux | `linux-x64.AppImage`, `linux-x64.deb`, `linux-x64.rpm`, `linux-x64-portable.tar.gz` |
| Arch Linux | `linux-x86_64.pkg.tar.zst` (`sudo pacman -U`), or the `PKGBUILD` |
| Android | `android-universal.apk` |
| Webmail server | `webmail-linux-x64.tar.gz`, `webmail-linux-arm64.tar.gz`, or `ghcr.io/lowbloat/lowmail` |

`SHA256SUMS` lists every file's checksum, and each file has a build provenance attestation (`gh attestation verify <file> --repo LowBloat/LowMail`). The macOS app is not notarized: open it the first time with right click → **Open**.

## Features

- **Mail**: folder tree with unread counters and quota, three layouts (reading pane right, below or full screen), a virtual list that scrolls through tens of thousands of messages, multiple selection, drag and drop to folders, flags, read/unread, move and copy, archive, spam/not spam (`$Junk`), delete to Trash, empty folder, keyboard shortcuts (`?` lists them).
- **Search**: sender, recipients, subject, body or everything; unread, flagged, answered; date range; in the folder or **in every folder**; server-side sorting (IMAP SORT) by date, arrival, sender, recipient, subject or size.
- **Reading**: HTML sanitized on the server and shown in a sandboxed frame with its own Content-Security-Policy; **remote images blocked** until allowed (once, or always for a sender); inline images; attachments to preview, open or save; read-receipt requests; one-click unsubscribe; all headers, raw source, `.eml` download, print.
- **Writing**: rich text or plain text, reply / reply all / forward inline or as attachment / edit as new, quotes in the reader's language, drafts saved on the server (autosave), attachments by picker or drag and drop, images pasted into the text, address suggestions from contacts and past recipients, Cc/Bcc, priority, read receipt, checks for a missing subject or a forgotten attachment.
- **Identities**: several From addresses with their own HTML signature, reply-to and automatic Bcc; replies use the identity the message was sent to.
- **Contacts**: address book with groups, collected addresses, vCard import and export, CSV import (Roundcube, Gmail, Outlook).
- **Server filters (ManageSieve)**: rules on sender, recipients, subject, body, any header or size, with move, copy, mark as read, flag, forward, delete or reject; the **vacation reply** with dates. Filters run on the server, with LowMail closed.
- **Folders**: create, rename, move, delete, show or hide (subscriptions); Sent, Drafts, Trash, Spam and Archive found through SPECIAL-USE or common names in English and Portuguese, `INBOX.` namespaces (Courier, cPanel) shown as top level.
- **Interface**: Portuguese (Brazil) and English, light and dark themes, compact or comfortable density, phone layout, LowBloat design built with [@codeatlasdev/ui](https://ui.codeatlas.com.br/).

## Run the webmail

With Docker Compose, edit the `LOWMAIL_*` variables in [`compose.yaml`](compose.yaml) and run:

```sh
docker compose up -d --build
```

Open `http://<server>:8480` and sign in with an e-mail address and its password. Put it behind your HTTPS reverse proxy (see [docs/roundcube.pt-BR.md](docs/roundcube.pt-BR.md) for Nginx and Caddy examples) and keep `LOWMAIL_SECURE_COOKIES=true`.

Without Docker, build the interface and the server, then run the binary:

```sh
bun install && bun run build
cargo build --release -p lowmail-server
LOWMAIL_IMAP_HOST=mail.example.com LOWMAIL_DOMAIN=example.com ./target/release/lowmail serve
```

### Check an account before going live

`lowmail check` signs in with one account and runs what the webmail does: folders and their roles, the inbox list, reading, search, sorting, quota, a temporary folder, a draft (both removed afterwards), the filters (read only) and the SMTP sign-in. `--send-to` also sends one test message. It asks for the password without echo (or reads `LOWMAIL_CHECK_PASSWORD`) and uses the same `LOWMAIL_*` settings as the server:

```sh
docker compose run --rm -it lowmail check user@example.com --send-to user@example.com
```

### Settings

| Variable | Default | Meaning |
| --- | --- | --- |
| `LOWMAIL_IMAP_HOST` | `localhost` | IMAP server. `{domain}` is replaced by the domain of the address signing in (`mail.{domain}`). |
| `LOWMAIL_IMAP_PORT` / `_SECURITY` | `993` / `tls` | `tls`, `starttls` or `none` (143 for STARTTLS). |
| `LOWMAIL_SMTP_HOST` | IMAP host | Submission server. |
| `LOWMAIL_SMTP_PORT` / `_SECURITY` | `587` / `starttls` | `465` with `tls`. |
| `LOWMAIL_SMTP_AUTH` | `true` | Sign in to the SMTP server; `false` for relays that trust the network. |
| `LOWMAIL_SIEVE` | `true` | Filters and vacation reply through ManageSieve. |
| `LOWMAIL_SIEVE_HOST` / `_PORT` / `_SECURITY` | IMAP host / `4190` / `starttls` | |
| `LOWMAIL_DOMAIN` | | Domain added to user names typed without `@`. |
| `LOWMAIL_ALLOWED_DOMAINS` | any | Comma-separated domains allowed to sign in. Set it whenever a host uses `{domain}`. |
| `LOWMAIL_LOGIN_LOCAL_PART` | `false` | Sign in to the servers with the part before `@` instead of the full address. |
| `LOWMAIL_ACCEPT_INVALID_CERTS` | `false` | Accept a self-signed certificate on the mail servers. |
| `LOWMAIL_LISTEN` | `0.0.0.0:8480` | Address and port of the web server. |
| `LOWMAIL_DATA_DIR` | `data` | Folder of `lowmail.db` (preferences, identities, contacts). |
| `LOWMAIL_SESSION_MINUTES` | `240` | Idle time before a session ends (an open tab keeps it alive). |
| `LOWMAIL_SECURE_COOKIES` | from `X-Forwarded-Proto` | `true` behind HTTPS. |
| `LOWMAIL_TRUST_PROXY` | `false` | Use `X-Forwarded-For` for the sign-in rate limit. |
| `LOWMAIL_IMAP_CONNECTIONS` | `3` | IMAP connections per signed-in user. |
| `LOWMAIL_PRODUCT_NAME` | `LowMail` | Name on the sign-in page and in the title. |
| `LOWMAIL_LOG` | `info` | Log level (`debug` for protocol problems). |

### Security

- Passwords are never written to disk on the server: they stay in the memory of the signed-in session and are gone when it ends or the server restarts.
- Session cookies are random, `HttpOnly` and `SameSite=Strict`; state-changing requests also need a custom header, so other sites cannot forge requests.
- Failed sign-ins are rate limited per address and per IP.
- Message HTML is cleaned with [ammonia](https://crates.io/crates/ammonia) (no scripts, forms, event handlers or remote styles) and then shown in a sandboxed frame without script permission and with a Content-Security-Policy that blocks remote content unless the reader allows it.
- Downloads are served as attachments with `nosniff` and a sandbox policy; previews are limited to images, PDF, text, audio and video.

## Desktop and Android apps

The apps are Tauri 2 builds of the same interface:

```sh
bun install
bun run tauri build                 # desktop
bun run tauri android build --apk   # Android (needs the Android SDK and NDK)
```

The first start asks for the e-mail address and password; LowMail tries the usual host names (`imap.`, `mail.`, `smtp.`, the domain and its MX hosts) and ports, keeps the first server whose certificate is valid, checks the sign-in on IMAP and SMTP, and lets you adjust everything under **Server settings**. Passwords go to the Windows Credential Manager, the macOS Keychain or the Secret Service on Linux; on Android, to the app's private storage.

Installed desktop copies (Windows installers, the macOS app, AppImage) update themselves from signed GitHub releases; packages and portable copies show when a new version exists.

## How it works

```
crates/lowmail-core        IMAP (async-imap: pool, SORT/SEARCH, IDLE), MIME (mail-parser),
                           sanitizing (ammonia), composing (mail-builder), SMTP (lettre),
                           ManageSieve, SQLite store, vCard, discovery and the RPC dispatcher
crates/lowmail-server      axum web server: sessions, CSRF, uploads, downloads, SSE, embedded UI
crates/lowmail-testserver  in-memory IMAP/SMTP/ManageSieve server for tests and demos
src-tauri                  desktop shell: Tauri commands, credential store, native dialogs
src                        Svelte 5 interface (both shells)
```

The interface calls one function, `lowmail_core::rpc::dispatch(method, params)`, as `POST /api/rpc/<method>` on the web and through the `rpc` command on the desktop, so both run exactly the same code. More in [docs/architecture.md](docs/architecture.md).

## Development

Requirements: Rust 1.95 or later, [Bun](https://bun.sh) 1.4, and on Linux the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
bun install
bun run server     # demo webmail with a built-in test mail server on http://127.0.0.1:8480
bun run dev        # interface with hot reload on http://127.0.0.1:1430 (proxies /api to :8480)
bun run tauri dev  # desktop app
```

Demo accounts: `alice@lowmail.test` / `alice` and `bob@lowmail.test` / `bob` (the demo server listens on IMAP 1143, SMTP 1025 and ManageSieve 4190 without TLS, so the desktop app can use it too).

Checks:

```sh
cargo test --workspace     # engine against the test mail server, web server over HTTP
cargo clippy --workspace --all-targets
bun run check              # svelte-check
bun run lint:ui            # design system guard (@codeatlasdev/ui tokens, no raw colors or sizes)
bun test src               # translations complete in both languages
```

## License

[AGPL-3.0-only](LICENSE). A LowBloat product.
