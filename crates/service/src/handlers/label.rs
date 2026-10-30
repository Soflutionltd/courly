//! Service handlers for label and label_group IPC.

use std::sync::Arc;

use common::typed_ids::LabelId;
use serde_json::Value;
use service_api::actions::ActionOutcome;
use service_api::{
    LabelDeleteAck, LabelDeleteParams, LabelGroupReorderAck, LabelGroupReorderParams,
    LabelSaveAck, LabelSaveParams, ServiceError,
};

use crate::actions::ActionContext;
use crate::bifrost::resident::ResidentActionAccount;
use crate::boot::BootSharedState;

pub(crate) async fn handle_reorder(
    boot_state: &Arc<BootSharedState>,
    params: LabelGroupReorderParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_write(move |conn| {
            db::db::queries_extra::label_groups::update_label_group_sort_order_sync(
                conn,
                &params.orders,
            )
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(LabelGroupReorderAck).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(super) async fn action_context_and_account(
    boot_state: &Arc<BootSharedState>,
    account_id: &str,
) -> Result<(ActionContext, ResidentActionAccount), ServiceError> {
    let sync_runtime = boot_state
        .sync_runtime()
        .ok_or_else(|| ServiceError::Internal("sync runtime is not running yet".into()))?;
    let ctx = crate::actions::worker::build_action_context(
        boot_state.write_db_state()?,
        boot_state
            .read_db_state()
            .ok_or_else(|| ServiceError::Internal("read DB is not initialized".into()))?,
        boot_state
            .encryption_key()
            .ok_or_else(|| ServiceError::Internal("encryption key is not loaded".into()))?,
        boot_state.app_data_dir(),
    )
    .map_err(ServiceError::Internal)?;
    let account = sync_runtime
        .resident_action_account(account_id)
        .await
        .map_err(|error| ServiceError::Internal(format!("resident action account: {error}")))?;
    Ok((ctx, account))
}

pub(super) fn outcome_to_result(outcome: ActionOutcome) -> Result<(), ServiceError> {
    match outcome {
        ActionOutcome::Success | ActionOutcome::NoOp => Ok(()),
        ActionOutcome::LocalOnly { reason, .. } => Err(ServiceError::Internal(reason.user_message())),
        ActionOutcome::Failed { error } => Err(ServiceError::Internal(error.user_message())),
    }
}

pub(crate) async fn handle_save(
    boot_state: &Arc<BootSharedState>,
    params: LabelSaveParams,
) -> Result<Value, ServiceError> {
    let name = params.name.trim();
    if name.is_empty() {
        return Err(ServiceError::InvalidParams {
            method: "label.save".into(),
            message: "label name is empty".into(),
        });
    }
    let (ctx, account) = action_context_and_account(boot_state, &params.account_id).await?;
    let style = params
        .color
        .as_ref()
        .map(|(bg, fg)| (bg.as_str(), fg.as_str()));
    let label_id = match params.label_id {
        None => {
            let (outcome, created) =
                crate::actions::create_label(&ctx, Some(&account), &params.account_id, name, style)
                    .await;
            outcome_to_result(outcome)?;
            created.ok_or_else(|| {
                ServiceError::Internal("label created without a storage id".into())
            })?
        }
        Some(label_id) => {
            let outcome = crate::actions::rename_label(
                &ctx,
                Some(&account),
                &params.account_id,
                &LabelId::from(label_id.clone()),
                name,
                style,
            )
            .await;
            outcome_to_result(outcome)?;
            label_id
        }
    };
    serde_json::to_value(LabelSaveAck { label_id })
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_delete(
    boot_state: &Arc<BootSharedState>,
    params: LabelDeleteParams,
) -> Result<Value, ServiceError> {
    let (ctx, account) = action_context_and_account(boot_state, &params.account_id).await?;
    let outcome = crate::actions::delete_label(
        &ctx,
        Some(&account),
        &params.account_id,
        &LabelId::from(params.label_id),
    )
    .await;
    outcome_to_result(outcome)?;
    serde_json::to_value(LabelDeleteAck).map_err(|e| ServiceError::Internal(e.to_string()))
}
