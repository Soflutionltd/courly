//! Local state of a thread captured before an action's local half, put
//! back when the provider refuses the action for good. Without it a
//! rejected move or flag would stay applied locally only, and the app
//! would show a mailbox the server does not have.

use rusqlite::{OptionalExtension as _, params};

use super::context::ActionContext;
use super::operation::MailOperation;
use super::outcome::{ActionError, ActionOutcome};

pub(crate) struct LocalSnapshot {
    account_id: String,
    thread_id: String,
    folders: Vec<String>,
    thread_flags: Option<(bool, bool)>,
    message_flags: Vec<(String, bool, bool)>,
}

/// Whether the local half of `op` only touches what a snapshot restores:
/// folder membership and the read / starred flags.
fn restorable(op: &MailOperation) -> bool {
    matches!(
        op,
        MailOperation::Archive
            | MailOperation::Trash
            | MailOperation::SetSpam { .. }
            | MailOperation::MoveToFolder { .. }
            | MailOperation::SetStarred { .. }
            | MailOperation::SetRead { .. }
    )
}

pub(crate) async fn capture(
    ctx: &ActionContext,
    op: &MailOperation,
    account_id: &str,
    thread_id: &str,
) -> Result<Option<LocalSnapshot>, ActionError> {
    if !restorable(op) {
        return Ok(None);
    }
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    ctx.write_db
        .clone()
        .with_write(move |conn| {
            let folders = conn
                .prepare(
                    "SELECT folder_id FROM thread_folders WHERE account_id = ?1 AND thread_id = ?2",
                )
                .and_then(|mut stmt| {
                    stmt.query_map(params![aid, tid], |row| row.get(0))?
                        .collect::<Result<Vec<String>, _>>()
                })
                .map_err(|e| format!("snapshot thread folders: {e}"))?;
            let thread_flags = conn
                .query_row(
                    "SELECT is_read, is_starred FROM threads WHERE account_id = ?1 AND id = ?2",
                    params![aid, tid],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|e| format!("snapshot thread flags: {e}"))?;
            let message_flags = conn
                .prepare(
                    "SELECT id, is_read, is_starred FROM messages \
                     WHERE account_id = ?1 AND thread_id = ?2",
                )
                .and_then(|mut stmt| {
                    stmt.query_map(params![aid, tid], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?
                    .collect::<Result<Vec<(String, bool, bool)>, _>>()
                })
                .map_err(|e| format!("snapshot message flags: {e}"))?;
            Ok(Some(LocalSnapshot {
                account_id: aid,
                thread_id: tid,
                folders,
                thread_flags,
                message_flags,
            }))
        })
        .await
        .map_err(ActionError::db)
}

impl LocalSnapshot {
    pub(crate) async fn restore(self, ctx: &ActionContext) -> Result<(), ActionError> {
        ctx.write_db
            .clone()
            .with_write(move |conn| {
                let tx = conn
                    .transaction()
                    .map_err(|e| format!("begin snapshot restore: {e}"))?;
                tx.execute(
                    "DELETE FROM thread_folders WHERE account_id = ?1 AND thread_id = ?2",
                    params![self.account_id, self.thread_id],
                )
                .map_err(|e| format!("clear thread folders: {e}"))?;
                for folder_id in &self.folders {
                    tx.execute(
                        "INSERT INTO thread_folders (thread_id, account_id, folder_id) \
                         VALUES (?1, ?2, ?3)",
                        params![self.thread_id, self.account_id, folder_id],
                    )
                    .map_err(|e| format!("restore thread folder: {e}"))?;
                }
                if let Some((is_read, is_starred)) = self.thread_flags {
                    tx.execute(
                        "UPDATE threads SET is_read = ?1, is_starred = ?2 \
                         WHERE account_id = ?3 AND id = ?4",
                        params![is_read, is_starred, self.account_id, self.thread_id],
                    )
                    .map_err(|e| format!("restore thread flags: {e}"))?;
                }
                for (message_id, is_read, is_starred) in &self.message_flags {
                    tx.execute(
                        "UPDATE messages SET is_read = ?1, is_starred = ?2 \
                         WHERE account_id = ?3 AND id = ?4",
                        params![is_read, is_starred, self.account_id, message_id],
                    )
                    .map_err(|e| format!("restore message flags: {e}"))?;
                }
                tx.commit()
                    .map_err(|e| format!("commit snapshot restore: {e}"))?;
                Ok::<(), String>(())
            })
            .await
            .map_err(ActionError::db)
    }
}

/// The outcome of an action whose remote half failed after its local half
/// ran. A failure the retry queue will replay keeps the local change; a
/// permanent one puts the thread back and reports the action as failed.
pub(crate) async fn settle_remote_failure(
    ctx: &ActionContext,
    snapshot: Option<LocalSnapshot>,
    outcome: ActionOutcome,
) -> ActionOutcome {
    let ActionOutcome::LocalOnly {
        retryable: false,
        reason,
    } = outcome
    else {
        return outcome;
    };
    let Some(snapshot) = snapshot else {
        return ActionOutcome::LocalOnly {
            retryable: false,
            reason,
        };
    };
    if let Err(error) = snapshot.restore(ctx).await {
        log::warn!("could not undo a rejected action locally: {error}");
        return ActionOutcome::LocalOnly {
            retryable: false,
            reason,
        };
    }
    ActionOutcome::Failed { error: reason }
}
