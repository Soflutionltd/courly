//! Folders a conversation was just moved out of by the user.
//!
//! A move is written locally and accepted by the provider, yet a sync pass
//! can still file the conversation back in the folder it left: the provider
//! hands out a state read before the move settled, and nothing corrects it
//! until the provider reports the same change again, a minute or so later.
//! The deleted mail came back in the list, then left on its own.
//!
//! For a few minutes after a move, sync therefore does not put the
//! conversation back in the folder it left. Each time it tries, a line is
//! logged with the messages it filed there, which is what tells the cause.
//! A message newer than the move lifts nothing but is never hidden: a reply
//! that arrives in the folder shows at once. When the time is up the
//! conversation's folders are rebuilt from its messages, so a move the
//! provider really undid shows then.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use db::db::WriteTxn;
use db::db::queries_extra::recompute_thread_folders_from_messages;
use rusqlite::params;

/// How long sync is kept from undoing a move. The provider has been seen
/// repeating a move about 70 seconds after it was made.
const HOLD: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Left {
    account_id: String,
    thread_id: String,
    folder_id: String,
}

#[derive(Debug, Clone, Copy)]
struct Hold {
    /// When the move was made, in the unit of `messages.date`.
    moved_at_ms: i64,
    until: Instant,
    /// Sync filed the conversation back at least once and was overruled, so
    /// its folders no longer mirror its messages.
    overruled: bool,
}

static HOLDS: LazyLock<Mutex<HashMap<Left, Hold>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn holds() -> std::sync::MutexGuard<'static, HashMap<Left, Hold>> {
    HOLDS.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

/// The user moved a conversation: out of `left`, into `entered`. Sync may
/// file it in `entered` again, and not in `left` for a while.
pub(crate) fn record(account_id: &str, thread_id: &str, left: Option<&str>, entered: Option<&str>) {
    record_at(
        account_id,
        thread_id,
        left,
        entered,
        now_ms(),
        Instant::now() + HOLD,
    );
}

fn record_at(
    account_id: &str,
    thread_id: &str,
    left: Option<&str>,
    entered: Option<&str>,
    moved_at_ms: i64,
    until: Instant,
) {
    let key = |folder_id: &str| Left {
        account_id: account_id.to_owned(),
        thread_id: thread_id.to_owned(),
        folder_id: folder_id.to_owned(),
    };
    let mut holds = holds();
    if let Some(entered) = entered {
        holds.remove(&key(entered));
    }
    if let Some(left) = left {
        holds.insert(
            key(left),
            Hold {
                moved_at_ms,
                until,
                overruled: false,
            },
        );
    }
}

/// The move was taken back locally (the provider refused it): the
/// conversation is where it was, nothing is held.
pub(crate) fn forget(account_id: &str, thread_id: &str) {
    holds().retain(|left, _| !(left.account_id == account_id && left.thread_id == thread_id));
}

/// Whether [`enforce`] has anything to do for this account.
pub(crate) fn concerns(account_id: &str) -> bool {
    holds().keys().any(|left| left.account_id == account_id)
}

/// Run after sync wrote `thread_ids`: take each of them back out of a folder
/// it was just moved out of, and rebuild the folders of the conversations
/// whose hold is over. Returns whether a row changed.
pub(crate) fn enforce(
    tx: &WriteTxn<'_>,
    account_id: &str,
    thread_ids: &[String],
) -> Result<bool, String> {
    enforce_at(tx, account_id, thread_ids, Instant::now())
}

fn enforce_at(
    tx: &WriteTxn<'_>,
    account_id: &str,
    thread_ids: &[String],
    now: Instant,
) -> Result<bool, String> {
    type Held = Vec<(Left, Hold)>;
    let (over, held): (Held, Held) = {
        let mut holds = holds();
        let over: Held = holds
            .iter()
            .filter(|(left, hold)| left.account_id == account_id && hold.until <= now)
            .map(|(left, hold)| (left.clone(), *hold))
            .collect();
        for (left, _) in &over {
            holds.remove(left);
        }
        let held = holds
            .iter()
            .filter(|(left, _)| {
                left.account_id == account_id && thread_ids.contains(&left.thread_id)
            })
            .map(|(left, hold)| (left.clone(), *hold))
            .collect();
        (over, held)
    };

    let mut changed = false;
    for (left, hold) in over {
        if hold.overruled {
            recompute_thread_folders_from_messages(tx, &left.account_id, &left.thread_id)?;
            changed = true;
        }
    }
    for (left, hold) in held {
        if take_back_out(tx, &left, hold)? {
            changed = true;
            if let Some(hold) = holds().get_mut(&left) {
                hold.overruled = true;
            }
        }
    }
    Ok(changed)
}

