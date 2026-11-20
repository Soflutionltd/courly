#![allow(dead_code)]

use std::borrow::Cow;

use iced::widget::{Space, button, column, container, mouse_area, row, text};
use iced::{Alignment, Element, Length, Theme};
use rust_i18n::t;

use crate::db::{DateDisplay, Thread, ThreadMessage};
use crate::font;
use crate::icon;
use crate::ui::label_paint::LabelPaint;
use crate::ui::layout::{
    AVATAR_MESSAGE_CARD, Density, ICON_SM, ICON_XS, PAD_BADGE, PAD_BODY, PAD_CARD, PAD_HTML_BODY,
    PAD_ICON_BTN, PAD_REMOTE_BANNER, SPACE_SM, SPACE_XS, SPACE_XXS, SPACE_XXXS, TEXT_LG, TEXT_MD,
    TEXT_SM, TEXT_XS, UNREAD_DOT_GAP, UNREAD_DOT_SIZE,
};
use crate::ui::theme::{self, RowActionTone};

use super::avatars::{label_dot, sender_avatar};

/// The host-zone wall clock for a stored message timestamp, which is Unix
/// milliseconds (`messages.date`, `threads.last_message_at`). Card timestamps
/// are labels a user reads against their own clock; the chrono-era UTC
/// rendering (an accidental missing `with_timezone(&Local)`) was off by the
/// UTC offset for every non-UTC user.
pub(crate) fn local_dt(millis: i64) -> Option<jiff::Zoned> {
    jiff::Timestamp::from_millisecond(millis)
        .ok()
        .map(|ts| ts.to_zoned(jiff::tz::TimeZone::system()))
}

/// "Mar 12", "12 mars"; "Mar 12, 2025", "12 mars 2025" before this year.
pub(crate) fn short_date(dt: &jiff::Zoned) -> String {
    let month = crate::i18n::month_short(dt.month());
    if dt.year() == jiff::Zoned::now().year() {
        t!("main.date.short", month = month, day = dt.day()).into_owned()
    } else {
        t!(
            "main.date.short_year",
            month = month,
            day = dt.day(),
            year = dt.year()
        )
        .into_owned()
    }
}

/// "Mar 12, 2026, 10:13 AM", "12 mars 2026, 10:13".
fn medium_date_time(dt: &jiff::Zoned) -> String {
    t!(
        "main.date.medium_time",
        month = crate::i18n::month_short(dt.month()),
        day = dt.day(),
        year = dt.year(),
        time = crate::i18n::time_of_day(dt)
    )
    .into_owned()
}

/// Sender shown when a message has neither a name nor an address.
fn sender_or_unknown<'a>(name: Option<&'a str>, address: Option<&'a str>) -> Cow<'a, str> {
    name.or(address)
        .map_or_else(|| t!("main.unknown_sender"), Cow::Borrowed)
}
use super::highlighted::highlighted_text_body;
use super::search_highlight::highlighted_line;

#[cfg_attr(feature = "hotpath", hotpath::measure)]
/// Pin, delete or flag, from the buttons kept visible on every row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadRowAction {
    Pin,
    Trash,
    Flag,
}

/// What a thread row shows beyond the thread itself.
#[derive(Clone, Copy)]
pub struct ThreadCardMarks<'b> {
    pub label_paints: &'b [LabelPaint],
    /// Query words to highlight; empty outside search results.
    pub highlight_terms: &'b [String],
    /// Name the row by its recipients, as Sent and Drafts do.
    pub shows_recipients: bool,
    /// The row is in the Drafts folder: a draft can only be deleted there,
    /// whether or not it has reached the server yet.
    pub in_drafts: bool,
    pub density: Density,
}

