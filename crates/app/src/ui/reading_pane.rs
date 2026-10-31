use std::collections::HashMap;

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length, Padding, Task};
use rust_i18n::t;

use cmdk::{BindingTable, CommandContext, CommandId, CommandRegistry};

use crate::Message;
use crate::component::Component;
use crate::db::{
    AppThreadDetail, DateDisplay, ResolvedLabel, Thread, ThreadAttachment, ThreadMessage,
};
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;

// ── Messages & Events ──────────────────────────────────

#[derive(Debug, Clone)]
pub enum ReadingPaneMessage {
    ToggleMessageExpanded(usize),
    ToggleAllMessages,
    /// Unfold or fold the Preview / Open / Download menu of the attachment
    /// with this id.
    ToggleAttachmentMenu(String),
    CloseAttachmentMenu,
    /// Quick-look the attachment with this id.
    PreviewAttachment(String),
    /// Open the attachment with this id in the system default handler.
    OpenAttachment(String),
    /// Save the attachment with this id through a file picker.
    SaveAttachment(String),
    PopOut(usize),
    /// Reply to the message at the given index.
    Reply(usize),
    /// Reply All to the message at the given index.
    ReplyAll(usize),
    /// Forward the message at the given index.
    Forward(usize),
    /// Open the inline contact editor popover for the given email address.
    EditContact(String),
    /// Create a calendar event from this message (calendar button).
    CreateEventFromEmail(usize),
    /// Edit or answer the invite card of the message with this id.
    Invite(String, super::invite_card::InviteMessage),
    /// The invite of the message with this id resolved against the calendar.
    InviteLoaded(
        String,
        Result<Option<Box<super::invite_card::InviteDetails>>, String>,
    ),
    /// Navigate to the next message in the thread (expand it).
    NextMessage,
    /// Navigate to the previous message in the thread (expand it).
    PrevMessage,
    ToggleStar,
    /// User clicked a hyperlink in the HTML body; open it in the system browser.
    LinkClicked(String),
    /// The HTML body of the message at this index was laid out at a new width.
    HtmlBodyResized(usize, f32),
    /// A background HTML render finished.
    HtmlBodyRendered {
        index: usize,
        inputs: super::mail_body::RenderInputs,
        body: Option<std::sync::Arc<super::mail_body::RenderedBody>>,
    },
    /// Load remote images for the message at this index.
    ShowRemoteImages(usize),
    Noop,
}

/// Events the reading pane emits upward to the App.
#[derive(Debug, Clone)]
pub enum ReadingPaneEvent {
    /// User clicked the pop-out button on a message; open it in a new window.
    OpenMessagePopOut {
        message_index: usize,
    },
    /// User clicked Reply on a specific message.
    ReplyToMessage {
        message_index: usize,
    },
    /// User clicked Reply All on a specific message.
    ReplyAllToMessage {
        message_index: usize,
    },
    /// User clicked Forward on a specific message.
    ForwardMessage {
        message_index: usize,
    },
    /// User clicked a sender/recipient to edit their contact info.
    EditContact {
        email: String,
    },
    /// User clicked the calendar button on a message to create a calendar event.
    CreateEventFromEmail {
        message_index: usize,
    },
    /// Answer a synced meeting invite. Unsynced invites deliberately have
    /// no standalone iMIP mail-reply path.
    RespondToMeetingInvite {
        message_id: String,
        account_id: String,
        event_id: String,
        response: service_api::RsvpResponse,
        reply: service_api::WireRsvpReply,
    },
    /// User toggled the star on the thread.
    ToggleStar,
    /// User asked for a quick look at an attachment.
    PreviewAttachment(crate::handlers::attachments::OpenAttachmentParams),
    /// User clicked Open on an attachment chip.
    OpenAttachment(crate::handlers::attachments::OpenAttachmentParams),
    /// User clicked Download on an attachment chip.
    SaveAttachment(crate::handlers::attachments::SaveAttachmentParams),
}

// ── State ──────────────────────────────────────────────

