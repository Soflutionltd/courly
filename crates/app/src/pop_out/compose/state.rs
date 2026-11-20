use rte::EditorState;
use rust_i18n::t;

use crate::db;
use crate::ui::emoji_picker::EmojiCategory;
use crate::ui::layout::{COMPOSE_DEFAULT_HEIGHT, COMPOSE_DEFAULT_WIDTH};
use crate::ui::token_input::{self, TokenId, TokenInputValue};

use super::helpers::{accounts_to_info, csv_to_token_input};
use super::messages::ComposeMode;
use super::quote::{HtmlBlock, ReplySource, draft_html, outgoing_body, split_draft_html};
use super::types::{
    AccountInfo, AutocompleteState, BccNudgeBanner, BulkPasteBanner, ComposeAttachment,
    ComposeTokenDrag, RecipientField, TokenContextMenuState,
};

/// Per-window state for a compose pop-out.
pub struct ComposeState {
    // Recipients
    pub to: TokenInputValue,
    pub cc: TokenInputValue,
    pub bcc: TokenInputValue,
    pub show_cc: bool,
    /// Ask the recipients of this message for a read receipt. Starts from
    /// the general setting, changed per message from the footer.
    pub request_read_receipt: bool,
    pub show_bcc: bool,
    pub selected_to_token: Option<TokenId>,
    pub selected_cc_token: Option<TokenId>,
    pub selected_bcc_token: Option<TokenId>,

    // From account
    pub from_account: Option<AccountInfo>,
    pub from_accounts: Vec<AccountInfo>,
    pub from_dropdown_open: bool,

    // Subject
    /// The subject without its reply or forward mark, as the field shows
    /// and edits it.
    pub subject: String,
    /// The mark the subject is sent with: see `full_subject`.
    pub subject_kind: crate::subject::SubjectKind,

    // Body (rich text editor)
    pub body: EditorState,

    /// Signature and original message of a reply or forward, kept outside
    /// the editor so their HTML reaches the recipient untouched.
    pub signature: Option<HtmlBlock>,
    pub quote: Option<HtmlBlock>,

    // Compose mode
    pub mode: ComposeMode,

    /// Shown in the main window rather than in its own window.
    pub inline: bool,

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
    /// The recipient row under the pointer, where a dragged bubble lands.
    pub drop_target: Option<RecipientField>,

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
    /// The color, font or size menu of the formatting toolbar that is open.
    pub format_bar: crate::ui::widgets::FormatBarState,
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
            request_read_receipt: false,
            show_bcc: false,
            selected_to_token: None,
            selected_cc_token: None,
            selected_bcc_token: None,
            from_account,
            from_accounts,
            from_dropdown_open: false,
            subject: String::new(),
            subject_kind: crate::subject::SubjectKind::Plain,
            body: EditorState::new(),
            signature: None,
            quote: None,
            mode: ComposeMode::New,
            inline: false,
            reply_thread_id: None,
            reply_message_id: None,
            reply_source_message_id: None,
            status: None,
            recipients_error: None,
            discard_confirm_open: false,
            autocomplete: AutocompleteState::new(),
            context_menu: None,
            drag: None,
            drop_target: None,
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
            format_bar: crate::ui::widgets::FormatBarState::default(),
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

