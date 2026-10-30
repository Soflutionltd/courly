use rust_i18n::t;

/// Formats a unix timestamp as a relative time string (e.g. "5 min ago", "2 hours ago").
pub(crate) fn format_relative_time(timestamp: i64) -> String {
    let Ok(dt) = jiff::Timestamp::from_second(timestamp) else {
        return t!("main.relative_time.unknown").into_owned();
    };
    let delta = jiff::Timestamp::now().duration_since(dt);

    if delta.as_secs() < 60 {
        t!("main.relative_time.just_now")
    } else if delta.as_mins() < 60 {
        let m = delta.as_mins();
        t!("main.relative_time.minutes_ago", count = m)
    } else if delta.as_hours() < 24 {
        let h = delta.as_hours();
        t!("main.relative_time.hours_ago", count = h)
    } else {
        let d = delta.as_hours() / 24;
        t!("main.relative_time.days_ago", count = d)
    }
    .into_owned()
}