pub struct ReadingPane {
    pub thread_messages: Vec<ThreadMessage>,
    pub thread_attachments: Vec<ThreadAttachment>,
    pub thread_labels: Vec<ResolvedLabel>,
    pub message_expanded: Vec<bool>,
    /// Index of the currently focused message (for keyboard navigation).
    pub focused_message: Option<usize>,
    pub date_display: DateDisplay,
    /// The currently viewed thread (set by App when a thread is selected).
    current_thread: Option<ThreadRef>,
    /// Search terms to highlight in message bodies. Empty when not in search mode.
    pub search_highlight_terms: Vec<String>,
    /// Attachment whose Preview / Open / Download menu is unfolded.
    attachment_menu: Option<String>,
    /// HTML render state, one per message.
    html_bodies: Vec<super::mail_body::HtmlBodyState>,
    /// Inline images for CID resolution in HTML bodies, keyed by Content-ID.
    inline_image_bytes: HashMap<String, mail_html::Bytes>,
    /// Meeting request cards, keyed by message id.
    invites: HashMap<String, super::invite_card::InviteCard>,
}

/// Minimal reference to the currently selected thread for display purposes.
#[derive(Debug, Clone)]
struct ThreadRef {
    account_id: String,
    id: String,
    subject: Option<String>,
    is_starred: bool,
}

/// Attachments roadmap Phase 5: which event variant to build inside
/// `build_attachment_event` for a single attachment chip click.
#[derive(Debug, Clone, Copy)]
enum AttachmentAction {
    Preview,
    Open,
    Save,
}

impl ReadingPane {
    /// Resolve an attachment id to a `ReadingPaneEvent`. Returns
    /// `None` if there's no current thread (nothing's loaded) or the
    /// id is unknown (defensive - the UI should only emit ids that
    /// exist in `thread_attachments`).
    fn build_attachment_event(
        &self,
        attachment_id: &str,
        action: AttachmentAction,
    ) -> Option<ReadingPaneEvent> {
        let thread = self.current_thread.as_ref()?;
        let row = self
            .thread_attachments
            .iter()
            .find(|a| a.id == attachment_id)?;
        let item = crate::handlers::attachments::AttachmentRef {
            account_id: thread.account_id.clone(),
            message_id: row.message_id.clone(),
            attachment_id: row.id.clone(),
            filename: row.filename.clone(),
            mime_type: row.mime_type.clone(),
        };
        Some(match action {
            AttachmentAction::Preview => ReadingPaneEvent::PreviewAttachment(
                crate::handlers::attachments::OpenAttachmentParams { item },
            ),
            AttachmentAction::Open => ReadingPaneEvent::OpenAttachment(
                crate::handlers::attachments::OpenAttachmentParams { item },
            ),
            AttachmentAction::Save => ReadingPaneEvent::SaveAttachment(
                crate::handlers::attachments::SaveAttachmentParams {
                    thread_id: thread.id.clone(),
                    item,
                },
            ),
        })
    }

    /// Start a background render of the HTML body at `index` when `start`
    /// is set and the body has a known width.
    fn start_html_render(&mut self, index: usize, start: bool) -> Task<ReadingPaneMessage> {
        if !start {
            return Task::none();
        }
        let Some(html) = self
            .thread_messages
            .get(index)
            .and_then(|msg| msg.body_html.clone())
        else {
            return Task::none();
        };
        let Some(inputs) = self
            .html_bodies
            .get_mut(index)
            .and_then(super::mail_body::HtmlBodyState::start)
        else {
            return Task::none();
        };
        let inline_images = self.inline_image_bytes.clone();
        Task::perform(
            super::mail_body::render(html, inputs, inline_images),
            move |body| ReadingPaneMessage::HtmlBodyRendered {
                index,
                inputs,
                body,
            },
        )
    }

    /// Re-render every open HTML body at a new reading zoom.
    pub fn set_reading_zoom(&mut self, zoom: f32) -> Task<ReadingPaneMessage> {
        let starts: Vec<usize> = self
            .html_bodies
            .iter_mut()
            .enumerate()
            .filter_map(|(index, state)| state.set_zoom(zoom).then_some(index))
            .collect();
        Task::batch(
            starts
                .into_iter()
                .map(|index| self.start_html_render(index, true)),
        )
    }

