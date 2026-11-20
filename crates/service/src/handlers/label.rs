//! Service handlers for label and label_group IPC.

use std::sync::Arc;

use common::typed_ids::LabelId;
use serde_json::Value;
use service_api::actions::ActionOutcome;
use service_api::{
    LabelDeleteAck, LabelDeleteParams, LabelGroupDeleteAck, LabelGroupDeleteParams,
    LabelGroupReorderAck, LabelGroupReorderParams, LabelGroupSaveAck, LabelGroupSaveParams,
    LabelSaveAck, LabelSaveParams, ServiceError,
};

use crate::actions::ActionContext;
use crate::boot::BootSharedState;
use crate::conduit::resident::ResidentActionAccount;

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
        ActionOutcome::LocalOnly { reason, .. } => {
            Err(ServiceError::Internal(reason.user_message()))
        }
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

/// What one provider said, for the error shown under the editor.
fn failure_line(account_id: &str, error: &ServiceError) -> String {
    format!("{account_id}: {error}")
}

fn failures_to_result(failures: &[String]) -> Result<(), ServiceError> {
    if failures.is_empty() {
        Ok(())
    } else {
        Err(ServiceError::Internal(failures.join("\n")))
    }
}

pub(crate) async fn handle_group_save(
    boot_state: &Arc<BootSharedState>,
    params: LabelGroupSaveParams,
) -> Result<Value, ServiceError> {
    let invalid = |message: &str| ServiceError::InvalidParams {
        method: "label_group.save".into(),
        message: message.into(),
    };
    let name = params.name.trim().to_owned();
    if name.is_empty() {
        return Err(invalid("label name is empty"));
    }
    let style = (params.color_bg.clone(), params.color_fg.clone());
    let group_id = match params.group_id {
        None => create_group(boot_state, &name, &style, params.members, &params.accounts).await?,
        Some(group_id) => {
            update_group(boot_state, group_id, &name, &style, params.members, invalid).await?;
            group_id
        }
    };
    serde_json::to_value(LabelGroupSaveAck { group_id })
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

/// A new user-visible label. With `members` it gathers provider labels
/// that exist. Without, the label is made on the provider of each of
/// `accounts`: writing each one locally files it in the group of its name,
/// which then takes the colours chosen here.
async fn create_group(
    boot_state: &Arc<BootSharedState>,
    name: &str,
    style: &(String, String),
    members: Option<Vec<(String, String)>>,
    accounts: &[String],
) -> Result<i64, ServiceError> {
    let members = members.filter(|members| !members.is_empty());
    let mut failures = Vec::new();
    if members.is_none() {
        for account_id in accounts {
            let created = async {
                let (ctx, account) = action_context_and_account(boot_state, account_id).await?;
                let (outcome, _) = crate::actions::create_label(
                    &ctx,
                    Some(&account),
                    account_id,
                    name,
                    Some((style.0.as_str(), style.1.as_str())),
                )
                .await;
                outcome_to_result(outcome)
            }
            .await;
            if let Err(error) = created {
                failures.push(failure_line(account_id, &error));
            }
        }
        if !accounts.is_empty() && failures.len() == accounts.len() {
            return Err(ServiceError::Internal(failures.join("\n")));
        }
    }

    let write_db = boot_state.write_db_state()?;
    let (name, (bg, fg)) = (name.to_owned(), style.clone());
    let group_id = write_db
        .with_write(move |conn| {
            let group_id = db::db::queries_extra::label_groups::create_label_group_sync(
                conn, &name, &bg, &fg,
            )?;
            if let Some(members) = members {
                db::db::queries_extra::label_groups::set_label_group_members_sync(
                    conn, group_id, &members,
                )?;
            }
            Ok(group_id)
        })
        .await
        .map_err(ServiceError::Internal)?;
    failures_to_result(&failures)?;
    Ok(group_id)
}

/// Rename and recolour a user-visible label. The name stays local: an
/// Outlook category cannot be renamed. A new colour is also written to
/// the provider labels of the group.
async fn update_group(
    boot_state: &Arc<BootSharedState>,
    group_id: i64,
    name: &str,
    style: &(String, String),
    members: Option<Vec<(String, String)>>,
    invalid: impl Fn(&str) -> ServiceError,
) -> Result<(), ServiceError> {
    use db::db::queries_extra::label_groups::{self, LabelGroupUpdateError};

    let write_db = boot_state.write_db_state()?;
    let (new_name, (bg, fg)) = (name.to_owned(), style.clone());
    let written = write_db
        .with_write(move |conn| {
            let Some(stored) = label_groups::get_label_group_sync(conn, group_id)? else {
                return Ok(Err(LabelGroupUpdateError::Missing));
            };
            if let Err(error) =
                label_groups::update_label_group_sync(conn, group_id, &new_name, &bg, &fg)
            {
                return Ok(Err(error));
            }
            if let Some(members) = members {
                label_groups::set_label_group_members_sync(conn, group_id, &members)?;
            }
            let recoloured = !stored.color_bg.eq_ignore_ascii_case(&bg)
                || !stored.color_fg.eq_ignore_ascii_case(&fg);
            let to_recolour = if recoloured {
                label_groups::label_group_members_sync(conn, group_id)?
            } else {
                Vec::new()
            };
            Ok(Ok(to_recolour))
        })
        .await
        .map_err(ServiceError::Internal)?;
    let to_recolour = match written {
        Ok(members) => members,
        Err(LabelGroupUpdateError::Missing) => return Err(invalid("this label no longer exists")),
        Err(LabelGroupUpdateError::NameTaken) => {
            return Err(invalid("another label already has this name"));
        }
        Err(LabelGroupUpdateError::Database(error)) => return Err(ServiceError::Internal(error)),
    };

    let mut failures = Vec::new();
    for member in to_recolour {
        let recoloured = async {
            let (ctx, account) = action_context_and_account(boot_state, &member.account_id).await?;
            let outcome = crate::actions::recolor_label(
                &ctx,
                Some(&account),
                &member.account_id,
                &LabelId::from(member.label_id.clone()),
                &member.name,
                (style.0.as_str(), style.1.as_str()),
            )
            .await;
            outcome_to_result(outcome)
        }
        .await;
        if let Err(error) = recoloured {
            failures.push(failure_line(&member.account_id, &error));
        }
    }
    failures_to_result(&failures)
}

/// Delete a user-visible label and the provider labels behind it. The
/// group goes only once every provider label is gone: one left on its
/// provider would bring the group back on the next sync.
pub(crate) async fn handle_group_delete(
    boot_state: &Arc<BootSharedState>,
    params: LabelGroupDeleteParams,
) -> Result<Value, ServiceError> {
    let group_id = params.group_id;
    let write_db = boot_state.write_db_state()?;
    let members = write_db
        .with_write(move |conn| {
            db::db::queries_extra::label_groups::label_group_members_sync(conn, group_id)
        })
        .await
        .map_err(ServiceError::Internal)?;

    let mut failures = Vec::new();
    for member in members {
        let deleted = async {
            let (ctx, account) = action_context_and_account(boot_state, &member.account_id).await?;
            let outcome = crate::actions::delete_label(
                &ctx,
                Some(&account),
                &member.account_id,
                &LabelId::from(member.label_id.clone()),
            )
            .await;
            outcome_to_result(outcome)
        }
        .await;
        if let Err(error) = deleted {
            failures.push(failure_line(&member.account_id, &error));
        }
    }
    failures_to_result(&failures)?;

    write_db
        .with_write(move |conn| {
            db::db::queries_extra::label_groups::delete_label_group_sync(conn, group_id)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(LabelGroupDeleteAck).map_err(|e| ServiceError::Internal(e.to_string()))
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
