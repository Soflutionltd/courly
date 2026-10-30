use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use crate::bifrost::BifrostProviderKind;
use crate::bifrost::resident::ResidentActionAccount;
use db::db::queries::{set_thread_messages_pinned, set_thread_pinned};

/// Set pin state on a single thread. The local write is the whole outcome;
/// Outlook's own pin follows through [`push_pin_to_provider`].
pub async fn pin(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    pinned: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("pin", account_id, thread_id);

    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let local_result = db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|e| format!("begin pin transaction: {e}"))?;
            set_thread_pinned(&tx, &aid, &tid, pinned)?;
            set_thread_messages_pinned(&tx, &aid, &tid, pinned)?;
            tx.commit()
                .map_err(|e| format!("commit pin transaction: {e}"))
        })
        .await
        .map_err(ActionError::db);

    let outcome = match local_result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => ActionOutcome::Failed { error: e },
    };
    mlog.emit(&outcome);
    outcome
}

/// Mirror a local pin onto the provider when it has one (Outlook only).
///
/// Runs in the background and never changes the action outcome: the pin is
/// already visible locally, and if the push fails the next sync of the thread
/// brings back whatever the server holds instead of leaving the two apart.
pub(crate) async fn push_pin_to_provider(
    ctx: &ActionContext,
    action_account: &ResidentActionAccount,
    account_id: &str,
    thread_id: &str,
    pinned: bool,
) {
    if action_account.provider != BifrostProviderKind::Graph {
        return;
    }
    let ids = match super::dispatch_target::resolve_thread_messages(
        ctx,
        account_id,
        thread_id,
        action_account.provider,
    )
    .await
    {
        Ok(ids) => ids,
        Err(error) => {
            log::warn!("pin push skipped for thread {thread_id}: {error}");
            return;
        }
    };
    let action_account = action_account.clone();
    let account_id = account_id.to_string();
    let thread_id = thread_id.to_string();
    tokio::spawn(async move {
        if let Err(error) =
            super::dispatch_target::dispatch_pin(&action_account, &account_id, ids, pinned).await
        {
            log::warn!("Outlook pin push failed for thread {thread_id}: {error}");
        }
    });
}
