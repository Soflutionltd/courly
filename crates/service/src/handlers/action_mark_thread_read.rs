//! `action.mark_thread_read` handler.
//!
//! Read-on-open for one mail thread. The local flip commits here and the
//! pair is journaled as a quiet `mark_chat_read` job: that job kind only
//! carries "threads to mark read remotely", which is exactly this work,
//! and `action_jobs.kind` is CHECK-constrained. A thread that was
//! already read schedules nothing.

use crate::boot::BootSharedState;
use db::db::action_journal::insert_quiet_job;
use db::db::queries::set_thread_read;
use rusqlite::params;
use serde_json::Value;
use service_api::{MarkChatReadAck, PlanId, ServiceError};
use std::sync::Arc;

use super::JournaledChatRead;

pub(super) async fn handle(
    state: &Arc<BootSharedState>,
    account_id: String,
    thread_id: String,
) -> Result<Value, ServiceError> {
    let db = state.write_db_state()?;
    let job_id = uuid::Uuid::now_v7();
    let job_id_bytes = *job_id.as_bytes();

    let journaled = db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|e| format!("begin mark-thread-read: {e}"))?;
            let unread: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM messages \
                     WHERE account_id = ?1 AND thread_id = ?2 AND is_read = 0",
                    params![account_id, thread_id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("count unread: {e}"))?;
            if unread == 0 {
                return Ok(false);
            }
            set_thread_read(&tx, &account_id, &thread_id, true)?;
            tx.execute(
                "UPDATE messages SET is_read = 1 WHERE account_id = ?1 AND thread_id = ?2",
                params![account_id, thread_id],
            )
            .map_err(|e| format!("update message read flags: {e}"))?;
            let payload = JournaledChatRead {
                chat_email: String::new(),
                affected: vec![(account_id.clone(), thread_id)],
            };
            let payload_blob = serde_json::to_vec(&payload)
                .map_err(|e| format!("serialize mark-thread-read payload: {e}"))?;
            insert_quiet_job(
                &tx,
                &job_id_bytes,
                "mark_chat_read",
                &account_id,
                &payload_blob,
            )?;
            tx.commit()
                .map_err(|e| format!("commit mark-thread-read: {e}"))?;
            Ok::<bool, String>(true)
        })
        .await
        .map_err(ServiceError::Internal)?;

    if journaled {
        state.notify_action_worker();
    }
    let ack = MarkChatReadAck {
        job_id: PlanId(job_id),
        journaled,
    };
    serde_json::to_value(&ack).map_err(|error| ServiceError::Internal(error.to_string()))
}
