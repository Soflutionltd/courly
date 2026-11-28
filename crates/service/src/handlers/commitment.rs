//! Commitment handlers: `commitment.save` and `commitment.delete`.

use std::sync::Arc;

use serde_json::Value;
use service_api::{CommitmentAck, CommitmentDeleteParams, CommitmentSaveParams, ServiceError};

use crate::boot::BootSharedState;

pub(crate) async fn handle_save(
    boot_state: &Arc<BootSharedState>,
    params: CommitmentSaveParams,
) -> Result<Value, ServiceError> {
    let save = db::db::queries_extra::CommitmentSave {
        id: params.id,
        account_id: params.account_id,
        direction: params.direction,
        text: params.text,
        person_name: params.person_name,
        person_address: params.person_address,
        due_date: params.due_date,
        thread_id: params.thread_id,
        message_id: params.message_id,
        done: params.done,
    };
    boot_state
        .write_db_state()?
        .with_write(move |conn| db::db::queries_extra::save_commitment(conn, &save))
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(CommitmentAck).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_delete(
    boot_state: &Arc<BootSharedState>,
    params: CommitmentDeleteParams,
) -> Result<Value, ServiceError> {
    boot_state
        .write_db_state()?
        .with_write(move |conn| db::db::queries_extra::delete_commitment(conn, &params.id))
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(CommitmentAck).map_err(|e| ServiceError::Internal(e.to_string()))
}
