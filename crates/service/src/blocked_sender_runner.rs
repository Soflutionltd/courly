//! Blocked sender sweep.
//!
//! On each action-worker wakeup the sweep looks for inbox mail that arrived
//! from a blocked sender after the block, and sends each such thread to Spam
//! through the standard `SetSpam` action, so the provider moves it too. It
//! shares the wakeup with the snooze runner: any action, a `pending_ops.kick`
//! or the UI's 60 s tick.
//!
//! A message is recorded once the move succeeds and is never picked again,
//! so a thread the user moves back to the inbox by hand stays there. A failed
//! move is left unrecorded and retried on the next wakeup.

use super::actions::{ActionContext, ActionOutcome, MailOperation, batch_execute};
use crate::sync::SyncRuntime;

pub(crate) async fn drain_blocked_senders(ctx: &ActionContext, sync_runtime: Option<&SyncRuntime>) {
    if let Some(runtime) = sync_runtime {
        crate::handlers::sender::create_missing_junk_rules(runtime, &ctx.write_db).await;
    }
    let found = match ctx
        .write_db
        .with_write(db::db::queries_extra::blocked_inbox_threads)
        .await
    {
        Ok(found) => found,
        Err(error) => {
            log::warn!("blocked sender sweep: query failed: {error}");
            return;
        }
    };
    if found.is_empty() {
        return;
    }

    let operations = found
        .iter()
        .map(|thread| {
            (
                thread.account_id.clone(),
                thread.thread_id.clone(),
                MailOperation::SetSpam { to: true },
            )
        })
        .collect();
    let outcomes = batch_execute(ctx, sync_runtime, operations).await;

    let mut moved = 0usize;
    for (thread, outcome) in found.into_iter().zip(outcomes) {
        if let ActionOutcome::Failed { error } = &outcome {
            log::warn!(
                "blocked sender sweep: spam {}/{} failed: {}",
                thread.account_id,
                thread.thread_id,
                error.user_message(),
            );
            continue;
        }
        let account_id = thread.account_id;
        let message_ids = thread.message_ids;
        match ctx
            .write_db
            .with_write(move |conn| {
                db::db::queries_extra::record_blocked_moves(conn, &account_id, &message_ids)
            })
            .await
        {
            Ok(()) => moved += 1,
            Err(error) => log::warn!("blocked sender sweep: record failed: {error}"),
        }
    }
    if moved > 0 {
        log::info!("blocked sender sweep: sent {moved} thread(s) to Spam");
    }
}