pub fn thread_card<'a, M: Clone + 'a>(
    thread: &'a Thread,
    index: usize,
    selected: bool,
    marks: ThreadCardMarks<'_>,
    (on_press, on_release, on_exit): (
        impl Fn(usize) -> M,
        impl Fn(usize) -> M,
        impl Fn(usize) -> M,
    ),
    on_action: impl Fn(usize, ThreadRowAction) -> M,
) -> Element<'a, M> {
    let ThreadCardMarks {
        label_paints,
        highlight_terms,
        shows_recipients,
        in_drafts,
        density,
    } = marks;
    let recipients = if shows_recipients {
        crate::ui::invite_card::display_names(thread.to_addresses.as_deref()).join(", ")
    } else {
        String::new()
    };
    let sender = if recipients.is_empty() {
        sender_or_unknown(thread.from_name.as_deref(), thread.from_address.as_deref())
    } else {
        Cow::Owned(recipients)
    };

    // The reply and forward marks piled up in front of a subject are not
    // shown: one arrow says what the message is.
    let (subject_kind, bare_subject) =
        crate::subject::split(thread.subject.as_deref().unwrap_or(""));
    let subject = if bare_subject.is_empty() {
        t!("main.no_subject")
    } else {
        Cow::Borrowed(bare_subject)
    };
    // Phase 7-8: when the search hit's primary match was an attachment,
    // surface that fact in the snippet slot so the user sees "matched in
    // <filename>" rather than an unrelated body snippet. Otherwise show
    // the message snippet as before.
    let attachment_match = thread.match_kind.as_ref().and_then(|mk| match mk {
        rtsk::search::MatchKind::Attachment {
            filename, snippet, ..
        } => Some((filename.as_str(), snippet.as_str())),
        _ => None,
    });
    let snippet = thread.snippet.as_deref().unwrap_or("");

    let date_str = thread
        .last_message_at
        .and_then(|ts| {
            local_dt(ts).map(|dt| {
                let diff = jiff::Timestamp::now().duration_since(dt.timestamp());
                if diff.as_hours() < 24 {
                    crate::i18n::time_of_day(&dt)
                } else if diff.as_hours() < 24 * 7 {
                    crate::i18n::weekday_short(dt.weekday().to_monday_zero_offset().unsigned_abs())
                } else {
                    short_date(&dt)
                }
            })
        })
        .unwrap_or_default();

    // Unread reads as in Outlook: a blue dot, the sender and subject in
    // bold, the subject and time in the accent color.
    let emphasis_font = if thread.is_read {
        font::text()
    } else {
        font::text_bold()
    };

    let subject_style: fn(&Theme) -> text::Style = if thread.is_read {
        theme::TextClass::Muted.style()
    } else {
        theme::TextClass::Accent.style()
    };
    let date_style: fn(&Theme) -> text::Style = if thread.is_read {
        theme::TextClass::Tertiary.style()
    } else {
        theme::TextClass::Accent.style()
    };

    let mut indicators = row![].spacing(SPACE_XXS).align_y(Alignment::Center);
    if thread.is_local_draft {
        indicators = indicators.push(
            container(
                text(t!("main.thread_list.draft_badge"))
                    .size(TEXT_XS)
                    .style(theme::TextClass::Accent.style()),
            )
            .padding(PAD_BADGE)
            .style(theme::ContainerClass::KeyBadge.style()),
        );
    }
    for &paint in label_paints {
        indicators = indicators.push(label_dot(paint));
    }
    if thread.is_replied {
        indicators = indicators.push(
            icon::reply()
                .size(ICON_XS)
                .style(theme::TextClass::Tertiary.style()),
        );
    }
    if thread.is_forwarded {
        indicators = indicators.push(
            icon::forward()
                .size(ICON_XS)
                .style(theme::TextClass::Tertiary.style()),
        );
    }
    if thread.has_attachments {
        indicators = indicators.push(
            icon::paperclip()
                .size(ICON_XS)
                .style(theme::TextClass::Tertiary.style()),
        );
    }
    // A thread flagged again after completion shows the flag, not the check,
    // as Outlook does.
    if thread.is_flag_complete && !thread.is_starred {
        indicators = indicators.push(icon::check().size(ICON_XS).style(text::success));
    }
    if let Some(folder) = thread.folder_name.as_deref() {
        indicators = indicators.push(
            container(
                text(folder)
                    .size(TEXT_XS)
                    .style(theme::TextClass::Tertiary.style())
                    .wrapping(text::Wrapping::None),
            )
            .padding(PAD_BADGE)
            .style(theme::ContainerClass::KeyBadge.style()),
        );
    }

    let top_row = row![
        container(highlighted_line(
            sender.clone(),
            highlight_terms,
            TEXT_MD,
            emphasis_font,
            text::base,
        ))
        .width(Length::Fill)
        .clip(true),
        container(
            text(date_str)
                .size(TEXT_XS)
                .font(emphasis_font)
                .style(date_style)
        ),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    let trash = row_action(
        icon::trash(),
        RowActionTone::Danger,
        false,
        on_action(index, ThreadRowAction::Trash),
    );
    // A draft can only be deleted: pinning or flagging is for received and
    // sent conversations. Every row of Drafts shows the same single action,
    // so a draft already on the server does not grow two more buttons than
    // its neighbours.
    let actions = if thread.is_local_draft || in_drafts {
        row![trash]
    } else {
        row![
            row_action(
                icon::pin(),
                RowActionTone::Accent,
                thread.is_pinned,
                on_action(index, ThreadRowAction::Pin),
            ),
            trash,
            row_action(
                icon::flag(),
                RowActionTone::Warning,
                thread.is_starred,
                on_action(index, ThreadRowAction::Flag),
            ),
        ]
    }
    .spacing(SPACE_XXXS)
    .align_y(Alignment::Center);

    let subject_row = row![
        subject_mark(subject_kind, ICON_XS),
        container(highlighted_line(
            subject,
            highlight_terms,
            TEXT_MD,
            emphasis_font,
            subject_style,
        ))
        .width(Length::Fill)
        .clip(true),
        actions,
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    // Phase 7-8: branch the snippet slot. Attachment matches show
    // "in {filename}: {fragment}" (filename italicised, fragment plain)
    // so the user can connect the search hit to the matching file.
    // Other matches show the message snippet as before.
    let snippet_slot: Element<'_, _> =
        if let Some((filename, attachment_snippet)) = attachment_match {
            container(
                row![
                    text(t!("main.search.attachment_match", filename = filename))
                        .size(TEXT_SM)
                        .style(text::secondary)
                        .font(font::text_italic())
                        .wrapping(text::Wrapping::None),
                    text(attachment_snippet.to_string())
                        .size(TEXT_SM)
                        .style(text::secondary)
                        .wrapping(text::Wrapping::None)
                        .ellipsis(text::Ellipsis::End)
                        .width(Length::Fill),
                ]
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .clip(true)
            .into()
        } else {
            container(highlighted_line(
                Cow::Borrowed(snippet),
                highlight_terms,
                TEXT_SM,
                font::text(),
                text::secondary,
            ))
            .width(Length::Fill)
            .clip(true)
            .into()
        };

    let snippet_row = row![snippet_slot, indicators]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center);

    let text_content = column![top_row, subject_row, snippet_row]
        .spacing(density.thread_card_line_gap())
        .width(Length::Fill);

    // A row that names its recipients has no single address to picture.
    let pictured_address = if shows_recipients {
        None
    } else {
        thread.from_address.as_deref()
    };
    let avatar = sender_avatar(&sender, pictured_address, density.thread_card_avatar());

    let unread_dot: Element<'a, M> = if thread.is_read {
        Space::new().width(UNREAD_DOT_SIZE).into()
    } else {
        container(Space::new())
            .width(UNREAD_DOT_SIZE)
            .height(UNREAD_DOT_SIZE)
            .style(theme::ContainerClass::UnreadDot.style())
            .into()
    };

    let content = row![
        row![unread_dot, avatar]
            .spacing(UNREAD_DOT_GAP)
            .align_y(Alignment::Center),
        text_content
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    // The press goes to the inner area, so a row can start a drag; a click
    // is a release on the row pressed. The button only gives the styling.
    let card_padding = density.thread_card_padding();
    let hit = mouse_area(
        container(content)
            .padding(iced::Padding {
                left: UNREAD_DOT_GAP,
                ..card_padding
            })
            .height(density.thread_card_height())
            .width(Length::Fill),
    )
    .on_press(on_press(index))
    .on_release(on_release(index))
    .on_exit(on_exit(index));
    button(hit)
        .on_press(on_release(index))
        .padding(0)
        .style(
            theme::ButtonClass::ThreadCard {
                selected,
                starred: thread.is_starred,
            }
            .style(),
        )
        .width(Length::Fill)
        .into()
}

/// One of the pin, delete and flag buttons kept visible on every row.
fn row_action<'a, M: Clone + 'a>(
    ico: iced::widget::Text<'a>,
    tone: RowActionTone,
    active: bool,
    on_press: M,
) -> Element<'a, M> {
    button(container(ico.size(ICON_SM)))
        .on_press(on_press)
        .padding(SPACE_XXXS)
        .style(theme::ButtonClass::RowAction { tone, active }.style())
        .into()
}

