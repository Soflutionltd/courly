//! The signature rows as the mailbox sync reads and writes them.
//!
//! An account whose provider can keep application documents in the mailbox
//! shares its signatures with every other Courly attached to it. This
//! module is the table side of that exchange; what to pull, push or delete
//! is decided by the service.

use crate::db::{ReadConn, WriteTarget, WriteTransactionTarget};
use rusqlite::params;

/// `signatures.source` of a row the mailbox handed over.
const MAILBOX_SOURCE: &str = "mailbox_sync";

/// `signatures.source` of a row mirrored from Gmail's own signature
/// setting, which has its own sync and stays out of this one.
const GMAIL_SOURCE: &str = "gmail_sync";

/// One signature as the mailbox sync sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxSignatureRow {
    pub id: String,
    /// The signature's id in the mailbox document, once it has been there.
    pub server_id: Option<String>,
    pub name: String,
    pub body_html: String,
    pub body_text: Option<String>,
    pub is_default: bool,
    pub is_reply_default: bool,
    pub sort_order: i64,
    /// Hash of the content the mailbox and this row last agreed on.
    pub synced_hash: Option<String>,
}

/// A row that now matches the mailbox without its content changing here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxSignatureMark {
    pub id: String,
    pub server_id: String,
    pub synced_hash: String,
}

/// What one sync pass changes in the table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MailboxSignatureWrite {
    /// Rows to create or overwrite with the mailbox's content. Each carries
    /// its `server_id` and `synced_hash`.
    pub pulled: Vec<MailboxSignatureRow>,
    pub marked: Vec<MailboxSignatureMark>,
    /// Ids of rows the mailbox no longer has.
    pub deleted: Vec<String>,
    /// Deletions the mailbox has now been told about.
    pub cleared_tombstones: Vec<String>,
}

impl MailboxSignatureWrite {
    pub fn is_empty(&self) -> bool {
        self.pulled.is_empty()
            && self.marked.is_empty()
            && self.deleted.is_empty()
            && self.cleared_tombstones.is_empty()
    }
}

/// Every signature of `account_id` that takes part in the mailbox sync.
pub fn mailbox_signature_rows_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<Vec<MailboxSignatureRow>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, server_id, name, body_html, body_text, is_default, \
             is_reply_default, sort_order, server_html_hash \
             FROM signatures WHERE account_id = ?1 AND source <> ?2 \
             ORDER BY sort_order, created_at, id",
        )
        .map_err(|e| format!("prepare mailbox signature rows: {e}"))?;
    let rows = statement
        .query_map(params![account_id, GMAIL_SOURCE], |row| {
            Ok(MailboxSignatureRow {
                id: row.get("id")?,
                server_id: row.get("server_id")?,
                name: row.get("name")?,
                body_html: row.get("body_html")?,
                body_text: row.get("body_text")?,
                is_default: row.get::<_, i64>("is_default")? != 0,
                is_reply_default: row.get::<_, i64>("is_reply_default")? != 0,
                sort_order: row.get::<_, Option<i64>>("sort_order")?.unwrap_or(0),
                synced_hash: row.get("server_html_hash")?,
            })
        })
        .map_err(|e| format!("query mailbox signature rows: {e}"))?;
    rows.map(|row| row.map_err(|e| format!("read mailbox signature row: {e}")))
        .collect()
}

/// Mailbox ids of the signatures deleted here since the last sync.
pub fn signature_tombstones_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare("SELECT server_id FROM signature_tombstones WHERE account_id = ?1")
        .map_err(|e| format!("prepare signature tombstones: {e}"))?;
    let rows = statement
        .query_map(params![account_id], |row| row.get::<_, String>(0))
        .map_err(|e| format!("query signature tombstones: {e}"))?;
    rows.map(|row| row.map_err(|e| format!("read signature tombstone: {e}")))
        .collect()
}

