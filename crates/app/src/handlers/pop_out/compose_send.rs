use std::io::Write;
use std::path::PathBuf;

use iced::Task;
use rust_i18n::t;

use crate::pop_out::PopOutWindow;
use crate::pop_out::compose::{ComposeMode, ComposeState};
use crate::service_client::ServiceClient;
use crate::{Message, ReadyApp};

use service_api::actions::{SendAttachment, SendIntent};

/// Where a send came from, so its outcome reaches the right place.
pub(crate) enum SendOrigin {
    /// A compose still open in this window.
    Window(iced::window::Id),
    /// A compose closed during the undo delay; reopened if the send fails.
    Detached(Box<ComposeState>),
}

/// A sent message held back for the undo delay.
pub(crate) struct DelayedSend {
    send_id: service_api::PlanId,
    request: service_api::actions::SendRequest,
    compose: Box<ComposeState>,
}

impl ReadyApp {
    /// Build a MIME message from the compose state, save it to the draft row
    /// as base64url in the `attachments` column, mark the draft `'queued'`,
    /// Validate compose state, build a SendRequest, and dispatch to the
    /// action service. The compose window stays open with a "Sending..." status
    /// until SendCompleted arrives.
    pub(crate) fn handle_compose_send(&mut self, window_id: iced::window::Id) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) else {
            return Task::none();
        };

        // Prevent double-send
        if state.sending {
            return Task::none();
        }

        // Validate recipients
        let has_recipients = !state.to.tokens.is_empty()
            || !state.cc.tokens.is_empty()
            || !state.bcc.tokens.is_empty();
        if !has_recipients {
            state.status = Some(t!("compose.add_recipient_status").into_owned());
            return Task::none();
        }

        // Build SendRequest from compose state
        let account_info = match state.from_account.as_ref() {
            Some(a) => a.clone(),
            None => {
                state.status = Some(t!("compose.no_sending_account").into_owned());
                return Task::none();
            }
        };

        let from = if let Some(ref name) = account_info.display_name {
            format!("{name} <{}>", account_info.email)
        } else {
            account_info.email.clone()
        };

        let to: Vec<String> = state.to.tokens.iter().map(|t| t.email.clone()).collect();
        let cc: Vec<String> = state.cc.tokens.iter().map(|t| t.email.clone()).collect();
        let bcc: Vec<String> = state.bcc.tokens.iter().map(|t| t.email.clone()).collect();

        let subject = if state.subject.is_empty() {
            None
        } else {
            Some(state.subject.clone())
        };

        let (body_html, body_text) = state.outgoing_body();
        let (body_html, inline_images) = crate::signature_html::extract_data_images(&body_html);

        let attachments: Vec<SendAttachment> = state
            .attachments
            .iter()
            .map(|a| SendAttachment {
                filename: a.name.clone(),
                mime_type: a.mime_type.clone(),
                data: a.data.as_ref().clone().into(),
                content_id: None,
            })
            .chain(inline_images.into_iter().map(|image| SendAttachment {
                filename: image.filename,
                mime_type: image.mime_type,
                data: image.data.into(),
                content_id: Some(image.content_id),
            }))
            .collect();

        // Reuse draft_id on retry so the action updates the existing
        // 'failed' row instead of creating a new one.
        let draft_id = state
            .send_draft_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();

        let send_req = service_api::actions::SendRequest {
            draft_id,
            account_id: account_info.id.clone(),
            from,
            to,
            cc,
            bcc,
            subject,
            body_html,
            body_text,
            attachments,
            in_reply_to: state.reply_message_id.clone(),
            references: state.reply_message_id.clone(),
            thread_id: state.reply_thread_id.clone(),
            source_message_id: state.reply_source_message_id.clone(),
            intent: send_intent_from_mode(&state.mode),
        };

        let delay =
            crate::ui::settings::types::undo_send_delay(&self.settings.undo_delay);
        let send_id = service_api::PlanId::new_v7();
        if delay.is_zero() {
            state.sending = true;
            state.status = Some(t!("compose.sending").into_owned());
            return self.dispatch_send(SendOrigin::Window(window_id), send_id, send_req);
        }

        let Some(PopOutWindow::Compose(compose)) = self.pop_out_windows.remove(&window_id) else {
            return Task::none();
        };
        let close = if self.inline_compose == Some(window_id) {
            self.inline_compose = None;
            Task::none()
        } else {
            iced::window::close(window_id)
        };
        self.delayed_sends.push(DelayedSend {
            send_id,
            request: send_req,
            compose,
        });
        self.status_bar
            .show_pending_send(t!("compose.waiting_to_send").into_owned());
        Task::batch([
            close,
            Task::perform(
                async move {
                    tokio::time::sleep(delay).await;
                    send_id
                },
                Message::DelayedSendDue,
            ),
        ])
    }

    /// The undo delay ran out; a cancelled send is no longer queued.
    pub(crate) fn dispatch_delayed_send(&mut self, send_id: service_api::PlanId) -> Task<Message> {
        let Some(index) = self.delayed_sends.iter().position(|d| d.send_id == send_id) else {
            return Task::none();
        };
        let delayed = self.delayed_sends.remove(index);
        if self.delayed_sends.is_empty() {
            self.status_bar.clear_pending_send();
        }
        self.dispatch_send(
            SendOrigin::Detached(delayed.compose),
            delayed.send_id,
            delayed.request,
        )
    }

    /// Cancel the most recent message still in its undo delay and reopen it.
    pub(crate) fn undo_delayed_send(&mut self) -> Task<Message> {
        let Some(delayed) = self.delayed_sends.pop() else {
            return Task::none();
        };
        if self.delayed_sends.is_empty() {
            self.status_bar.clear_pending_send();
        }
        self.restore_compose(delayed.compose, None)
    }

    /// Messages still in their undo delay when the app quits: they were
    /// sent, so they go to the Service before it stops.
    pub(crate) fn take_delayed_sends_for_shutdown(&mut self) -> Vec<service_api::SendWireRequest> {
        let Some(app_data_dir) = crate::APP_DATA_DIR.get() else {
            return Vec::new();
        };
        self.status_bar.clear_pending_send();
        std::mem::take(&mut self.delayed_sends)
            .into_iter()
            .filter_map(|delayed| {
                stage_and_build_wire(app_data_dir, delayed.send_id, delayed.request)
                    .inspect_err(|error| log::error!("held send could not be staged: {error}"))
                    .ok()
            })
            .collect()
    }

    /// Put a compose back where it was, inline or in its own window.
    fn restore_compose(
        &mut self,
        mut compose: Box<ComposeState>,
        status: Option<String>,
    ) -> Task<Message> {
        compose.sending = false;
        compose.status = status;
        if compose.inline && self.release_inline_compose() {
            let id = iced::window::Id::unique();
            self.pop_out_windows.insert(id, PopOutWindow::Compose(compose));
            self.inline_compose = Some(id);
            return Task::none();
        }
        compose.inline = false;
        self.open_compose_window_for(*compose).1
    }

    /// Report a send that could not be handed to the Service.
    fn fail_send(&mut self, origin: SendOrigin, status: String) -> Task<Message> {
        match origin {
            SendOrigin::Window(window_id) => {
                if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id)
                {
                    state.sending = false;
                    state.status = Some(status);
                }
                Task::none()
            }
            SendOrigin::Detached(compose) => self.restore_compose(compose, Some(status)),
        }
    }

    pub(crate) fn handle_send_dispatch_failed(
        &mut self,
        send_id: service_api::PlanId,
        error: &str,
    ) -> Task<Message> {
        let Some(origin) = self.in_flight_sends.remove(&send_id) else {
            return Task::none();
        };
        self.fail_send(origin, t!("compose.send_failed", error = error).into_owned())
    }

    /// Route the Service's verdict on a send to where it came from.
    pub(crate) fn finish_send(
        &mut self,
        origin: SendOrigin,
        outcome: &service_api::actions::ActionOutcome,
    ) -> Task<Message> {
        match origin {
            SendOrigin::Window(window_id) => self.handle_send_completed(window_id, outcome),
            SendOrigin::Detached(compose) => match outcome {
                service_api::actions::ActionOutcome::Success
                | service_api::actions::ActionOutcome::NoOp => {
                    self.status_bar
                        .show_confirmation(t!("compose.message_sent").into_owned());
                    Task::none()
                }
                service_api::actions::ActionOutcome::Failed { error }
                | service_api::actions::ActionOutcome::LocalOnly { reason: error, .. } => {
                    let status =
                        t!("compose.send_failed", error = error.user_message()).into_owned();
                    self.restore_compose(compose, Some(status))
                }
            },
        }
    }

    /// Dispatch send through the Service via `action.send` IPC.
    ///
    /// Phase 2 task 13. UI writes attachment bytes to
    /// `<app_data>/staging/<send_id>/<index>.bin` (UI-owned), then
    /// issues the IPC. Service handler verifies SHA-256, atomically
    /// renames into the Service-owned vault, journals the send, and
    /// returns `SendAck`. The compose window stays in "sending"
    /// until the eventual `ActionCompleted` notification (matching
    /// `send_id`) fires `Message::SendCompleted`.
    fn dispatch_send(
        &mut self,
        origin: SendOrigin,
        send_id: service_api::PlanId,
        request: service_api::actions::SendRequest,
    ) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            return self.fail_send(origin, t!("compose.send_unavailable").into_owned());
        };
        let app_data_dir = crate::APP_DATA_DIR
            .get()
            .expect("APP_DATA_DIR set before send")
            .clone();

        // Stage every attachment to disk and build the wire request.
        // Returns the wire request on success, an error string on
        // first per-attachment failure.
        let wire = match stage_and_build_wire(&app_data_dir, send_id, request) {
            Ok(w) => w,
            Err(error) => {
                return self
                    .fail_send(origin, t!("compose.send_failed", error = error).into_owned());
            }
        };

        // Track the in-flight send so the ActionCompleted notification
        // reaches where it came from.
        self.in_flight_sends.insert(send_id, origin);

        let staging_dir = app_data_dir.join("staging").join(send_id.to_string());
        Task::perform(
            async move {
                let result = client.send_email(wire).await;
                (send_id, staging_dir, result)
            },
            move |(send_id, staging_dir, result)| {
                // Cleanup the staging directory regardless of ack
                // outcome: on success the bytes are now in the
                // Service vault, on failure they're not in the
                // vault and the user will retry from compose state.
                let _ = std::fs::remove_dir_all(&staging_dir);
                match result {
                    // Ack received: the Service journaled the send. The
                    // worker will dispatch SMTP and emit ActionCompleted;
                    // the in_flight_sends entry routes that to the
                    // compose window. Nothing to do at the UI layer
                    // here.
                    Ok(_ack) => Message::Noop,
                    // IPC failure (ServiceCrashed / Timeout / handler
                    // returned ServiceError): back to the compose.
                    Err(error) => Message::SendDispatchFailed {
                        send_id,
                        error: error.to_string(),
                    },
                }
            },
        )
    }

    /// Handle send completion: close compose on success, restore on failure.
    pub(crate) fn handle_send_completed(
        &mut self,
        window_id: iced::window::Id,
        outcome: &service_api::actions::ActionOutcome,
    ) -> Task<Message> {
        match outcome {
            service_api::actions::ActionOutcome::Success
            | service_api::actions::ActionOutcome::NoOp => {
                self.status_bar
                    .show_confirmation(t!("compose.message_sent").into_owned());
                self.close_compose(window_id)
            }
            // LocalOnly should not occur for send (send uses Failed for all
            // failures), but handle it defensively as failure for safety.
            service_api::actions::ActionOutcome::Failed { error }
            | service_api::actions::ActionOutcome::LocalOnly { reason: error, .. } => {
                if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id)
                {
                    state.sending = false;
                    state.status =
                        Some(t!("compose.send_failed", error = error.user_message()).into_owned());
                }
                Task::none()
            }
        }
    }
}

