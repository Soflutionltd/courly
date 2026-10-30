//! My Day: a mini calendar over a week-long agenda of the calendars synced
//! from the user's accounts (Outlook, Google, CalDAV). Events open and are
//! created through the calendar's own popover and editor.

use std::collections::HashSet;

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Border, Element, Length, Padding, Theme};
use jiff::Span;
use jiff::civil::{Date, Weekday};
use rust_i18n::t;

use crate::icon;
use crate::ui::calendar::local_midnight_unix;
use crate::ui::calendar_month::{self, DayGrid};
use crate::ui::calendar_time_grid::TimeGridEvent;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;

/// Days the agenda lists, from the chosen day on.
const AGENDA_DAYS: i64 = 7;
/// Weeks the folded mini calendar shows.
const FOLDED_WEEKS: u8 = 2;
/// Weeks the unfolded mini calendar can need for one month.
const MONTH_WEEKS: u8 = 6;

pub struct MyDay {
    /// Month the unfolded mini calendar shows.
    pub year: i32,
    pub month: u32,
    /// First day of the agenda.
    pub start: Date,
    pub today: Date,
    pub unfolded: bool,
    pub events: Vec<TimeGridEvent>,
    dates_with_events: HashSet<Date>,
    generation: u64,
}

#[derive(Debug, Clone)]
pub enum MyDayMessage {
    PrevMonth,
    NextMonth,
    SelectDate(Date),
    ToggleMonth,
    Today,
    NewEvent,
    OpenEvent(String),
    Loaded(u64, Result<Vec<TimeGridEvent>, String>),
    /// Minute tick: moves the agenda along when the day changes.
    Tick,
}

impl MyDay {
    pub fn new(today: Date) -> Self {
        Self {
            year: i32::from(today.year()),
            month: u32::from(today.month().unsigned_abs()),
            start: today,
            today,
            unfolded: false,
            events: Vec::new(),
            dates_with_events: HashSet::new(),
            generation: 0,
        }
    }

    /// Makes `date` the first agenda day and the month the mini calendar shows.
    pub fn select(&mut self, date: Date) {
        self.start = date;
        self.year = i32::from(date.year());
        self.month = u32::from(date.month().unsigned_abs());
    }

    pub fn shift_month(&mut self, months: i64) {
        if let Some(first) = self
            .first_of_month()
            .and_then(|first| first.checked_add(Span::new().months(months)).ok())
        {
            self.year = i32::from(first.year());
            self.month = u32::from(first.month().unsigned_abs());
        }
    }

    /// Follows the clock past midnight. An agenda left on the old today
    /// moves to the new one; one the user moved elsewhere stays put.
    /// Returns whether the view changed.
    pub fn roll_over(&mut self, today: Date) -> bool {
        if today == self.today {
            return false;
        }
        if self.start == self.today {
            self.select(today);
        }
        self.today = today;
        true
    }

    /// Starts a load: its generation and the window, in Unix seconds,
    /// covering both the agenda and every day the mini calendar can show.
    pub fn begin_load(&mut self, week_start: Weekday) -> (u64, i64, i64) {
        self.generation += 1;
        let agenda_week = calendar_month::rewind_to_weekday(self.start, week_start);
        let month_grid = self.first_of_month().map_or(agenda_week, |first| {
            calendar_month::rewind_to_weekday(first, week_start)
        });
        let from = agenda_week.min(month_grid);
        let to = [
            add_days(self.start, AGENDA_DAYS),
            add_days(agenda_week, i64::from(FOLDED_WEEKS) * 7),
            add_days(month_grid, i64::from(MONTH_WEEKS) * 7),
        ]
        .into_iter()
        .max()
        .unwrap_or(self.start);
        (
            self.generation,
            local_midnight_unix(from),
            local_midnight_unix(to),
        )
    }

    /// Keeps a load's events unless a newer load started since.
    pub fn finish_load(&mut self, generation: u64, events: Vec<TimeGridEvent>) -> bool {
        if generation != self.generation {
            return false;
        }
        self.dates_with_events = event_dates(&events);
        self.events = events;
        true
    }

    /// Whether a load result is still the one awaited.
    pub fn is_current(&self, generation: u64) -> bool {
        generation == self.generation
    }

    fn first_of_month(&self) -> Option<Date> {
        Date::new(
            i16::try_from(self.year).ok()?,
            i8::try_from(self.month).ok()?,
            1,
        )
        .ok()
    }
}

