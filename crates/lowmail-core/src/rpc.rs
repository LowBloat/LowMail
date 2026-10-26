//! The one API of the UI: `dispatch(method, params)`. The web server exposes
//! it as `POST /api/rpc/<method>` and the desktop app as the `rpc` command,
//! so both front ends run exactly the same code. Binary transfers (uploads,
//! downloads) are handled by each host.

use std::sync::Arc;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::compose::QuoteStyle;
use crate::error::{Error, Result};
use crate::model::*;
use crate::service::{FlagChange, MailClient};
use crate::sieve::Filters;
use crate::vcard;

fn params<T: DeserializeOwned>(value: Value) -> Result<T> {
    let value = if value.is_null() { json!({}) } else { value };
    serde_json::from_value(value).map_err(|e| Error::invalid("Invalid request.").with_detail(e.to_string()))
}

fn ok<T: serde::Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| Error::internal("Could not encode the answer.").with_detail(e.to_string()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FolderArg {
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessagesArg {
    folder: String,
    uids: Vec<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageArg {
    folder: String,
    uid: u32,
}

pub async fn dispatch(client: &Arc<MailClient>, method: &str, p: Value) -> Result<Value> {
    let owner = client.owner().to_string();
    let store = client.store().clone();
    match method {
        "account.info" => ok(client.info()),

        // ---- folders ----
        "folders.list" => {
            #[derive(Deserialize)]
            struct A {
                #[serde(default = "yes")]
                counts: bool,
            }
            let a: A = params(p)?;
            ok(client.folders(a.counts).await?)
        }
        "folders.counts" => {
            #[derive(Deserialize)]
            struct A {
                paths: Vec<String>,
            }
            let a: A = params(p)?;
            ok(client.counts(&a.paths).await?)
        }
        "folders.create" => {
            #[derive(Deserialize)]
            struct A {
                parent: Option<String>,
                name: String,
            }
            let a: A = params(p)?;
            ok(json!({ "path": client.create_folder(a.parent, &a.name).await? }))
        }
        "folders.rename" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                path: String,
                name: String,
                /// Present to move the folder; `null` moves it to the top level.
                #[serde(default, deserialize_with = "double_option")]
                parent: Option<Option<String>>,
            }
            let a: A = params(p)?;
            ok(json!({ "path": client.rename_folder(&a.path, &a.name, a.parent).await? }))
        }
        "folders.delete" => {
            let a: FolderArg = params(p)?;
            client.delete_folder(&a.path).await?;
            ok(())
        }
        "folders.subscribe" => {
            #[derive(Deserialize)]
            struct A {
                path: String,
                subscribed: bool,
            }
            let a: A = params(p)?;
            client.set_subscribed(&a.path, a.subscribed).await?;
            ok(())
        }
        "folders.empty" => {
            let a: FolderArg = params(p)?;
            client.empty_folder(&a.path).await?;
            ok(())
        }
        "folders.markRead" => {
            let a: FolderArg = params(p)?;
            client.mark_folder_read(&a.path).await?;
            ok(())
        }

        // ---- messages ----
        "messages.list" => {
            #[derive(Deserialize)]
            struct A {
                folder: String,
                #[serde(default)]
                offset: u32,
                #[serde(default = "page_size")]
                limit: u32,
                #[serde(default)]
                sort: Sort,
                #[serde(default)]
                filter: Filter,
            }
            let a: A = params(p)?;
            ok(client.list(&a.folder, a.offset, a.limit, &a.sort, &a.filter).await?)
        }
        "messages.summaries" => {
            let a: MessagesArg = params(p)?;
            ok(client.summaries(&a.folder, &a.uids).await?)
        }
        "messages.searchAll" => {
            #[derive(Deserialize)]
            struct A {
                filter: Filter,
            }
            let a: A = params(p)?;
            ok(client.search_all(&a.filter).await?)
        }
        "messages.get" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                folder: String,
                uid: u32,
                #[serde(default)]
                allow_remote: bool,
                #[serde(default)]
                mark_seen: bool,
            }
            let a: A = params(p)?;
            ok(client.message(&a.folder, a.uid, a.allow_remote, a.mark_seen).await?)
        }
        "messages.source" => {
            let a: MessageArg = params(p)?;
            let raw = client.raw(&a.folder, a.uid).await?;
            ok(json!({ "source": String::from_utf8_lossy(&raw) }))
        }
        "messages.flags" => {
            #[derive(Deserialize)]
            struct A {
                folder: String,
                uids: Vec<u32>,
                change: FlagChange,
            }
            let a: A = params(p)?;
            client.set_flags(&a.folder, &a.uids, &a.change).await?;
            ok(())
        }
        "messages.move" | "messages.copy" => {
            #[derive(Deserialize)]
            struct A {
                folder: String,
                uids: Vec<u32>,
                dest: String,
            }
            let a: A = params(p)?;
            if method == "messages.move" {
                client.move_messages(&a.folder, &a.uids, &a.dest).await?;
            } else {
                client.copy_messages(&a.folder, &a.uids, &a.dest).await?;
            }
            ok(())
        }
        "messages.delete" => {
            #[derive(Deserialize)]
            struct A {
                folder: String,
                uids: Vec<u32>,
                #[serde(default)]
                permanent: bool,
            }
            let a: A = params(p)?;
            client.delete_messages(&a.folder, &a.uids, a.permanent).await?;
            ok(())
        }
        "messages.archive" => {
            let a: MessagesArg = params(p)?;
            ok(json!({ "folder": client.archive(&a.folder, &a.uids).await? }))
        }
        "messages.junk" => {
            #[derive(Deserialize)]
            struct A {
                folder: String,
                uids: Vec<u32>,
                junk: bool,
            }
            let a: A = params(p)?;
            ok(json!({ "folder": client.junk(&a.folder, &a.uids, a.junk).await? }))
        }
        "messages.sendReceipt" => {
            let a: MessageArg = params(p)?;
            client.send_receipt(&a.folder, a.uid).await?;
            ok(())
        }

        // ---- composing ----
        "compose.prepare" => {
            #[derive(Deserialize)]
            struct A {
                #[serde(default)]
                mode: ComposeMode,
                #[serde(default)]
                source: Option<MessageRef>,
                #[serde(default = "yes")]
                html: bool,
                #[serde(default)]
                style: QuoteStyle,
            }
            let a: A = params(p)?;
            ok(client.prepare(a.mode, a.source, a.html, &a.style).await?)
        }
        "compose.saveDraft" => {
            #[derive(Deserialize)]
            struct A {
                draft: Draft,
            }
            let a: A = params(p)?;
            ok(client.save_draft(a.draft).await?)
        }
        "compose.discardDraft" => {
            let a: MessageArg = params(p)?;
            client.discard_draft(&MessageRef { folder: a.folder, uid: a.uid }).await?;
            ok(())
        }
        "compose.send" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                draft: Draft,
                #[serde(default = "yes")]
                save_copy: bool,
            }
            let a: A = params(p)?;
            ok(client.send(a.draft, a.save_copy).await?)
        }
        "compose.suggest" => {
            #[derive(Deserialize)]
            struct A {
                query: String,
            }
            let a: A = params(p)?;
            ok(store.suggest(&owner, &a.query, 12)?)
        }
        "uploads.delete" => {
            #[derive(Deserialize)]
            struct A {
                id: String,
            }
            let a: A = params(p)?;
            client.delete_upload(&a.id);
            ok(())
        }

        // ---- identities ----
        "identities.list" => ok(client.identities()?),
        "identities.save" => {
            #[derive(Deserialize)]
            struct A {
                identity: Identity,
            }
            let a: A = params(p)?;
            ok(store.save_identity(&owner, a.identity)?)
        }
        "identities.delete" => {
            #[derive(Deserialize)]
            struct A {
                id: String,
            }
            let a: A = params(p)?;
            store.delete_identity(&owner, &a.id)?;
            ok(())
        }

        // ---- contacts ----
        "contacts.list" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                #[serde(default)]
                query: String,
                #[serde(default)]
                include_collected: bool,
            }
            let a: A = params(p)?;
            ok(store.contacts(&owner, &a.query, a.include_collected, 5000)?)
        }
        "contacts.get" => {
            #[derive(Deserialize)]
            struct A {
                id: String,
            }
            let a: A = params(p)?;
            ok(store.contact(&owner, &a.id)?)
        }
        "contacts.save" => {
            #[derive(Deserialize)]
            struct A {
                contact: Contact,
            }
            let a: A = params(p)?;
            ok(store.save_contact(&owner, a.contact)?)
        }
        "contacts.delete" => {
            #[derive(Deserialize)]
            struct A {
                ids: Vec<String>,
            }
            let a: A = params(p)?;
            store.delete_contacts(&owner, &a.ids)?;
            ok(())
        }
        "contacts.groups" => ok(store.contact_groups(&owner)?),
        "contacts.import" => {
            #[derive(Deserialize)]
            struct A {
                text: String,
                #[serde(default)]
                group: Option<String>,
            }
            let a: A = params(p)?;
            let text = a.text.trim_start_matches('\u{feff}');
            let contacts = if text.trim_start().to_ascii_uppercase().starts_with("BEGIN:VCARD") {
                vcard::import(text)
            } else {
                vcard::import_csv(text)
            };
            let existing = store.contacts(&owner, "", false, 100_000)?;
            let (mut added, mut skipped) = (0, 0);
            for mut c in contacts {
                let duplicate = c.emails.iter().any(|e| existing.iter().any(|x| x.emails.iter().any(|y| y.eq_ignore_ascii_case(e))));
                if duplicate {
                    skipped += 1;
                    continue;
                }
                if let Some(g) = a.group.as_ref().filter(|g| !g.trim().is_empty())
                    && !c.groups.contains(g)
                {
                    c.groups.push(g.trim().to_string());
                }
                c.emails.retain(|e| crate::compose::validate_address(e).is_ok());
                match store.save_contact(&owner, c) {
                    Ok(_) => added += 1,
                    Err(_) => skipped += 1,
                }
            }
            ok(json!({ "added": added, "skipped": skipped }))
        }
        "contacts.export" => {
            #[derive(Deserialize)]
            struct A {
                #[serde(default)]
                ids: Option<Vec<String>>,
            }
            let a: A = params(p)?;
            let mut contacts = store.contacts(&owner, "", false, 100_000)?;
            if let Some(ids) = a.ids {
                contacts.retain(|c| ids.contains(&c.id));
            }
            ok(json!({ "vcard": vcard::export(&contacts) }))
        }

        // ---- settings, quota, filters ----
        "settings.get" => ok(store.get_value(&owner, "settings")?.unwrap_or_else(|| json!({}))),
        "settings.set" => {
            #[derive(Deserialize)]
            struct A {
                settings: Value,
            }
            let a: A = params(p)?;
            store.set_value(&owner, "settings", &a.settings)?;
            ok(())
        }
        "quota.get" => ok(client.quota().await?),
        "filters.get" => ok(client.filters().await?),
        "filters.save" => {
            #[derive(Deserialize)]
            struct A {
                filters: Filters,
            }
            let a: A = params(p)?;
            client.save_filters(&a.filters).await?;
            ok(())
        }
        _ => Err(Error::not_found(format!("Unknown method: {method}"))),
    }
}

fn yes() -> bool {
    true
}

fn page_size() -> u32 {
    100
}

fn double_option<'de, D>(deserializer: D) -> std::result::Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}