/// Hand messages held for their undo delay to the Service at quit. The
/// Service journals each one, so it goes out even if it stops right after.
pub(crate) async fn submit_held_sends(
    client: &ServiceClient,
    held: Vec<service_api::SendWireRequest>,
) {
    let Some(app_data_dir) = crate::APP_DATA_DIR.get() else {
        return;
    };
    for wire in held {
        let staging_dir = app_data_dir.join("staging").join(wire.send_id.to_string());
        if let Err(error) = client.send_email(wire).await {
            log::error!("held send was not handed to the Service: {error}");
        }
        let _ = std::fs::remove_dir_all(&staging_dir);
    }
}

/// Stage every attachment under `<app_data>/staging/<send_id>/`,
/// SHA-256 the bytes, and assemble a `SendWireRequest` referencing
/// the staged paths. Removes the staging directory and returns the
/// first error if any per-attachment write fails - the caller
/// surfaces the error in the compose status line.
///
/// Synchronous. Attachments are typically small (the dev fixtures
/// cap at ~50 MB) and the staging write happens on the iced runtime
/// only when the user clicks Send; the brief blocking is
/// indistinguishable from the existing in-process MIME build.
fn stage_and_build_wire(
    app_data_dir: &std::path::Path,
    send_id: service_api::PlanId,
    request: service_api::actions::SendRequest,
) -> Result<service_api::SendWireRequest, String> {
    let staging_dir: PathBuf = app_data_dir.join("staging").join(send_id.to_string());
    if let Err(error) = std::fs::create_dir_all(&staging_dir) {
        return Err(format!(
            "create staging dir {}: {error}",
            staging_dir.display()
        ));
    }

    let mut wire_attachments = Vec::with_capacity(request.attachments.len());
    for (index, att) in request.attachments.into_iter().enumerate() {
        let relative = format!("{index}.bin");
        let staged_path = staging_dir.join(&relative);
        if let Err(error) = write_atomic(&staged_path, &att.data) {
            let _ = std::fs::remove_dir_all(&staging_dir);
            return Err(format!("write staging {}: {error}", staged_path.display()));
        }
        let content_hash = *rtsk::blob_hash::BlobHash::hash(&att.data).as_bytes();
        let size = att.data.len() as u64;
        wire_attachments.push(service_api::SendWireAttachment {
            source: service_api::SendAttachmentSource::StagingFile {
                relative_path: relative,
                content_hash,
            },
            size,
            mime: att.mime_type,
            filename: att.filename,
            content_id: att.content_id,
        });
    }

    Ok(service_api::SendWireRequest {
        send_id,
        from_account_id: request.account_id,
        message: service_api::SendWireMessage {
            draft_id: request.draft_id,
            from: request.from,
            to: request.to,
            cc: request.cc,
            bcc: request.bcc,
            subject: request.subject,
            body_html: request.body_html,
            body_text: request.body_text,
            in_reply_to: request.in_reply_to,
            references: request.references,
            thread_id: request.thread_id,
            source_message_id: request.source_message_id,
            intent: wire_send_intent(request.intent),
            scheduled_at: None,
        },
        attachments: wire_attachments,
        scheduled_at: None,
    })
}

fn wire_send_intent(intent: SendIntent) -> service_api::SendIntent {
    match intent {
        SendIntent::New => service_api::SendIntent::New,
        SendIntent::Reply => service_api::SendIntent::Reply,
        SendIntent::Forward => service_api::SendIntent::Forward,
    }
}

/// Write bytes to a file. Uses `File::create` + `write_all` rather
/// than tempfile-rename: the staging directory is freshly created
/// for each send_id, so there is no concurrent-writer concern.
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    Ok(())
}

fn send_intent_from_mode(mode: &ComposeMode) -> SendIntent {
    match mode {
        ComposeMode::New => SendIntent::New,
        ComposeMode::Reply { .. } | ComposeMode::ReplyAll { .. } => SendIntent::Reply,
        ComposeMode::Forward { .. } => SendIntent::Forward,
    }
}