pub fn message_card<'a, M: 'a>(thread: &'a Thread) -> Element<'a, M> {
    let sender = sender_or_unknown(thread.from_name.as_deref(), thread.from_address.as_deref());

    let avatar = sender_avatar(&sender, thread.from_address.as_deref(), AVATAR_MESSAGE_CARD);
    let date_str = thread
        .last_message_at
        .and_then(|ts| {
            local_dt(ts).map(|dt| {
                t!(
                    "main.date.full_time",
                    weekday = crate::i18n::weekday_short(
                        dt.weekday().to_monday_zero_offset().unsigned_abs()
                    ),
                    month = crate::i18n::month_short(dt.month()),
                    day = dt.day(),
                    year = dt.year(),
                    time = crate::i18n::time_of_day(&dt)
                )
                .into_owned()
            })
        })
        .unwrap_or_default();

    let header = row![
        avatar,
        column![
            row![
                text(sender).size(TEXT_LG).style(text::base),
                Space::new().width(Length::Fill),
                text(date_str)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            ],
            text(thread.from_address.as_deref().unwrap_or(""))
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Start);

    let body_text = thread
        .snippet
        .as_deref()
        .map_or_else(|| t!("main.no_preview"), Cow::Borrowed);
    let body = container(text(body_text).size(TEXT_LG).style(text::secondary)).padding(PAD_BODY);

    container(column![header, body].spacing(SPACE_XS))
        .padding(PAD_CARD)
        .width(Length::Fill)
        .style(theme::ContainerClass::MessageCard.style())
        .into()
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
#[allow(clippy::too_many_arguments)]
pub fn expanded_message_card<'a, M: Clone + 'a>(
    msg: &'a ThreadMessage,
    index: usize,
    date_display: DateDisplay,
    first_message_date: Option<i64>,
    search_highlight_terms: &'a [String],
    // The state of leaving the list, the address a mail goes to when the
    // list is left by mail, and the message that leaves it.
    unsubscribe: Option<(crate::unsubscribe::UnsubscribeStatus, Option<String>, M)>,
    verification_code: Option<Element<'a, M>>,
    invite: Option<Element<'a, M>>,
    attachments: Option<Element<'a, M>>,
    html_body: Option<crate::ui::mail_body::BodyCards<'a, M>>,
    quoted_senders: Vec<Option<QuotedSender>>,
    body_tone: Option<(bool, M)>,
    on_link: impl Fn(String) -> M + 'a,
    on_pop_out: impl Fn(usize) -> M,
    on_reply: impl Fn(usize) -> M,
    on_reply_all: impl Fn(usize) -> M,
    on_forward: impl Fn(usize) -> M,
    on_edit_contact: impl Fn(String) -> M + 'a,
    on_create_event: impl Fn(usize) -> M,
    on_recipients: impl Fn(usize, Option<usize>) -> M,
) -> Element<'a, M> {
    let sender_name = sender_or_unknown(msg.from_name.as_deref(), msg.from_address.as_deref());
    let sender_email_str = msg.from_address.as_deref().unwrap_or("");

    let avatar = sender_avatar(
        &sender_name,
        msg.from_address.as_deref(),
        AVATAR_MESSAGE_CARD,
    );
    let date_str = format_message_date(msg.date, first_message_date, date_display);

    let pop_out_btn = button(icon::external_link().size(ICON_SM).style(text::secondary))
        .on_press(on_pop_out(index))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    let sender_email_owned = msg.from_address.clone().unwrap_or_default();
    let sender_element: Element<'a, M> = button(
        text(sender_name)
            .size(TEXT_LG)
            .font(font::text_semibold())
            .style(text::base),
    )
    .on_press(on_edit_contact(sender_email_owned))
    .padding(0)
    .style(theme::ButtonClass::BareTransparent.style())
    .into();

    let showing_original = body_tone.as_ref().is_some_and(|(original, _)| *original);
    let tone_button = body_tone.map(|(original, on_press)| {
        let ico = if original { icon::moon() } else { icon::sun() };
        header_icon_button(ico, on_press)
    });
    let message_actions = row![
        tone_button,
        header_icon_button(icon::reply(), on_reply(index)),
        header_icon_button(icon::reply_all(), on_reply_all(index)),
        header_icon_button(icon::forward(), on_forward(index)),
        header_icon_button(icon::calendar(), on_create_event(index)),
    ]
    .spacing(SPACE_XXXS)
    .align_y(Alignment::Center);

    let mut state_indicators = row![].spacing(SPACE_XXS).align_y(Alignment::Center);
    if msg.is_replied {
        state_indicators = state_indicators.push(
            icon::reply()
                .size(ICON_XS)
                .style(theme::TextClass::Tertiary.style()),
        );
    }
    if msg.is_forwarded {
        state_indicators = state_indicators.push(
            icon::forward()
                .size(ICON_XS)
                .style(theme::TextClass::Tertiary.style()),
        );
    }

    let avatar_rows = row![
        avatar,
        column![
            row![
                container(sender_element).align_y(Alignment::Center),
                state_indicators,
                Space::new().width(Length::Fill),
                container(
                    text(date_str)
                        .size(TEXT_SM)
                        .style(theme::TextClass::Tertiary.style()),
                )
                .align_y(Alignment::Center),
                message_actions,
                pop_out_btn,
            ]
            .align_y(Alignment::Center)
            .spacing(SPACE_XS),
            text(sender_email_str)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Start);

    let to_row = recipients_row(
        msg.to_addresses.as_deref(),
        msg.cc_addresses.as_deref(),
        Some(on_recipients(index, None)),
    );

    let header = column![avatar_rows, to_row].spacing(SPACE_XXS);

    let (html_message, quoted) = match html_body {
        Some(cards) => (Some(cards.message), cards.quoted),
        None => (None, Vec::new()),
    };
    let body_inner: Element<'_, M> = if let Some(rendered) = html_message {
        container(rendered).padding(PAD_HTML_BODY).into()
    } else if let Some(display) = msg.body_text.as_deref().or(msg.snippet.as_deref()) {
        if search_highlight_terms.is_empty() {
            container(super::linked_text(display, TEXT_LG, on_link))
                .padding(PAD_BODY)
                .into()
        } else {
            container(highlighted_text_body::<M>(display, search_highlight_terms))
                .padding(PAD_BODY)
                .into()
        }
    } else {
        container(text(t!("main.no_preview")).size(TEXT_LG))
            .padding(PAD_BODY)
            .into()
    };

    let body_class = if showing_original {
        theme::ContainerClass::EmailBodyOriginal
    } else {
        theme::ContainerClass::EmailBody
    };
    let body: Element<'_, M> = container(body_inner)
        .width(Length::Fill)
        .style(body_class.style())
        .into();

    let mut card_content = column![header].spacing(SPACE_XS);
    if let Some((status, mail_to, on_unsubscribe)) = unsubscribe {
        card_content = card_content.push(unsubscribe_banner(status, mail_to, on_unsubscribe));
    }
    if let Some(verification_code) = verification_code {
        card_content = card_content.push(verification_code);
    }
    if let Some(attachments) = attachments {
        card_content = card_content.push(attachments);
    }
    if let Some(invite) = invite {
        card_content = card_content.push(invite);
    }
    card_content = card_content.push(body);

    let card = message_card_frame(card_content.into());
    if quoted.is_empty() {
        return card;
    }
    // Each earlier message quoted at the end of this one is a card of its
    // own under it: same width, same outline, a gap between them. A quoted
    // message the conversation holds is left out: it has its own card
    // further down, with its attachments and the buttons that answer it,
    // and shown here too it would be read twice.
    let shown = quotes_shown(&quoted_senders, quoted.len());
    let mut cards = column![card].spacing(SPACE_XS);
    let mut senders = quoted_senders.into_iter();
    for (position, quoted_message) in quoted.into_iter().enumerate().take(shown) {
        let sender = senders.next().flatten();
        cards = cards.push(quoted_message_card(
            sender,
            None,
            quoted_message.body,
            body_class,
            Some(on_recipients(index, Some(position))),
        ));
    }
    cards.into()
}

/// How many of a message's quotes get a card under it: the ones before the
/// first the conversation holds. That message has its own card further
/// down, and the older ones it quotes in turn are shown under that card;
/// left under this one, they would sit above a message written after them.
fn quotes_shown(senders: &[Option<QuotedSender>], quotes: usize) -> usize {
    senders
        .iter()
        .position(|sender| {
            sender
                .as_ref()
                .is_some_and(|sender| sender.message.is_some())
        })
        .unwrap_or(quotes)
}

/// The "To" line under a card's sender: the first recipients and a count
/// of the others, opening the full list through `on_open`. Nothing for a
/// message without a recipient.
fn recipients_row<'a, M: Clone + 'a>(
    to: Option<&str>,
    cc: Option<&str>,
    on_open: Option<M>,
) -> Option<Element<'a, M>> {
    let line = crate::ui::recipients::line(to, cc, on_open)?;
    Some(
        row![
            container(
                text(t!("main.reading_pane.to"))
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .width(AVATAR_MESSAGE_CARD + SPACE_SM)
            .padding(iced::Padding {
                top: 0.0,
                right: SPACE_XS,
                bottom: 0.0,
                left: 0.0,
            })
            .align_x(Alignment::End),
            line,
        ]
        .align_y(Alignment::Start)
        .into(),
    )
}

/// The outline and padding every message of a conversation is shown in.
pub fn message_card_frame<'a, M: 'a>(content: Element<'a, M>) -> Element<'a, M> {
    container(content)
        .padding(PAD_CARD)
        .width(Length::Fill)
        .style(theme::ContainerClass::MessageCard.style())
        .into()
}

/// The card of a quoted message: its header when it is known, then its
/// body on the page colour `body_class` paints. With `on_recipients` the
/// line naming its recipients opens their full list. `attachments` are the
/// files that message carried, shown under its header as on a message.
pub fn quoted_message_card<'a, M: Clone + 'a>(
    sender: Option<QuotedSender>,
    attachments: Option<Element<'a, M>>,
    body: Element<'a, M>,
    body_class: theme::ContainerClass,
    on_recipients: Option<M>,
) -> Element<'a, M> {
    let body = container(container(body).padding(PAD_HTML_BODY))
        .width(Length::Fill)
        .style(body_class.style());
    let header = sender.map(|sender| quoted_header(sender, on_recipients));
    message_card_frame(column![header, attachments, body].spacing(SPACE_XS).into())
}

/// The arrow that stands for the reply or forward marks of a subject, which
/// are not shown: nothing for a subject that had none.
pub fn subject_mark<'a, M: 'a>(
    kind: crate::subject::SubjectKind,
    size: f32,
) -> Option<Element<'a, M>> {
    use crate::subject::SubjectKind;
    let arrow = match kind {
        SubjectKind::Plain => return None,
        SubjectKind::Reply => icon::reply(),
        SubjectKind::Forward => icon::forward(),
    };
    Some(
        container(arrow.size(size).style(theme::TextClass::Tertiary.style()))
            .align_y(Alignment::Center)
            .into(),
    )
}