fn add_days(date: Date, days: i64) -> Date {
    date.checked_add(Span::new().days(days)).unwrap_or(date)
}

/// Every day an event covers. An end is exclusive: an all-day event ends at
/// the next midnight, and a meeting ending at 00:00 does not reach that day.
fn event_dates(events: &[TimeGridEvent]) -> HashSet<Date> {
    let mut dates = HashSet::new();
    for event in events {
        let (Some(mut day), Some(last)) = (
            local_date(event.start_time),
            local_date(event.end_time.max(event.start_time + 1) - 1),
        ) else {
            continue;
        };
        while day <= last {
            dates.insert(day);
            match day.tomorrow() {
                Ok(next) => day = next,
                Err(_) => break,
            }
        }
    }
    dates
}

fn local_date(seconds: i64) -> Option<Date> {
    jiff::Timestamp::from_second(seconds)
        .ok()
        .map(|instant| instant.to_zoned(jiff::tz::TimeZone::system()).date())
}

/// The events on `day`, all-day ones first, then by start.
fn events_on(events: &[TimeGridEvent], day: Date) -> Vec<&TimeGridEvent> {
    let day_start = local_midnight_unix(day);
    let day_end = local_midnight_unix(add_days(day, 1));
    let mut on: Vec<&TimeGridEvent> = events
        .iter()
        .filter(|event| {
            let end = event.end_time.max(event.start_time + 1);
            event.start_time < day_end && end > day_start
        })
        .collect();
    on.sort_by_key(|event| (!event.all_day, event.start_time));
    on
}

pub fn view(state: &MyDay, week_start: Weekday) -> Element<'_, MyDayMessage> {
    let mut agenda = column![].spacing(SPACE_0);
    for offset in 0..AGENDA_DAYS {
        let day = add_days(state.start, offset);
        agenda = agenda.push(day_header(day, state.today));
        let events = events_on(&state.events, day);
        if events.is_empty() {
            agenda = agenda.push(
                container(
                    text(t!("calendar.my_day.no_events"))
                        .size(TEXT_SM)
                        .style(text::secondary),
                )
                .padding(Padding::from([SPACE_XS, SPACE_SM])),
            );
        }
        for event in events {
            agenda = agenda.push(event_row(event, day));
        }
    }

    column![mini_calendar(state, week_start), agenda]
        .spacing(SPACE_XS)
        .into()
}

fn mini_calendar(state: &MyDay, week_start: Weekday) -> Element<'_, MyDayMessage> {
    let (year, month) = if state.unfolded {
        (state.year, state.month)
    } else {
        (
            i32::from(state.start.year()),
            u32::from(state.start.month().unsigned_abs()),
        )
    };
    let month_name = i8::try_from(month).map_or_else(|_| String::new(), crate::i18n::month_name);
    let mut header = row![
        container(
            text(t!("calendar.month_year", month = month_name, year = year))
                .size(TEXT_XL)
                .style(text::base)
                .font(crate::font::text_semibold()),
        )
        .width(Length::Fill)
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center);
    if state.unfolded {
        header = header
            .push(widgets::tip_icon_button(
                icon::chevron_left(),
                t!("calendar.my_day.previous_month"),
                false,
                MyDayMessage::PrevMonth,
            ))
            .push(widgets::tip_icon_button(
                icon::chevron_right(),
                t!("calendar.my_day.next_month"),
                false,
                MyDayMessage::NextMonth,
            ));
    }
    if state.start != state.today {
        header = header.push(
            button(
                text(t!("calendar.my_day.today"))
                    .size(TEXT_SM)
                    .style(text::secondary),
            )
            .on_press(MyDayMessage::Today)
            .padding(PAD_ICON_BTN)
            .style(theme::ButtonClass::Action.style()),
        );
    }
    header = header.push(widgets::tip_icon_button(
        icon::plus(),
        t!("calendar.my_day.new_event"),
        false,
        MyDayMessage::NewEvent,
    ));

    let grid = match state.first_of_month().filter(|_| state.unfolded) {
        Some(first) => {
            let first_day = calendar_month::rewind_to_weekday(first, week_start);
            let weeks = if add_days(first_day, 35) <= first.last_of_month() {
                MONTH_WEEKS
            } else {
                MONTH_WEEKS - 1
            };
            DayGrid {
                first_day,
                weeks: usize::from(weeks),
                month: Some((year, month)),
                selected: Some(state.start),
                today: state.today,
                week_start,
                dates_with_events: &state.dates_with_events,
            }
        }
        None => DayGrid {
            first_day: calendar_month::rewind_to_weekday(state.start, week_start),
            weeks: usize::from(FOLDED_WEEKS),
            month: None,
            selected: Some(state.start),
            today: state.today,
            week_start,
            dates_with_events: &state.dates_with_events,
        },
    };

    let (fold_icon, fold_tip) = if state.unfolded {
        (icon::chevron_up(), t!("calendar.my_day.hide_month"))
    } else {
        (icon::chevron_down(), t!("calendar.my_day.show_month"))
    };
    let fold = container(widgets::tip_icon_button(
        fold_icon,
        fold_tip,
        false,
        MyDayMessage::ToggleMonth,
    ))
    .center_x(Length::Fill);

    container(
        column![
            header,
            calendar_month::day_grid(grid, MyDayMessage::SelectDate),
            fold
        ]
        .spacing(SPACE_XXS),
    )
    .padding(PAD_SIDE_PANEL)
    .into()
}

