/// Formats a unix timestamp as a relative time string (e.g. "5 min ago", "2 hours ago").
pub(crate) fn format_relative_time(timestamp: i64) -> String {
    let Ok(dt) = jiff::Timestamp::from_second(timestamp) else {
        return "Unknown".to_string();
    };
    let delta = jiff::Timestamp::now().duration_since(dt);

    if delta.as_secs() < 60 {
        "just now".to_string()
    } else if delta.as_mins() < 60 {
        let m = delta.as_mins();
        format!("{m} min ago")
    } else if delta.as_hours() < 24 {
        let h = delta.as_hours();
        format!("{h} hours ago")
    } else {
        let d = delta.as_hours() / 24;
        format!("{d} days ago")
    }
}
