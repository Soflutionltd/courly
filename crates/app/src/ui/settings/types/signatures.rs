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
    /// The color, font or size menu of the formatting toolbar that is open.
    pub format_bar: crate::ui::widgets::FormatBarState,
    /// Signature kept as HTML because the editor cannot hold it (images,
    /// tables), shown as a rendered preview instead of the editor.
    pub raw_html: Option<String>,
    pub preview: crate::ui::mail_body::HtmlBodyState,
    /// Why the last HTML import failed, shown under the preview.
    pub import_error: Option<String>,
    /// What Save found missing, shown next to the button until it is filled.
    pub save_error: Option<String>,
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
            format_bar: crate::ui::widgets::FormatBarState::default(),
            raw_html: None,
            preview: crate::ui::mail_body::HtmlBodyState::default(),
            import_error: None,
            save_error: None,
            is_default: false,
            is_reply_default: false,
            dirty: false,
        }
    }

    pub fn for_signature(sig: &SignatureEntry) -> Self {
        let mut editor = Self {
            signature_id: Some(sig.id.clone()),
            account_id: sig.account_id.clone(),
            name: UndoableText::with_initial(&sig.name),
            is_default: sig.is_default,
            is_reply_default: sig.is_reply_default,
            ..Self::blank()
        };
        editor.set_body(sig.body_html.clone());
        editor
    }

    /// Take `html` as the signature body: in the rich text editor when it
    /// can hold it, as HTML shown in the preview otherwise. Returns true
    /// when the preview is ready to render.
    pub fn set_body(&mut self, html: String) -> bool {
        // A signature copied out of a mail client carries the widths its
        // tables had on screen: with them a cell cannot grow as it is edited.
        let mut html = html;
        html = crate::signature_html::free_measured_table_widths(&html);
        if needs_raw_html(&html) {
            self.raw_html = Some(html);
            return self.preview.restart();
        }
        self.body_editor = RteEditorState::from_html(&html);
        self.raw_html = None;
        false
    }

    pub fn body_html(&self) -> String {
        self.raw_html
            .clone()
            .unwrap_or_else(|| self.body_editor.to_html())
    }
}

/// Whether editing this HTML in the rich text editor would lose part of it:
/// a style sheet, whose rules the editor does not carry.
fn needs_raw_html(html: &str) -> bool {
    html.to_ascii_lowercase().contains("<style")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_for(body_html: &str) -> SignatureEditorState {
        SignatureEditorState::for_signature(&SignatureEntry {
            id: "signature-1".into(),
            account_id: "account-1".into(),
            name: "MonApp".into(),
            body_html: body_html.into(),
            body_text: None,
            is_default: false,
            is_reply_default: false,
        })
    }

    #[test]
    fn a_signature_with_an_image_on_its_own_line_opens_in_the_editor() {
        let editor = editor_for("<p>MonApp</p><img src=\"cid:logo\" width=\"120\" height=\"40\">");
        assert_eq!(editor.raw_html, None);
        assert!(matches!(
            editor.body_editor.document.block(1),
            Some(rte::Block::Image {
                width: Some(120),
                ..
            })
        ));
    }

    #[test]
    fn a_signature_with_a_linked_image_in_its_text_opens_in_the_editor_unchanged() {
        let body_html = "<p>MonApp <a href=\"https://example.com\">\
            <img src=\"cid:logo\" width=\"31\" height=\"31\"></a></p>";
        let editor = editor_for(body_html);
        assert_eq!(editor.raw_html, None);
        assert_eq!(editor.body_html(), body_html);
    }

    #[test]
    fn a_signature_laid_out_in_a_table_opens_in_the_editor_with_its_attributes() {
        let body_html = "<table cellpadding=\"0\" style=\"width:520px\"><tr>\
            <td style=\"padding:0cm 13.5pt 0cm 0.75pt;width:150pt\">\
            <p style=\"margin:0\">MonApp</p></td><td></td></tr></table>";
        let editor = editor_for(body_html);
        assert_eq!(editor.raw_html, None);
        assert!(matches!(
            editor.body_editor.document.block(0),
            Some(rte::Block::Table(_))
        ));
        assert_eq!(editor.body_html(), body_html);
    }

    #[test]
    fn a_signature_carrying_a_style_sheet_stays_as_html() {
        let body_html = "<style>p{margin:0}</style><p>MonApp</p>";
        let editor = editor_for(body_html);
        assert_eq!(editor.raw_html.as_deref(), Some(body_html));
        assert_eq!(editor.body_html(), body_html);
    }
}
