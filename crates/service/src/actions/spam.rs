use super::context::ActionContext;
use super::outcome::ActionError;
use db::db::queries_extra::{insert_folder, move_thread_messages, remove_folder};

/// Local DB mutation for spam (idempotent).
pub(crate) async fn spam_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    is_spam: bool,
) -> Result<(), ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write(move |conn| {
        if is_spam {
            move_thread_messages(conn, &aid, &tid, "INBOX", Some("SPAM"))?;
            remove_folder(conn, &aid, &tid, "INBOX")?;
            insert_folder(conn, &aid, &tid, "SPAM").map(|_| ())
        } else {
            move_thread_messages(conn, &aid, &tid, "SPAM", Some("INBOX"))?;
            remove_folder(conn, &aid, &tid, "SPAM")?;
            insert_folder(conn, &aid, &tid, "INBOX").map(|_| ())
        }
    })
    .await
    .map_err(ActionError::db)
}
