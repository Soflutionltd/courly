//! Blocked sender handlers: `sender.block` and `sender.unblock`.
//!
//! The blocked list lives in `blocked_senders`. On a Microsoft (Graph)
//! account the block is also enforced server-side by an inbox rule that moves
//! the sender's mail to Junk Email, so it applies on every device even while
//! Courly is closed; the rule id is stored with the block so unblocking can
//! delete it. Other providers rely on the blocked sender sweep alone: Gmail
//! filters cannot target Spam, and IMAP and JMAP have no rule Courly sets.
//!
//! Moving mail is not done here: the open thread goes to Spam through the
//! regular action pipeline and later mail through the sweep.

use std::sync::Arc;

use common::types::NamespaceAttribution;
use conduit_types::{
    AccountErrorKind, ContainerId, FilterAction, FilterCondition, FilterRuleCreate, FolderRole,
    ServerFilterCreate, ServerFilterId,
};
use serde_json::Value;
use service_api::{SenderBlockAck, SenderBlockParams, SenderUnblockAck, ServiceError};

use crate::boot::BootSharedState;
use crate::conduit::ConduitProviderKind;
use crate::conduit::resident::ResidentActionAccount;
use crate::conduit::server_filters::{ServerFilterError, ServerFilterSurface};

pub(crate) async fn handle_block(
    boot_state: &Arc<BootSharedState>,
    params: SenderBlockParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let account_id = params.account_id.clone();
    let address = params.sender_address.clone();
    let newly_blocked = write_db
        .with_write(move |conn| db::db::queries_extra::block_sender(conn, &account_id, &address))
        .await
        .map_err(ServiceError::Internal)?;

    if newly_blocked
        && let Some((account, filters)) = graph_account(boot_state, &params.account_id).await
    {
        // The local block already holds: if the rule cannot be created the
        // sweep still sends this sender's mail to Spam while Courly runs.
        match create_junk_rule(
            &account,
            &filters,
            &params.account_id,
            &params.sender_address,
        )
        .await
        {
            Ok(rule_id) => {
                let account_id = params.account_id.clone();
                let address = params.sender_address.clone();
                write_db
                    .with_write(move |conn| {
                        db::db::queries_extra::set_blocked_sender_rule_id(
                            conn,
                            &account_id,
                            &address,
                            Some(&rule_id.0),
                        )
                    })
                    .await
                    .map_err(ServiceError::Internal)?;
            }
            Err(error) => log::warn!("sender.block: junk rule not created: {error}"),
        }
    }

    serde_json::to_value(SenderBlockAck { newly_blocked })
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_unblock(
    boot_state: &Arc<BootSharedState>,
    params: SenderBlockParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let account_id = params.account_id.clone();
    let rule_id = write_db
        .with_write(move |conn| db::db::queries_extra::list_blocked_senders(conn, &account_id))
        .await
        .map_err(ServiceError::Internal)?
        .into_iter()
        .find(|blocked| {
            blocked.sender_address
                == db::db::queries_extra::normalize_sender_address(&params.sender_address)
        })
        .and_then(|blocked| blocked.provider_rule_id);

    // The server rule goes first: if it stayed behind, the sender's mail
    // would keep landing in Junk after the user unblocked them. A failure
    // leaves the block in place so the user can retry.
    if let Some(rule_id) = rule_id {
        let (_, filters) = graph_account(boot_state, &params.account_id)
            .await
            .ok_or_else(|| {
                ServiceError::Internal(
                    "the account is not connected; its junk rule cannot be removed yet".into(),
                )
            })?;
        delete_junk_rule(&filters, &params.account_id, rule_id).await?;
    }

    let account_id = params.account_id.clone();
    let removed = write_db
        .with_write(move |conn| {
            db::db::queries_extra::unblock_sender(conn, &account_id, &params.sender_address)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(SenderUnblockAck {
        was_blocked: removed.is_some(),
    })
    .map_err(|e| ServiceError::Internal(e.to_string()))
}

/// The account's resident handle, with the server filter surface to change
/// its rules, when it is a Microsoft (Graph) account the engine can reach
/// now; `None` for other providers or an offline account.
async fn graph_account(
    boot_state: &Arc<BootSharedState>,
    account_id: &str,
) -> Option<(ResidentActionAccount, ServerFilterSurface)> {
    let runtime = boot_state.sync_runtime()?;
    match runtime.resident_action_account(account_id).await {
        Ok(account) if account.provider == ConduitProviderKind::Graph => {
            Some((account, runtime.server_filters()))
        }
        Ok(_) => None,
        Err(error) => {
            log::warn!("sender rule: account {account_id} not reachable: {error}");
            None
        }
    }
}

async fn create_junk_rule(
    account: &ResidentActionAccount,
    filters: &ServerFilterSurface,
    account_id: &str,
    sender_address: &str,
) -> Result<ServerFilterId, String> {
    let junk = account
        .containers
        .role_target(&NamespaceAttribution::Personal, FolderRole::Spam)
        .ok_or_else(|| "the account has no Junk Email folder".to_string())?
        .to_string();
    let rule = FilterRuleCreate {
        name: Some(format!(
            "Courly: {}",
            db::db::queries_extra::normalize_sender_address(sender_address)
        )),
        is_enabled: true,
        condition: FilterCondition::From(sender_address.trim().to_string()),
        actions: vec![FilterAction::MoveTo(ContainerId(junk))],
        stop_processing: true,
    };
    filters
        .create(account_id, ServerFilterCreate::Rule(rule))
        .await
        .map_err(|error| error.to_string())
}

async fn delete_junk_rule(
    filters: &ServerFilterSurface,
    account_id: &str,
    rule_id: String,
) -> Result<(), ServiceError> {
    match filters.delete(account_id, ServerFilterId(rule_id)).await {
        Ok(()) => Ok(()),
        // Removed by hand in Outlook already: nothing left to undo.
        Err(ServerFilterError::Engine(conduit_sync::Error::Account(error)))
            if matches!(error.kind(), AccountErrorKind::NotFound(_)) =>
        {
            Ok(())
        }
        Err(error) => Err(ServiceError::Internal(format!(
            "junk rule not removed: {error}"
        ))),
    }
}