    pub fn new() -> Self {
        Self {
            thread_messages: Vec::new(),
            thread_attachments: Vec::new(),
            thread_labels: Vec::new(),
            message_expanded: Vec::new(),
            focused_message: None,
            date_display: DateDisplay::RelativeOffset,
            current_thread: None,
            search_highlight_terms: Vec::new(),
            attachment_menu: None,
            html_bodies: Vec::new(),
            inline_image_bytes: HashMap::new(),
            invites: HashMap::new(),
        }
    }

    /// Set the currently viewed thread. Called by App on thread selection.
    /// Whether the messages of this thread are loaded and on display.
    pub fn has_loaded_thread(&self, account_id: &str, thread_id: &str) -> bool {
        self.current_thread
            .as_ref()
            .is_some_and(|t| t.account_id == account_id && t.id == thread_id)
            && !self.thread_messages.is_empty()
    }

    pub fn set_thread(&mut self, thread: Option<&Thread>) {
        self.current_thread = thread.map(|t| ThreadRef {
            account_id: t.account_id.clone(),
            id: t.id.clone(),
            subject: t.subject.clone(),
            is_starred: t.is_starred,
        });
        self.thread_messages.clear();
        self.thread_attachments.clear();
        self.thread_labels.clear();
        self.message_expanded.clear();
        self.focused_message = None;
        self.attachment_menu = None;
        self.html_bodies.clear();
        self.inline_image_bytes.clear();
        self.invites.clear();
    }

    /// Apply message expansion rules after messages are loaded.
    ///
    /// Rules (messages are newest-first):
    /// 1. Unread messages are expanded
    /// 2. Most recent received message is expanded (the most recent message
    ///    when the thread only holds the user's own)
    /// 3. First message in thread (last index) is expanded
    /// 4. Own read messages are collapsed (unless rule 2 or 3 applies)
    /// 5. When search terms are present, messages containing any term are expanded
    pub fn apply_message_expansion(&mut self, messages: &[ThreadMessage]) {
        let len = messages.len();
        let mut expanded = vec![false; len];
        let latest = messages
            .iter()
            .position(|msg| !msg.is_own_message)
            .unwrap_or(0);

        if !self.search_highlight_terms.is_empty() {
            // Search mode: expand messages that contain any search term
            for (i, msg) in messages.iter().enumerate() {
                if message_matches_search_terms(msg, &self.search_highlight_terms) {
                    expanded[i] = true;
                }
            }
            // Always expand first and last if nothing matched
            if !expanded.iter().any(|&e| e) && len > 0 {
                expanded[0] = true;
                if len > 1 {
                    expanded[len - 1] = true;
                }
            }
        } else {
            for (i, msg) in messages.iter().enumerate() {
                // Rules 1-3: unread, most recent received, or initial message
                if !msg.is_read || i == latest || i == len - 1 {
                    expanded[i] = true;
                }
                // Rule 4: own messages default to collapsed
                if msg.is_own_message && msg.is_read && i != latest && i != len - 1 {
                    expanded[i] = false;
                }
            }
        }

        self.message_expanded = expanded;
    }

    /// Load thread detail from core's get_thread_detail response.
    pub fn load_thread_detail(&mut self, detail: AppThreadDetail) {
        // Update thread ref with detail data
        self.current_thread = Some(ThreadRef {
            account_id: detail.account_id.clone(),
            id: detail.thread_id.clone(),
            subject: detail.subject,
            is_starred: detail.is_starred,
        });

        // Apply expansion rules then set messages
        self.apply_message_expansion(&detail.messages);
        self.html_bodies = detail
            .messages
            .iter()
            .map(|_| super::mail_body::HtmlBodyState::default())
            .collect();

        self.invites = detail
            .messages
            .iter()
            .filter(|msg| {
                super::invite_card::InviteKind::of(msg)
                    == Some(super::invite_card::InviteKind::Request)
            })
            .map(|msg| (msg.id.clone(), super::invite_card::InviteCard::new(msg)))
            .collect();
        self.thread_messages = detail.messages;
        self.thread_attachments = detail.attachments;
        self.attachment_menu = None;
        self.thread_labels = detail.labels;
        self.inline_image_bytes = super::mail_body::inline_image_bytes(&detail.inline_images);
    }

