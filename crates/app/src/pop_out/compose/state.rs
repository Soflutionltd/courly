use rte::EditorState;

use crate::db;
use crate::ui::emoji_picker::EmojiCategory;
use crate::ui::layout::{COMPOSE_DEFAULT_HEIGHT, COMPOSE_DEFAULT_WIDTH};
use crate::ui::token_input::{self, TokenId, TokenInputValue};

use super::helpers::{accounts_to_info, csv_to_token_input};
use super::messages::ComposeMode;
use super::quote::{QuotedOriginal, ReplySource, split_draft_html};
use super::types::{
    AccountInfo, AutocompleteState, BccNudgeBanner, BulkPasteBanner, ComposeAttachment,
    ComposeTokenDrag, TokenContextMenuState,
};

/// Per-window state for a compose pop-out.
pub struct ComposeState {
    // Recipients
    pub to: TokenInputValue,
    pub cc: TokenInputValue,
    pub bcc: TokenInputValue,
    pub show_cc: bool,
    pub show_bcc: bool,
    pub selected_to_token: Option<TokenId>,
    pub selected_cc_token: Option<TokenId>,
    pub selected_bcc_token: Option<TokenId>,

    // From account
    pub from_account: Option<AccountInfo>,
    pub from_accounts: Vec<AccountInfo>,
    pub from_dropdown_open: bool,

    // Subject
    pub subject: String,

    // Body (rich text editor)
    pub body: EditorState,

    /// Original message carried under a reply or forward, outside the
    /// editor so its HTML reaches the recipient untouched.
    pub quote: Option<QuotedOriginal>,

    // Compose mode
    pub mode: ComposeMode,

    // Reply context
    pub reply_thread_id: Option<String>,
    pub reply_message_id: Option<String>,
    pub reply_source_message_id: Option<String>,

    // Status message (e.g. "Send not yet wired")
    pub status: Option<String>,

    /// Inline validation error shown directly under the To row when the
    /// user tries to send with no recipients. Cleared as soon as any
    /// recipient is added so the message disappears the moment the
    /// problem is fixed - no separate "dismiss" UI needed.
    pub recipients_error: Option<String>,

    // Discard confirmation
    pub discard_confirm_open: bool,

    // Autocomplete
    pub autocomplete: AutocompleteState,

    // Attachments
    pub attachments: Vec<ComposeAttachment>,

    // Context menu
    pub context_menu: Option<TokenContextMenuState>,

    // Drag and drop
    pub drag: Option<ComposeTokenDrag>,

    // Banners
    pub bcc_nudges: Vec<BccNudgeBanner>,
    pub bulk_paste_banner: Option<BulkPasteBanner>,

    // Link dialog
    pub link_dialog_open: bool,
    pub link_url: String,
    pub link_text: String,

    // Save-as-group dialog (driven from the bulk-paste banner)
    pub save_group_dialog_open: bool,
    pub save_group_name: String,
    pub save_group_error: Option<String>,
    pub save_group_in_flight: bool,

    // Emoji picker (toolbar overlay)
    pub emoji_picker_open: bool,
    pub emoji_picker_query: String,
    pub emoji_picker_category: EmojiCategory,

    // Window geometry
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,

    // Signature tracking
    pub active_signature_id: Option<String>,
    pub signature_separator_index: Option<usize>,

    // Draft auto-save
    pub draft_id: String,
    pub draft_dirty: bool,

    // Send in progress - disables Send button, shows "Sending..." status
    pub sending: bool,

    // Draft ID for the send path - set on first send attempt, reused on retry
    // so that failed retries update the existing draft row instead of creating
    // a new one.
    pub send_draft_id: Option<String>,
}