/// Who wrote a quoted message, when and to whom, as its card shows it.
pub struct QuotedSender {
    pub name: String,
    pub address: Option<String>,
    pub date: Option<String>,
    pub recipients: Option<String>,
    /// Who it was copied to.
    pub copies: Option<String>,
    /// Its place in the conversation, when the conversation holds it.
    pub message: Option<usize>,
}

impl QuotedSender {
    /// The header as the mail quoting the message wrote it.
    pub fn from_header(header: &mail_html::QuoteHeader) -> Self {
        Self {
            name: header
                .name
                .clone()
                .or_else(|| header.address.clone())
                .unwrap_or_default(),
            address: header.address.clone(),
            date: header.date.clone(),
            recipients: header.to.clone(),
            copies: header.cc.clone(),
            message: None,
        }
    }
}

/// The header of the `position`-th message quoted by message `index` of a
/// conversation, newest first.
///
/// A reply quotes the messages before it in order, so that quote is the
/// message `position + 1` places further down: when its sender is the one
/// the quote names, the conversation's own record is shown, complete and
/// dated like every other message. Otherwise, as for a message the
/// conversation does not hold, the header is what the quoting mail wrote.
pub fn quoted_sender(
    messages: &[ThreadMessage],
    index: usize,
    position: usize,
    header: &mail_html::QuoteHeader,
    date_display: DateDisplay,
) -> QuotedSender {
    let next = index + position + 1;
    // The message the quote most likely is, one place down per quote. When
    // the conversation holds other messages in between (the reader's own
    // replies, a message filed elsewhere), the quote is looked for among
    // all the older ones, by its sender and the time its header gives.
    let place = if messages
        .get(next)
        .is_some_and(|message| wrote_the_quote(message, header))
    {
        Some(next)
    } else {
        messages
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, message)| {
                wrote_the_quote(message, header)
                    && written_then(header.date.as_deref(), message.date)
            })
            .map(|(place, _)| place)
    };
    match place.and_then(|place| Some((place, messages.get(place)?))) {
        Some((place, message)) => QuotedSender {
            name: sender_or_unknown(
                message.from_name.as_deref(),
                message.from_address.as_deref(),
            )
            .into_owned(),
            address: message.from_address.clone(),
            date: Some(format_message_date(
                message.date,
                messages.last().and_then(|first| first.date),
                date_display,
            )),
            recipients: message.to_addresses.clone(),
            copies: message.cc_addresses.clone(),
            message: Some(place),
        },
        None => {
            let mut sender = QuotedSender::from_header(header);
            // The quote is not the next message down, but its writer may
            // have written another message of the conversation: that one
            // gives the address the quote leaves out.
            if sender.address.is_none() {
                sender.address = messages
                    .iter()
                    .find(|message| wrote_the_quote(message, header))
                    .and_then(|message| message.from_address.clone());
            }
            sender
        }
    }
}