    /// Invites still waiting for their calendar lookup, as
    /// `(message_id, account_id, uid)`. `rich_reply` says which accounts
    /// take comments and proposed times with a response.
    pub fn pending_invite_lookups(
        &mut self,
        rich_reply: impl Fn(&str) -> bool,
    ) -> Vec<(String, String, String)> {
        let mut lookups = Vec::new();
        for msg in &self.thread_messages {
            let Some(card) = self.invites.get_mut(&msg.id) else {
                continue;
            };
            card.rich_reply = rich_reply(&msg.account_id);
            if let (super::invite_card::InviteLookup::Loading, Some(uid)) =
                (&card.lookup, msg.meeting_invite_uid.as_ref())
            {
                lookups.push((msg.id.clone(), msg.account_id.clone(), uid.clone()));
            }
        }
        lookups
    }

    /// Settle the in-flight response of an invite card.
    pub fn finish_invite_response(&mut self, message_id: &str, result: Result<(), String>) {
        if let Some(card) = self.invites.get_mut(message_id) {
            card.finish(result);
        }
    }

    /// Update the star state for a thread if it's currently displayed.
    pub fn update_star(&mut self, account_id: &str, thread_id: &str, is_starred: bool) {
        if let Some(ref mut t) = self.current_thread
            && t.account_id == account_id
            && t.id == thread_id
        {
            t.is_starred = is_starred;
        }
    }

    /// Get the message ID of the currently focused message (for CommandContext).
    #[allow(dead_code)] // CommandContext consumer not yet calling this
    pub fn focused_message_id(&self) -> Option<String> {
        let idx = self.focused_message?;
        self.thread_messages.get(idx).map(|m| m.id.clone())
    }
}

// ── Component impl ─────────────────────────────────────

impl Component for ReadingPane {
    type Message = ReadingPaneMessage;
    type Event = ReadingPaneEvent;

