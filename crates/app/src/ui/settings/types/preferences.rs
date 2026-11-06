use crate::db::DateDisplay;
use crate::pop_out::RenderingMode;

/// Controls the background color of email body containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmailBodyBackground {
    /// Always use a white background (best for email rendering fidelity).
    AlwaysWhite,
    /// Use the current theme's background color.
    MatchTheme,
    /// White in light themes; in dark themes the email is recoloured
    /// dark, like Outlook.
    #[default]
    Auto,
}

impl EmailBodyBackground {
    pub fn label(self) -> &'static str {
        match self {
            Self::AlwaysWhite => "Always White",
            Self::MatchTheme => "Match Theme",
            Self::Auto => "Auto",
        }
    }

    pub fn from_label(s: &str) -> Self {
        match s {
            "Always White" => Self::AlwaysWhite,
            "Match Theme" => Self::MatchTheme,
            _ => Self::Auto,
        }
    }
}

/// Reading zoom choices for HTML email bodies.
pub const EMAIL_ZOOM_LEVELS: &[&str] = &[
    "50%", "67%", "75%", "80%", "90%", "100%", "110%", "125%", "150%", "175%", "200%",
];
pub const DEFAULT_EMAIL_ZOOM: &str = "90%";
/// Zoom restored by "Actual Size" (Cmd+0).
pub const ACTUAL_SIZE_EMAIL_ZOOM: &str = "100%";

/// New-mail check choices, in seconds, as stored under `mail_check_interval`
/// and parsed by the Service's hot poll loop.
pub const MAIL_CHECK_INTERVALS: &[&str] = &["2", "15", "60"];
pub const DEFAULT_MAIL_CHECK_INTERVAL: &str = "2";

/// A keyboard zoom request (Cmd +, Cmd -, Cmd 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailZoomStep {
    In,
    Out,
    ActualSize,
}

/// The zoom factor for a zoom label, `None` for anything not in
/// [`EMAIL_ZOOM_LEVELS`].
pub fn email_zoom_factor(label: &str) -> Option<f32> {
    EMAIL_ZOOM_LEVELS
        .contains(&label)
        .then(|| label.trim_end_matches('%').parse::<f32>().ok())
        .flatten()
        .map(|percent| percent / 100.0)
}

/// The zoom label `step` levels away from `label`, clamped to the list.
pub fn step_email_zoom(label: &str, step: i32) -> &'static str {
    let current = EMAIL_ZOOM_LEVELS
        .iter()
        .position(|level| *level == label)
        .or_else(|| {
            EMAIL_ZOOM_LEVELS
                .iter()
                .position(|level| *level == DEFAULT_EMAIL_ZOOM)
        })
        .unwrap_or(0);
    let last = EMAIL_ZOOM_LEVELS.len() - 1;
    let target = current.saturating_add_signed(step as isize).min(last);
    EMAIL_ZOOM_LEVELS[target]
}

/// Undo-send delays in seconds; "0" sends at once.
pub const UNDO_SEND_DELAYS: &[&str] = &["0", "5", "10", "30"];

/// How long a sent message waits so it can still be cancelled.
pub fn undo_send_delay(seconds: &str) -> std::time::Duration {
    std::time::Duration::from_secs(seconds.parse().unwrap_or(0))
}

/// Inbox categories a notification can be limited to, in display order.
pub const NOTIFY_CATEGORIES: &[&str] = &["Primary", "Updates", "Promotions", "Social", "Newsletters"];

/// Categories stored as "Primary,Updates", keeping only known ones.
pub fn notify_categories_from_setting(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|category| NOTIFY_CATEGORIES.contains(category))
        .map(str::to_owned)
        .collect()
}

/// VIP senders stored one address per line, lowercased.
pub fn vip_senders_from_setting(value: &str) -> Vec<String> {
    value
        .lines()
        .map(|line| line.trim().to_lowercase())
        .filter(|email| !email.is_empty())
        .collect()
}

/// Settings values for when an opened message is marked as read.
pub const MARK_AS_READ_OPTIONS: &[&str] = &["Instantly", "After 2 Seconds", "Manually"];

/// When an opened message is marked as read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkAsRead {
    Instantly,
    AfterDelay,
    Manually,
}

impl MarkAsRead {
    /// How long a message stays open before `AfterDelay` marks it read.
    pub const DELAY: std::time::Duration = std::time::Duration::from_secs(2);

    pub fn from_setting(value: &str) -> Self {
        match value {
            "Instantly" => Self::Instantly,
            "Manually" => Self::Manually,
            _ => Self::AfterDelay,
        }
    }
}

/// Snapshot of user-facing preferences that support live preview.
/// Compared with `PartialEq` for change detection.
#[derive(Debug, Clone, PartialEq)]
pub struct PreferencesState {
    pub theme: String,
    pub scale: f32,
    pub density: String,
    pub font_size: String,
    pub date_display: DateDisplay,
    pub reading_pane_position: String,
    pub sync_status_bar: bool,
    pub auto_update: bool,
    pub block_remote_images: bool,
    pub phishing_detection: bool,
    pub phishing_sensitivity: String,
    pub default_rendering_mode: RenderingMode,
    pub email_body_background: EmailBodyBackground,
    pub email_zoom: String,
    pub thread_date_groups: bool,
    pub compose_always_show_cc: bool,
    /// One of `MARK_AS_READ_OPTIONS`.
    pub mark_as_read: String,
    /// One of `UNDO_SEND_DELAYS`.
    pub undo_delay: String,
    /// Seconds between new-mail checks, one of `MAIL_CHECK_INTERVALS`.
    pub mail_check_interval: String,
    pub notifications_enabled: bool,
    pub smart_notifications: bool,
    /// Subset of `NOTIFY_CATEGORIES`.
    pub notify_categories: Vec<String>,
    /// Lowercased addresses that always notify.
    pub vip_senders: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_labels_map_to_factors() {
        assert_eq!(email_zoom_factor("90%"), Some(0.9));
        assert_eq!(email_zoom_factor("100%"), Some(1.0));
        assert_eq!(email_zoom_factor("93%"), None);
        assert_eq!(email_zoom_factor(DEFAULT_EMAIL_ZOOM), Some(0.9));
    }

    #[test]
    fn notification_lists_parse_their_stored_form() {
        assert_eq!(
            notify_categories_from_setting("Primary, Social,Unknown,"),
            vec!["Primary".to_string(), "Social".to_string()]
        );
        assert_eq!(
            vip_senders_from_setting(" Boss@Example.com \n\nceo@example.com"),
            vec!["boss@example.com".to_string(), "ceo@example.com".to_string()]
        );
    }

    #[test]
    fn zoom_steps_clamp_to_the_list() {
        assert_eq!(step_email_zoom("90%", 1), "100%");
        assert_eq!(step_email_zoom("90%", -1), "80%");
        assert_eq!(step_email_zoom("200%", 1), "200%");
        assert_eq!(step_email_zoom("50%", -1), "50%");
        assert_eq!(step_email_zoom("junk", 1), "100%");
    }
}