/// Remember that the signature `id` was deleted here, when the mailbox
/// knows it, so the next sync removes it there instead of bringing it back.
pub(super) fn record_signature_tombstone(conn: &impl WriteTarget, id: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO signature_tombstones (account_id, server_id) \
         SELECT account_id, server_id FROM signatures \
         WHERE id = ?1 AND server_id IS NOT NULL AND source <> ?2",
        params![id, GMAIL_SOURCE],
    )
    .map_err(|e| format!("record signature tombstone: {e}"))?;
    Ok(())
}

/// Apply one sync pass to the table, all of it or none.
pub fn apply_mailbox_signature_sync_sync(
    conn: &impl WriteTransactionTarget,
    account_id: &str,
    write: &MailboxSignatureWrite,
    now: i64,
) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for id in &write.deleted {
        tx.execute(
            "DELETE FROM signatures WHERE id = ?1 AND account_id = ?2",
            params![id, account_id],
        )
        .map_err(|e| format!("delete signature gone from the mailbox: {e}"))?;
    }
    for row in &write.pulled {
        tx.execute(
            "INSERT INTO signatures \
             (id, account_id, name, body_html, body_text, is_default, is_reply_default, \
              sort_order, server_id, source, server_html_hash, last_synced_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
             ON CONFLICT(id) DO UPDATE SET \
               name = excluded.name, body_html = excluded.body_html, \
               body_text = excluded.body_text, is_default = excluded.is_default, \
               is_reply_default = excluded.is_reply_default, \
               sort_order = excluded.sort_order, server_id = excluded.server_id, \
               source = excluded.source, server_html_hash = excluded.server_html_hash, \
               last_synced_at = excluded.last_synced_at",
            params![
                row.id,
                account_id,
                row.name,
                row.body_html,
                row.body_text,
                i64::from(row.is_default),
                i64::from(row.is_reply_default),
                row.sort_order,
                row.server_id,
                MAILBOX_SOURCE,
                row.synced_hash,
                now,
            ],
        )
        .map_err(|e| format!("write signature from the mailbox: {e}"))?;
    }
    for mark in &write.marked {
        tx.execute(
            "UPDATE signatures SET server_id = ?1, server_html_hash = ?2, last_synced_at = ?3 \
             WHERE id = ?4 AND account_id = ?5",
            params![mark.server_id, mark.synced_hash, now, mark.id, account_id],
        )
        .map_err(|e| format!("mark signature synced: {e}"))?;
    }
    for server_id in &write.cleared_tombstones {
        tx.execute(
            "DELETE FROM signature_tombstones WHERE account_id = ?1 AND server_id = ?2",
            params![account_id, server_id],
        )
        .map_err(|e| format!("clear signature tombstone: {e}"))?;
    }
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::WriteConn;
    use crate::db::queries_extra::{
        InsertSignatureParams, db_delete_signature_sync, db_insert_signature_sync,
    };
    use rusqlite::Connection;

    fn open_with_account() -> Connection {
        let raw = Connection::open_in_memory().expect("open");
        crate::db::migrations::run_all(&raw).expect("migrate");
        raw.execute(
            "INSERT INTO accounts (id, email, provider) VALUES ('acc', 'me@example.com', 'graph')",
            [],
        )
        .expect("account");
        raw
    }

    fn insert(conn: &WriteConn<'_>, name: &str) -> String {
        db_insert_signature_sync(
            conn,
            &InsertSignatureParams {
                account_id: "acc".to_owned(),
                name: name.to_owned(),
                body_html: format!("<p>{name}</p>"),
                body_text: Some(name.to_owned()),
                is_default: false,
                is_reply_default: false,
            },
        )
        .expect("insert signature")
    }

    fn pulled(id: &str, name: &str) -> MailboxSignatureRow {
        MailboxSignatureRow {
            id: id.to_owned(),
            server_id: Some(id.to_owned()),
            name: name.to_owned(),
            body_html: format!("<p>{name}</p>"),
            body_text: None,
            is_default: true,
            is_reply_default: false,
            sort_order: 2,
            synced_hash: Some("hash".to_owned()),
        }
    }

    #[test]
    fn a_pass_creates_overwrites_marks_and_deletes_in_one_go() {
        let raw = open_with_account();
        let conn = WriteConn::from_raw(&raw);
        let kept = insert(&conn, "Kept");
        let gone = insert(&conn, "Gone");

        let write = MailboxSignatureWrite {
            pulled: vec![pulled("from-mailbox", "Shared")],
            marked: vec![MailboxSignatureMark {
                id: kept.clone(),
                server_id: kept.clone(),
                synced_hash: "kept-hash".to_owned(),
            }],
            deleted: vec![gone],
            cleared_tombstones: Vec::new(),
        };
        apply_mailbox_signature_sync_sync(&conn, "acc", &write, 1_700_000_000).expect("apply");

        let mut rows = mailbox_signature_rows_sync(&conn.as_read(), "acc").expect("rows");
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Kept");
        assert_eq!(rows[0].server_id.as_deref(), Some(kept.as_str()));
        assert_eq!(rows[0].synced_hash.as_deref(), Some("kept-hash"));
        assert_eq!(rows[1], pulled("from-mailbox", "Shared"));

        // A second pass with the mailbox's new text lands on the same row.
        let mut renamed = pulled("from-mailbox", "Shared");
        renamed.body_html = "<p>new</p>".to_owned();
        let write = MailboxSignatureWrite {
            pulled: vec![renamed.clone()],
            ..MailboxSignatureWrite::default()
        };
        apply_mailbox_signature_sync_sync(&conn, "acc", &write, 1_700_000_100).expect("apply");
        let rows = mailbox_signature_rows_sync(&conn.as_read(), "acc").expect("rows");
        assert_eq!(rows.len(), 2);
        assert!(rows.contains(&renamed));
    }

    #[test]
    fn deleting_a_synced_signature_leaves_a_tombstone_until_the_mailbox_is_told() {
        let raw = open_with_account();
        let conn = WriteConn::from_raw(&raw);
        let never_synced = insert(&conn, "Local only");
        let synced = insert(&conn, "Shared");
        let write = MailboxSignatureWrite {
            marked: vec![MailboxSignatureMark {
                id: synced.clone(),
                server_id: "mailbox-id".to_owned(),
                synced_hash: "hash".to_owned(),
            }],
            ..MailboxSignatureWrite::default()
        };
        apply_mailbox_signature_sync_sync(&conn, "acc", &write, 1_700_000_000).expect("apply");

        db_delete_signature_sync(&conn, &never_synced).expect("delete local");
        db_delete_signature_sync(&conn, &synced).expect("delete synced");
        assert_eq!(
            signature_tombstones_sync(&conn.as_read(), "acc").expect("tombstones"),
            vec!["mailbox-id".to_owned()]
        );

        let write = MailboxSignatureWrite {
            cleared_tombstones: vec!["mailbox-id".to_owned()],
            ..MailboxSignatureWrite::default()
        };
        apply_mailbox_signature_sync_sync(&conn, "acc", &write, 1_700_000_100).expect("apply");
        assert!(
            signature_tombstones_sync(&conn.as_read(), "acc")
                .expect("tombstones")
                .is_empty()
        );
    }

    #[test]
    fn a_signature_mirrored_from_gmail_stays_out_of_the_mailbox_sync() {
        let raw = open_with_account();
        let conn = WriteConn::from_raw(&raw);
        raw.execute(
            "INSERT INTO signatures (id, account_id, name, body_html, server_id, source) \
             VALUES ('g', 'acc', 'Gmail', '<p>g</p>', 'me@example.com', 'gmail_sync')",
            [],
        )
        .expect("gmail row");
        assert!(
            mailbox_signature_rows_sync(&conn.as_read(), "acc")
                .expect("rows")
                .is_empty()
        );
        db_delete_signature_sync(&conn, "g").expect("delete");
        assert!(
            signature_tombstones_sync(&conn.as_read(), "acc")
                .expect("tombstones")
                .is_empty()
        );
    }
}