    fn update(
        &mut self,
        message: ReadingPaneMessage,
    ) -> (Task<ReadingPaneMessage>, Option<ReadingPaneEvent>) {
        match message {
            ReadingPaneMessage::ToggleMessageExpanded(index) => {
                if let Some(e) = self.message_expanded.get_mut(index) {
                    *e = !*e;
                }
                (Task::none(), None)
            }
            ReadingPaneMessage::ToggleAllMessages => {
                let all_expanded = self.message_expanded.iter().all(|&e| e);
                self.message_expanded.fill(!all_expanded);
                (Task::none(), None)
            }
            ReadingPaneMessage::ToggleAttachmentMenu(att_id) => {
                self.attachment_menu = if self.attachment_menu.as_deref() == Some(att_id.as_str()) {
                    None
                } else {
                    Some(att_id)
                };
                (Task::none(), None)
            }
            ReadingPaneMessage::CloseAttachmentMenu => {
                self.attachment_menu = None;
                (Task::none(), None)
            }
            ReadingPaneMessage::PreviewAttachment(att_id) => {
                self.attachment_menu = None;
                let event = self.build_attachment_event(&att_id, AttachmentAction::Preview);
                (Task::none(), event)
            }
            ReadingPaneMessage::OpenAttachment(att_id) => {
                self.attachment_menu = None;
                let event = self.build_attachment_event(&att_id, AttachmentAction::Open);
                (Task::none(), event)
            }
            ReadingPaneMessage::SaveAttachment(att_id) => {
                self.attachment_menu = None;
                let event = self.build_attachment_event(&att_id, AttachmentAction::Save);
                (Task::none(), event)
            }
            ReadingPaneMessage::PopOut(index) => {
                let event = ReadingPaneEvent::OpenMessagePopOut {
                    message_index: index,
                };
                (Task::none(), Some(event))
            }
            ReadingPaneMessage::Reply(index) => (
                Task::none(),
                Some(ReadingPaneEvent::ReplyToMessage {
                    message_index: index,
                }),
            ),
            ReadingPaneMessage::ReplyAll(index) => (
                Task::none(),
                Some(ReadingPaneEvent::ReplyAllToMessage {
                    message_index: index,
                }),
            ),
            ReadingPaneMessage::Forward(index) => (
                Task::none(),
                Some(ReadingPaneEvent::ForwardMessage {
                    message_index: index,
                }),
            ),
            ReadingPaneMessage::EditContact(email) => {
                (Task::none(), Some(ReadingPaneEvent::EditContact { email }))
            }
            ReadingPaneMessage::CreateEventFromEmail(index) => (
                Task::none(),
                Some(ReadingPaneEvent::CreateEventFromEmail {
                    message_index: index,
                }),
            ),
            ReadingPaneMessage::Invite(message_id, invite_message) => {
                let Some(card) = self.invites.get_mut(&message_id) else {
                    return (Task::none(), None);
                };
                let tz = jiff::tz::TimeZone::system();
                match card.update(invite_message, &tz) {
                    Some(super::invite_card::InviteAction::Open(url)) => {
                        open_url_in_browser(&url);
                        (Task::none(), None)
                    }
                    Some(super::invite_card::InviteAction::Respond {
                        event_id,
                        response,
                        reply,
                    }) => {
                        let account_id = self
                            .thread_messages
                            .iter()
                            .find(|msg| msg.id == message_id)
                            .map(|msg| msg.account_id.clone());
                        let Some(account_id) = account_id else {
                            card.finish(Err(t!("main.reading_pane.message_not_open").into_owned()));
                            return (Task::none(), None);
                        };
                        (
                            Task::none(),
                            Some(ReadingPaneEvent::RespondToMeetingInvite {
                                message_id,
                                account_id,
                                event_id,
                                response,
                                reply,
                            }),
                        )
                    }
                    None => (Task::none(), None),
                }
            }
            ReadingPaneMessage::InviteLoaded(message_id, result) => {
                if let Some(card) = self.invites.get_mut(&message_id) {
                    card.set_lookup(match result {
                        Ok(Some(details)) => super::invite_card::InviteLookup::Found(details),
                        Ok(None) => super::invite_card::InviteLookup::Unsynced,
                        Err(error) => super::invite_card::InviteLookup::Failed(error),
                    });
                }
                (Task::none(), None)
            }
            ReadingPaneMessage::NextMessage => {
                let len = self.thread_messages.len();
                if len == 0 {
                    return (Task::none(), None);
                }
                let next = match self.focused_message {
                    Some(idx) if idx + 1 < len => idx + 1,
                    Some(_) => return (Task::none(), None),
                    None => 0,
                };
                self.focused_message = Some(next);
                if let Some(e) = self.message_expanded.get_mut(next) {
                    *e = true;
                }
                (Task::none(), None)
            }
            ReadingPaneMessage::PrevMessage => {
                if self.thread_messages.is_empty() {
                    return (Task::none(), None);
                }
                let prev = match self.focused_message {
                    Some(idx) if idx > 0 => idx - 1,
                    Some(_) => return (Task::none(), None),
                    None => self.thread_messages.len() - 1,
                };
                self.focused_message = Some(prev);
                if let Some(e) = self.message_expanded.get_mut(prev) {
                    *e = true;
                }
                (Task::none(), None)
            }
            ReadingPaneMessage::ToggleStar => (Task::none(), Some(ReadingPaneEvent::ToggleStar)),
            ReadingPaneMessage::LinkClicked(url) => {
                open_url_in_browser(&url);
                (Task::none(), None)
            }
            ReadingPaneMessage::HtmlBodyResized(index, width) => {
                let start = self
                    .html_bodies
                    .get_mut(index)
                    .is_some_and(|state| state.set_width(width));
                (self.start_html_render(index, start), None)
            }
            ReadingPaneMessage::HtmlBodyRendered {
                index,
                inputs,
                body,
            } => {
                let again = self
                    .html_bodies
                    .get_mut(index)
                    .is_some_and(|state| state.finish(inputs, body));
                (self.start_html_render(index, again), None)
            }
            ReadingPaneMessage::ShowRemoteImages(index) => {
                let start = self
                    .html_bodies
                    .get_mut(index)
                    .is_some_and(super::mail_body::HtmlBodyState::allow_remote);
                (self.start_html_render(index, start), None)
            }
            ReadingPaneMessage::Noop => (Task::none(), None),
        }
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn view(&self) -> Element<'_, ReadingPaneMessage> {
        match self.current_thread.as_ref() {
            None => empty_reading_pane(),
            Some(thread_ref) => thread_view(self, thread_ref),
        }
    }
}