/// Remove the conversation from the folder it left, unless a message newer
/// than the move is filed there. Returns whether it had been filed back.
fn take_back_out(tx: &WriteTxn<'_>, left: &Left, hold: Hold) -> Result<bool, String> {
    let removed = tx
        .execute(
            "DELETE FROM thread_folders \
             WHERE account_id = ?1 AND thread_id = ?2 AND folder_id = ?3 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM messages m \
                   JOIN message_folders mf \
                     ON mf.account_id = m.account_id AND mf.message_id = m.id \
                   WHERE m.account_id = ?1 AND m.thread_id = ?2 \
                     AND mf.folder_id = ?3 AND m.date > ?4)",
            params![
                left.account_id,
                left.thread_id,
                left.folder_id,
                hold.moved_at_ms
            ],
        )
        .map_err(|error| format!("keep a moved conversation out of its old folder: {error}"))?;
    if removed == 0 {
        return Ok(false);
    }

    let mut statement = tx
        .prepare(
            "SELECT m.id FROM messages m \
             JOIN message_folders mf ON mf.account_id = m.account_id AND mf.message_id = m.id \
             WHERE m.account_id = ?1 AND m.thread_id = ?2 AND mf.folder_id = ?3 \
             ORDER BY m.date",
        )
        .map_err(|error| format!("list the messages filed back: {error}"))?;
    let filed_back = statement
        .query_map(
            params![left.account_id, left.thread_id, left.folder_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("list the messages filed back: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("list the messages filed back: {error}"))?;
    log::warn!(
        "sync filed conversation {} of {} back in {}, {}s after it was moved out; kept out. \
         Messages filed there: {}",
        left.thread_id,
        left.account_id,
        left.folder_id,
        now_ms().saturating_sub(hold.moved_at_ms) / 1000,
        filed_back.join(", ")
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::ActionContext;
    use crate::actions::tests::make_test_ctx;

    /// A conversation moved to the trash, which sync then files back in the
    /// inbox under a new message id, as Outlook does after a move.
    fn seed(ctx: &ActionContext, account: &str) {
        ctx.write_db
            .with_write_sync(|conn| {
                conn.execute_batch(&format!(
                    "INSERT INTO accounts (id, email, provider, is_active) \
                         VALUES ('{account}', '{account}@test.com', 'graph', 1); \
                     INSERT INTO folders (account_id, id, name) VALUES \
                         ('{account}', 'INBOX', 'Inbox'), ('{account}', 'TRASH', 'Trash'); \
                     INSERT INTO threads (id, account_id, subject, snippet, last_message_at, is_read, is_starred) \
                         VALUES ('t1', '{account}', 'Hello', '', 0, 0, 0); \
                     INSERT INTO messages (id, account_id, thread_id, date, is_read) VALUES \
                         ('m-moved', '{account}', 't1', 1000, 1); \
                     INSERT INTO message_folders (account_id, message_id, folder_id) VALUES \
                         ('{account}', 'm-moved', 'INBOX'); \
                     INSERT INTO thread_folders (account_id, thread_id, folder_id) VALUES \
                         ('{account}', 't1', 'INBOX');"
                ))
                .map_err(|e| e.to_string())
            })
            .expect("seed");
    }

    fn run(ctx: &ActionContext, account: &str, now: Instant) -> bool {
        let account = account.to_owned();
        ctx.write_db
            .with_write_sync(move |conn| {
                let tx = conn.transaction().map_err(|e| e.to_string())?;
                let changed = enforce_at(&tx, &account, &["t1".to_owned()], now)?;
                tx.commit().map_err(|e| e.to_string())?;
                Ok(changed)
            })
            .expect("enforce")
    }

    fn in_inbox(ctx: &ActionContext, account: &str) -> bool {
        let account = account.to_owned();
        ctx.write_db
            .with_write_sync(move |conn| {
                conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM thread_folders \
                     WHERE account_id = ?1 AND thread_id = 't1' AND folder_id = 'INBOX')",
                    params![account],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())
            })
            .expect("thread folders")
    }

    #[test]
    fn sync_does_not_put_a_moved_conversation_back_and_the_truth_shows_when_the_hold_ends() {
        let (ctx, _tmp) = make_test_ctx();
        let account = "held";
        seed(&ctx, account);
        let start = Instant::now();
        record_at(
            account,
            "t1",
            Some("INBOX"),
            Some("TRASH"),
            5000,
            start + HOLD,
        );

        assert!(run(&ctx, account, start));
        assert!(!in_inbox(&ctx, account), "kept out while held");
        assert!(!run(&ctx, account, start), "nothing left to take out");

        assert!(run(&ctx, account, start + HOLD));
        assert!(
            in_inbox(&ctx, account),
            "its messages still say inbox: shown again"
        );
        assert!(!concerns(account));
    }

    #[test]
    fn a_message_newer_than_the_move_keeps_the_conversation_in_the_folder() {
        let (ctx, _tmp) = make_test_ctx();
        let account = "reply";
        seed(&ctx, account);
        let start = Instant::now();
        record_at(
            account,
            "t1",
            Some("INBOX"),
            Some("TRASH"),
            500,
            start + HOLD,
        );

        assert!(
            !run(&ctx, account, start),
            "the message is newer than the move"
        );
        assert!(in_inbox(&ctx, account));
        forget(account, "t1");
    }

    #[test]
    fn moving_the_conversation_back_lifts_the_hold() {
        let (ctx, _tmp) = make_test_ctx();
        let account = "undone";
        seed(&ctx, account);
        let start = Instant::now();
        record_at(
            account,
            "t1",
            Some("INBOX"),
            Some("TRASH"),
            5000,
            start + HOLD,
        );
        record_at(
            account,
            "t1",
            Some("TRASH"),
            Some("INBOX"),
            6000,
            start + HOLD,
        );

        assert!(!run(&ctx, account, start));
        assert!(in_inbox(&ctx, account));
        forget(account, "t1");
        assert!(!concerns(account));
    }
}