/// "Today • Friday • October 30" over the day's events.
fn day_header<'a>(day: Date, today: Date) -> Element<'a, MyDayMessage> {
    let date = t!(
        "calendar.my_day.date",
        weekday = crate::i18n::weekday_name(day.weekday()),
        day = day.day(),
        month = crate::i18n::month_name(day.month())
    );
    let relative = if day == today {
        Some(t!("calendar.my_day.today"))
    } else if today.tomorrow().ok() == Some(day) {
        Some(t!("calendar.my_day.tomorrow"))
    } else {
        None
    };
    let label = match relative {
        Some(relative) => t!("calendar.my_day.relative", relative = relative, date = date),
        None => date,
    };
    let style: fn(&Theme) -> text::Style = if day == today {
        theme::TextClass::Accent.style()
    } else {
        text::base
    };
    container(
        text(label)
            .size(TEXT_SM)
            .style(style)
            .font(crate::font::text_semibold()),
    )
    .padding(Padding::from([SPACE_XXS, SPACE_SM]))
    .width(Length::Fill)
    .style(theme::ContainerClass::Surface.style())
    .into()
}

/// Start time and length, a calendar color bar, then title and place.
fn event_row(event: &TimeGridEvent, day: Date) -> Element<'_, MyDayMessage> {
    let starts_today = local_date(event.start_time) == Some(day);
    let mut timing = column![].spacing(SPACE_XXXS);
    if event.all_day || !starts_today {
        timing = timing.push(
            text(t!("calendar.my_day.all_day"))
                .size(TEXT_XS)
                .style(text::base),
        );
    } else {
        if let Ok(instant) = jiff::Timestamp::from_second(event.start_time) {
            let start = crate::i18n::time_of_day(&instant.to_zoned(jiff::tz::TimeZone::system()));
            timing = timing.push(text(start).size(TEXT_SM).style(text::base));
        }
        timing = timing.push(
            text(length_label((event.end_time - event.start_time) / 60))
                .size(TEXT_XS)
                .style(theme::TextClass::Tertiary.style()),
        );
    }

    let color = theme::hex_to_color(&event.color);
    let bar = container(Space::new())
        .width(MY_DAY_BAR_WIDTH)
        .height(Length::Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(color.into()),
            border: Border::default().rounded(RADIUS_SM),
            ..container::Style::default()
        });

    let title = if event.title.trim().is_empty() {
        t!("calendar.untitled_event").into_owned()
    } else {
        event.title.clone()
    };
    let mut details = column![
        text(title)
            .size(TEXT_SM)
            .style(text::base)
            .font(crate::font::text_semibold())
            .wrapping(text::Wrapping::None),
    ]
    .spacing(SPACE_XXXS)
    .width(Length::Fill);
    if let Some(location) = event.location.as_deref().filter(|l| !l.trim().is_empty()) {
        details = details.push(
            text(location)
                .size(TEXT_XS)
                .style(theme::TextClass::Tertiary.style())
                .wrapping(text::Wrapping::None),
        );
    }

    button(
        row![container(timing).width(MY_DAY_TIME_WIDTH), bar, details]
            .spacing(SPACE_XS)
            .height(Length::Shrink),
    )
    .on_press(MyDayMessage::OpenEvent(event.id.clone()))
    .padding(Padding::from([SPACE_XS, SPACE_SM]))
    .width(Length::Fill)
    .style(theme::ButtonClass::Ghost.style())
    .into()
}