// ── Command-backed view ─────────────────────────────────

impl ReadingPane {
    /// Render the reading pane with a command-backed toolbar.
    ///
    /// Unlike the `Component::view()` trait method (which returns
    /// `Element<ReadingPaneMessage>`), this returns `Element<Message>`
    /// so that toolbar buttons can emit `Message::ExecuteCommand`
    /// directly. Internal reading pane messages are mapped via
    /// `Message::ReadingPane`.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn view_with_commands(
        &self,
        registry: &CommandRegistry,
        binding_table: &BindingTable,
        ctx: &CommandContext,
    ) -> Element<'_, Message> {
        match self.current_thread.as_ref() {
            None => empty_reading_pane().map(Message::ReadingPane),
            Some(thread_ref) => {
                thread_view_with_commands(self, thread_ref, registry, binding_table, ctx)
            }
        }
    }
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
fn thread_view_with_commands<'a>(
    pane: &'a ReadingPane,
    thread_ref: &'a ThreadRef,
    registry: &CommandRegistry,
    binding_table: &BindingTable,
    ctx: &CommandContext,
) -> Element<'a, Message> {
    let mut col = column![].spacing(0).width(Length::Fill);

    // Reply, Reply All and Forward live in each message's header; the
    // conversation-wide actions sit on the thread header's info line.
    col = col.push(thread_header(
        thread_ref,
        pane,
        ctx.allows_set_keywords(),
        Message::ReadingPane,
        vec![
            widgets::command_icon_button(
                CommandId::EmailArchive,
                icon::archive(),
                registry,
                binding_table,
                ctx,
            ),
            widgets::command_icon_button(
                CommandId::EmailTrash,
                icon::trash(),
                registry,
                binding_table,
                ctx,
            ),
        ],
    ));

    col = col.push(message_list(pane).map(Message::ReadingPane));
    col.into()
}

// ── View helpers ────────────────────────────────────────

