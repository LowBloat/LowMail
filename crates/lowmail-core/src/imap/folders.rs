//! Folder list, special folders and folder management.

use std::collections::{HashMap, HashSet};

use async_imap::imap_proto::{MailboxDatum, NameAttribute, Response, StatusAttribute};

use super::{Conn, quote};
use crate::error::{Error, Result};
use crate::model::{Folder, FolderCount, FolderRole};
use crate::utf7;

#[derive(Debug, Clone)]
pub struct FolderList {
    pub folders: Vec<Folder>,
    pub delimiter: Option<String>,
    /// Personal namespace prefix such as `INBOX.` on Courier-style servers.
    pub prefix: String,
}

impl FolderList {
    pub fn by_role(&self, role: FolderRole) -> Option<&Folder> {
        self.folders.iter().find(|f| f.role == Some(role))
    }

    pub fn get(&self, path: &str) -> Option<&Folder> {
        self.folders.iter().find(|f| f.path == path)
    }
}

struct RawFolder {
    path: String,
    delimiter: Option<String>,
    attrs: Vec<String>,
    special: Option<FolderRole>,
}

impl Conn {
    pub async fn list_folders(&mut self, counts: bool) -> Result<FolderList> {
        let list_status = counts && self.caps.has("LIST-STATUS");
        let command = if list_status {
            "LIST \"\" \"*\" RETURN (SUBSCRIBED SPECIAL-USE STATUS (MESSAGES UNSEEN))".to_string()
        } else if self.caps.has("SPECIAL-USE") && self.caps.has("LIST-EXTENDED") {
            "LIST \"\" \"*\" RETURN (SUBSCRIBED SPECIAL-USE)".to_string()
        } else {
            "LIST \"\" \"*\"".to_string()
        };
        let mut raw = Vec::new();
        let mut statuses: HashMap<String, (u32, u32)> = HashMap::new();
        let mut subscribed: HashSet<String> = HashSet::new();
        let extended = command.contains("RETURN");
        self.raw_each(&command, |resp| match resp {
            Response::MailboxData(MailboxDatum::List(data)) => {
                let mut attrs = Vec::new();
                let mut special = None;
                for attr in &data.name_attributes {
                    match attr {
                        NameAttribute::Sent => special = Some(FolderRole::Sent),
                        NameAttribute::Drafts => special = Some(FolderRole::Drafts),
                        NameAttribute::Trash => special = Some(FolderRole::Trash),
                        NameAttribute::Junk => special = Some(FolderRole::Junk),
                        NameAttribute::Archive => special = Some(FolderRole::Archive),
                        NameAttribute::NoSelect => attrs.push("\\NOSELECT".into()),
                        NameAttribute::NoInferiors => attrs.push("\\NOINFERIORS".into()),
                        NameAttribute::Extension(e) => attrs.push(e.to_ascii_uppercase()),
                        _ => {}
                    }
                }
                if extended && attrs.iter().any(|a| a == "\\SUBSCRIBED") {
                    subscribed.insert(data.name.to_string());
                }
                raw.push(RawFolder {
                    path: data.name.to_string(),
                    delimiter: data.delimiter.as_ref().map(|d| d.to_string()),
                    attrs,
                    special,
                });
            }
            Response::MailboxData(MailboxDatum::Status { mailbox, status }) => {
                statuses.insert(mailbox.to_string(), status_counts(status));
            }
            _ => {}
        })
        .await?;
        if !extended {
            let _ = self
                .raw_each("LSUB \"\" \"*\"", |resp| {
                    if let Response::MailboxData(MailboxDatum::List(data)) = resp {
                        subscribed.insert(data.name.to_string());
                    }
                })
                .await;
        }
        if !raw.iter().any(|f| f.path.eq_ignore_ascii_case("INBOX")) {
            raw.push(RawFolder { path: "INBOX".into(), delimiter: None, attrs: vec![], special: None });
        }
        let delimiter = raw.iter().find_map(|f| f.delimiter.clone());
        let prefix = personal_prefix(&raw, delimiter.as_deref());

        if counts && !list_status {
            for f in &raw {
                if f.attrs.iter().any(|a| a == "\\NOSELECT" || a == "\\NONEXISTENT") {
                    continue;
                }
                if let Ok(c) = self.status_counts(&f.path).await {
                    statuses.insert(f.path.clone(), c);
                }
            }
        }

        let folders = build_tree(raw, &prefix, delimiter.as_deref(), &subscribed, &statuses);
        Ok(FolderList { folders, delimiter, prefix })
    }