/// "25 min", "1 h 30", "2 h".
fn length_label(minutes: i64) -> String {
    let minutes = minutes.max(0);
    let (hours, rest) = (minutes / 60, minutes % 60);
    match (hours, rest) {
        (0, rest) => t!("calendar.my_day.minutes", count = rest),
        (hours, 0) => t!("calendar.my_day.hours", count = hours),
        (hours, rest) => t!(
            "calendar.my_day.hours_minutes",
            hours = hours,
            minutes = rest
        ),
    }
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, start: i64, end: i64, all_day: bool) -> TimeGridEvent {
        TimeGridEvent {
            id: id.to_owned(),
            title: id.to_owned(),
            start_time: start,
            end_time: end,
            all_day,
            color: "#4285f4".to_owned(),
            calendar_name: None,
            location: None,
            recurrence_rule: None,
            calendar_id: None,
            account_id: "account".to_owned(),
            organizer_name: None,
            organizer_email: None,
            rsvp_status: None,
            description: None,
            availability: None,
            visibility: None,
            timezone: None,
        }
    }

    fn day(year: i16, month: i8, day: i8) -> Date {
        Date::new(year, month, day).expect("valid date")
    }

    #[test]
    fn an_all_day_event_stays_on_its_own_day() {
        let friday = day(2026, 10, 30);
        let saturday = day(2026, 10, 31);
        let events = vec![event(
            "holiday",
            local_midnight_unix(friday),
            local_midnight_unix(saturday),
            true,
        )];
        assert_eq!(events_on(&events, friday).len(), 1);
        assert!(events_on(&events, saturday).is_empty());
        assert_eq!(event_dates(&events), HashSet::from([friday]));
    }

    #[test]
    fn a_meeting_ending_at_midnight_does_not_reach_the_next_day() {
        let friday = day(2026, 10, 30);
        let saturday = day(2026, 10, 31);
        let midnight = local_midnight_unix(saturday);
        let events = vec![event("late", midnight - 3600, midnight, false)];
        assert_eq!(events_on(&events, friday).len(), 1);
        assert!(events_on(&events, saturday).is_empty());
    }

    #[test]
    fn all_day_events_come_first_then_by_start() {
        let friday = day(2026, 10, 30);
        let midnight = local_midnight_unix(friday);
        let events = vec![
            event(
                "afternoon",
                midnight + 14 * 3600,
                midnight + 15 * 3600,
                false,
            ),
            event("morning", midnight + 9 * 3600, midnight + 10 * 3600, false),
            event(
                "all-day",
                midnight,
                local_midnight_unix(add_days(friday, 1)),
                true,
            ),
        ];
        let order: Vec<&str> = events_on(&events, friday)
            .iter()
            .map(|event| event.id.as_str())
            .collect();
        assert_eq!(order, ["all-day", "morning", "afternoon"]);
    }

    #[test]
    fn the_load_window_covers_the_agenda_and_the_month_grid() {
        let mut state = MyDay::new(day(2026, 10, 30));
        let (_, from, to) = state.begin_load(Weekday::Monday);
        assert!(from <= local_midnight_unix(day(2026, 9, 28)));
        assert!(to >= local_midnight_unix(day(2026, 11, 6)));
        assert!(to >= local_midnight_unix(day(2026, 11, 8)));
    }

    #[test]
    fn a_stale_load_is_dropped() {
        let mut state = MyDay::new(day(2026, 10, 30));
        let (first, _, _) = state.begin_load(Weekday::Monday);
        let (second, _, _) = state.begin_load(Weekday::Monday);
        assert!(!state.finish_load(first, vec![event("old", 0, 1, false)]));
        assert!(state.events.is_empty());
        assert!(state.finish_load(second, Vec::new()));
    }

    #[test]
    fn midnight_moves_an_agenda_left_on_today() {
        let mut state = MyDay::new(day(2026, 10, 30));
        assert!(state.roll_over(day(2026, 10, 31)));
        assert_eq!(state.start, day(2026, 10, 31));

        let mut moved = MyDay::new(day(2026, 10, 30));
        moved.select(day(2026, 11, 12));
        assert!(moved.roll_over(day(2026, 10, 31)));
        assert_eq!(moved.start, day(2026, 11, 12));
    }
}