/// Whether `message` is the one a quote names as its sender: by address
/// when the quote gives one, otherwise by name. Outlook leaves the address
/// out of the header of a quoted message, and a card told apart from its
/// own message would show the name alone, in another colour and without
/// the address the conversation knows.
fn wrote_the_quote(message: &ThreadMessage, header: &mail_html::QuoteHeader) -> bool {
    match (header.address.as_deref(), header.name.as_deref()) {
        (Some(quoted), _) => message
            .from_address
            .as_deref()
            .is_some_and(|known| known.eq_ignore_ascii_case(quoted)),
        (None, Some(quoted)) => message
            .from_name
            .as_deref()
            .is_some_and(|known| same_name(known, quoted)),
        (None, None) => false,
    }
}

/// Whether the date a quote header writes, in whatever language and order,
/// can be the moment `millis` of a stored message: it gives the same minute
/// of the hour, and the day of the month or one next to it. The hour is
/// left out, since the header was written in its writer's time zone, which
/// also moves the day by one at most.
fn written_then(header_date: Option<&str>, millis: Option<i64>) -> bool {
    let (Some(header_date), Some(when)) = (header_date, millis.and_then(local_dt)) else {
        return false;
    };
    if !header_date.contains(&format!(":{:02}", when.minute())) {
        return false;
    }
    let numbers: Vec<i64> = header_date
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|digits| digits.parse().ok())
        .collect();
    [-1, 0, 1].into_iter().any(|shift| {
        when.checked_add(jiff::Span::new().days(shift))
            .is_ok_and(|day| numbers.contains(&i64::from(day.day())))
    })
}

