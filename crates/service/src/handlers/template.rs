//! Email template handlers: `template.save` and `template.delete`, each one
//! statement through `WriteDbState::with_write`.

use std::sync::Arc;

use serde_json::Value;
use service_api::{
    ServiceError, TemplateDeleteAck, TemplateDeleteParams, TemplateSaveAck, TemplateSaveParams,
};

use crate::boot::BootSharedState;

pub(crate) async fn handle_save(
    boot_state: &Arc<BootSharedState>,
    params: TemplateSaveParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let id = write_db
        .with_write(move |conn| {
            let p = db::db::queries_extra::SaveTemplateParams {
                id: params.id,
                name: params.name,
                subject: params.subject,
                body_html: params.body_html,
            };
            db::db::queries_extra::db_save_template_sync(conn, &p)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(TemplateSaveAck { id }).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_delete(
    boot_state: &Arc<BootSharedState>,
    params: TemplateDeleteParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_write(move |conn| db::db::queries_extra::db_delete_template_sync(conn, &params.id))
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(TemplateDeleteAck).map_err(|e| ServiceError::Internal(e.to_string()))
}
