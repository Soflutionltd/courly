use iced::Task;
use iced::time::Instant;
use rust_i18n::t;

use crate::ui::settings::types::*;

impl Settings {
    pub(super) fn handle_signature_save(
        &mut self,
    ) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        let Some(ref mut editor) = self.signature_editor else {
            return (Task::none(), None);
        };
        let name = editor.name.text().trim().to_string();
        // Save stays clickable so that a missing field is named instead of
        // leaving a dead button.
        if editor.account_id.is_empty() {
            editor.save_error = Some(t!("settings.signatures.account_required").into_owned());
            return (Task::none(), None);
        }
        if name.is_empty() {
            editor.save_error = Some(t!("settings.signatures.name_required").into_owned());
            return (iced::widget::operation::focus("sig-name".to_string()), None);
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
                editor.import_error = None;
                editor.dirty = true;
                editor.set_body(html)
            }
            SettingsMessage::SignatureEditorInsertImage => {
                return Task::perform(
                    crate::ui::widgets::pick_image(),
                    SettingsMessage::SignatureEditorImagePicked,
                );
            }
            SettingsMessage::SignatureEditorImagePicked(Ok(Some(action))) => {
                editor.body_editor.perform(rte::Action::Edit(action));
                editor.import_error = None;
                editor.dirty = true;
                false
            }
            SettingsMessage::SignatureEditorImagePicked(Ok(None)) => false,
            SettingsMessage::SignatureEditorImagePicked(Err(error)) => {
                editor.import_error = Some(error);
                false
            }
            SettingsMessage::SignatureEditorHtmlImported(Err(error)) => {
                editor.import_error =
                    Some(t!("settings.signatures.import_failed", error = error).into_owned());
                false
            }
            SettingsMessage::SignatureEditorHtmlPasted(Ok(pasted)) => {
                editor.import_error = (pasted.missing_images > 0).then(|| {
                    t!(
                        "settings.signatures.images_missing",
                        count = pasted.missing_images
                    )
                    .into_owned()
                });
                editor.dirty = true;
                editor.set_body(pasted.html)
            }
            SettingsMessage::SignatureEditorHtmlPasted(Err(error)) => {
                editor.import_error =
                    Some(t!("settings.signatures.paste_failed", error = error).into_owned());
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
        start_signature_preview(editor)
    }

    /// Re-render the signature preview on a new page appearance.
    pub fn set_signature_preview_appearance(
        &mut self,
        appearance: mail_html::Appearance,
    ) -> Task<SettingsMessage> {
        let Some(editor) = self.signature_editor.as_mut() else {
            return Task::none();
        };
        if !editor.preview.set_appearance(appearance) {
            return Task::none();
        }
        start_signature_preview(editor)
    }
}

fn start_signature_preview(editor: &mut SignatureEditorState) -> Task<SettingsMessage> {
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
