//! Folder-wide message actions: mark every message filed in one folder as
//! read, or move every message filed in one folder to Trash. They act on the
//! messages of that folder only, never on whole conversations: a conversation
//! can span folders (a reply in Sent), and emptying one folder must not touch
//! the others.

use std::collections::BTreeSet;

use bifrost_types::{CursorScope, FolderId as BifrostFolderId, ObjectId};
use common::types::{FolderKind, NamespaceAttribution, SystemFolderId};
use db::db::WriteTxn;
use db::db::queries_extra::{recompute_thread_folders_from_messages, replace_message_folder_rows};
use rusqlite::params;

use super::context::ActionContext;
use super::dispatch_target::{RemoteBatchKey, dispatch_bulk_mutation, message_object_id};
use super::outcome::ActionError;
use crate::bifrost::BifrostProviderKind;
use crate::bifrost::resident::ResidentActionAccount;

struct FolderMessage {
    id: String,
    thread_id: String,
    object: ObjectId,
}

/// Messages filed in `folder_id`: through `message_folders`, or through the
/// IMAP path for IMAP messages, whose membership rides `imap_folder`.
async fn folder_messages(
    ctx: &ActionContext,
    provider: BifrostProviderKind,
    account_id: &str,
    folder_id: &str,
    unread_only: bool,
) -> Result<Vec<FolderMessage>, ActionError> {
    let aid = account_id.to_string();
    let fid = folder_id.to_string();
    let rows = ctx
        .db
        .with_read(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT m.id, m.thread_id, m.imap_folder, m.imap_uid, m.imap_uidvalidity \
                     FROM messages m \
                     WHERE m.account_id = ?1 \
                       AND (?3 = 0 OR COALESCE(m.is_read, 0) = 0) \
                       AND (EXISTS (SELECT 1 FROM message_folders mf \
                                    WHERE mf.account_id = m.account_id \
                                      AND mf.message_id = m.id AND mf.folder_id = ?2) \
                            OR (m.imap_folder IS NOT NULL AND m.imap_folder IN \
                                (SELECT f.imap_folder_path FROM folders f \
                                 WHERE f.account_id = ?1 AND f.id = ?2)))",
                )
                .map_err(|error| format!("prepare folder messages: {error}"))?;
            stmt.query_map(params![aid, fid, unread_only], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })
            .map_err(|error| format!("query folder messages: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read folder messages: {error}"))
        })
        .await
        .map_err(ActionError::db)?;

    rows.into_iter()
        .map(|(id, thread_id, imap_folder, imap_uid, imap_uidvalidity)| {
            let object = message_object_id(
                provider,
                id.clone(),
                imap_folder,
                imap_uid,
                imap_uidvalidity,
            )?;
            Ok(FolderMessage {
                id,
                thread_id,
                object,
            })
        })
        .collect()
}

fn folder_namespace(account: &ResidentActionAccount, folder_id: &str) -> NamespaceAttribution {
    account
        .containers
        .native_for_storage_id(folder_id)
        .and_then(|native| {
            account
                .containers
                .attribution_for_scope(&CursorScope::Folder(BifrostFolderId(native.to_string())))
        })
        .unwrap_or(NamespaceAttribution::Personal)
}

/// Mark every unread message of the folder read on the provider, then
/// locally. Returns how many messages changed.
pub(crate) async fn mark_folder_read(
    ctx: &ActionContext,
    account: &ResidentActionAccount,
    account_id: &str,
    folder_id: &str,
) -> Result<usize, ActionError> {
    let messages = folder_messages(ctx, account.provider, account_id, folder_id, true).await?;
    if messages.is_empty() {
        return Ok(0);
    }
    let ids = messages.iter().map(|m| m.object.clone()).collect();
    dispatch_bulk_mutation(
        account,
        account_id,
        &RemoteBatchKey::Read { to: true },
        ids,
        &folder_namespace(account, folder_id),
    )
    .await?;

    write_local(ctx, account_id, messages, mark_read_rows).await
}

/// Move every message of the folder to Trash on the provider, then file
/// them under Trash locally. Returns how many messages moved.
pub(crate) async fn empty_folder(
    ctx: &ActionContext,
    account: &ResidentActionAccount,
    account_id: &str,
    folder_id: &str,
) -> Result<usize, ActionError> {
    let messages = folder_messages(ctx, account.provider, account_id, folder_id, false).await?;
    if messages.is_empty() {
        return Ok(0);
    }
    let ids = messages.iter().map(|m| m.object.clone()).collect();
    dispatch_bulk_mutation(
        account,
        account_id,
        &RemoteBatchKey::Trash,
        ids,
        &folder_namespace(account, folder_id),
    )
    .await?;

    write_local(ctx, account_id, messages, file_in_trash_rows).await
}

/// Run one local write over the messages in a single transaction; returns
/// how many messages it covered.
async fn write_local(
    ctx: &ActionContext,
    account_id: &str,
    messages: Vec<FolderMessage>,
    write: fn(&WriteTxn<'_>, &str, &[FolderMessage]) -> Result<(), String>,
) -> Result<usize, ActionError> {
    let count = messages.len();
    let aid = account_id.to_string();
    ctx.write_db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|e| format!("begin folder contents write: {e}"))?;
            write(&tx, &aid, &messages)?;
            tx.commit()
                .map_err(|e| format!("commit folder contents write: {e}"))
        })
        .await
        .map_err(ActionError::db)?;
    Ok(count)
}

fn thread_ids(messages: &[FolderMessage]) -> BTreeSet<&str> {
    messages.iter().map(|m| m.thread_id.as_str()).collect()
}