fn empty_reading_pane<'a>() -> Element<'a, ReadingPaneMessage> {
    container(widgets::empty_placeholder(
        t!("main.reading_pane.empty_title"),
        t!("main.reading_pane.empty_subtitle"),
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
fn thread_view<'a>(
    pane: &'a ReadingPane,
    thread_ref: &'a ThreadRef,
) -> Element<'a, ReadingPaneMessage> {
    let mut col = column![].spacing(0).width(Length::Fill);

    // No command context → allow star by default (non-command path)
    col = col.push(thread_header(
        thread_ref,
        pane,
        true,
        std::convert::identity,
        Vec::new(),
    ));

    col = col.push(message_list(pane));
    col.into()
}

fn thread_header<'a, M: Clone + 'a>(
    thread_ref: &'a ThreadRef,
    pane: &'a ReadingPane,
    may_set_keywords: bool,
    wrap: fn(ReadingPaneMessage) -> M,
    actions: Vec<Element<'a, M>>,
) -> Element<'a, M> {
    let subject = thread_ref
        .subject
        .as_deref()
        .map_or_else(|| t!("main.no_subject"), std::borrow::Cow::Borrowed);

    let star_icon_style: fn(&iced::Theme) -> text::Style = if !may_set_keywords {
        theme::TextClass::Tertiary.style()
    } else if thread_ref.is_starred {
        text::warning
    } else {
        text::secondary
    };

    let mut star_btn = button(icon::star().size(ICON_XL).style(star_icon_style))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    if may_set_keywords {
        star_btn = star_btn.on_press(wrap(ReadingPaneMessage::ToggleStar));
    }

    let toggle_label = if pane.message_expanded.iter().all(|&e| e) {
        t!("main.reading_pane.collapse_all")
    } else {
        t!("main.reading_pane.expand_all")
    };

    let expand_collapse_btn = button(
        text(toggle_label)
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    )
    .on_press(wrap(ReadingPaneMessage::ToggleAllMessages))
    .style(theme::ButtonClass::Ghost.style())
    .padding(PAD_ICON_BTN);

    let mut info_row = row![
        container(
            text(message_count_label(pane.thread_messages.len()))
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    // Label group pills.
    for label in &pane.thread_labels {
        let paint = label.paint;
        info_row = info_row.push(
            container(text(&label.name).size(TEXT_XS).color(paint.fg()))
                .padding(Padding {
                    top: 2.0,
                    right: 6.0,
                    bottom: 2.0,
                    left: 6.0,
                })
                .style(move |_theme: &iced::Theme| container::Style {
                    background: Some(paint.bg().into()),
                    border: iced::Border {
                        radius: RADIUS_LG.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        );
    }

    info_row = info_row.push(Space::new().width(Length::Fill));
    info_row = info_row.push(expand_collapse_btn);
    for action in actions {
        info_row = info_row.push(action);
    }

    container(
        column![
            row![
                container(text(subject).size(TEXT_HEADING).style(text::base)).width(Length::Fill),
                star_btn,
            ]
            .align_y(Alignment::Center),
            info_row,
        ]
        .spacing(SPACE_XXS),
    )
    .padding(Padding {
        bottom: SPACE_XXS,
        ..PAD_CONTENT
    })
    .into()
}

fn message_count_label(count: usize) -> String {
    if count == 1 {
        t!("main.reading_pane.messages_one", count = count)
    } else {
        t!("main.reading_pane.messages_other", count = count)
    }
    .into_owned()
}

/// Check if a message's content matches any of the search terms (case-insensitive).
fn message_matches_search_terms(msg: &ThreadMessage, terms: &[String]) -> bool {
    let check = |field: &Option<String>| {
        field.as_deref().is_some_and(|text| {
            let lower = text.to_lowercase();
            terms.iter().any(|t| lower.contains(&t.to_lowercase()))
        })
    };
    check(&msg.body_text)
        || check(&msg.body_html)
        || check(&msg.subject)
        || check(&msg.from_name)
        || check(&msg.from_address)
        || check(&msg.snippet)
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
fn message_list<'a>(pane: &'a ReadingPane) -> Element<'a, ReadingPaneMessage> {
    let messages_pad = Padding::from([0.0, SPACE_SM]);
    let first_message_date = pane.thread_messages.last().and_then(|m| m.date);
    let mut msg_col = column![].spacing(SPACE_XS).padding(messages_pad);

    for (i, msg) in pane.thread_messages.iter().enumerate() {
        let is_expanded = pane.message_expanded.get(i).copied().unwrap_or(false);
        if is_expanded {
            let html_body = msg.body_html.as_ref().map(|_| {
                super::mail_body::view(
                    pane.html_bodies.get(i),
                    msg.body_text.as_deref(),
                    move |width| ReadingPaneMessage::HtmlBodyResized(i, width),
                    ReadingPaneMessage::LinkClicked,
                    ReadingPaneMessage::ShowRemoteImages(i),
                )
            });
            let message_id = msg.id.clone();
            let invite = super::invite_card::view(msg, pane.invites.get(&msg.id), move |m| {
                ReadingPaneMessage::Invite(message_id.clone(), m)
            });
            msg_col = msg_col.push(widgets::expanded_message_card(
                msg,
                i,
                pane.date_display,
                first_message_date,
                &pane.search_highlight_terms,
                invite,
                message_attachments(pane, &msg.id),
                html_body,
                ReadingPaneMessage::LinkClicked,
                ReadingPaneMessage::ToggleMessageExpanded,
                ReadingPaneMessage::PopOut,
                ReadingPaneMessage::Reply,
                ReadingPaneMessage::ReplyAll,
                ReadingPaneMessage::Forward,
                ReadingPaneMessage::EditContact,
                ReadingPaneMessage::CreateEventFromEmail,
            ));
        } else {
            msg_col = msg_col.push(widgets::collapsed_message_row(
                msg,
                i,
                ReadingPaneMessage::ToggleMessageExpanded,
            ));
        }
    }

    msg_col = msg_col.push(Space::new().height(SPACE_MD));

    scrollable(msg_col)
        .spacing(SCROLLBAR_SPACING)
        .height(Length::Fill)
        .into()
}

// ── Attachments ─────────────────────────────────────────

/// The message's attachments as one row of chips under its header,
/// scrolling sideways when they overflow the card.
fn message_attachments<'a>(
    pane: &'a ReadingPane,
    message_id: &str,
) -> Option<Element<'a, ReadingPaneMessage>> {
    let mut chips = row![].spacing(SPACE_XS).align_y(Alignment::Center);
    let mut any = false;
    for attachment in pane
        .thread_attachments
        .iter()
        .filter(|attachment| attachment.message_id == message_id)
    {
        any = true;
        let id = &attachment.id;
        chips = chips.push(widgets::attachment_chip(
            attachment,
            pane.attachment_menu.as_deref() == Some(id.as_str()),
            widgets::AttachmentChipActions {
                on_preview: ReadingPaneMessage::PreviewAttachment(id.clone()),
                on_open: ReadingPaneMessage::OpenAttachment(id.clone()),
                on_save: ReadingPaneMessage::SaveAttachment(id.clone()),
                on_toggle_menu: ReadingPaneMessage::ToggleAttachmentMenu(id.clone()),
                on_close_menu: ReadingPaneMessage::CloseAttachmentMenu,
            },
        ));
    }
    any.then(|| {
        scrollable(chips)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new()
                    .width(TOOLBAR_SCROLLBAR_WIDTH)
                    .scroller_width(TOOLBAR_SCROLLBAR_WIDTH)
                    .spacing(SCROLLBAR_SPACING),
            ))
            .width(Length::Fill)
            .into()
    })
}

