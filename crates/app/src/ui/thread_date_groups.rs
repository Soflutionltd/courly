//! Outlook-style date headers for the thread list (Today, Yesterday, ...).

use jiff::ToSpan as _;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use rust_i18n::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateGroup {
    Pinned,
    Today,
    Yesterday,
    ThisWeek,
    LastWeek,
    EarlierThisMonth,
    LastMonth,
    Older,
}

impl DateGroup {
    pub fn label(self) -> String {
        match self {
            Self::Pinned => t!("main.thread_list.group.pinned"),
            Self::Today => t!("date.today"),
            Self::Yesterday => t!("date.yesterday"),
            Self::ThisWeek => t!("main.thread_list.group.this_week"),
            Self::LastWeek => t!("main.thread_list.group.last_week"),
            Self::EarlierThisMonth => t!("main.thread_list.group.earlier_this_month"),
            Self::LastMonth => t!("main.thread_list.group.last_month"),
            Self::Older => t!("main.thread_list.group.older"),
        }
        .into_owned()
    }
}

/// One thread as the grouping sees it: pinned flag and last message time in
/// Unix milliseconds.
#[derive(Debug, Clone, Copy)]
pub struct GroupKey {
    pub pinned: bool,
    pub last_message_at: Option<i64>,
}

fn local_date(millis: i64, tz: &TimeZone) -> Option<Date> {
    jiff::Timestamp::from_millisecond(millis)
        .ok()
        .map(|ts| ts.to_zoned(tz.clone()).date())
}

/// The group of one non-pinned thread. Weeks start on Monday; dates after
/// today (clock skew, scheduled sends) count as today.
fn group_for(date: Option<Date>, today: Date) -> DateGroup {
    let Some(date) = date else {
        return DateGroup::Older;
    };
    let days_before = |n: i64| today.checked_sub(n.days()).unwrap_or(Date::MIN);
    let week_start = days_before(i64::from(today.weekday().to_monday_zero_offset()));
    let last_week_start = week_start.checked_sub(7.days()).unwrap_or(Date::MIN);
    let month_start = today.first_of_month();
    let last_month_start = month_start
        .checked_sub(1.day())
        .map_or(Date::MIN, |d| d.first_of_month());

    if date >= today {
        DateGroup::Today
    } else if date >= days_before(1) {
        DateGroup::Yesterday
    } else if date >= week_start {
        DateGroup::ThisWeek
    } else if date >= last_week_start {
        DateGroup::LastWeek
    } else if date >= month_start {
        DateGroup::EarlierThisMonth
    } else if date >= last_month_start {
        DateGroup::LastMonth
    } else {
        DateGroup::Older
    }
}

/// The header to show above each row: `Some` where a new group starts.
/// Lists not sorted by date (relevance-ranked search) get no headers, since
/// the groups would repeat.
pub fn headers(rows: &[GroupKey], today: Date, tz: &TimeZone) -> Vec<Option<DateGroup>> {
    let unpinned: Vec<i64> = rows
        .iter()
        .filter(|row| !row.pinned)
        .map(|row| row.last_message_at.unwrap_or(0))
        .collect();
    let pinned_first = rows
        .iter()
        .skip_while(|row| row.pinned)
        .all(|row| !row.pinned);
    let date_ordered = unpinned.windows(2).all(|pair| pair[0] >= pair[1]);
    if !pinned_first || !date_ordered {
        return vec![None; rows.len()];
    }

    let mut previous = None;
    rows.iter()
        .map(|row| {
            let group = if row.pinned {
                DateGroup::Pinned
            } else {
                group_for(row.last_message_at.and_then(|ms| local_date(ms, tz)), today)
            };
            let header = (previous != Some(group)).then_some(group);
            previous = Some(group);
            header
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn paris() -> TimeZone {
        TimeZone::get("Europe/Paris").expect("known zone")
    }

    fn at(day: Date, hour: i8, minute: i8) -> i64 {
        day.at(hour, minute, 0, 0)
            .to_zoned(paris())
            .expect("valid local time")
            .timestamp()
            .as_millisecond()
    }

    fn key(millis: i64) -> GroupKey {
        GroupKey {
            pinned: false,
            last_message_at: Some(millis),
        }
    }

    // Friday 30 October 2026.
    const TODAY: Date = date(2026, 10, 30);

    #[test]
    fn groups_follow_outlook_buckets() {
        let rows = [
            key(at(TODAY, 9, 0)),
            key(at(TODAY, 0, 5)),
            key(at(date(2026, 10, 29), 23, 55)),
            key(at(date(2026, 10, 26), 8, 0)),
            key(at(date(2026, 10, 25), 22, 0)),
            key(at(date(2026, 10, 19), 8, 0)),
            key(at(date(2026, 10, 3), 8, 0)),
            key(at(date(2026, 9, 30), 8, 0)),
            key(at(date(2026, 8, 31), 8, 0)),
        ];
        let groups: Vec<_> = headers(&rows, TODAY, &paris());
        assert_eq!(
            groups,
            vec![
                Some(DateGroup::Today),
                None,
                Some(DateGroup::Yesterday),
                Some(DateGroup::ThisWeek),
                Some(DateGroup::LastWeek),
                None,
                Some(DateGroup::EarlierThisMonth),
                Some(DateGroup::LastMonth),
                Some(DateGroup::Older),
            ]
        );
    }

    #[test]
    fn local_midnight_decides_the_day_not_utc() {
        // 00:30 in Paris is still the previous day in UTC.
        let rows = [key(at(TODAY, 0, 30))];
        assert_eq!(
            headers(&rows, TODAY, &paris()),
            vec![Some(DateGroup::Today)]
        );
    }

    #[test]
    fn pinned_threads_get_their_own_header() {
        let rows = [
            GroupKey {
                pinned: true,
                last_message_at: Some(at(date(2026, 1, 1), 8, 0)),
            },
            key(at(TODAY, 9, 0)),
        ];
        assert_eq!(
            headers(&rows, TODAY, &paris()),
            vec![Some(DateGroup::Pinned), Some(DateGroup::Today)]
        );
    }

    #[test]
    fn relevance_ordered_lists_get_no_headers() {
        let rows = [key(at(date(2026, 9, 1), 8, 0)), key(at(TODAY, 9, 0))];
        assert_eq!(headers(&rows, TODAY, &paris()), vec![None, None]);
    }

    #[test]
    fn missing_dates_are_older() {
        let rows = [
            key(at(TODAY, 9, 0)),
            GroupKey {
                pinned: false,
                last_message_at: None,
            },
        ];
        assert_eq!(
            headers(&rows, TODAY, &paris()),
            vec![Some(DateGroup::Today), Some(DateGroup::Older)]
        );
    }
}
