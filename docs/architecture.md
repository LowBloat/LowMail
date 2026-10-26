# Architecture

## One engine, two shells

```
            ┌──────────────── Svelte 5 interface (src/) ────────────────┐
            │  api.ts: call(method, params), upload, downloads, events  │
            └───────────────┬───────────────────────────┬───────────────┘
                 POST /api/rpc/<method>          invoke("rpc", …)
                 SSE /api/events                 event "mail-event"
            ┌───────────────┴──────────┐   ┌────────────┴───────────────┐
            │ lowmail-server (axum)    │   │ src-tauri (Tauri 2)        │
            │ sessions, CSRF, uploads, │   │ accounts, keyring, native  │
            │ downloads, embedded UI   │   │ dialogs, notifications     │
            └───────────────┬──────────┘   └────────────┬───────────────┘
                            └──────────┬────────────────┘
                     lowmail_core::rpc::dispatch(client, method, params)
                                       │
                         lowmail_core::service::MailClient
                ┌───────────┬──────────┼───────────┬──────────────┐
             imap/        mime.rs   compose.rs   smtp.rs      sieve.rs     store.rs
          pool, SORT,    reading,   replies,     lettre      ManageSieve   SQLite
          IDLE watch     sanitize   MIME build               rules ⇄ Sieve
```

`MailClient` is one signed-in account: the web server keeps one per browser session (the password lives only inside it), the desktop one per account (the password comes from the OS credential store).

## IMAP

- `imap::Pool` keeps up to `IMAP_CONNECTIONS` signed-in connections. Every operation runs through the `with_conn!` macro: a time limit, and one retry on a fresh connection after a network error for read-only operations.
- A connection remembers the selected folder; reselecting the same folder is a `NOOP`, so new and removed messages are seen.
- The list is `UID SORT` (or `UID SEARCH` without SORT), returning every UID in order; the UI loads summaries for the rows it shows (`messages.summaries`), so a folder of 100 000 messages scrolls without loading them all.
- Summaries come from `BODY.PEEK[HEADER.FIELDS (...)]` parsed with mail-parser (encoded words and charsets) plus `BODYSTRUCTURE` for the attachment icon.
- Messages are fetched whole with `BODY.PEEK[]` and cached in memory (64 MB per account); attachments are cut from the cached message.
- `imap::watch` runs IDLE on a connection of its own (polling every minute when the server has no IDLE), reports new mail with the newest unread summaries, and refreshes other folders' counters every four minutes.
- Folder names are modified UTF-7 on the wire and decoded for display; roles come from SPECIAL-USE or well-known names; a personal `INBOX.` namespace is shown as top level.

## Reading safely

`mime::parse_message` builds the HTML view (HTML parts as they are, text parts converted with links and quote levels, inline images as `data:` URLs) and `sanitize::sanitize` cleans it with ammonia: no scripts, event handlers, forms, `javascript:` or `data:` links; remote images, CSS `url()` and `@import` removed unless the reader allows remote content. The UI then renders it in an `<iframe sandbox="allow-same-origin allow-popups">` (no scripts) whose document carries its own Content-Security-Policy; link clicks are intercepted and opened outside the app.

## Composing

`compose::prepare` builds replies and forwards (recipients, `Re:`/`Fwd:` without stacking `RES:`/`ENC:`, `In-Reply-To`/`References`, localized quote header), `compose::build` the MIME message with mail-builder: `multipart/alternative` text + HTML, `multipart/related` for pasted images (moved from `data:` URLs to `cid:` parts), `multipart/mixed` for files. The SMTP copy has no `Bcc` header; the copies in Sent and Drafts keep it. Drafts carry an `X-LowMail-Draft` header (identity, original message, format) so they reopen as they were.

## Filters

The rules are kept as JSON inside the Sieve script LowMail writes (`# lowmail:<base64>`), so they can be edited again; the script itself is plain Sieve (`fileinto`, `imap4flags`, `copy`, `vacation` with `date`/`relational`, `body`, `reject`). Scripts written by other tools are shown, not overwritten.

## Tests

`lowmail-testserver` is a real IMAP4rev1/SMTP/ManageSieve server in memory (SORT, IDLE, UIDPLUS, MOVE, SPECIAL-USE, QUOTA, LITERAL+), seeded with varied messages. The engine's end-to-end tests (`crates/lowmail-core/tests/e2e.rs`) and the web server's HTTP tests (`crates/lowmail-server/tests/http.rs`) run against it, and `lowmail demo` uses it for local development.
