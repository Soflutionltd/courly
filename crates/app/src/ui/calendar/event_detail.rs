use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length};
use rust_i18n::t;

use crate::ui::layout::*;
use crate::ui::theme;

use super::format::{format_event_time_range, format_recurrence_rule, format_reminder};
use super::messages::CalendarMessage;
use super::types::CalendarEventData;

/// Compact event detail popover (~300px, quick glance).
pub(super) fn event_detail_popover(event: &CalendarEventData) -> Element<'_, CalendarMessage> {
    let mut content = column![].spacing(SPACE_SM);

    let title_text = if event.title.is_empty() {
        t!("calendar.untitled_event").into_owned()
    } else {
        event.title.clone()
    };
    let expand_btn = button(text("\u{2197}").size(TEXT_MD))
        .on_press(CalendarMessage::ExpandPopoverToModal)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    let close_btn = button(text("\u{2715}").size(TEXT_SM))
        .on_press(CalendarMessage::ClosePopover)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    let title_row = row![
        container(
            text(title_text)
                .size(TEXT_HEADING)
                .font(crate::font::text_semibold()),
        )
        .width(Length::Fill),
        expand_btn,
        close_btn,
    ]
    .align_y(Alignment::Center);
    content = content.push(title_row);

    let time_label = format_event_time_range(event);
    content = content.push(text(time_label).size(TEXT_MD).style(text::secondary));

    if !event.location.is_empty() {
        content = content.push(text(&event.location).size(TEXT_MD).style(text::secondary));
    }

    if let Some(ref rrule) = event.recurrence_rule {
        content = content.push(
            row![
                text("\u{1F501}").size(TEXT_SM),
                text(format_recurrence_rule(rrule))
                    .size(TEXT_SM)
                    .style(text::secondary),
            ]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
        );
    }

    if let Some(ref organizer) = event.organizer_name {
        if !organizer.is_empty() {
            content = content.push(
                text(t!("calendar.invited_by", name = organizer).into_owned())
                    .size(TEXT_SM)
                    .style(text::secondary),
            );
        }
    } else if let Some(ref email) = event.organizer_email
        && !email.is_empty()
    {
        content = content.push(
            text(t!("calendar.invited_by", name = email).into_owned())
                .size(TEXT_SM)
                .style(text::secondary),
        );
    }

    if !event.attendees.is_empty() {
        let mut attendee_col = column![].spacing(SPACE_XXXS);
        for att in &event.attendees {
            let status_icon = match att.rsvp_status.as_str() {
                "accepted" => "\u{2713}",
                "declined" => "\u{2717}",
                "tentative" => "~",
                _ => "?",
            };
            let display = att.name.as_deref().unwrap_or(&att.email);
            let display = if att.is_organizer {
                t!("calendar.attendee_organizer", name = display).into_owned()
            } else {
                display.to_string()
            };
            attendee_col = attendee_col.push(
                text(format!("{status_icon} {display}"))
                    .size(TEXT_SM)
                    .style(text::secondary),
            );
        }
        content = content.push(attendee_col);
    }

    if !event.description.is_empty() {
        content = content.push(
            text(&event.description)
                .size(TEXT_SM)
                .style(theme::TextClass::Muted.style()),
        );
    }

    if !event.reminders.is_empty() {
        let reminder_text = event
            .reminders
            .iter()
            .map(|r| format_reminder(r.minutes_before))
            .collect::<Vec<_>>()
            .join(", ");
        content = content.push(
            text(t!("calendar.reminders", list = reminder_text).into_owned())
                .size(TEXT_SM)
                .style(text::secondary),
        );
    }

    if let Some(ref status) = event.rsvp_status {
        let status_display = match status.as_str() {
            "accepted" => t!("calendar.rsvp.accepted"),
            "declined" => t!("calendar.rsvp.declined"),
            "tentative" => t!("calendar.rsvp.tentative"),
            "needs-action" => t!("calendar.rsvp.no_response"),
            _ => status.as_str().into(),
        };
        content = content.push(
            text(t!("calendar.your_rsvp", status = status_display).into_owned())
                .size(TEXT_SM)
                .font(crate::font::text_semibold())
                .style(text::secondary),
        );
    }

    if let (Some(event_id), Some(account_id)) = (&event.id, &event.account_id) {
        let rsvp_button = |label: std::borrow::Cow<'static, str>, response| {
            button(text(label).size(TEXT_SM))
                .on_press(CalendarMessage::RsvpEvent {
                    event_id: event_id.clone(),
                    account_id: account_id.clone(),
                    response,
                })
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Ghost.style())
        };
        content = content.push(
            row![
                rsvp_button(t!("calendar.accept"), service_api::RsvpResponse::Accepted),
                rsvp_button(t!("calendar.decline"), service_api::RsvpResponse::Declined),
                rsvp_button(
                    t!("calendar.tentative"),
                    service_api::RsvpResponse::Tentative
                ),
            ]
            .spacing(SPACE_XS),
        );
    }

    let edit_btn = button(text(t!("calendar.edit")).size(TEXT_SM))
        .on_press(CalendarMessage::OpenEventEditor(Some(event.clone())))
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Ghost.style());

    let delete_btn = if let Some(ref id) = event.id {
        button(text(t!("calendar.delete")).size(TEXT_SM))
            .on_press(CalendarMessage::ConfirmDeleteEvent {
                event_id: id.clone(),
                title: event.title.clone(),
                account_id: event.account_id.clone(),
            })
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style())
    } else {
        button(text(t!("calendar.delete")).size(TEXT_SM))
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style())
    };

    content = content.push(Space::new().height(SPACE_XS));
    content = content.push(row![edit_btn, delete_btn].spacing(SPACE_XS));

    let scrollable_content = scrollable(content).height(Length::Shrink);

    container(scrollable_content)
        .width(Length::Fixed(300.0))
        .height(Length::Fit.max(CALENDAR_OVERLAY_MAX_HEIGHT))
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}
