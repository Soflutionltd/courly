use jiff::civil::Date;
use rust_i18n::t;

use crate::i18n;

use super::types::CalendarEventData;

/// Format time range like "10:00 - 11:30" or "All day".
pub(super) fn format_event_time_range(event: &CalendarEventData) -> String {
    if event.all_day {
        return t!("calendar.all_day").into_owned();
    }
    format!(
        "{} \u{2013} {}",
        clock(
            event.start_date,
            event.start_hour_u32(),
            event.start_minute_u32()
        ),
        clock(
            event.start_date,
            event.end_hour_u32(),
            event.end_minute_u32()
        ),
    )
}

/// Wall-clock time of day in the language's 12 h or 24 h form.
fn clock(date: Date, hour: u32, minute: u32) -> String {
    let (Ok(hour), Ok(minute)) = (i8::try_from(hour), i8::try_from(minute)) else {
        return String::new();
    };
    date.at(hour, minute, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .map(|zoned| i18n::time_of_day(&zoned))
        .unwrap_or_default()
}

/// "Mon, Oct 30, 2026".
pub(super) fn format_event_date(date: Date) -> String {
    t!(
        "calendar.date_full",
        weekday = i18n::weekday_short(date.weekday().to_monday_zero_offset().unsigned_abs()),
        month = i18n::month_short(date.month()),
        day = date.day(),
        year = date.year()
    )
    .into_owned()
}

/// "Mon, Oct 30".
pub(super) fn format_event_day(date: Date) -> String {
    t!(
        "calendar.date_day",
        weekday = i18n::weekday_short(date.weekday().to_monday_zero_offset().unsigned_abs()),
        month = i18n::month_short(date.month()),
        day = date.day()
    )
    .into_owned()
}

/// Format a recurrence rule for display.
pub(super) fn format_recurrence_rule(rrule: &str) -> String {
    let rule = rrule.strip_prefix("RRULE:").unwrap_or(rrule);
    let mut freq = "";
    let mut interval = 1u32;
    for part in rule.split(';') {
        if let Some(val) = part.strip_prefix("FREQ=") {
            freq = val;
        }
        if let Some(val) = part.strip_prefix("INTERVAL=") {
            interval = val.parse().unwrap_or(1);
        }
    }
    let label = match (freq, interval <= 1) {
        ("DAILY", true) => t!("calendar.recurrence.every_day"),
        ("WEEKLY", true) => t!("calendar.recurrence.every_week"),
        ("MONTHLY", true) => t!("calendar.recurrence.every_month"),
        ("YEARLY", true) => t!("calendar.recurrence.every_year"),
        ("DAILY", false) => t!("calendar.recurrence.every_n_days", count = interval),
        ("WEEKLY", false) => t!("calendar.recurrence.every_n_weeks", count = interval),
        ("MONTHLY", false) => t!("calendar.recurrence.every_n_months", count = interval),
        ("YEARLY", false) => t!("calendar.recurrence.every_n_years", count = interval),
        _ => return rule.to_string(),
    };
    label.into_owned()
}

/// Format a reminder as human-readable text.
pub(super) fn format_reminder(minutes_before: i64) -> String {
    if minutes_before <= 0 {
        t!("calendar.reminder.at_event")
    } else if minutes_before < 60 {
        t!("calendar.reminder.minutes", count = minutes_before)
    } else if minutes_before < 1440 {
        let hours = minutes_before / 60;
        if hours == 1 {
            t!("calendar.reminder.hours_one")
        } else {
            t!("calendar.reminder.hours_other", count = hours)
        }
    } else {
        let days = minutes_before / 1440;
        if days == 1 {
            t!("calendar.reminder.days_one")
        } else {
            t!("calendar.reminder.days_other", count = days)
        }
    }
    .into_owned()
}

/// Parse a hex color string (e.g. "#4285f4") to an iced Color.
pub(super) fn parse_hex_color(hex: &str) -> iced::Color {
    let hex = hex.trim_start_matches('#');
    if hex.len() >= 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(100);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(100);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(100);
        iced::Color::from_rgb8(r, g, b)
    } else {
        iced::Color::from_rgb8(100, 100, 200)
    }
}
