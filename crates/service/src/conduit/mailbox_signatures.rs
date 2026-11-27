//! Signatures shared through the mailbox.
//!
//! On an account whose provider keeps application documents in the mailbox
//! (`pim_methods.app_data`), every signature of the account lives in one
//! document there. Each Courly attached to the account merges it with its
//! own table: what another one wrote is pulled, what was written here is
//! pushed, and a signature changed on both sides since they last agreed
//! takes the mailbox's version.

use std::collections::{HashMap, HashSet};

use conduit_sync::SyncEngine;
use conduit_types::AccountId;
use db::db::ReadDbState;
use db::db::queries_extra::{
    MailboxSignatureMark, MailboxSignatureRow, MailboxSignatureWrite,
    apply_mailbox_signature_sync_sync, mailbox_signature_rows_sync, signature_tombstones_sync,
};
use serde::{Deserialize, Serialize};
use service_state::WriteDbState;
use sha2::{Digest, Sha256};

/// Key of the signatures document in the mailbox's application store.
const DOCUMENT_KEY: &str = "signatures";

/// Layout version this build reads and writes.
const DOCUMENT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Document {
    version: u32,
    signatures: Vec<Entry>,
}

/// One signature in the mailbox document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    id: String,
    name: String,
    body_html: String,
    body_text: Option<String>,
    is_default: bool,
    is_reply_default: bool,
    sort_order: i64,
}

impl Entry {
    fn of(row: &MailboxSignatureRow, id: String) -> Self {
        Self {
            id,
            name: row.name.clone(),
            body_html: row.body_html.clone(),
            body_text: row.body_text.clone(),
            is_default: row.is_default,
            is_reply_default: row.is_reply_default,
            sort_order: row.sort_order,
        }
    }

    /// Whether `row` holds exactly this signature's content.
    fn matches(&self, row: &MailboxSignatureRow) -> bool {
        Self::of(row, self.id.clone()) == *self
    }

