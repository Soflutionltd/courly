use iced::widget::{
    Space, button, column, container, pick_list, row, scrollable, text, text_input,
};
use iced::{Alignment, Element, Length};
use rust_i18n::t;

use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::undoable_text_input::undoable_text_input;

use super::format::{format_event_date, format_recurrence_rule};
use super::messages::{CalendarMessage, EventField, EventTextField};
use super::types::{CalendarEventData, CalendarListEntry};

/// Event creation/editing form (rendered as a centered modal).
pub(super) fn event_editor_card<'a>(
    event: &'a CalendarEventData,
    is_creating: bool,
    calendars: &'a [CalendarListEntry],
) -> Element<'a, CalendarMessage> {
    let heading = if is_creating {
        t!("calendar.editor.new_event")
    } else {
        t!("calendar.editor.edit_event")
    };

    let mut content = column![].spacing(SPACE_SM);

    let close_btn = button(text("\u{2715}").size(TEXT_SM))
        .on_press(CalendarMessage::CloseModal)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());
    content = content.push(
        row![
            text(heading)
                .size(TEXT_HEADING)
                .font(crate::font::text_semibold()),
            Space::new().width(Length::Fill),
            close_btn,
        ]
        .align_y(Alignment::Center),
    );

    if is_creating {
        let selected = calendars
            .iter()
            .find(|c| Some(&c.id) == event.calendar_id.as_ref())
            .cloned();
        let options: Vec<CalendarListEntry> = calendars.to_vec();
        let picker = pick_list(selected, options, |c: &CalendarListEntry| {
            c.display_name.clone()
        })
        .on_select(|entry: CalendarListEntry| {
            CalendarMessage::EventFieldChanged(EventField::CalendarSelected {
                calendar_id: Some(entry.id),
                account_id: Some(entry.account_id),
            })
        })
        .placeholder(t!("calendar.editor.select_calendar"))
        .text_size(TEXT_MD)
        .padding(PAD_INPUT)
        .width(Length::Fill)
        .style(theme::PickListClass::Ghost.style());
        content = content.push(form_field(t!("calendar.editor.calendar"), picker.into()));
    } else {
        let label = event
            .calendar_name
            .clone()
            .or_else(|| event.calendar_id.clone())
            .unwrap_or_else(|| t!("calendar.editor.unknown_calendar").into_owned());
        content = content.push(form_field(
            t!("calendar.editor.calendar"),
            text(label).size(TEXT_MD).into(),
        ));
    }

    content = content.push(form_field(
        t!("calendar.editor.title"),
        undoable_text_input(t!("calendar.editor.title_placeholder"), &event.title)
            .on_input(|s| CalendarMessage::EventFieldChanged(EventField::Title(s)))
            .on_undo(CalendarMessage::EventFieldUndo(EventTextField::Title))
            .on_redo(CalendarMessage::EventFieldRedo(EventTextField::Title))
            .padding(PAD_INPUT)
            .size(TEXT_MD)
            .into(),
    ));

    let date_str = format_event_date(event.start_date);
    content = content.push(form_field(
        t!("calendar.editor.date"),
        text(date_str).size(TEXT_MD).into(),
    ));

    if !event.all_day {
        let time_row = time_input_row(event);
        content = content.push(form_field(t!("calendar.editor.time"), time_row));
    }

    let all_day_label = if event.all_day {
        t!("calendar.editor.all_day_yes")
    } else {
        t!("calendar.editor.all_day_no")
    };
    let all_day_btn = button(text(all_day_label).size(TEXT_SM))
        .on_press(CalendarMessage::EventFieldChanged(EventField::AllDay(
            !event.all_day,
        )))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());
    content = content.push(all_day_btn);

    content = content.push(form_field(
        t!("calendar.editor.location"),
        undoable_text_input(t!("calendar.editor.location_placeholder"), &event.location)
            .on_input(|s| CalendarMessage::EventFieldChanged(EventField::Location(s)))
            .on_undo(CalendarMessage::EventFieldUndo(EventTextField::Location))
            .on_redo(CalendarMessage::EventFieldRedo(EventTextField::Location))
            .padding(PAD_INPUT)
            .size(TEXT_MD)
            .into(),
    ));

    content = content.push(form_field(
        t!("calendar.editor.description"),
        undoable_text_input(
            t!("calendar.editor.description_placeholder"),
            &event.description,
        )
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::Description(s)))
        .on_undo(CalendarMessage::EventFieldUndo(EventTextField::Description))
        .on_redo(CalendarMessage::EventFieldRedo(EventTextField::Description))
        .padding(PAD_INPUT)
        .size(TEXT_MD)
        .into(),
    ));

    let tz_display = event
        .timezone
        .clone()
        .unwrap_or_else(|| t!("calendar.editor.timezone_local").into_owned());
    let tz_input = text_input(t!("calendar.editor.timezone_placeholder"), tz_display)
        .on_input(|s| {
            let tz = if s.is_empty() { None } else { Some(s) };
            CalendarMessage::EventFieldChanged(EventField::Timezone(tz))
        })
        .padding(PAD_INPUT)
        .size(TEXT_SM);
    content = content.push(form_field(t!("calendar.editor.timezone"), tz_input.into()));

    let avail = event.availability.as_deref().unwrap_or("busy");
    let avail_options = ["busy", "free", "tentative", "oof"];
    let mut avail_row = row![].spacing(SPACE_XXS);
    for opt in &avail_options {
        let is_active = avail == *opt;
        let label = match *opt {
            "busy" => t!("calendar.editor.busy"),
            "free" => t!("calendar.editor.free"),
            "tentative" => t!("calendar.editor.tentative"),
            "oof" => t!("calendar.editor.out_of_office"),
            _ => (*opt).into(),
        };
        avail_row = avail_row.push(
            button(text(label).size(TEXT_XS).style(if is_active {
                text::primary
            } else {
                text::secondary
            }))
            .on_press(CalendarMessage::EventFieldChanged(
                EventField::Availability(Some((*opt).to_string())),
            ))
            .padding(PAD_ICON_BTN)
            .style(if is_active {
                theme::ButtonClass::Nav { active: true }.style()
            } else {
                theme::ButtonClass::Ghost.style()
            }),
        );
    }
    content = content.push(form_field(
        t!("calendar.editor.availability"),
        avail_row.into(),
    ));

    let vis = event.visibility.as_deref().unwrap_or("default");
    let vis_options = ["default", "public", "private"];
    let mut vis_row = row![].spacing(SPACE_XXS);
    for opt in &vis_options {
        let is_active = vis == *opt;
        let label = match *opt {
            "default" => t!("calendar.editor.visibility_default"),
            "public" => t!("calendar.editor.visibility_public"),
            "private" => t!("calendar.editor.visibility_private"),
            _ => (*opt).into(),
        };
        vis_row = vis_row.push(
            button(text(label).size(TEXT_XS).style(if is_active {
                text::primary
            } else {
                text::secondary
            }))
            .on_press(CalendarMessage::EventFieldChanged(EventField::Visibility(
                Some((*opt).to_string()),
            )))
            .padding(PAD_ICON_BTN)
            .style(if is_active {
                theme::ButtonClass::Nav { active: true }.style()
            } else {
                theme::ButtonClass::Ghost.style()
            }),
        );
    }
    content = content.push(form_field(t!("calendar.editor.visibility"), vis_row.into()));

    let has_recurrence = event.recurrence_rule.is_some();
    let recurrence_label = if has_recurrence {
        t!(
            "calendar.editor.recurring",
            rule = format_recurrence_rule(event.recurrence_rule.as_deref().unwrap_or(""))
        )
        .into_owned()
    } else {
        t!("calendar.editor.not_recurring").into_owned()
    };
    let recurrence_toggle = button(text(recurrence_label).size(TEXT_SM))
        .on_press(CalendarMessage::EventFieldChanged(
            EventField::RecurrenceRule(if has_recurrence {
                None
            } else {
                Some("RRULE:FREQ=WEEKLY".to_string())
            }),
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());
    content = content.push(form_field(
        t!("calendar.editor.recurrence"),
        recurrence_toggle.into(),
    ));

    let can_save = event.calendar_id.is_some();
    let save_btn = button(text(t!("calendar.editor.save")).size(TEXT_SM))
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Nav { active: true }.style());
    let save_btn = if can_save {
        save_btn.on_press(CalendarMessage::SaveEvent)
    } else {
        save_btn
    };

    let cancel_btn = button(text(t!("calendar.editor.cancel")).size(TEXT_SM))
        .on_press(CalendarMessage::CloseModal)
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Ghost.style());

    let mut action_row = row![save_btn, cancel_btn].spacing(SPACE_XS);

    if !is_creating && let Some(ref id) = event.id {
        action_row = action_row.push(Space::new().width(Length::Fill));
        action_row = action_row.push(
            button(
                text(t!("calendar.delete"))
                    .size(TEXT_SM)
                    .style(text::danger),
            )
            .on_press(CalendarMessage::ConfirmDeleteEvent {
                event_id: id.clone(),
                title: event.title.clone(),
                account_id: event.account_id.clone(),
            })
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style()),
        );
    }

    content = content.push(Space::new().height(SPACE_XS));
    content = content.push(action_row);

    let scrollable_content = scrollable(content).height(Length::Shrink);

    container(scrollable_content)
        .width(Length::Fixed(CALENDAR_OVERLAY_WIDTH))
        .height(Length::Fit.max(CALENDAR_OVERLAY_MAX_HEIGHT))
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}

