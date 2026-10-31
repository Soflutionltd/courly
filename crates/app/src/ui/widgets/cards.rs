#![allow(dead_code)]

use std::borrow::Cow;
use std::path::Path;

use iced::widget::{Space, button, column, container, mouse_area, row, text};
use iced::{Alignment, Element, Length, Theme};
use rust_i18n::t;

use crate::db::{DateDisplay, Thread, ThreadMessage};
use crate::font;
use crate::icon;
use crate::ui::label_paint::LabelPaint;
use crate::ui::layout::{
    AVATAR_MESSAGE_CARD, AVATAR_THREAD_CARD, ICON_SM, ICON_XS, PAD_BADGE, PAD_BODY, PAD_CARD,
    PAD_HTML_BODY, PAD_ICON_BTN, PAD_THREAD_CARD, SPACE_SM, SPACE_XS, SPACE_XXS, SPACE_XXXS,
    TEXT_LG, TEXT_MD, TEXT_SM, TEXT_XS, THREAD_CARD_HEIGHT,
};
use crate::ui::theme::{self, RowActionTone};

use super::avatars::{avatar_circle, label_dot, sender_avatar};

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

/// "Mar 12", "12 mars".
pub(crate) fn short_date(dt: &jiff::Zoned) -> String {
    t!(
        "main.date.short",
        month = crate::i18n::month_short(dt.month()),
        day = dt.day()
    )
    .into_owned()
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

#[cfg_attr(feature = "hotpath", hotpath::measure)]
/// Pin, delete or flag, from the buttons kept visible on every row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadRowAction {
    Pin,
    Trash,
    Flag,
}

