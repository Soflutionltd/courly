use super::context::ActionContext;
use super::outcome::ActionError;
use db::db::queries_extra::{insert_folder, move_thread_messages, remove_inbox_folder};
use rusqlite::OptionalExtension;

/// The folder an archived conversation is filed in, as the providers file
/// it: Graph, IMAP and JMAP move it to their archive mailbox, and Gmail's
/// sync synthesizes the same `archive` folder for mail outside every system
/// label.
const ARCHIVE: &str = "archive";

/// Local DB mutation for archive: the conversation leaves the Inbox and,
/// when the account has an Archive folder, is filed there at once. That
/// keeps it out of the all-accounts Inbox, which lists every thread not in
/// Archive, Trash, Sent, Drafts or Spam, before the next sync and while
/// offline. An account without an Archive folder (an IMAP server with no
/// archive mailbox) only loses the Inbox membership.
///
/// Returns true if state changed. A conversation that was not in the Inbox
/// is left where it is.
pub(crate) async fn archive_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> Result<bool, ActionError> {
    ctx.verify_thread_exists(account_id, thread_id)?;
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write(move |conn| {
        let has_archive = conn
            .query_row(
                "SELECT 1 FROM folders WHERE account_id = ?1 AND id = ?2",
                rusqlite::params![aid, ARCHIVE],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| format!("look up the archive folder: {e}"))?
            .is_some();
        if !has_archive {
            move_thread_messages(conn, &aid, &tid, "INBOX", None)?;
            return remove_inbox_folder(conn, &aid, &tid).map(|n| n > 0);
        }
        if remove_inbox_folder(conn, &aid, &tid)? == 0 {
            return Ok(false);
        }
        move_thread_messages(conn, &aid, &tid, "INBOX", Some(ARCHIVE))?;
        insert_folder(conn, &aid, &tid, ARCHIVE).map(|_| true)
    })
    .await
    .map_err(ActionError::db)
}
