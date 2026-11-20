use super::context::ActionContext;
use super::outcome::ActionError;
use db::db::queries_extra::{move_thread_messages, remove_inbox_folder};

/// Local DB mutation for archive. Returns true if state changed.
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
        move_thread_messages(conn, &aid, &tid, "INBOX", None)?;
        remove_inbox_folder(conn, &aid, &tid).map(|n| n > 0)
    })
    .await
    .map_err(ActionError::db)
}