    /// Start from the sender `(account id, address)` when it is still on
    /// offer, instead of the first account.
    pub fn with_from(mut self, preferred: Option<&(String, String)>) -> Self {
        let remembered = preferred.and_then(|(id, email)| {
            self.from_accounts
                .iter()
                .find(|account| &account.id == id && &account.email == email)
        });
        if let Some(account) = remembered {
            self.from_account = Some(account.clone());
        }
        self
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

        let parts = draft.body_html.as_deref().map(split_draft_html);
        let body = parts
            .as_ref()
            .map(|p| p.editor_html)
            .filter(|h| !h.trim().is_empty())
            .map(EditorState::from_html)
            .unwrap_or_default();
        let (signature, quote) = parts.map(|p| (p.signature, p.quote)).unwrap_or_default();

        let signature_separator_index = draft
            .signature_separator_index
            .filter(|&i| i >= 0)
            .and_then(|i| usize::try_from(i).ok());

        // A draft is stored with its mark; the field shows the subject alone.
        let (subject_kind, subject) =
            crate::subject::split(draft.subject.as_deref().unwrap_or_default());

        Self {
            to,
            cc,
            bcc,
            show_cc,
            request_read_receipt: false,
            show_bcc,
            selected_to_token: None,
            selected_cc_token: None,
            selected_bcc_token: None,
            from_account,
            from_accounts,
            from_dropdown_open: false,
            subject: subject.to_owned(),
            subject_kind,
            body,
            signature,
            quote,
            mode: ComposeMode::New,
            inline: false,
            reply_thread_id: draft.thread_id.clone(),
            reply_message_id: draft.reply_to_message_id.clone(),
            reply_source_message_id: None,
            status: None,
            recipients_error: None,
            discard_confirm_open: false,
            autocomplete: AutocompleteState::new(),
            context_menu: None,
            drag: None,
            drop_target: None,
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
            format_bar: crate::ui::widgets::FormatBarState::default(),
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

    pub fn with_recipient(mut self, email: String) -> Self {
        push_recipient(&mut self.to, None, email);
        self
    }

    pub fn new_reply(
        accounts: &[db::Account],
        mode: &ComposeMode,
        source: &ReplySource<'_>,
    ) -> Self {
        let mut state = Self::new(accounts);
        state.mode = mode.clone();
        // A reply or a forward leaves from the account that holds the
        // message, not from the first account of the list.
        if let Some(account) = source
            .account_id
            .and_then(|id| state.from_accounts.iter().find(|account| account.id == id))
        {
            state.from_account = Some(account.clone());
        }

        state.set_marked_subject(&mode.prefixed_subject());

        let (to, cc) = reply_recipients(accounts, &state.mode, source);
        for (name, email) in to {
            push_recipient(&mut state.to, name, email);
        }
        for (name, email) in cc {
            push_recipient(&mut state.cc, name, email);
        }
        if !state.cc.tokens.is_empty() {
            state.show_cc = true;
        }

        state.quote = HtmlBlock::quote(source);

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
            ComposeMode::New => t!("compose.new_message").into_owned(),
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
            account_name: Some(t!("compose.shared_mailbox", email = shared_email).into_owned()),
        };
        self.from_accounts.insert(0, shared_info.clone());
        self.from_account = Some(shared_info);
    }

    /// HTML and plain text sent to the recipient. A signature region left
    /// in the editor by an older draft loses its marker rule: the only rule
    /// the recipient sees is the one above the quoted original.
    pub fn outgoing_body(&self) -> (String, String) {
        let mut document = self.body.document.clone();
        if let Some(index) = self.signature_separator_index
            && matches!(document.block(index), Some(&rte::Block::HorizontalRule))
        {
            document.remove_block(index);
        }
        outgoing_body(
            &rte::html_serialize::to_html(&document),
            &document.flattened_text(),
            self.signature.as_ref(),
            self.quote.as_ref(),
        )
    }

    /// Take a subject as it is sent or stored, "Re: Invoice": the field
    /// gets the subject alone and the mark is kept aside.
    pub fn set_marked_subject(&mut self, marked: &str) {
        let (kind, bare) = crate::subject::split(marked);
        self.subject_kind = kind;
        self.subject = bare.to_owned();
    }

    /// The subject as it is sent and stored: with the one mark a reply or a
    /// forward carries. The field never shows that mark, an arrow stands
    /// for it, but the recipient's mail client files the message by it.
    pub fn full_subject(&self) -> String {
        match self.subject_kind {
            crate::subject::SubjectKind::Plain => self.subject.clone(),
            kind => crate::subject::marked(kind, &self.subject),
        }
    }

    /// HTML persisted for a draft, signature and quoted original included.
    pub fn draft_html(&self) -> String {
        draft_html(
            &self.body.to_html(),
            self.signature.as_ref(),
            self.quote.as_ref(),
        )
    }

    /// Returns true if the compose body has user content beyond the
    /// initial quoted text / signature.
    pub fn has_user_content(&self) -> bool {
        let body_text = self.body.document.flattened_text();
        let trimmed = body_text.trim();
        !trimmed.is_empty()
    }
}

type Recipient = (Option<String>, String);

/// Outlook's rules: replying to a message we sent goes back to its original
/// recipients, Reply All keeps the original To and Cc, and our own addresses
/// are never added.
fn reply_recipients(
    accounts: &[db::Account],
    mode: &ComposeMode,
    source: &ReplySource<'_>,
) -> (Vec<Recipient>, Vec<Recipient>) {
    use rtsk::seen_addresses::parse::parse_address_list;

    let is_own = |email: &str| accounts.iter().any(|a| a.email.eq_ignore_ascii_case(email));
    let sender: Vec<Recipient> = source
        .from_address
        .map(|email| {
            let name = source.from_name.filter(|n| !n.is_empty()).map(String::from);
            vec![(name, email.to_string())]
        })
        .unwrap_or_default();
    let original_to = parse_address_list(source.to_addresses.unwrap_or_default());
    let original_cc = parse_address_list(source.cc_addresses.unwrap_or_default());
    let from_self = source.from_address.is_some_and(is_own);

    let (to, cc) = match mode {
        ComposeMode::New | ComposeMode::Forward { .. } => return (Vec::new(), Vec::new()),
        ComposeMode::Reply { .. } if from_self => (original_to, Vec::new()),
        ComposeMode::Reply { .. } => (sender.clone(), Vec::new()),
        ComposeMode::ReplyAll { .. } if from_self => (original_to, original_cc),
        ComposeMode::ReplyAll { .. } => (
            sender.iter().cloned().chain(original_to).collect(),
            original_cc,
        ),
    };

    let mut seen: Vec<String> = Vec::new();
    let mut keep = |list: Vec<Recipient>| -> Vec<Recipient> {
        list.into_iter()
            .filter(|(_, email)| {
                let key = email.to_lowercase();
                if is_own(email) || seen.contains(&key) {
                    return false;
                }
                seen.push(key);
                true
            })
            .collect()
    };
    let to = keep(to);
    let cc = keep(cc);
    if to.is_empty() && cc.is_empty() {
        // A note to ourselves: answer it to ourselves rather than to nobody.
        return (sender, Vec::new());
    }
    (to, cc)
}

fn push_recipient(input: &mut TokenInputValue, name: Option<String>, email: String) {
    let id = input.next_token_id();
    input.tokens.push(token_input::Token {
        id,
        label: name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| email.clone()),
        email,
        is_group: false,
        group_id: None,
        member_count: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(email: &str) -> db::Account {
        db::Account {
            id: email.to_string(),
            email: email.to_string(),
            display_name: None,
            provider: "imap".to_string(),
            account_name: None,
            account_color: None,
            last_sync_at: None,
            token_expires_at: None,
            is_active: true,
            sort_order: 0,
            is_deleting: false,
        }
    }

    fn emails(list: &[Recipient]) -> Vec<&str> {
        list.iter().map(|(_, email)| email.as_str()).collect()
    }

    fn reply() -> ComposeMode {
        ComposeMode::Reply {
            original_subject: String::new(),
        }
    }

    fn reply_all() -> ComposeMode {
        ComposeMode::ReplyAll {
            original_subject: String::new(),
        }
    }

    const ME: &str = "me@example.com";

    fn sender(state: &ComposeState) -> Option<&str> {
        state.from_account.as_ref().map(|a| a.email.as_str())
    }

    #[test]
    fn a_reply_leaves_from_the_account_that_received_the_message() {
        let accounts = [account(ME), account("work@example.com")];
        let source = ReplySource {
            from_address: Some("ann@example.com"),
            account_id: Some("work@example.com"),
            ..Default::default()
        };
        let state = ComposeState::new_reply(&accounts, &reply(), &source);
        assert_eq!(sender(&state), Some("work@example.com"));
    }

    #[test]
    fn a_reply_shows_its_subject_without_the_mark_it_is_sent_with() {
        let accounts = [account(ME)];
        let mode = ComposeMode::Reply {
            original_subject: "RE: Re: Test".to_owned(),
        };
        let mut state = ComposeState::new_reply(&accounts, &mode, &ReplySource::default());
        assert_eq!(state.subject, "Test");
        assert_eq!(state.full_subject(), "Re: Test");

        // The reader retitles it: the mark stays, once.
        state.subject = "Test, suite".to_owned();
        assert_eq!(state.full_subject(), "Re: Test, suite");

        // A new message is sent as typed, even if it looks marked.
        let mut new_message = ComposeState::new(&accounts);
        new_message.subject = "Re: the plan".to_owned();
        assert_eq!(new_message.full_subject(), "Re: the plan");
    }

    #[test]
    fn a_reply_without_a_known_account_keeps_the_first_one() {
        let accounts = [account(ME), account("work@example.com")];
        let source = ReplySource {
            from_address: Some("ann@example.com"),
            account_id: Some("gone@example.com"),
            ..Default::default()
        };
        let state = ComposeState::new_reply(&accounts, &reply(), &source);
        assert_eq!(sender(&state), Some(ME));
    }

    #[test]
    fn a_new_message_keeps_the_sender_picked_last_time() {
        let accounts = [account(ME), account("work@example.com")];
        let picked = (
            "work@example.com".to_string(),
            "work@example.com".to_string(),
        );
        let state = ComposeState::new(&accounts).with_from(Some(&picked));
        assert_eq!(sender(&state), Some("work@example.com"));
    }

    #[test]
    fn a_new_message_starts_from_the_first_account_without_a_usable_pick() {
        let accounts = [account(ME), account("work@example.com")];
        assert_eq!(
            sender(&ComposeState::new(&accounts).with_from(None)),
            Some(ME)
        );
        let removed = ("old@example.com".to_string(), "old@example.com".to_string());
        let state = ComposeState::new(&accounts).with_from(Some(&removed));
        assert_eq!(sender(&state), Some(ME));
    }

    #[test]
    fn reply_goes_to_the_sender() {
        let source = ReplySource {
            from_name: Some("Ann"),
            from_address: Some("ann@example.com"),
            to_addresses: Some("Me <ME@example.com>, bob@example.com"),
            ..Default::default()
        };
        let (to, cc) = reply_recipients(&[account(ME)], &reply(), &source);
        assert_eq!(emails(&to), ["ann@example.com"]);
        assert!(cc.is_empty());
    }

    #[test]
    fn reply_all_keeps_everyone_but_us() {
        let source = ReplySource {
            from_address: Some("ann@example.com"),
            to_addresses: Some("Me <me@example.com>, Bob <bob@example.com>"),
            cc_addresses: Some("carl@example.com, ann@example.com"),
            ..Default::default()
        };
        let (to, cc) = reply_recipients(&[account(ME)], &reply_all(), &source);
        assert_eq!(emails(&to), ["ann@example.com", "bob@example.com"]);
        assert_eq!(emails(&cc), ["carl@example.com"]);
    }

    #[test]
    fn replying_to_our_own_message_goes_to_its_recipients() {
        let source = ReplySource {
            from_address: Some(ME),
            to_addresses: Some("Bob <bob@example.com>"),
            cc_addresses: Some("carl@example.com"),
            ..Default::default()
        };
        let (to, cc) = reply_recipients(&[account(ME)], &reply(), &source);
        assert_eq!(emails(&to), ["bob@example.com"]);
        assert!(cc.is_empty());
        let (to, cc) = reply_recipients(&[account(ME)], &reply_all(), &source);
        assert_eq!(emails(&to), ["bob@example.com"]);
        assert_eq!(emails(&cc), ["carl@example.com"]);
    }

    #[test]
    fn a_note_to_ourselves_is_answered_to_ourselves() {
        let source = ReplySource {
            from_address: Some(ME),
            to_addresses: Some(ME),
            ..Default::default()
        };
        let (to, _) = reply_recipients(&[account(ME)], &reply(), &source);
        assert_eq!(emails(&to), [ME]);
    }
}
