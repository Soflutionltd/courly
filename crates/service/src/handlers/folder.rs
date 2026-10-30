//! Service handlers for provider folder IPC (`folder.*`).

use std::sync::Arc;

use common::typed_ids::FolderId;
use db::db::folder_roles::SYSTEM_FOLDER_ROLES;
use serde_json::Value;
use service_api::{
    FolderContentsAck, FolderContentsParams, FolderCreateParams, FolderDeleteParams, FolderDoneAck,
    FolderMoveParams, FolderRenameParams, FolderWriteAck, ServiceError,
};

use super::label::{action_context_and_account, outcome_to_result};
use crate::boot::BootSharedState;

fn refuse_system_folder(method: &str, folder_id: &str) -> Result<(), ServiceError> {
    if SYSTEM_FOLDER_ROLES.iter().any(|role| role.label_id == folder_id) {
        return Err(ServiceError::InvalidParams {
            method: method.into(),
            message: format!("{folder_id} is a system folder"),
        });
    }
    Ok(())
}

fn non_empty_name<'a>(method: &str, name: &'a str) -> Result<&'a str, ServiceError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ServiceError::InvalidParams {
            method: method.into(),
            message: "folder name is empty".into(),
        });
    }
    Ok(name)
}

fn ack(value: impl serde::Serialize) -> Result<Value, ServiceError> {
    serde_json::to_value(value).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_create(
    boot_state: &Arc<BootSharedState>,
    params: FolderCreateParams,
) -> Result<Value, ServiceError> {
    let name = non_empty_name("folder.create", &params.name)?;
    let (ctx, account) = action_context_and_account(boot_state, &params.account_id).await?;
    let parent = params.parent_id.map(FolderId::from);
    let (outcome, created) = crate::actions::create_folder(
        &ctx,
        Some(&account),
        &params.account_id,
        name,
        parent.as_ref(),
    )
    .await;
    outcome_to_result(outcome)?;
    let folder_id = created
        .ok_or_else(|| ServiceError::Internal("folder created without a storage id".into()))?;
    ack(FolderWriteAck { folder_id })
}

pub(crate) async fn handle_rename(
    boot_state: &Arc<BootSharedState>,
    params: FolderRenameParams,
) -> Result<Value, ServiceError> {
    refuse_system_folder("folder.rename", &params.folder_id)?;
    let name = non_empty_name("folder.rename", &params.name)?;
    let (ctx, account) = action_context_and_account(boot_state, &params.account_id).await?;
    let (outcome, renamed) = crate::actions::rename_folder(
        &ctx,
        Some(&account),
        &params.account_id,
        &FolderId::from(params.folder_id.clone()),
        name,
    )
    .await;
    outcome_to_result(outcome)?;
    ack(FolderWriteAck {
        folder_id: renamed.unwrap_or(params.folder_id),
    })
}

pub(crate) async fn handle_move(
    boot_state: &Arc<BootSharedState>,
    params: FolderMoveParams,
) -> Result<Value, ServiceError> {
    refuse_system_folder("folder.move", &params.folder_id)?;
    if params.parent_id.as_deref() == Some(params.folder_id.as_str()) {
        return Err(ServiceError::InvalidParams {
            method: "folder.move".into(),
            message: "a folder cannot be moved into itself".into(),
        });
    }
    let (ctx, account) = action_context_and_account(boot_state, &params.account_id).await?;
    let parent = params.parent_id.map(FolderId::from);
    let outcome = crate::actions::move_folder(
        &ctx,
        Some(&account),
        &params.account_id,
        &FolderId::from(params.folder_id),
        parent.as_ref(),
    )
    .await;
    outcome_to_result(outcome)?;
    ack(FolderDoneAck)
}

pub(crate) async fn handle_delete(
    boot_state: &Arc<BootSharedState>,
    params: FolderDeleteParams,
) -> Result<Value, ServiceError> {
    refuse_system_folder("folder.delete", &params.folder_id)?;
    let (ctx, account) = action_context_and_account(boot_state, &params.account_id).await?;
    let outcome = crate::actions::delete_folder(
        &ctx,
        Some(&account),
        &params.account_id,
        &FolderId::from(params.folder_id),
    )
    .await;
    outcome_to_result(outcome)?;
    ack(FolderDoneAck)
}

pub(crate) async fn handle_mark_all_read(
    boot_state: &Arc<BootSharedState>,
    params: FolderContentsParams,
) -> Result<Value, ServiceError> {
    let (ctx, account) = action_context_and_account(boot_state, &params.account_id).await?;
    let count =
        crate::actions::mark_folder_read(&ctx, &account, &params.account_id, &params.folder_id)
            .await
            .map_err(|error| ServiceError::Internal(error.user_message()))?;
    ack(FolderContentsAck { count })
}

pub(crate) async fn handle_empty(
    boot_state: &Arc<BootSharedState>,
    params: FolderContentsParams,
) -> Result<Value, ServiceError> {
    refuse_system_folder("folder.empty", &params.folder_id)?;
    let (ctx, account) = action_context_and_account(boot_state, &params.account_id).await?;
    let count = crate::actions::empty_folder(&ctx, &account, &params.account_id, &params.folder_id)
        .await
        .map_err(|error| ServiceError::Internal(error.user_message()))?;
    ack(FolderContentsAck { count })
}

#[cfg(test)]
mod tests {
    use super::refuse_system_folder;

    #[test]
    fn system_folders_are_refused_and_user_folders_pass() {
        for system in ["INBOX", "SENT", "TRASH", "DRAFT"] {
            assert!(refuse_system_folder("folder.delete", system).is_err(), "{system}");
        }
        assert!(refuse_system_folder("folder.delete", "graph-AAMk").is_ok());
    }
}
