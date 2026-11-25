//! Blocked sender handlers: `sender.block` and `sender.unblock`, each one
//! statement through `WriteDbState::with_write`. Moving mail to Spam is the
//! caller's job for the open thread and the sync hook's job for new mail.

use std::sync::Arc;

use serde_json::Value;
use service_api::{SenderBlockAck, SenderBlockParams, SenderUnblockAck, ServiceError};

use crate::boot::BootSharedState;

pub(crate) async fn handle_block(
    boot_state: &Arc<BootSharedState>,
    params: SenderBlockParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let newly_blocked = write_db
        .with_write(move |conn| {
            db::db::queries_extra::block_sender(conn, &params.account_id, &params.sender_address)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(SenderBlockAck { newly_blocked })
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_unblock(
    boot_state: &Arc<BootSharedState>,
    params: SenderBlockParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let removed = write_db
        .with_write(move |conn| {
            db::db::queries_extra::unblock_sender(conn, &params.account_id, &params.sender_address)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(SenderUnblockAck {
        was_blocked: removed.is_some(),
    })
    .map_err(|e| ServiceError::Internal(e.to_string()))
}
