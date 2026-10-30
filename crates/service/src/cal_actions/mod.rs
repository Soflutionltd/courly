//! Calendar action dispatcher - the Service-side write path for calendar
//! event mutations.
//!
//! Phase 6c task 6c-6: this module is the calendar pipeline's
//! `batch_execute`. The action worker (Phase 6c-7) reads the journaled
//! `kind = 'calendar_plan'` row, deserialises each
//! `WireCalendarOperation` blob, builds a `CalendarActionContext` from
//! the boot-shared writer-half + encryption key, and calls
//! `batch_execute`. Per-op `CalendarOperationOutcome` notifications and
//! the per-plan `CalendarActionCompleted` are emitted on the way back.
//!
//! `CalendarOperationOutcome` is `MustDeliver` class (see
//! `service-api::Notification`); the UI's `pending_calendar_action_plans`
//! map (Phase 6c-9) keys on `plan_id` and unblocks the awaiting caller
//! when the matching `CalendarActionCompleted` arrives. Phase 5 used
//! the latch pattern for `CalendarRunCompleted` to dodge the
//! late-subscriber race; this pipeline reuses the same shape.
//!
//! `ActionOutcome` is the in-process domain type returned by
//! `cal::actions::*`; it is converted to the wire-narrow
//! `CalendarOperationResult` at the IPC boundary inside this
//! function. Mail's `OperationResult` has the rich
//! `RemoteFailure { http_status, retryable }` taxonomy because the
//! mail action pipeline classifies provider errors that way; the
//! calendar action pipeline returns `ActionOutcome::LocalOnly { reason,
//! retryable }` on provider failure for `CreateEvent` and a flat
//! `ActionOutcome::Failed { error }` for `Update` / `Delete`.
//! `CalendarOperationResult` mirrors that narrower taxonomy.

use action_types::{ActionError, ActionOutcome, CalendarAccountOpener, CalendarActionContext};
use async_trait::async_trait;
use bifrost_types::{Account, AccountId, ProtocolKind};
use cal::actions::{
    CalendarEventInput, create_calendar_event, delete_calendar_event, rsvp_calendar_event,
    update_calendar_event,
};
use service_api::{
    CalendarActionWireOperation, CalendarOperationResult, RsvpResponse, WireCalendarEventInput,
    WireCalendarOperation, WireRsvpReply,
};
use std::sync::Arc;

/// Service implementation of the calendar factory seam. `cal` only knows the
/// small trait, keeping the service factory graph out of the calendar crate.
pub struct ServiceCalendarAccountOpener {
    read_db: db::db::ReadDbState,
    write_db: service_state::WriteDbState,
    encryption_key: [u8; 32],
}

impl ServiceCalendarAccountOpener {
    pub fn new(
        read_db: db::db::ReadDbState,
        write_db: service_state::WriteDbState,
        encryption_key: [u8; 32],
    ) -> Self {
        Self {
            read_db,
            write_db,
            encryption_key,
        }
    }
}

#[async_trait]
impl CalendarAccountOpener for ServiceCalendarAccountOpener {
    async fn open(
        &self,
        account_id: &str,
    ) -> Result<Option<(Arc<dyn Account>, ProtocolKind)>, ActionError> {
        let aid = account_id.to_string();
        let provider = self.read_db.with_read_mapped(move |conn| {
            conn.query_row("SELECT provider, calendar_provider, caldav_url FROM accounts WHERE id = ?1", rusqlite::params![aid], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?)))
                .map_err(|e| ActionError::db(format!("calendar account lookup: {e}")))
        }, |e| ActionError::db(e.clone())).await?;
        // Resolve calendar-provider precedence through the SAME helper the
        // factory uses, so the CalDAV-vs-mail rule lives in one place. `None`
        // (IMAP-only / unrecognised) means no calendar backend.
        let Some(kind) = crate::bifrost::factory::calendar_protocol_kind(
            &provider.0,
            provider.1.as_deref(),
            provider.2.as_deref(),
        ) else {
            return Ok(None);
        };
        let factory = crate::bifrost::factory::build_calendar_account_factory(
            &self.read_db,
            self.write_db.writer_pool(),
            account_id,
            self.encryption_key,
        )
        .await
        .map_err(|e| ActionError::remote(e.to_string()))?;
        let Some(factory) = factory else {
            return Ok(None);
        };
        let opened = factory
            .open(AccountId(account_id.to_string()))
            .await
            .map_err(|e| ActionError::remote(e.to_string()))?;
        Ok(Some((opened.account, kind)))
    }
}

/// Run every operation in `ops` sequentially, returning per-op
/// results in original order.
///
/// Calendar plans today are 1:1 (one user intent = one operation),
/// so the sequential loop is exactly right. The shape mirrors mail's
/// `batch_execute` so that the future Phase 6d work (RSVP /
/// series-vs-occurrence) can layer in N-op plans without a structural
/// refactor.
pub async fn batch_execute(
    ctx: &CalendarActionContext,
    ops: Vec<CalendarActionWireOperation>,
) -> Vec<CalendarOperationResult> {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        let outcome = run_one(ctx, &op).await;
        out.push(outcome_to_wire(outcome));
    }
    out
}