    pub async fn status_counts(&mut self, path: &str) -> Result<(u32, u32)> {
        // Dovecot answers STATUS on the selected folder from this session's
        // view, which misses what other connections added until a NOOP.
        if self.selected.as_ref().is_some_and(|s| s.path == path) {
            self.select(path).await?;
        }
        let mut counts = None;
        self.raw_each(&format!("STATUS {} (MESSAGES UNSEEN)", quote(path)), |resp| {
            if let Response::MailboxData(MailboxDatum::Status { status, .. }) = resp {
                counts = Some(status_counts(status));
            }
        })
        .await?;
        counts.ok_or_else(|| Error::server("The server did not report the folder status."))
    }

    pub async fn counts(&mut self, paths: &[String]) -> Vec<FolderCount> {
        let mut out = Vec::new();
        for path in paths {
            if let Ok((total, unread)) = self.status_counts(path).await {
                out.push(FolderCount { path: path.clone(), total, unread });
            }
        }
        out
    }

    pub async fn create_folder(&mut self, path: &str) -> Result<()> {
        self.session.create(path).await?;
        let _ = self.session.subscribe(path).await;
        Ok(())
    }

    pub async fn rename_folder(&mut self, from: &str, to: &str) -> Result<()> {
        if self.selected.as_ref().is_some_and(|s| s.path == from) {
            self.unselect().await;
        }
        self.session.rename(from, to).await?;
        let _ = self.session.unsubscribe(from).await;
        let _ = self.session.subscribe(to).await;
        Ok(())
    }

    pub async fn delete_folder(&mut self, path: &str) -> Result<()> {
        if self.selected.as_ref().is_some_and(|s| s.path == path) {
            self.unselect().await;
        }
        let _ = self.session.unsubscribe(path).await;
        self.session.delete(path).await?;
        Ok(())
    }

    pub async fn set_subscribed(&mut self, path: &str, subscribed: bool) -> Result<()> {
        if subscribed {
            self.session.subscribe(path).await?;
        } else {
            self.session.unsubscribe(path).await?;
        }
        Ok(())
    }
}

/// `INBOX.` when every other folder lives under INBOX, as on Courier and
/// cPanel servers (their personal namespace). Such folders are shown at the
/// top level, like the other clients do.
fn personal_prefix(raw: &[RawFolder], delimiter: Option<&str>) -> String {
    let Some(delim) = delimiter else { return String::new() };
    let prefix = format!("INBOX{delim}");
    let others: Vec<&RawFolder> = raw.iter().filter(|f| !f.path.eq_ignore_ascii_case("INBOX")).collect();
    if !others.is_empty() && others.iter().all(|f| f.path.starts_with(&prefix)) { prefix } else { String::new() }
}

fn status_counts(status: &[StatusAttribute]) -> (u32, u32) {
    let mut total = 0;
    let mut unread = 0;
    for s in status {
        match s {
            StatusAttribute::Messages(n) => total = *n,
            StatusAttribute::Unseen(n) => unread = *n,
            _ => {}
        }
    }
    (total, unread)
}