pub fn thread_card<'a, M: Clone + 'a>(
    thread: &'a Thread,
    index: usize,
    selected: bool,
    label_paints: &[LabelPaint],
    bimi_logo: Option<&Path>,
    (on_press, on_release, on_exit): (
        impl Fn(usize) -> M,
        impl Fn(usize) -> M,
        impl Fn(usize) -> M,
    ),
    on_action: impl Fn(usize, ThreadRowAction) -> M,
) -> Element<'a, M> {
    let sender = sender_or_unknown(thread.from_name.as_deref(), thread.from_address.as_deref());

    let subject = thread
        .subject
        .as_deref()
        .map_or_else(|| t!("main.no_subject"), Cow::Borrowed);
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

    let sender_font = if thread.is_read {
        font::text()
    } else {
        font::text_semibold()
    };

    let subject_style: fn(&Theme) -> text::Style = if thread.is_read {
        theme::TextClass::Muted.style()
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

    let top_row = row![
        container(
            text(sender.clone())
                .size(TEXT_MD)
                .style(text::base)
                .font(sender_font)
                .wrapping(text::Wrapping::None)
                .ellipsis(text::Ellipsis::End)
                .width(Length::Fill),
        )
        .width(Length::Fill),
        container(
            text(date_str)
                .size(TEXT_XS)
                .style(theme::TextClass::Tertiary.style())
        ),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    let actions = row![
        row_action(
            icon::pin(),
            RowActionTone::Accent,
            thread.is_pinned,
            on_action(index, ThreadRowAction::Pin),
        ),
        row_action(
            icon::trash(),
            RowActionTone::Danger,
            false,
            on_action(index, ThreadRowAction::Trash),
        ),
        row_action(
            icon::flag(),
            RowActionTone::Warning,
            thread.is_starred,
            on_action(index, ThreadRowAction::Flag),
        ),
    ]
    .spacing(SPACE_XXXS)
    .align_y(Alignment::Center);

    let subject_row = row![
        container(
            text(subject)
                .size(TEXT_MD)
                .style(subject_style)
                .font(font::text())
                .wrapping(text::Wrapping::None)
                .ellipsis(text::Ellipsis::End)
                .width(Length::Fill),
        )
        .width(Length::Fill),
    ]
    // Selecting a local draft opens it in the compose, so its row carries
    // no actions that would select it first.
    .push((!thread.is_local_draft).then_some(actions))
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
            .into()
        } else {
            container(
                text(snippet)
                    .size(TEXT_SM)
                    .style(text::secondary)
                    .wrapping(text::Wrapping::None)
                    .ellipsis(text::Ellipsis::End)
                    .width(Length::Fill),
            )
            .width(Length::Fill)
            .into()
        };

    let snippet_row = row![snippet_slot, indicators]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center);

    let text_content = column![top_row, subject_row, snippet_row]
        .spacing(SPACE_XXXS)
        .width(Length::Fill);

    let avatar = sender_avatar(&sender, bimi_logo, AVATAR_THREAD_CARD);

    let content = row![avatar, text_content]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center);

    // The press goes to the inner area, so a row can start a drag; a click
    // is a release on the row pressed. The button only gives the styling.
    let hit = mouse_area(
        container(content)
            .padding(PAD_THREAD_CARD)
            .height(THREAD_CARD_HEIGHT)
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

    let avatar = avatar_circle(&sender, AVATAR_MESSAGE_CARD);
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
    invite: Option<Element<'a, M>>,
    html_body: Option<Element<'a, M>>,
    on_link: impl Fn(String) -> M + 'a,
    on_toggle: impl Fn(usize) -> M,
    on_pop_out: impl Fn(usize) -> M,
    on_reply: impl Fn(usize) -> M,
    on_reply_all: impl Fn(usize) -> M,
    on_forward: impl Fn(usize) -> M,
    on_edit_contact: impl Fn(String) -> M + 'a,
    on_create_event: impl Fn(usize) -> M,
) -> Element<'a, M> {
    let sender_name = sender_or_unknown(msg.from_name.as_deref(), msg.from_address.as_deref());
    let sender_email_str = msg.from_address.as_deref().unwrap_or("");

    let avatar = avatar_circle(&sender_name, AVATAR_MESSAGE_CARD);
    let date_str = format_message_date(msg.date, first_message_date, date_display);

    let recipients = msg.to_addresses.as_deref().unwrap_or("");

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

    let collapse_btn = button(icon::chevron_up().size(ICON_SM).style(text::secondary))
        .on_press(on_toggle(index))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    let message_actions = row![
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
                collapse_btn,
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

    let to_row = row![
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
            left: 0.0
        })
        .align_x(Alignment::End),
        text(recipients).size(TEXT_SM).style(text::base),
    ]
    .spacing(0)
    .align_y(Alignment::Start);

    let header = column![avatar_rows, to_row].spacing(SPACE_XXS);

    let body_inner: Element<'_, M> = if let Some(rendered) = html_body {
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

    let body: Element<'_, M> = container(body_inner)
        .width(Length::Fill)
        .style(theme::ContainerClass::EmailBody.style())
        .into();

    let mut card_content = column![header].spacing(SPACE_XS);
    if let Some(invite) = invite {
        card_content = card_content.push(invite);
    }
    card_content = card_content.push(body);

    container(card_content)
        .padding(PAD_CARD)
        .width(Length::Fill)
        .style(theme::ContainerClass::MessageCard.style())
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

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn collapsed_message_row<'a, M: Clone + 'a>(
    msg: &'a ThreadMessage,
    index: usize,
    on_toggle: impl Fn(usize) -> M,
) -> Element<'a, M> {
    let sender = sender_or_unknown(msg.from_name.as_deref(), msg.from_address.as_deref());

    let short_date = msg
        .date
        .and_then(|ts| local_dt(ts).map(|dt| short_date(&dt)))
        .unwrap_or_default();

    let snippet = truncate_snippet(msg.snippet.as_deref(), 60);

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

    let content = row![
        container(
            icon::chevron_right()
                .size(ICON_XS)
                .style(theme::TextClass::Tertiary.style())
        )
        .align_y(Alignment::Center),
        container(
            text(sender)
                .size(TEXT_SM)
                .font(font::text_semibold())
                .style(text::base),
        )
        .align_y(Alignment::Center),
        container(
            text("\u{00B7}")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .align_y(Alignment::Center),
        container(
            text(short_date)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .align_y(Alignment::Center),
        state_indicators,
        container(
            text("\u{00B7}")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .align_y(Alignment::Center),
        container(
            text(snippet)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
                .wrapping(text::Wrapping::None),
        )
        .width(Length::Fill)
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    button(container(content).width(Length::Fill))
        .on_press(on_toggle(index))
        .padding(0)
        .style(theme::ButtonClass::CollapsedMessage.style())
        .width(Length::Fill)
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

fn truncate_snippet(snippet: Option<&str>, max_chars: usize) -> String {
    let s = snippet.unwrap_or("");
    if s.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", &s[..s.floor_char_boundary(max_chars)])
    }
}

#[cfg(test)]
mod tests {
    use super::local_dt;

    #[test]
    fn local_dt_reads_stored_milliseconds() {
        let dt = local_dt(1_793_308_640_000).expect("stored message timestamp renders");
        assert_eq!(dt.year(), 2026);
    }
}
