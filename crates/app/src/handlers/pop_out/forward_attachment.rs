//! Forward as attachment, as in Outlook: the original message travels as
//! an `.eml` file in a new forward, byte for byte as its provider holds it,
//! headers and attachments included, instead of being quoted in the body.

use std::sync::Arc;

use iced::Task;
use rust_i18n::t;

use crate::pop_out::compose::{ComposeAttachment, ComposeMode, ComposeState, ReplySource};
use crate::service_client::ServiceClient;
use crate::{Message, ReadyApp};

/// The message being forwarded, kept until its source has been fetched.
#[derive(Debug, Clone)]
pub struct ForwardAsAttachment {
    account_id: String,
    message_id: String,
    message_id_header: Option<String>,
    thread_id: Option<String>,
    subject: String,
}

impl ReadyApp {
    /// Fetch the open conversation's latest message as an `.eml` and open a
    /// forward that carries it.
    pub(crate) fn forward_as_attachment(&mut self) -> Task<Message> {
        let thread_id = self
            .thread_list
            .selected_thread
            .and_then(|idx| self.thread_list.threads.get(idx))
            .map(|thread| thread.id.clone());
        let Some(message) = self.reading_pane.thread_messages.first() else {
            return Task::none();
        };
        let Some(client) = self.service_client.clone() else {
            self.status_bar.show_confirmation(
                t!(
                    "compose.forward_attachment_failed",
                    error = t!("compose.save_message_offline")
                )
                .into_owned(),
            );
            return Task::none();
        };
        let request = ForwardAsAttachment {
            account_id: message.account_id.clone(),
            message_id: message.id.clone(),
            message_id_header: message.message_id_header.clone(),
            thread_id,
            subject: message.subject.clone().unwrap_or_default(),
        };
        let fetch = request.clone();
        Task::perform(fetch_eml(client, fetch), move |result| {
            Message::ForwardAsAttachmentReady(request.clone(), result)
        })
    }

    /// The source arrived: open the forward with the `.eml` attached and an
    /// empty body, or say why it could not be fetched.
    pub(crate) fn handle_forward_as_attachment_ready(
        &mut self,
        request: &ForwardAsAttachment,
        result: Result<ComposeAttachment, String>,
    ) -> Task<Message> {
        let attachment = match result {
            Ok(attachment) => attachment,
            Err(error) => {
                self.status_bar.show_confirmation(
                    t!("compose.forward_attachment_failed", error = error).into_owned(),
                );
                return Task::none();
            }
        };
        let mode = ComposeMode::Forward {
            original_subject: request.subject.clone(),
        };
        let source = ReplySource {
            subject: Some(&request.subject),
            thread_id: request.thread_id.as_deref(),
            message_id: Some(&request.message_id),
            message_id_header: request.message_id_header.as_deref(),
            account_id: Some(&request.account_id),
            ..ReplySource::default()
        };
        let mut state = ComposeState::new_reply(&self.sidebar.accounts, &mode, &source);
        state.attachments.push(attachment);
        self.open_inline_compose(state, mode)
    }
}

/// Read the message's original source through the Service and turn it into
/// an attachment named after its subject. The temporary copy the Service
/// wrote is removed once read.
async fn fetch_eml(
    client: Arc<ServiceClient>,
    request: ForwardAsAttachment,
) -> Result<ComposeAttachment, String> {
    let data_dir = crate::APP_DATA_DIR
        .get()
        .ok_or_else(|| "app data directory unknown".to_string())?;
    let ack = client
        .message_source(request.account_id, request.message_id)
        .await
        .map_err(|e| e.to_string())?;
    let path = data_dir.join(&ack.relative_path);
    let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if let Err(error) = std::fs::remove_file(&path) {
        log::warn!(
            "forward as attachment: cannot remove {}: {error}",
            path.display()
        );
    }
    Ok(ComposeAttachment {
        name: eml_name(&request.subject),
        mime_type: "message/rfc822".to_string(),
        data: Arc::new(bytes),
    })
}

/// The attachment's file name: the subject with characters a file system
/// refuses replaced, as Outlook names it.
fn eml_name(subject: &str) -> String {
    let safe: String = subject
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = safe.trim();
    if trimmed.is_empty() {
        "message.eml".to_string()
    } else {
        format!("{trimmed}.eml")
    }
}

#[cfg(test)]
mod tests {
    use super::eml_name;

    #[test]
    fn the_attachment_is_named_after_the_subject() {
        assert_eq!(
            eml_name("Facture: novembre / 2026"),
            "Facture_ novembre _ 2026.eml"
        );
    }

    #[test]
    fn a_message_without_subject_is_message_eml() {
        assert_eq!(eml_name("   "), "message.eml");
    }
}
