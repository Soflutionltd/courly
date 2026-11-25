use std::sync::Arc;

use iced::Task;
use rust_i18n::t;

use crate::pop_out::message_view::MessageViewMessage;
use crate::pop_out::{PopOutMessage, PopOutWindow};
use crate::service_client::ServiceClient;
use crate::{Message, ReadyApp};

impl ReadyApp {
    /// Handle Save As action from the overflow menu.
    pub(crate) fn handle_save_as(&self, window_id: iced::window::Id) -> Task<Message> {
        let Some(PopOutWindow::MessageView(state)) = self.pop_out_windows.get(&window_id) else {
            return Task::none();
        };

        let export = MessageExport {
            service: self.service_client.clone(),
            account_id: state.account_id.clone(),
            message_id: state.message_id.clone(),
            text: plain_text(state.body_text.as_deref(), state.body_html.as_deref()),
        };
        let subject = state
            .subject
            .clone()
            .unwrap_or_else(|| "message".to_string());

        Task::perform(save_message_dialog(export, subject), move |result| {
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::SaveAsFinished(result)),
            )
        })
    }
}

/// What a message is saved from.
struct MessageExport {
    service: Option<Arc<ServiceClient>>,
    account_id: String,
    message_id: String,
    /// The whole body as plain text, `None` before it has loaded.
    text: Option<String>,
}

/// The body as plain text: its text part, else its HTML part flattened.
fn plain_text(body_text: Option<&str>, body_html: Option<&str>) -> Option<String> {
    match (body_text, body_html) {
        (Some(text), _) if !text.trim().is_empty() => Some(text.to_string()),
        (_, Some(html)) if !html.trim().is_empty() => {
            Some(rte::html_parse::from_html(html).flattened_text())
        }
        _ => None,
    }
}

/// Sanitize a subject line for use as a filename.
fn sanitize_filename(subject: &str) -> String {
    let safe: String = subject
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = safe.trim().to_string();
    if trimmed.is_empty() {
        "message".to_string()
    } else {
        trimmed
    }
}

/// Open a native file picker and save the message as .eml or .txt. A
/// message that cannot be read in full is an error, never a partial file.
async fn save_message_dialog(export: MessageExport, subject: String) -> Result<(), String> {
    let safe_name = sanitize_filename(&subject);

    let file_handle = rfd::AsyncFileDialog::new()
        .set_title(t!("compose.save_message_as"))
        .set_file_name(format!("{safe_name}.eml"))
        .add_filter(t!("compose.filter_eml"), &["eml"])
        .add_filter(t!("compose.filter_txt"), &["txt"])
        .save_file()
        .await;

    let Some(handle) = file_handle else {
        return Ok(()); // user cancelled
    };

    let path = handle.path().to_path_buf();
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("eml");

    match extension {
        "txt" => {
            let text = export
                .text
                .ok_or_else(|| t!("compose.save_message_not_loaded").into_owned())?;
            std::fs::write(&path, text.as_bytes()).map_err(|e| e.to_string())
        }
        _ => {
            // The message as its provider holds it, attachments included.
            let service = export
                .service
                .ok_or_else(|| t!("compose.save_message_offline").into_owned())?;
            let data_dir = crate::APP_DATA_DIR
                .get()
                .ok_or_else(|| "app data directory unknown".to_string())?;
            let ack = service
                .message_source(export.account_id, export.message_id)
                .await
                .map_err(|e| e.to_string())?;
            let source = data_dir.join(&ack.relative_path);
            std::fs::copy(&source, &path).map_err(|e| e.to_string())?;
            if let Err(error) = std::fs::remove_file(&source) {
                log::warn!("save as: cannot remove {}: {error}", source.display());
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::plain_text;

    #[test]
    fn the_text_part_is_kept_as_it_is() {
        assert_eq!(
            plain_text(Some("Hello\nBob"), Some("<p>ignored</p>")).as_deref(),
            Some("Hello\nBob")
        );
    }

    #[test]
    fn an_html_only_body_is_flattened() {
        let text = plain_text(None, Some("<p>Hello <b>Bob</b></p>")).expect("text");
        assert!(text.contains("Hello Bob"), "{text}");
    }

    #[test]
    fn an_unloaded_body_gives_no_text() {
        assert_eq!(plain_text(None, None), None);
        assert_eq!(plain_text(Some("  "), Some("")), None);
    }
}
