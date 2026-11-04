//! `sync.start_account` + `sync.cancel_account` JSON-RPC handlers.
//!
//! Phase 3 task 9: forwards the wire request into the
//! `SyncRuntime` installed on `BootSharedState` by the boot task. The
//! `SyncRuntime` itself spawns the runner under a panic supervisor and
//! emits the terminal `sync.completed` notification; the handler's
//! responsibility is purely to translate the request into a runtime
//! call and serialize the ack.
//!
//! Both handlers return a `ServiceError::Internal` if the runtime is
//! not yet installed. In production the UI parks on `boot.ready`
//! before issuing any sync request, so this branch should not be
//! reachable; an Internal error is a debug aid for a UI bug or a
//! future test that races boot.

use crate::boot::BootSharedState;
use serde_json::Value;
use service_api::{
    ServiceError, SyncCancelAccountParams, SyncFocusFolderAck, SyncFocusFolderParams,
    SyncResumeAccountParams, SyncResumeAck, SyncResyncAccountParams, SyncResyncAck,
    SyncServerSearchAck, SyncServerSearchParams, SyncStartAccountParams,
};
use std::sync::Arc;

pub(crate) async fn handle_start_account(
    boot_state: &Arc<BootSharedState>,
    params: SyncStartAccountParams,
) -> Result<Value, ServiceError> {
    let runtime = boot_state.sync_runtime().ok_or_else(|| {
        ServiceError::Internal(
            "sync.start_account received before SyncRuntime was installed; \
             UI must wait for boot.ready"
                .into(),
        )
    })?;
    let ack = runtime.start_account(params.account_id).await;

    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_cancel_account(
    boot_state: &Arc<BootSharedState>,
    params: SyncCancelAccountParams,
) -> Result<Value, ServiceError> {
    let runtime = boot_state.sync_runtime().ok_or_else(|| {
        ServiceError::Internal(
            "sync.cancel_account received before SyncRuntime was installed; \
             UI must wait for boot.ready"
                .into(),
        )
    })?;
    let mut ack = runtime.cancel_account(&params.account_id).await;

    if let Err(error) = runtime.detach_resident_account(&params.account_id).await {
        log::debug!(
            "sync.cancel_account: resident detach for {} returned {error}",
            params.account_id
        );
    }

    // Phase 5 task 9: piggyback calendar cancel server-side, mirroring
    // the push pattern. Stamps `calendar_run_id` on the ack so the UI's
    // cancel_and_await path can subscribe to CalendarRunCompleted for
    // the cancelled run before issuing the DB DELETE. Calendar tables
    // CASCADE from `accounts` (db/src/db/schema/05_calendar.sql:5);
    // without this piggyback a calendar runner with an open
    // WriteDbState write borrow could race the DELETE FROM accounts.
    if let Some(calendar_runtime) = boot_state.calendar_runtime() {
        let cal_ack = calendar_runtime.cancel_account(&params.account_id).await;
        ack.calendar_run_id = cal_ack.run_id;
    }

    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_server_search(
    boot_state: &Arc<BootSharedState>,
    params: SyncServerSearchParams,
) -> Result<Value, ServiceError> {
    let address = params.address.trim();
    if !is_plain_address(address) {
        return Err(ServiceError::InvalidParams {
            method: "sync.server_search".into(),
            message: "address must be a single email address".into(),
        });
    }
    let runtime = boot_state.sync_runtime().ok_or_else(|| {
        ServiceError::Internal(
            "sync.server_search received before SyncRuntime was installed; \
             UI must wait for boot.ready"
                .into(),
        )
    })?;
    let fetched = runtime
        .fetch_participant_mail(params.account_id.as_deref(), address)
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(SyncServerSearchAck { fetched })
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

/// One `local@domain` token, which Graph's KQL takes as a single value.
fn is_plain_address(address: &str) -> bool {
    address.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty() && domain.contains('.') && !domain.contains('@')
    }) && !address
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '(' | ')' | ':'))
}

pub(crate) async fn handle_focus_folder(
    boot_state: &Arc<BootSharedState>,
    params: SyncFocusFolderParams,
) -> Result<Value, ServiceError> {
    let runtime = boot_state.sync_runtime().ok_or_else(|| {
        ServiceError::Internal(
            "sync.focus_folder received before SyncRuntime was installed; \
             UI must wait for boot.ready"
                .into(),
        )
    })?;
    let focused_accounts = runtime
        .focus_folder(params.account_id.as_deref(), params.folder_id.as_deref())
        .await;
    serde_json::to_value(SyncFocusFolderAck { focused_accounts })
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_resume_account(
    boot_state: &Arc<BootSharedState>,
    params: SyncResumeAccountParams,
) -> Result<Value, ServiceError> {
    let runtime = boot_state.sync_runtime().ok_or_else(|| {
        ServiceError::Internal(
            "sync.resume_account received before SyncRuntime was installed; \
             UI must wait for boot.ready"
                .into(),
        )
    })?;
    let resumed = runtime
        .resume_account(&params.account_id)
        .await
        .map_err(ServiceError::Internal)?;
    let ack = SyncResumeAck {
        account_id: params.account_id,
        resumed,
    };

    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_resync_account(
    boot_state: &Arc<BootSharedState>,
    params: SyncResyncAccountParams,
) -> Result<Value, ServiceError> {
    let runtime = boot_state.sync_runtime().ok_or_else(|| {
        ServiceError::Internal(
            "sync.resync_account received before SyncRuntime was installed; \
             UI must wait for boot.ready"
                .into(),
        )
    })?;
    runtime
        .resync_account(&params.account_id)
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(SyncResyncAck {
        account_id: params.account_id,
    })
    .map_err(|e| ServiceError::Internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::is_plain_address;

    #[test]
    fn only_a_single_address_reaches_the_server_search() {
        assert!(is_plain_address("person@example.org"));
        assert!(!is_plain_address("person"));
        assert!(!is_plain_address("@example.org"));
        assert!(!is_plain_address("a@b@example.org"));
        assert!(!is_plain_address("person@example.org OR subject:x"));
        assert!(!is_plain_address("person@example.org\""));
    }
}