/// Two names that read the same, whatever their case and the blanks
/// between their words.
fn same_name(left: &str, right: &str) -> bool {
    let words =
        |name: &str| -> Vec<String> { name.split_whitespace().map(str::to_lowercase).collect() };
    let (left, right) = (words(left), words(right));
    !left.is_empty() && left == right
}

/// The header of a quoted message's card: the same lines as a message of
/// the conversation. It has no button to answer it: the conversation does
/// not hold that message, there is nothing to answer.
fn quoted_header<'a, M: Clone + 'a>(
    sender: QuotedSender,
    on_recipients: Option<M>,
) -> Element<'a, M> {
    let avatar = sender_avatar(&sender.name, sender.address.as_deref(), AVATAR_MESSAGE_CARD);
    let date = sender.date.map(|date| {
        container(
            text(date)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .align_y(Alignment::Center)
    });
    let address = sender
        .address
        .filter(|address| *address != sender.name)
        .map(|address| {
            text(address)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        });
    let identity = row![
        avatar,
        column![
            row![
                text(sender.name)
                    .size(TEXT_LG)
                    .font(font::text_semibold())
                    .style(text::base),
                Space::new().width(Length::Fill),
                date,
            ]
            .align_y(Alignment::Center)
            .spacing(SPACE_XS),
            address,
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Start);

    let recipients = recipients_row(
        sender.recipients.as_deref(),
        sender.copies.as_deref(),
        on_recipients,
    );

    column![identity, recipients].spacing(SPACE_XXS).into()
}

/// The bar above a mailing-list message: one button leaves the list, then
/// the bar says how that went.
fn unsubscribe_banner<'a, M: Clone + 'a>(
    status: crate::unsubscribe::UnsubscribeStatus,
    mail_to: Option<String>,
    on_unsubscribe: M,
) -> Element<'a, M> {
    use crate::unsubscribe::UnsubscribeStatus;

    let (message, action) = match status {
        // Leaving by mail sends a message under the user's name: the bar
        // says to whom before the button is pressed.
        UnsubscribeStatus::Ready => (
            match mail_to {
                Some(address) => t!(
                    "main.reading_pane.unsubscribe_offer_mail",
                    address = address
                )
                .into_owned(),
                None => t!("main.reading_pane.unsubscribe_offer").into_owned(),
            },
            Some(t!("main.reading_pane.unsubscribe")),
        ),
        UnsubscribeStatus::Working => (t!("main.reading_pane.unsubscribing").into_owned(), None),
        UnsubscribeStatus::Done => (t!("main.reading_pane.unsubscribed").into_owned(), None),
        UnsubscribeStatus::Requested => (
            t!("main.reading_pane.unsubscribe_requested").into_owned(),
            None,
        ),
        UnsubscribeStatus::Failed(error) => (
            t!("main.reading_pane.unsubscribe_failed", error = error).into_owned(),
            Some(t!("main.reading_pane.unsubscribe_retry")),
        ),
    };
    let button_slot = action.map(|label| {
        button(text(label).size(TEXT_SM))
            .on_press(on_unsubscribe)
            .padding(PAD_ICON_BTN)
            .style(theme::ButtonClass::Secondary.style())
    });
    container(
        row![
            container(
                icon::mail()
                    .size(ICON_SM)
                    .style(theme::TextClass::Muted.style()),
            )
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
            container(
                text(message)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Muted.style()),
            )
            .width(Length::Fill)
            .align_y(Alignment::Center),
            button_slot,
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .padding(PAD_REMOTE_BANNER)
    .width(Length::Fill)
    .style(theme::ContainerClass::Badge.style())
    .into()
}

/// The bar above a verification email: the one-time code it carries and a
/// button that copies it. `copied` swaps the button for a confirmation.
pub fn verification_code_banner<'a, M: Clone + 'a>(
    code: &str,
    copied: bool,
    on_copy: M,
) -> Element<'a, M> {
    let label = if copied {
        t!("main.reading_pane.code_copied")
    } else {
        t!("main.reading_pane.code_copy")
    };
    container(
        row![
            container(
                icon::copy()
                    .size(ICON_SM)
                    .style(theme::TextClass::Muted.style()),
            )
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
            container(
                text(t!("main.reading_pane.code_found", code = code).into_owned())
                    .size(TEXT_SM)
                    .style(theme::TextClass::Muted.style()),
            )
            .width(Length::Fill)
            .align_y(Alignment::Center),
            button(text(label).size(TEXT_SM))
                .on_press(on_copy)
                .padding(PAD_ICON_BTN)
                .style(theme::ButtonClass::Secondary.style()),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .padding(PAD_REMOTE_BANNER)
    .width(Length::Fill)
    .style(theme::ContainerClass::Badge.style())
    .into()
}

/// Icon-only action in a message header.
fn header_icon_button<'a, M: Clone + 'a>(
    ico: iced::widget::Text<'a>,
    on_press: M,
) -> Element<'a, M> {
    button(ico.size(ICON_SM).style(text::secondary))
        .on_press(on_press)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style())
        .into()
}