    fn hash(&self) -> String {
        let mut hasher = Sha256::new();
        for field in [&self.name, &self.body_html] {
            hasher.update(u64::try_from(field.len()).unwrap_or(u64::MAX).to_le_bytes());
            hasher.update(field.as_bytes());
        }
        match &self.body_text {
            Some(text) => {
                hasher.update([1]);
                hasher.update(text.as_bytes());
            }
            None => hasher.update([0]),
        }
        hasher.update([u8::from(self.is_default), u8::from(self.is_reply_default)]);
        hasher.update(self.sort_order.to_le_bytes());
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// The mailbox id a row answers to: the one it was synced under, else its
/// own id, which it takes there on its first push.
fn server_id_of(row: &MailboxSignatureRow) -> &str {
    row.server_id.as_deref().unwrap_or(&row.id)
}

/// Whether the table holds something the mailbox has not been told.
fn local_changes_pending(rows: &[MailboxSignatureRow], tombstones: &[String]) -> bool {
    !tombstones.is_empty()
        || rows.iter().any(|row| {
            row.server_id.is_none()
                || row.synced_hash.as_deref()
                    != Some(Entry::of(row, server_id_of(row).to_owned()).hash().as_str())
        })
}

#[derive(Debug, PartialEq, Eq)]
struct Merge {
    /// The document to store, when it differs from the one read.
    document: Option<Document>,
    write: MailboxSignatureWrite,
}

fn merge(remote: Option<&Document>, local: &[MailboxSignatureRow], tombstones: &[String]) -> Merge {
    let deleted_here: HashSet<&str> = tombstones.iter().map(String::as_str).collect();
    let by_server_id: HashMap<&str, &MailboxSignatureRow> =
        local.iter().map(|row| (server_id_of(row), row)).collect();
    let remote_entries: &[Entry] = remote.map_or(&[], |document| &document.signatures);
    let remote_ids: HashSet<&str> = remote_entries.iter().map(|e| e.id.as_str()).collect();

    let mut entries: Vec<Entry> = Vec::new();
    for entry in remote_entries {
        if deleted_here.contains(entry.id.as_str()) {
            continue;
        }
        let kept = match by_server_id.get(entry.id.as_str()) {
            // Written here since both sides agreed, untouched over there.
            Some(row)
                if row.synced_hash.as_deref() == Some(entry.hash().as_str())
                    && !entry.matches(row) =>
            {
                Entry::of(row, entry.id.clone())
            }
            _ => entry.clone(),
        };
        entries.push(kept);
    }
    let mut deleted = Vec::new();
    for row in local {
        if remote_ids.contains(server_id_of(row)) {
            continue;
        }
        // A row the mailbox once held and no longer lists was deleted by
        // another Courly. With no document at all nothing can have been
        // deleted from it: the store was emptied, and the rows fill it again.
        if remote.is_some() && row.server_id.is_some() && row.synced_hash.is_some() {
            deleted.push(row.id.clone());
        } else {
            entries.push(Entry::of(row, server_id_of(row).to_owned()));
        }
    }
    keep_one(&mut entries, |entry| &mut entry.is_default);
    keep_one(&mut entries, |entry| &mut entry.is_reply_default);

    let mut write = MailboxSignatureWrite {
        deleted,
        cleared_tombstones: tombstones.to_vec(),
        ..MailboxSignatureWrite::default()
    };
    for entry in &entries {
        let hash = entry.hash();
        match by_server_id.get(entry.id.as_str()) {
            Some(row) if entry.matches(row) => {
                if row.server_id.as_deref() != Some(entry.id.as_str())
                    || row.synced_hash.as_deref() != Some(hash.as_str())
                {
                    write.marked.push(MailboxSignatureMark {
                        id: row.id.clone(),
                        server_id: entry.id.clone(),
                        synced_hash: hash,
                    });
                }
            }
            known => write.pulled.push(MailboxSignatureRow {
                id: known.map_or_else(|| entry.id.clone(), |row| row.id.clone()),
                server_id: Some(entry.id.clone()),
                name: entry.name.clone(),
                body_html: entry.body_html.clone(),
                body_text: entry.body_text.clone(),
                is_default: entry.is_default,
                is_reply_default: entry.is_reply_default,
                sort_order: entry.sort_order,
                synced_hash: Some(hash),
            }),
        }
    }
    let document = Document {
        version: DOCUMENT_VERSION,
        signatures: entries,
    };
    let unchanged = match remote {
        Some(remote) => *remote == document,
        // Nothing to say is not worth creating the store for.
        None => document.signatures.is_empty(),
    };
    Merge {
        document: (!unchanged).then_some(document),
        write,
    }
}

/// Two sides each naming a default leaves two; the first in order stays.
fn keep_one(entries: &mut [Entry], flag: impl Fn(&mut Entry) -> &mut bool) {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&index| entries[index].sort_order);
    let mut seen = false;
    for index in order {
        let value = flag(&mut entries[index]);
        if *value && seen {
            *value = false;
        }
        seen |= *value;
    }
}

/// What one pass over an account's signatures did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SignatureSyncOutcome {
    /// Whether the table gained, lost or changed a signature.
    pub(crate) local_changed: bool,
}