impl ComposeState {
    pub fn new(accounts: &[db::Account]) -> Self {
        let from_accounts = accounts_to_info(accounts);
        let from_account = from_accounts.first().cloned();
        Self {
            to: TokenInputValue::new(),
            cc: TokenInputValue::new(),
            bcc: TokenInputValue::new(),
            show_cc: false,
            show_bcc: false,
            selected_to_token: None,
            selected_cc_token: None,
            selected_bcc_token: None,
            from_account,
            from_accounts,
            from_dropdown_open: false,
            subject: String::new(),
            body: EditorState::new(),
            quote: None,
            mode: ComposeMode::New,
            reply_thread_id: None,
            reply_message_id: None,
            reply_source_message_id: None,
            status: None,
            recipients_error: None,
            discard_confirm_open: false,
            autocomplete: AutocompleteState::new(),
            context_menu: None,
            drag: None,
            bcc_nudges: Vec::new(),
            bulk_paste_banner: None,
            attachments: Vec::new(),
            link_dialog_open: false,
            link_url: String::new(),
            link_text: String::new(),
            save_group_dialog_open: false,
            save_group_name: String::new(),
            save_group_error: None,
            save_group_in_flight: false,
            emoji_picker_open: false,
            emoji_picker_query: String::new(),
            emoji_picker_category: EmojiCategory::Smileys,
            active_signature_id: None,
            signature_separator_index: None,
            width: COMPOSE_DEFAULT_WIDTH,
            height: COMPOSE_DEFAULT_HEIGHT,
            x: None,
            y: None,
            draft_id: uuid::Uuid::new_v4().to_string(),
            draft_dirty: false,
            sending: false,
            send_draft_id: None,
        }
    }

    /// Restore a compose window from a persisted local draft.
    ///
    /// Reconstructs the body from saved HTML and restores the signature
    /// separator index and active signature ID so the editor knows where
    /// the signature boundary is.
    pub fn from_local_draft(
        accounts: &[db::Account],
        draft: &rtsk::db::types::DbLocalDraft,
    ) -> Self {
        let from_accounts = accounts_to_info(accounts);
        let from_account = draft
            .from_email
            .as_ref()
            .and_then(|email| from_accounts.iter().find(|a| &a.email == email).cloned())
            .or_else(|| from_accounts.first().cloned());

        let to = csv_to_token_input(draft.to_addresses.as_deref());
        let cc = csv_to_token_input(draft.cc_addresses.as_deref());
        let bcc = csv_to_token_input(draft.bcc_addresses.as_deref());
        let show_cc = !cc.tokens.is_empty();
        let show_bcc = !bcc.tokens.is_empty();

        let (editor_html, quote) = draft
            .body_html
            .as_deref()
            .map(split_draft_html)
            .unwrap_or_default();
        let body = Some(editor_html)
            .filter(|h| !h.trim().is_empty())
            .map(EditorState::from_html)
            .unwrap_or_default();

        let signature_separator_index = draft
            .signature_separator_index
            .filter(|&i| i >= 0)
            .and_then(|i| usize::try_from(i).ok());

        Self {
            to,
            cc,
            bcc,
            show_cc,
            show_bcc,
            selected_to_token: None,
            selected_cc_token: None,
            selected_bcc_token: None,
            from_account,
            from_accounts,
            from_dropdown_open: false,
            subject: draft.subject.clone().unwrap_or_default(),
            body,
            quote,
            mode: ComposeMode::New,
            reply_thread_id: draft.thread_id.clone(),
            reply_message_id: draft.reply_to_message_id.clone(),
            reply_source_message_id: None,
            status: None,
            recipients_error: None,
            discard_confirm_open: false,
            autocomplete: AutocompleteState::new(),
            context_menu: None,
            drag: None,
            bcc_nudges: Vec::new(),
            bulk_paste_banner: None,
            attachments: Vec::new(),
            link_dialog_open: false,
            link_url: String::new(),
            link_text: String::new(),
            save_group_dialog_open: false,
            save_group_name: String::new(),
            save_group_error: None,
            save_group_in_flight: false,
            emoji_picker_open: false,
            emoji_picker_query: String::new(),
            emoji_picker_category: EmojiCategory::Smileys,
            active_signature_id: draft.signature_id.clone(),
            signature_separator_index,
            width: COMPOSE_DEFAULT_WIDTH,
            height: COMPOSE_DEFAULT_HEIGHT,
            x: None,
            y: None,
            draft_id: draft.id.clone(),
            draft_dirty: false,
            sending: false,
            send_draft_id: None,
        }
    }