fn format_message_date(
    timestamp: Option<i64>,
    first_message_timestamp: Option<i64>,
    display: DateDisplay,
) -> String {
    let Some(ts) = timestamp else {
        return String::new();
    };
    let Some(dt) = local_dt(ts) else {
        return String::new();
    };

    match display {
        DateDisplay::RelativeOffset => {
            let abs = medium_date_time(&dt);
            match first_message_timestamp.and_then(local_dt) {
                Some(first_dt) => {
                    let days = dt
                        .timestamp()
                        .duration_since(first_dt.timestamp())
                        .as_hours()
                        / 24;
                    if days == 0 {
                        abs
                    } else {
                        t!("main.date.relative_offset", date = abs, days = days).into_owned()
                    }
                }
                None => abs,
            }
        }
        DateDisplay::Absolute => medium_date_time(&dt),
    }
}

#[cfg(test)]
mod tests {
    use super::{QuotedSender, local_dt, quoted_sender, quotes_shown, short_date};

    fn quote_card(message: Option<usize>) -> Option<QuotedSender> {
        Some(QuotedSender {
            name: String::new(),
            address: None,
            date: None,
            recipients: None,
            copies: None,
            message,
        })
    }

    #[test]
    fn quotes_older_than_a_message_of_the_conversation_stay_under_that_message() {
        // A reply quoting today's message, then one of two weeks ago the
        // conversation does not hold: neither gets a card under the reply,
        // or the older one would sit above today's message.
        assert_eq!(quotes_shown(&[quote_card(Some(1)), quote_card(None)], 2), 0);
        assert_eq!(quotes_shown(&[quote_card(None), quote_card(Some(2))], 2), 1);
        assert_eq!(quotes_shown(&[quote_card(None), None], 2), 2);
        assert_eq!(quotes_shown(&[], 3), 3);
    }
    use crate::db::{DateDisplay, ThreadMessage};

    fn message(from_name: &str, from_address: &str, to: &str) -> ThreadMessage {
        ThreadMessage {
            id: from_address.to_owned(),
            thread_id: "thread".to_owned(),
            account_id: "account".to_owned(),
            from_name: Some(from_name.to_owned()),
            from_address: Some(from_address.to_owned()),
            to_addresses: Some(to.to_owned()),
            cc_addresses: None,
            date: Some(1_760_000_000_000),
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
            list_unsubscribe: None,
            list_unsubscribe_post: None,
            auth_results: None,
            is_own_message: false,
        }
    }

    fn quote(name: &str, address: &str) -> mail_html::QuoteHeader {
        mail_html::QuoteHeader {
            name: Some(name.to_owned()),
            address: Some(address.to_owned()),
            date: Some("Monday at 10:02".to_owned()),
            to: None,
            cc: None,
        }
    }

