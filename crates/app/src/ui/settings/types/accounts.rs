use crate::ui::undoable::UndoableText;

/// An account card in the settings list.
#[derive(Debug, Clone)]
pub struct ManagedAccount {
    pub id: String,
    pub email: String,
    pub provider: String,
    pub account_name: Option<String>,
    pub account_color: Option<String>,
    pub display_name: Option<String>,
    pub last_sync_at: Option<i64>,
    pub health: AccountHealth,
}

/// Account health status for the settings card indicator.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AccountHealth {
    #[default]
    Healthy,
    Warning,
    Error,
    Disabled,
    /// We don't have enough signal to classify the account: it has never
    /// synced and there's no token expiry info to lean on. Common for
    /// freshly added accounts before the first sync runs and for synthetic
    /// dev-seed accounts that never go through the real sync path.
    Unknown,
}

/// Compute account health from token expiry and sync state.
pub fn compute_health(
    last_sync_at: Option<i64>,
    token_expires_at: Option<i64>,
    is_active: bool,
) -> AccountHealth {
    if !is_active {
        return AccountHealth::Disabled;
    }
    if last_sync_at.is_none() && token_expires_at.is_none() {
        return AccountHealth::Unknown;
    }
    let now = jiff::Timestamp::now().as_second();
    if let Some(expires) = token_expires_at {
        let no_recent_sync = last_sync_at.is_none_or(|ls| now - ls > 3600);
        if expires < now && no_recent_sync {
            return AccountHealth::Error;
        }
        // Access tokens last an hour and are refreshed on their own, so only
        // one left expired means the refresh is failing.
        if expires < now {
            return AccountHealth::Warning;
        }
    }
    AccountHealth::Healthy
}

/// The slide-in editor state for a single account.
#[derive(Debug, Clone)]
pub struct AccountEditor {
    pub account_id: String,
    pub account_email: String,
    pub account_name: UndoableText,
    pub display_name: UndoableText,
    pub account_color_index: Option<usize>,
    pub caldav_url: UndoableText,
    pub caldav_username: UndoableText,
    pub caldav_password: UndoableText,
    pub show_delete_confirmation: bool,
    pub dirty: bool,
}

/// State for an active account card drag operation.
#[derive(Debug, Clone)]
pub struct AccountDragState {
    pub dragging_index: usize,
    /// Y coordinate when the grip was pressed (list-relative, set on first move).
    pub start_y: f32,
    /// Whether the mouse has moved far enough to count as a real drag.
    pub is_dragging: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_refreshed_on_schedule_reads_healthy() {
        let now = jiff::Timestamp::now().as_second();
        assert_eq!(
            compute_health(Some(now - 60), Some(now + 3000), true),
            AccountHealth::Healthy
        );
        assert_eq!(
            compute_health(None, Some(now + 3000), true),
            AccountHealth::Healthy
        );
    }

    #[test]
    fn an_expired_token_warns_then_errors_once_sync_stalls() {
        let now = jiff::Timestamp::now().as_second();
        assert_eq!(
            compute_health(Some(now - 60), Some(now - 10), true),
            AccountHealth::Warning
        );
        assert_eq!(
            compute_health(Some(now - 7200), Some(now - 10), true),
            AccountHealth::Error
        );
    }
}