/// Open a URL in the system default browser.
pub(crate) fn open_url_in_browser(url: &str) {
    // Only allow safe URL schemes from untrusted email content
    let allowed =
        url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:");
    if !allowed {
        log::warn!("Blocked URL with disallowed scheme: {url}");
        return;
    }

    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .spawn();
    }
}

#[cfg(test)]
mod expansion_tests {
    use super::ReadingPane;
    use crate::db::ThreadMessage;

    fn message(id: &str, is_own_message: bool) -> ThreadMessage {
        ThreadMessage {
            id: id.to_owned(),
            thread_id: "thread".to_owned(),
            account_id: "account".to_owned(),
            from_name: None,
            from_address: None,
            to_addresses: None,
            cc_addresses: None,
            date: None,
            subject: None,
            message_id_header: None,
            snippet: None,
            body_html: None,
            body_text: None,
            is_read: true,
            is_starred: false,
            is_replied: false,
            is_forwarded: false,
            has_meeting_invite: false,
            meeting_invite_method: None,
            meeting_invite_uid: None,
            is_own_message,
        }
    }

    #[test]
    fn the_latest_received_message_opens_instead_of_the_users_reply() {
        let mut pane = ReadingPane::new();
        let messages = [
            message("my-second-reply", true),
            message("my-first-reply", true),
            message("their-answer", false),
            message("my-question", true),
        ];
        pane.apply_message_expansion(&messages);
        assert_eq!(pane.message_expanded, [false, false, true, true]);
    }

    #[test]
    fn a_thread_of_own_messages_opens_the_most_recent() {
        let mut pane = ReadingPane::new();
        let messages = [message("latest", true), message("middle", true), message("first", true)];
        pane.apply_message_expansion(&messages);
        assert_eq!(pane.message_expanded, [true, false, true]);
    }
}