/// A labeled form field row: label on left, widget on right.
fn form_field<'a>(
    label: impl text::IntoFragment<'a>,
    widget: Element<'a, CalendarMessage>,
) -> Element<'a, CalendarMessage> {
    row![
        container(
            text(label)
                .size(TEXT_SM)
                .style(theme::TextClass::Muted.style()),
        )
        .width(Length::Fixed(CALENDAR_FORM_LABEL_WIDTH))
        .height(CALENDAR_FORM_ROW_HEIGHT)
        .align_y(Alignment::Center),
        container(widget)
            .width(Length::Fill)
            .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

/// Time input row with start and end hour:minute text inputs.
fn time_input_row(event: &CalendarEventData) -> Element<'_, CalendarMessage> {
    let start_h = text_input("HH", &event.start_hour)
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::StartHour(s)))
        .padding(PAD_INPUT)
        .width(Length::Fixed(48.0))
        .size(TEXT_MD);

    let start_m = text_input("MM", &event.start_minute)
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::StartMinute(s)))
        .padding(PAD_INPUT)
        .width(Length::Fixed(48.0))
        .size(TEXT_MD);

    let end_h = text_input("HH", &event.end_hour)
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::EndHour(s)))
        .padding(PAD_INPUT)
        .width(Length::Fixed(48.0))
        .size(TEXT_MD);

    let end_m = text_input("MM", &event.end_minute)
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::EndMinute(s)))
        .padding(PAD_INPUT)
        .width(Length::Fixed(48.0))
        .size(TEXT_MD);

    row![
        start_h,
        text(":").size(TEXT_MD),
        start_m,
        text("\u{2013}").size(TEXT_MD),
        end_h,
        text(":").size(TEXT_MD),
        end_m,
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center)
    .into()
}
