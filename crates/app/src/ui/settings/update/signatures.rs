use iced::Task;
use iced::time::Instant;

use crate::ui::settings::types::*;

impl Settings {
    pub(super) fn handle_signature_save(
        &mut self,
    ) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        let Some(ref editor) = self.signature_editor else {
            return (Task::none(), None);
        };
        let name = editor.name.text().trim().to_string();
        if name.is_empty() || editor.account_id.is_empty() {
            return (Task::none(), None);
        }
        let request = SignatureSaveRequest {
            id: editor.signature_id.clone(),
            account_id: editor.account_id.clone(),
            name,
            body_html: editor.body_html(),
            is_default: editor.is_default,
            is_reply_default: editor.is_reply_default,
        };
        self.active_sheet = None;
        self.sheet_anim.go_mut(false, Instant::now());
        self.signature_editor = None;
        (Task::none(), Some(SettingsEvent::SaveSignature(request)))
    }
}

impl Settings {
    /// Importing an HTML signature and keeping its rendered preview current.
    pub(super) fn handle_signature_html(&mut self, msg: SettingsMessage) -> Task<SettingsMessage> {
        let Some(editor) = self.signature_editor.as_mut() else {
            return Task::none();
        };
        let start = match msg {
            SettingsMessage::SignatureEditorImportHtml => {
                return Task::perform(
                    pick_signature_file(),
                    SettingsMessage::SignatureEditorHtmlImported,
                );
            }
            SettingsMessage::SignatureEditorHtmlImported(Ok(None)) => false,
            SettingsMessage::SignatureEditorHtmlImported(Ok(Some(html))) => {
                editor.raw_html = Some(html);
                editor.import_error = None;
                editor.dirty = true;
                editor.preview.restart()
            }
            SettingsMessage::SignatureEditorHtmlImported(Err(error)) => {
                editor.import_error = Some(error);
                false
            }
            SettingsMessage::SignatureEditorUseRichText => {
                if let Some(html) = editor.raw_html.take() {
                    editor.body_editor = rte::EditorState::from_html(&html);
                    editor.dirty = true;
                }
                false
            }
            SettingsMessage::SignatureEditorPreviewResized(width) => {
                editor.preview.set_width(width)
            }
            SettingsMessage::SignatureEditorPreviewShowRemote => editor.preview.allow_remote(),
            SettingsMessage::SignatureEditorPreviewRendered { inputs, body } => {
                editor.preview.finish(inputs, body)
            }
            SettingsMessage::SignatureEditorPreviewLink(url) => {
                crate::ui::reading_pane::open_url_in_browser(&url);
                false
            }
            _ => false,
        };
        if !start {
            return Task::none();
        }
        let Some(html) = editor.raw_html.clone() else {
            return Task::none();
        };
        let Some(inputs) = editor.preview.start() else {
            return Task::none();
        };
        Task::perform(
            crate::ui::mail_body::render(html, inputs, Default::default()),
            move |body| SettingsMessage::SignatureEditorPreviewRendered { inputs, body },
        )
    }
}

/// `Ok(None)` when the picker is cancelled.
async fn pick_signature_file() -> Result<Option<String>, String> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .add_filter("HTML", &["htm", "html"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let path = file.path().to_path_buf();
    tokio::task::spawn_blocking(move || crate::signature_html::import_signature_file(&path))
        .await
        .map_err(|e| e.to_string())?
        .map(Some)
}