/// Dispatch one operation to the matching `cal::actions::*` function.
async fn run_one(ctx: &CalendarActionContext, op: &CalendarActionWireOperation) -> ActionOutcome {
    match &op.operation {
        WireCalendarOperation::CreateEvent {
            calendar_remote_id,
            input,
        } => {
            create_calendar_event(
                ctx,
                &op.account_id,
                calendar_remote_id,
                wire_input_to_domain(input),
            )
            .await
        }
        WireCalendarOperation::UpdateEvent { event_id, input } => {
            update_calendar_event(ctx, &op.account_id, event_id, wire_input_to_domain(input)).await
        }
        WireCalendarOperation::DeleteEvent { event_id } => {
            delete_calendar_event(ctx, &op.account_id, event_id).await
        }
        WireCalendarOperation::RsvpEvent {
            event_id,
            response,
            reply,
        } => {
            let status = match response {
                RsvpResponse::Accepted => bifrost_types::RsvpStatus::Accepted,
                RsvpResponse::Declined => bifrost_types::RsvpStatus::Declined,
                RsvpResponse::Tentative => bifrost_types::RsvpStatus::Tentative,
            };
            let reply = match wire_reply_to_domain(reply) {
                Ok(reply) => reply,
                Err(error) => return ActionOutcome::Failed { error },
            };
            rsvp_calendar_event(ctx, &op.account_id, event_id, status, reply).await
        }
    }
}

/// A proposed time needs both ends, in order.
fn wire_reply_to_domain(reply: &WireRsvpReply) -> Result<bifrost_types::RsvpReply, ActionError> {
    let proposed_time = match (reply.proposed_start, reply.proposed_end) {
        (None, None) => None,
        (Some(start), Some(end)) if start < end => {
            let at = |seconds| {
                jiff::Timestamp::from_second(seconds)
                    .map_err(|error| ActionError::invalid_state(format!("proposed time: {error}")))
            };
            Some(bifrost_types::ProposedTime {
                start: at(start)?,
                end: at(end)?,
            })
        }
        _ => {
            return Err(ActionError::invalid_state(
                "proposed time needs a start before its end",
            ));
        }
    };
    Ok(bifrost_types::RsvpReply {
        comment: reply
            .comment
            .clone()
            .filter(|comment| !comment.trim().is_empty()),
        send_response: reply.send_response,
        proposed_time,
    })
}

/// Convert the wire shape to the in-process domain shape. The two
/// have identical fields - the wire mirror exists so service-api
/// stays free of cal's transitive provider-trait graph.
pub(crate) fn wire_input_to_domain(input: &WireCalendarEventInput) -> CalendarEventInput {
    CalendarEventInput {
        title: input.title.clone(),
        description: input.description.clone(),
        location: input.location.clone(),
        start_time: input.start_time,
        end_time: input.end_time,
        is_all_day: input.is_all_day,
        timezone: input.timezone.clone(),
        recurrence_rule: input.recurrence_rule.clone(),
        availability: input.availability.clone(),
        visibility: input.visibility.clone(),
    }
}

/// Convert the in-process `ActionOutcome` to the wire-narrow
/// `CalendarOperationResult`. `LocalOnly` is reachable only for
/// `CreateEvent`; the mapping is exhaustive so a future variant on
/// either side surfaces here as a compile error.
fn outcome_to_wire(outcome: ActionOutcome) -> CalendarOperationResult {
    match outcome {
        ActionOutcome::Success | ActionOutcome::NoOp => CalendarOperationResult::Success,
        ActionOutcome::LocalOnly { reason, .. } => CalendarOperationResult::LocalOnly {
            reason: reason.user_message(),
        },
        ActionOutcome::Failed { error } => CalendarOperationResult::Failed {
            error: error.user_message(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use action_types::ActionError;

    #[test]
    fn outcome_to_wire_maps_success() {
        let result = outcome_to_wire(ActionOutcome::Success);
        assert!(matches!(result, CalendarOperationResult::Success));
    }

    #[test]
    fn outcome_to_wire_maps_local_only_with_reason() {
        let result = outcome_to_wire(ActionOutcome::LocalOnly {
            reason: ActionError::remote("provider 503"),
            retryable: true,
        });
        match result {
            CalendarOperationResult::LocalOnly { reason } => {
                assert!(reason.contains("provider 503"));
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[test]
    fn outcome_to_wire_maps_failed_to_user_message() {
        let result = outcome_to_wire(ActionOutcome::Failed {
            error: ActionError::not_found("calendar 404"),
        });
        assert!(matches!(result, CalendarOperationResult::Failed { .. }));
    }

    #[test]
    fn wire_reply_keeps_comment_and_proposed_slot() {
        let reply = wire_reply_to_domain(&WireRsvpReply {
            comment: Some("Next week?".to_string()),
            send_response: true,
            proposed_start: Some(1_793_628_000),
            proposed_end: Some(1_793_631_600),
        })
        .expect("valid reply");
        assert_eq!(reply.comment.as_deref(), Some("Next week?"));
        assert!(reply.send_response);
        let slot = reply.proposed_time.expect("proposed slot");
        assert_eq!(slot.start.as_second(), 1_793_628_000);
        assert_eq!(slot.end.as_second(), 1_793_631_600);
    }

    #[test]
    fn wire_reply_drops_blank_comment_and_rejects_reversed_slot() {
        let blank = wire_reply_to_domain(&WireRsvpReply {
            comment: Some("  ".to_string()),
            ..WireRsvpReply::default()
        })
        .expect("blank comment is valid");
        assert_eq!(blank.comment, None);
        assert!(
            wire_reply_to_domain(&WireRsvpReply {
                proposed_start: Some(10),
                proposed_end: Some(5),
                ..WireRsvpReply::default()
            })
            .is_err()
        );
    }
}