/// Merge the account's signatures with the mailbox document. With
/// `only_if_pending`, the mailbox is left alone unless the table holds a
/// change it has not been told about.
pub(crate) async fn sync_mailbox_signatures(
    engine: &SyncEngine,
    account_id: &str,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
    only_if_pending: bool,
) -> Result<SignatureSyncOutcome, String> {
    let aid = account_id.to_owned();
    let (local, tombstones) = read_db
        .with_read(move |conn| {
            Ok((
                mailbox_signature_rows_sync(conn, &aid)?,
                signature_tombstones_sync(conn, &aid)?,
            ))
        })
        .await?;
    if only_if_pending && !local_changes_pending(&local, &tombstones) {
        return Ok(SignatureSyncOutcome {
            local_changed: false,
        });
    }

    let account = AccountId(account_id.to_owned());
    let stored = engine
        .app_data_get(&account, DOCUMENT_KEY.to_owned())
        .await
        .map_err(|e| format!("read the signatures document: {e}"))?;
    let remote = stored
        .as_deref()
        .map(serde_json::from_str::<Document>)
        .transpose()
        .map_err(|e| format!("the signatures document does not parse: {e}"))?;
    if let Some(document) = &remote
        && document.version != DOCUMENT_VERSION
    {
        // A newer Courly wrote it: rewriting it in this layout would drop
        // what that version added.
        return Err(format!(
            "the signatures document is in layout {}, this build reads {DOCUMENT_VERSION}",
            document.version
        ));
    }

    let merged = merge(remote.as_ref(), &local, &tombstones);
    if let Some(document) = &merged.document {
        let text = serde_json::to_string(document)
            .map_err(|e| format!("encode the signatures document: {e}"))?;
        engine
            .app_data_set(&account, DOCUMENT_KEY.to_owned(), Some(text))
            .await
            .map_err(|e| format!("write the signatures document: {e}"))?;
    }
    let local_changed = !merged.write.pulled.is_empty() || !merged.write.deleted.is_empty();
    if !merged.write.is_empty() {
        let aid = account_id.to_owned();
        let now = jiff::Timestamp::now().as_second();
        write_db
            .with_write(move |conn| {
                apply_mailbox_signature_sync_sync(conn, &aid, &merged.write, now)
            })
            .await?;
    }
    Ok(SignatureSyncOutcome { local_changed })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, name: &str) -> MailboxSignatureRow {
        MailboxSignatureRow {
            id: id.to_owned(),
            server_id: None,
            name: name.to_owned(),
            body_html: format!("<p>{name}</p>"),
            body_text: Some(name.to_owned()),
            is_default: false,
            is_reply_default: false,
            sort_order: 0,
            synced_hash: None,
        }
    }

    /// `row` as it stands once it and the mailbox agree on `entry`.
    fn synced(mut row: MailboxSignatureRow, entry: &Entry) -> MailboxSignatureRow {
        row.server_id = Some(entry.id.clone());
        row.synced_hash = Some(entry.hash());
        row
    }

    fn entry(id: &str, name: &str) -> Entry {
        Entry::of(&row(id, name), id.to_owned())
    }

    fn document(signatures: Vec<Entry>) -> Document {
        Document {
            version: DOCUMENT_VERSION,
            signatures,
        }
    }

    #[test]
    fn an_account_without_signatures_leaves_the_mailbox_untouched() {
        let merged = merge(None, &[], &[]);
        assert_eq!(merged.document, None);
        assert!(merged.write.is_empty());
    }

    #[test]
    fn a_first_sync_pushes_the_local_signatures_under_their_own_ids() {
        let local = [row("a", "Work"), row("b", "Short")];
        let merged = merge(None, &local, &[]);
        assert_eq!(
            merged.document,
            Some(document(vec![entry("a", "Work"), entry("b", "Short")]))
        );
        assert!(merged.write.pulled.is_empty());
        assert_eq!(
            merged.write.marked,
            vec![
                MailboxSignatureMark {
                    id: "a".to_owned(),
                    server_id: "a".to_owned(),
                    synced_hash: entry("a", "Work").hash(),
                },
                MailboxSignatureMark {
                    id: "b".to_owned(),
                    server_id: "b".to_owned(),
                    synced_hash: entry("b", "Short").hash(),
                },
            ]
        );
        assert!(local_changes_pending(&local, &[]));
    }

    #[test]
    fn a_second_computer_pulls_what_the_first_one_pushed() {
        let remote = document(vec![entry("a", "Work")]);
        let merged = merge(Some(&remote), &[], &[]);
        assert_eq!(merged.document, None);
        assert_eq!(
            merged.write.pulled,
            vec![synced(row("a", "Work"), &entry("a", "Work"))]
        );
        assert!(merged.write.deleted.is_empty());
    }

    #[test]
    fn two_sides_in_agreement_change_nothing() {
        let remote = document(vec![entry("a", "Work")]);
        let local = [synced(row("a", "Work"), &entry("a", "Work"))];
        let merged = merge(Some(&remote), &local, &[]);
        assert_eq!(merged.document, None);
        assert!(merged.write.is_empty());
        assert!(!local_changes_pending(&local, &[]));
    }

    #[test]
    fn an_edit_made_here_is_pushed() {
        let remote = document(vec![entry("a", "Work")]);
        let mut edited = synced(row("a", "Work"), &entry("a", "Work"));
        edited.body_html = "<p>Work, new phone</p>".to_owned();
        let merged = merge(Some(&remote), std::slice::from_ref(&edited), &[]);

        let pushed = Entry::of(&edited, "a".to_owned());
        assert_eq!(merged.document, Some(document(vec![pushed.clone()])));
        assert!(merged.write.pulled.is_empty());
        assert_eq!(merged.write.marked[0].synced_hash, pushed.hash());
        assert!(local_changes_pending(&[edited], &[]));
    }

    #[test]
    fn an_edit_made_elsewhere_is_pulled_onto_the_local_row() {
        let before = entry("a", "Work");
        let mut after = before.clone();
        after.body_html = "<p>Work, new address</p>".to_owned();
        // The row answers to mailbox id "a" under a local id of its own.
        let local = [synced(row("local-1", "Work"), &before)];
        let merged = merge(Some(&document(vec![after.clone()])), &local, &[]);

        assert_eq!(merged.document, None);
        assert_eq!(merged.write.pulled.len(), 1);
        assert_eq!(merged.write.pulled[0].id, "local-1");
        assert_eq!(merged.write.pulled[0].body_html, after.body_html);
        assert_eq!(merged.write.pulled[0].synced_hash, Some(after.hash()));
    }

    #[test]
    fn a_signature_edited_on_both_sides_takes_the_mailbox_version() {
        let before = entry("a", "Work");
        let mut there = before.clone();
        there.body_html = "<p>there</p>".to_owned();
        let mut here = synced(row("a", "Work"), &before);
        here.body_html = "<p>here</p>".to_owned();
        let merged = merge(Some(&document(vec![there.clone()])), &[here], &[]);

        assert_eq!(merged.document, None);
        assert_eq!(merged.write.pulled[0].body_html, "<p>there</p>");
    }

    #[test]
    fn a_signature_deleted_here_leaves_the_mailbox_and_is_not_pulled_back() {
        let remote = document(vec![entry("a", "Work"), entry("b", "Short")]);
        let local = [synced(row("b", "Short"), &entry("b", "Short"))];
        let tombstones = ["a".to_owned()];
        let merged = merge(Some(&remote), &local, &tombstones);

        assert_eq!(merged.document, Some(document(vec![entry("b", "Short")])));
        assert!(merged.write.pulled.is_empty());
        assert_eq!(merged.write.cleared_tombstones, tombstones);
        assert!(local_changes_pending(&local, &tombstones));
    }

    #[test]
    fn a_signature_deleted_elsewhere_is_deleted_here() {
        let remote = document(vec![entry("b", "Short")]);
        let local = [
            synced(row("a", "Work"), &entry("a", "Work")),
            synced(row("b", "Short"), &entry("b", "Short")),
        ];
        let merged = merge(Some(&remote), &local, &[]);
        assert_eq!(merged.document, None);
        assert_eq!(merged.write.deleted, vec!["a".to_owned()]);
    }

    #[test]
    fn an_emptied_store_is_filled_again_instead_of_emptying_the_table() {
        let local = [synced(row("a", "Work"), &entry("a", "Work"))];
        let merged = merge(None, &local, &[]);
        assert_eq!(merged.document, Some(document(vec![entry("a", "Work")])));
        assert!(merged.write.deleted.is_empty());
    }

    #[test]
    fn two_defaults_named_on_two_computers_come_down_to_one_everywhere() {
        let mut there = entry("a", "Work");
        there.is_default = true;
        let mut here = row("b", "Short");
        here.is_default = true;
        here.sort_order = 1;
        let merged = merge(Some(&document(vec![there.clone()])), &[here], &[]);

        let stored = merged.document.expect("the new signature is pushed");
        let defaults: Vec<&str> = stored
            .signatures
            .iter()
            .filter(|entry| entry.is_default)
            .map(|entry| entry.id.as_str())
            .collect();
        assert_eq!(defaults, vec!["a"]);
        // The local row loses the flag it had claimed.
        let demoted = merged
            .write
            .pulled
            .iter()
            .find(|row| row.id == "b")
            .expect("the local row is rewritten");
        assert!(!demoted.is_default);
    }
}