    #[test]
    fn a_quote_of_a_message_of_the_conversation_shows_that_message() {
        // Newest first: Ann's reply quotes Bob's message, which quotes Ann's.
        let conversation = [
            message("Ann Example", "ann@example.com", "bob@example.com"),
            message("Bob Example", "bob@example.com", "ann@example.com"),
            message("Ann Example", "ann@example.com", "team@example.com"),
        ];
        let first = quoted_sender(
            &conversation,
            0,
            0,
            &quote("Bob", "BOB@example.com"),
            DateDisplay::Absolute,
        );
        assert_eq!(first.name, "Bob Example");
        assert_eq!(first.recipients.as_deref(), Some("ann@example.com"));
        // It is a message of the conversation: its card can answer it.
        assert_eq!(first.message, Some(1));
        assert_ne!(first.date.as_deref(), Some("Monday at 10:02"));
        let second = quoted_sender(
            &conversation,
            0,
            1,
            &quote("Ann", "ann@example.com"),
            DateDisplay::Absolute,
        );
        assert_eq!(second.recipients.as_deref(), Some("team@example.com"));
    }

    #[test]
    fn a_quote_the_conversation_does_not_hold_shows_what_the_mail_wrote() {
        let conversation = [
            message("Ann Example", "ann@example.com", "bob@example.com"),
            message("Bob Example", "bob@example.com", "ann@example.com"),
        ];
        // Another sender than the message one place down, then no message.
        for position in [0, 1] {
            let shown = quoted_sender(
                &conversation,
                0,
                position,
                &quote("Cy", "cy@example.com"),
                DateDisplay::Absolute,
            );
            assert_eq!(shown.name, "Cy");
            assert_eq!(shown.address.as_deref(), Some("cy@example.com"));
            assert_eq!(shown.date.as_deref(), Some("Monday at 10:02"));
            assert_eq!(shown.recipients, None);
            // Not a message the conversation holds: nothing to answer.
            assert_eq!(shown.message, None);
        }
    }

    #[test]
    fn a_quote_that_names_its_sender_without_an_address_is_matched_by_name() {
        // Outlook writes "De : Bob Example", with no address.
        let named = |name: &str| mail_html::QuoteHeader {
            name: Some(name.to_owned()),
            address: None,
            date: Some("Monday at 10:02".to_owned()),
            to: None,
            cc: None,
        };
        let conversation = [
            message("Ann Example", "ann@example.com", "bob@example.com"),
            message("Bob Example", "bob@example.com", "ann@example.com"),
            message("Ann Example", "ann@example.com", "team@example.com"),
        ];
        // One place down: the message itself, whatever the case and blanks.
        let next = quoted_sender(
            &conversation,
            0,
            0,
            &named("bob  example"),
            DateDisplay::Absolute,
        );
        assert_eq!(next.name, "Bob Example");
        assert_eq!(next.address.as_deref(), Some("bob@example.com"));
        assert_eq!(next.message, Some(1));

        // Not the next one down, but written by someone of the
        // conversation: the header as written, with the address known.
        let elsewhere = quoted_sender(
            &conversation,
            1,
            0,
            &named("Bob Example"),
            DateDisplay::Absolute,
        );
        assert_eq!(elsewhere.message, None);
        assert_eq!(elsewhere.date.as_deref(), Some("Monday at 10:02"));
        assert_eq!(elsewhere.address.as_deref(), Some("bob@example.com"));

        // Nobody of the conversation: no address is made up.
        let stranger = quoted_sender(
            &conversation,
            0,
            0,
            &named("Cy Example"),
            DateDisplay::Absolute,
        );
        assert_eq!(stranger.address, None);
        assert_eq!(stranger.message, None);
    }

    #[test]
    fn a_quote_further_down_the_conversation_is_found_by_sender_and_time() {
        // Newest first: Bob writes again, quoting his own first message,
        // with the reader's reply in between.
        let mut first = message("Bob Example", "bob@example.com", "ann@example.com");
        first.date = Some(1_793_615_520_000);
        let conversation = [
            message("Bob Example", "bob@example.com", "ann@example.com"),
            message("Ann Example", "ann@example.com", "bob@example.com"),
            first,
        ];
        let when = local_dt(1_793_615_520_000).expect("a date");
        let written = |minute: i8| mail_html::QuoteHeader {
            name: Some("Bob Example".to_owned()),
            address: None,
            date: Some(format!(
                "lundi {} novembre {} {}:{minute:02}",
                when.day(),
                when.year(),
                when.hour()
            )),
            to: None,
            cc: None,
        };

        let found = quoted_sender(
            &conversation,
            0,
            0,
            &written(when.minute()),
            DateDisplay::Absolute,
        );
        // The message itself: its card can answer and forward it.
        assert_eq!(found.message, Some(2));
        assert_eq!(found.address.as_deref(), Some("bob@example.com"));

        // Same sender at another minute: another message, which the
        // conversation does not hold. Nothing to answer.
        let other = quoted_sender(
            &conversation,
            0,
            0,
            &written((when.minute() + 7) % 60),
            DateDisplay::Absolute,
        );
        assert_eq!(other.message, None);
        assert_eq!(other.address.as_deref(), Some("bob@example.com"));
    }

    #[test]
    fn short_date_shows_the_year_only_before_this_year() {
        let last_year = jiff::Zoned::now()
            .checked_sub(jiff::Span::new().years(1))
            .expect("a year ago");
        assert!(short_date(&last_year).contains(&last_year.year().to_string()));
        let today = jiff::Zoned::now();
        assert!(!short_date(&today).contains(&today.year().to_string()));
    }

    #[test]
    fn local_dt_reads_stored_milliseconds() {
        let dt = local_dt(1_793_308_640_000).expect("stored message timestamp renders");
        assert_eq!(dt.year(), 2026);
    }
}