/// Well-known names for servers without SPECIAL-USE, in English and Portuguese.
fn role_by_name(leaf: &str) -> Option<FolderRole> {
    let name = leaf.trim().to_lowercase();
    let role = match name.as_str() {
        "sent" | "sent items" | "sent messages" | "sent mail" | "enviados" | "itens enviados" | "enviadas" | "mensagens enviadas" => {
            FolderRole::Sent
        }
        "drafts" | "draft" | "rascunhos" | "rascunho" => FolderRole::Drafts,
        "trash" | "deleted items" | "deleted messages" | "bin" | "lixeira" | "itens excluídos" | "itens excluidos" | "excluídos"
        | "excluidos" => FolderRole::Trash,
        "junk" | "spam" | "junk e-mail" | "junk email" | "lixo eletrônico" | "lixo eletronico" | "bulk mail" => FolderRole::Junk,
        "archive" | "archives" | "arquivo" | "arquivos" | "arquivados" => FolderRole::Archive,
        _ => return None,
    };
    Some(role)
}

fn role_rank(role: Option<FolderRole>) -> u8 {
    match role {
        Some(FolderRole::Inbox) => 0,
        Some(FolderRole::Drafts) => 1,
        Some(FolderRole::Sent) => 2,
        Some(FolderRole::Archive) => 3,
        Some(FolderRole::Junk) => 4,
        Some(FolderRole::Trash) => 5,
        None => 6,
    }
}

fn build_tree(
    raw: Vec<RawFolder>,
    prefix: &str,
    delimiter: Option<&str>,
    subscribed: &HashSet<String>,
    statuses: &HashMap<String, (u32, u32)>,
) -> Vec<Folder> {
    let delim = delimiter.unwrap_or("/");
    let all: HashSet<String> = raw.iter().map(|f| f.path.clone()).collect();
    let mut folders: Vec<Folder> = raw
        .into_iter()
        .filter(|f| !f.attrs.iter().any(|a| a == "\\NONEXISTENT"))
        .map(|f| {
            let is_inbox = f.path.eq_ignore_ascii_case("INBOX");
            // Paths shown without the personal prefix: `INBOX.Sent` reads `Sent`.
            let visible =
                if !is_inbox && !prefix.is_empty() { f.path.strip_prefix(prefix).unwrap_or(&f.path).to_string() } else { f.path.clone() };
            let segments: Vec<&str> = visible.split(delim).filter(|s| !s.is_empty()).collect();
            let decoded: Vec<String> = segments.iter().map(|s| utf7::decode(s)).collect();
            let parent = if segments.len() > 1 {
                let parent_visible = segments[..segments.len() - 1].join(delim);
                let parent_path = if prefix.is_empty() { parent_visible.clone() } else { format!("{prefix}{parent_visible}") };
                Some(if all.contains(&parent_path) { parent_path } else { parent_visible })
            } else {
                None
            };
            let name = if is_inbox { "INBOX".to_string() } else { decoded.last().cloned().unwrap_or_default() };
            let role = if is_inbox { Some(FolderRole::Inbox) } else { f.special };
            let (total, unread) = statuses.get(&f.path).copied().unwrap_or((0, 0));
            Folder {
                subscribed: is_inbox || subscribed.contains(&f.path),
                selectable: !f.attrs.iter().any(|a| a == "\\NOSELECT"),
                can_have_children: !f.attrs.iter().any(|a| a == "\\NOINFERIORS"),
                display_path: decoded.join(delim),
                depth: segments.len().saturating_sub(1),
                path: f.path,
                name,
                parent,
                delimiter: f.delimiter,
                role,
                total,
                unread,
            }
        })
        .collect();

    // Name-based roles only where the server named none, top level first.
    for role in [FolderRole::Sent, FolderRole::Drafts, FolderRole::Trash, FolderRole::Junk, FolderRole::Archive] {
        if folders.iter().any(|f| f.role == Some(role)) {
            continue;
        }
        let mut candidates: Vec<usize> = folders
            .iter()
            .enumerate()
            .filter(|(_, f)| f.role.is_none() && f.selectable && role_by_name(&f.name) == Some(role))
            .map(|(i, _)| i)
            .collect();
        candidates.sort_by_key(|&i| folders[i].depth);
        if let Some(&i) = candidates.first() {
            folders[i].role = Some(role);
        }
    }

    // Depth-first order: special folders first, then by name.
    let mut children: HashMap<Option<String>, Vec<usize>> = HashMap::new();
    let paths: HashSet<String> = folders.iter().map(|f| f.path.clone()).collect();
    for (i, f) in folders.iter().enumerate() {
        let parent = f.parent.clone().filter(|p| paths.contains(p));
        children.entry(parent).or_default().push(i);
    }
    for list in children.values_mut() {
        list.sort_by(|&a, &b| {
            let (fa, fb) = (&folders[a], &folders[b]);
            role_rank(fa.role).cmp(&role_rank(fb.role)).then_with(|| fa.name.to_lowercase().cmp(&fb.name.to_lowercase()))
        });
    }
    let mut order = Vec::with_capacity(folders.len());
    fn walk(key: Option<String>, children: &HashMap<Option<String>, Vec<usize>>, folders: &[Folder], order: &mut Vec<usize>) {
        if let Some(list) = children.get(&key) {
            for &i in list {
                order.push(i);
                walk(Some(folders[i].path.clone()), children, folders, order);
            }
        }
    }
    walk(None, &children, &folders, &mut order);
    // Orphans whose parent is missing were placed at the root by the map above.
    let mut slots: Vec<Option<Folder>> = folders.into_iter().map(Some).collect();
    let mut out: Vec<Folder> = order.into_iter().filter_map(|i| slots[i].take()).collect();
    out.extend(slots.into_iter().flatten());
    // Depth relative to the visible parent chain.
    let depth_of: HashMap<String, usize> = {
        let mut map = HashMap::new();
        for f in &out {
            let d = f.parent.as_ref().and_then(|p| map.get(p).map(|d: &usize| d + 1)).unwrap_or(0);
            map.insert(f.path.clone(), d);
        }
        map
    };
    for f in &mut out {
        f.depth = depth_of.get(&f.path).copied().unwrap_or(0);
        if f.parent.as_ref().is_some_and(|p| !paths.contains(p)) {
            f.parent = None;
        }
    }
    out
}

