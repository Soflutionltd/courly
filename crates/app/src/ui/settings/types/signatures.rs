use crate::ui::undoable::UndoableText;

use rte::EditorState as RteEditorState;

/// A signature entry for display in the settings list.
/// Mirrors the relevant fields of `DbSignature` without depending on the db crate.
#[derive(Debug, Clone)]
pub struct SignatureEntry {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub body_html: String,
    pub body_text: Option<String>,
    pub is_default: bool,
    pub is_reply_default: bool,
}

/// Request to save a signature, emitted upward to the App for DB persistence.
#[derive(Debug, Clone)]
pub struct SignatureSaveRequest {
    pub id: Option<String>,
    pub account_id: String,
    pub name: String,
    pub body_html: String,
    pub is_default: bool,
    pub is_reply_default: bool,
}

/// Editing state for the signature editor sheet.
#[derive(Debug, Clone)]
pub struct SignatureEditorState {
    /// The signature being edited (None = new).
    pub signature_id: Option<String>,
    pub account_id: String,
    pub name: UndoableText,
    /// Rich text editor state for the signature body.
    pub body_editor: RteEditorState,
    /// Signature kept as HTML because the editor cannot hold it (images,
    /// tables), shown as a rendered preview instead of the editor.
    pub raw_html: Option<String>,
    pub preview: crate::ui::mail_body::HtmlBodyState,
    /// Why the last HTML import failed, shown under the preview.
    pub import_error: Option<String>,
    pub is_default: bool,
    pub is_reply_default: bool,
    /// Whether fields have been modified since opening the editor.
    pub dirty: bool,
}

impl SignatureEditorState {
    pub fn blank() -> Self {
        Self {
            signature_id: None,
            account_id: String::new(),
            name: UndoableText::new(),
            body_editor: RteEditorState::new(),
            raw_html: None,
            preview: crate::ui::mail_body::HtmlBodyState::default(),
            import_error: None,
            is_default: false,
            is_reply_default: false,
            dirty: false,
        }
    }

    pub fn for_signature(sig: &SignatureEntry) -> Self {
        let raw_html = needs_raw_html(&sig.body_html).then(|| sig.body_html.clone());
        Self {
            signature_id: Some(sig.id.clone()),
            account_id: sig.account_id.clone(),
            name: UndoableText::with_initial(&sig.name),
            body_editor: RteEditorState::from_html(&sig.body_html),
            raw_html,
            preview: crate::ui::mail_body::HtmlBodyState::default(),
            import_error: None,
            is_default: sig.is_default,
            is_reply_default: sig.is_reply_default,
            dirty: false,
        }
    }

    pub fn body_html(&self) -> String {
        self.raw_html
            .clone()
            .unwrap_or_else(|| self.body_editor.to_html())
    }
}

/// Whether editing this HTML in the rich text editor would lose part of it.
fn needs_raw_html(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    ["<img", "<table", "<style"]
        .iter()
        .any(|tag| lower.contains(tag))
}