    pub fn new_reply(
        accounts: &[db::Account],
        mode: &ComposeMode,
        source: &ReplySource<'_>,
    ) -> Self {
        let mut state = Self::new(accounts);
        state.mode = mode.clone();

        state.subject = mode.prefixed_subject();

        if !matches!(state.mode, ComposeMode::Forward { .. })
            && let Some(email) = source.from_address
        {
            let label = source.from_name.filter(|n| !n.is_empty()).unwrap_or(email);
            let id = state.to.next_token_id();
            state.to.tokens.push(token_input::Token {
                id,
                email: email.to_string(),
                label: label.to_string(),
                is_group: false,
                group_id: None,
                member_count: None,
            });
        }

        if let ComposeMode::ReplyAll { .. } = &state.mode
            && let Some(cc_str) = source.cc_addresses
        {
            for addr in cc_str.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                let id = state.cc.next_token_id();
                state.cc.tokens.push(token_input::Token {
                    id,
                    email: addr.to_string(),
                    label: addr.to_string(),
                    is_group: false,
                    group_id: None,
                    member_count: None,
                });
            }
            if !state.cc.tokens.is_empty() {
                state.show_cc = true;
            }
        }

        state.quote = QuotedOriginal::from_source(source);

        state.reply_thread_id = source.thread_id.map(String::from);
        state.reply_source_message_id = source.message_id.map(String::from);
        state.reply_message_id = source
            .message_id_header
            .or(source.message_id)
            .map(String::from);

        state
    }

    /// Window title based on compose mode.
    pub fn window_title(&self) -> String {
        match &self.mode {
            ComposeMode::New => "New Message".to_string(),
            _ => self.mode.prefixed_subject(),
        }
    }

    /// Override the From identity for a shared mailbox context.
    ///
    /// Inserts a synthetic `AccountInfo` for the shared mailbox email at the
    /// front of the From dropdown and selects it. The `parent_account_id` is
    /// the authenticated account that will actually perform the send.
    pub fn set_shared_mailbox_from(&mut self, parent_account_id: &str, shared_email: &str) {
        let shared_info = AccountInfo {
            id: parent_account_id.to_string(),
            email: shared_email.to_string(),
            display_name: None,
            account_name: Some(format!("{shared_email} (shared)")),
        };
        self.from_accounts.insert(0, shared_info.clone());
        self.from_account = Some(shared_info);
    }

    /// HTML and plain text sent to the recipient. The rule that marks the
    /// signature region in the editor is left out: the only rule the
    /// recipient sees is the one above the quoted original.
    pub fn outgoing_body(&self) -> (String, String) {
        let mut document = self.body.document.clone();
        if let Some(index) = self.signature_separator_index
            && matches!(document.block(index), Some(&rte::Block::HorizontalRule))
        {
            document.remove_block(index);
        }
        let editor_html = rte::html_serialize::to_html(&document);
        let editor_text = document.flattened_text();
        match &self.quote {
            Some(quote) => (
                quote.outgoing_html(&editor_html),
                quote.outgoing_text(&editor_text),
            ),
            None => (editor_html, editor_text),
        }
    }

    /// HTML persisted for a draft, quoted original included.
    pub fn draft_html(&self) -> String {
        let editor_html = self.body.to_html();
        match &self.quote {
            Some(quote) => quote.draft_html(&editor_html),
            None => editor_html,
        }
    }

    /// Returns true if the compose body has user content beyond the
    /// initial quoted text / signature.
    pub fn has_user_content(&self) -> bool {
        let body_text = self.body.document.flattened_text();
        let trimmed = body_text.trim();
        !trimmed.is_empty()
    }
}
