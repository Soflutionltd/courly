//! Thread folder mutation helpers for the email action pipeline.
//!
//! Label-side equivalents have been retired: the action service writes
//! pending intent into `pending_thread_label_intents` and lets the
//! provider-truth path own `thread_labels` writes. See the doc comments
//! on `crates/db/src/db/queries_extra/label_intent.rs` for the overlay
//! lifecycle.

use crate::db::WriteTarget;
use rusqlite::params;

/// Remove a folder from a thread. Returns the number of rows affected.
pub fn remove_folder(
    conn: &impl WriteTarget,
    account_id: &str,
    thread_id: &str,
    folder_id: &str,
) -> Result<usize, String> {
    conn.execute(
        "DELETE FROM thread_folders WHERE account_id = ?1 AND thread_id = ?2 AND folder_id = ?3",
        params![account_id, thread_id, folder_id],
    )
    .map_err(|e| e.to_string())
}

/// Add a folder to a thread. Returns the number of rows affected.
pub fn insert_folder(
    conn: &impl WriteTarget,
    account_id: &str,
    thread_id: &str,
    folder_id: &str,
) -> Result<usize, String> {
    conn.execute(
        "INSERT OR IGNORE INTO thread_folders (account_id, thread_id, folder_id) VALUES (?1, ?2, ?3)",
        params![account_id, thread_id, folder_id],
    )
    .map_err(|e| e.to_string())
}

/// Remove the INBOX folder from a thread.
pub fn remove_inbox_folder(
    conn: &impl WriteTarget,
    account_id: &str,
    thread_id: &str,
) -> Result<usize, String> {
    remove_folder(conn, account_id, thread_id, "INBOX")
}

/// Carry a thread-level folder move over to the thread's messages: every
/// message filed in `from` is filed in `to` instead, or in no folder when
/// `to` is `None` (archive). Sync recomputes a thread's folders from its
/// messages, so a move written on the thread alone is undone by the next
/// sync pass that touches the thread, until the provider's own delta lands.
pub fn move_thread_messages(
    conn: &impl WriteTarget,
    account_id: &str,
    thread_id: &str,
    from: &str,
    to: Option<&str>,
) -> Result<(), String> {
    if let Some(to) = to {
        conn.execute(
            "INSERT OR IGNORE INTO message_folders (account_id, message_id, folder_id) \
             SELECT mf.account_id, mf.message_id, ?4 FROM message_folders mf \
             JOIN messages m ON m.account_id = mf.account_id AND m.id = mf.message_id \
             WHERE mf.account_id = ?1 AND m.thread_id = ?2 AND mf.folder_id = ?3",
            params![account_id, thread_id, from, to],
        )
        .map_err(|e| format!("file thread messages in destination folder: {e}"))?;
    }
    conn.execute(
        "DELETE FROM message_folders WHERE account_id = ?1 AND folder_id = ?3 \
           AND message_id IN (SELECT id FROM messages WHERE account_id = ?1 AND thread_id = ?2)",
        params![account_id, thread_id, from],
    )
    .map_err(|e| format!("take thread messages out of source folder: {e}"))?;
    Ok(())
}