/// A thread reads as read once none of its messages is unread.
fn mark_read_rows(tx: &WriteTxn<'_>, account_id: &str, messages: &[FolderMessage]) -> Result<(), String> {
    for message in messages {
        tx.execute(
            "UPDATE messages SET is_read = 1 WHERE account_id = ?1 AND id = ?2",
            params![account_id, message.id],
        )
        .map_err(|e| format!("mark message read: {e}"))?;
    }
    for thread_id in thread_ids(messages) {
        tx.execute(
            "UPDATE threads SET is_read = NOT EXISTS ( \
                 SELECT 1 FROM messages \
                 WHERE account_id = ?1 AND thread_id = ?2 AND COALESCE(is_read, 0) = 0) \
             WHERE account_id = ?1 AND id = ?2",
            params![account_id, thread_id],
        )
        .map_err(|e| format!("recompute thread read state: {e}"))?;
    }
    Ok(())
}

fn file_in_trash_rows(
    tx: &WriteTxn<'_>,
    account_id: &str,
    messages: &[FolderMessage],
) -> Result<(), String> {
    let trash = [FolderKind::System(SystemFolderId::Trash)];
    for message in messages {
        replace_message_folder_rows(tx, account_id, &message.id, &trash)?;
    }
    for thread_id in thread_ids(messages) {
        recompute_thread_folders_from_messages(tx, account_id, thread_id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::*;
    use crate::actions::tests::make_test_ctx;

    const ACCOUNT: &str = "acc";
    const FOLDER: &str = "graph-resa";

    /// One conversation: an unread and a read message filed in the user
    /// folder, plus our reply filed in Sent.
    fn seed(ctx: &ActionContext) {
        ctx.write_db
            .with_write_sync(|conn| {
                conn.execute_batch(
                    "INSERT INTO accounts (id, email, provider, is_active) \
                         VALUES ('acc', 'acc@test.com', 'graph', 1); \
                     INSERT INTO folders (account_id, id, name) VALUES \
                         ('acc', 'graph-resa', 'Réservations'), ('acc', 'SENT', 'Sent'), \
                         ('acc', 'TRASH', 'Trash'); \
                     INSERT INTO threads (id, account_id, subject, snippet, last_message_at, is_read, is_starred) \
                         VALUES ('t1', 'acc', 'Résa', '', 0, 0, 0); \
                     INSERT INTO messages (id, account_id, thread_id, date, is_read) VALUES \
                         ('m-unread', 'acc', 't1', 1, 0), ('m-read', 'acc', 't1', 2, 1), \
                         ('m-sent', 'acc', 't1', 3, 0); \
                     INSERT INTO message_folders (account_id, message_id, folder_id) VALUES \
                         ('acc', 'm-unread', 'graph-resa'), ('acc', 'm-read', 'graph-resa'), \
                         ('acc', 'm-sent', 'SENT'); \
                     INSERT INTO thread_folders (account_id, thread_id, folder_id) VALUES \
                         ('acc', 't1', 'graph-resa'), ('acc', 't1', 'SENT');",
                )
                .map_err(|e| e.to_string())
            })
            .expect("seed");
    }

    fn ids(messages: &[FolderMessage]) -> Vec<&str> {
        let mut ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        ids
    }

    #[tokio::test]
    async fn selection_stays_inside_the_folder() {
        let (ctx, _tmp) = make_test_ctx();
        seed(&ctx);
        let all = folder_messages(&ctx, BifrostProviderKind::Graph, ACCOUNT, FOLDER, false)
            .await
            .expect("all");
        assert_eq!(ids(&all), ["m-read", "m-unread"]);
        let unread = folder_messages(&ctx, BifrostProviderKind::Graph, ACCOUNT, FOLDER, true)
            .await
            .expect("unread");
        assert_eq!(ids(&unread), ["m-unread"]);
    }

    #[tokio::test]
    async fn mark_read_leaves_other_folders_unread() {
        let (ctx, _tmp) = make_test_ctx();
        seed(&ctx);
        let unread = folder_messages(&ctx, BifrostProviderKind::Graph, ACCOUNT, FOLDER, true)
            .await
            .expect("unread");
        assert_eq!(write_local(&ctx, ACCOUNT, unread, mark_read_rows).await.expect("write"), 1);
        let (sent_read, thread_read): (bool, bool) = ctx
            .write_db
            .with_write_sync(|conn| {
                conn.query_row(
                    "SELECT (SELECT is_read FROM messages WHERE id = 'm-sent'), \
                            (SELECT is_read FROM threads WHERE id = 't1')",
                    params![],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|e| e.to_string())
            })
            .expect("read state");
        assert!(!sent_read, "the reply in Sent is not part of the folder");
        assert!(!thread_read, "the conversation still has an unread message");
    }

    #[tokio::test]
    async fn empty_moves_only_the_folder_messages_to_trash() {
        let (ctx, _tmp) = make_test_ctx();
        seed(&ctx);
        let all = folder_messages(&ctx, BifrostProviderKind::Graph, ACCOUNT, FOLDER, false)
            .await
            .expect("all");
        write_local(&ctx, ACCOUNT, all, file_in_trash_rows)
            .await
            .expect("write");
        let folders: Vec<String> = ctx
            .write_db
            .with_write_sync(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT folder_id FROM thread_folders WHERE thread_id = 't1' \
                         ORDER BY folder_id",
                    )
                    .map_err(|e| e.to_string())?;
                stmt.query_map(params![], |row| row.get(0))
                    .map_err(|e| e.to_string())?
                    .collect::<Result<_, _>>()
                    .map_err(|e| e.to_string())
            })
            .expect("thread folders");
        assert_eq!(folders, ["SENT", "TRASH"]);
    }
}