/// Server path for a new folder named `name` (decoded) under `parent`.
pub fn child_path(list: &FolderList, parent: Option<&str>, name: &str) -> Result<String> {
    let name = name.trim();
    let delim = list.delimiter.as_deref().unwrap_or("/");
    if name.is_empty() {
        return Err(Error::invalid("Enter a folder name."));
    }
    if name.contains(delim) {
        return Err(Error::invalid(format!("Folder names cannot contain \"{delim}\".")));
    }
    let encoded = utf7::encode(name);
    Ok(match parent {
        Some(p) if !p.is_empty() => format!("{p}{delim}{encoded}"),
        _ => format!("{}{}", list.prefix, encoded),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(path: &str, special: Option<FolderRole>) -> RawFolder {
        RawFolder { path: path.into(), delimiter: Some(".".into()), attrs: vec![], special }
    }

    #[test]
    fn strips_inbox_prefix_and_finds_roles() {
        let folders = build_tree(
            vec![
                raw("INBOX", None),
                raw("INBOX.Sent", None),
                raw("INBOX.Lixeira", None),
                raw("INBOX.Projetos", None),
                raw("INBOX.Projetos.Relat&APM-rios", None),
            ],
            "INBOX.",
            Some("."),
            &HashSet::new(),
            &HashMap::new(),
        );
        let names: Vec<_> = folders.iter().map(|f| (f.display_path.as_str(), f.depth, f.role)).collect();
        assert_eq!(
            names,
            vec![
                ("INBOX", 0, Some(FolderRole::Inbox)),
                ("Sent", 0, Some(FolderRole::Sent)),
                ("Lixeira", 0, Some(FolderRole::Trash)),
                ("Projetos", 0, None),
                ("Projetos.Relatórios", 1, None),
            ]
        );
        assert_eq!(folders[4].parent.as_deref(), Some("INBOX.Projetos"));
    }
}
