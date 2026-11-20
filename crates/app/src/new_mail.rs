//! Which new inbox messages become desktop notifications.
//!
//! The first look at the database, right after launch, only records what is
//! already there; every later look notifies what it did not see before.
//! Messages dated before launch never notify, so marking an old message
//! unread stays quiet.

use std::collections::HashSet;

use rtsk::db::queries_extra::new_mail::NewMailRow;
use rust_i18n::t;

use crate::desktop_notifications::DesktopNotification;
use crate::ui::settings::PreferencesState;

/// Mail older than this before launch is never announced.
const LAUNCH_SLACK_MS: i64 = 15 * 60 * 1000;
/// More new messages than this in one look become a single summary banner.
const MAX_SEPARATE_BANNERS: usize = 3;
/// Rows read per look; the newest ones win.
pub(crate) const LOOK_LIMIT: i64 = 200;

pub(crate) struct NewMailWatch {
    since_ms: i64,
    primed: bool,
    seen: HashSet<(String, String)>,
}

impl NewMailWatch {
    pub(crate) fn new(launched_at_ms: i64) -> Self {
        Self {
            since_ms: launched_at_ms - LAUNCH_SLACK_MS,
            primed: false,
            seen: HashSet::new(),
        }
    }

    pub(crate) fn since_ms(&self) -> i64 {
        self.since_ms
    }

    /// Whether the next look must cover every account to record the
    /// starting state.
    pub(crate) fn needs_priming(&self) -> bool {
        !self.primed
    }

    /// Record `rows` and return the ones not seen before. The first call
    /// records only.
    pub(crate) fn take_new(&mut self, rows: Vec<NewMailRow>) -> Vec<NewMailRow> {
        let first_look = !self.primed;
        self.primed = true;
        let fresh: Vec<NewMailRow> = rows
            .into_iter()
            .filter(|row| {
                self.seen
                    .insert((row.account_id.clone(), row.message_id.clone()))
            })
            .collect();
        if first_look { Vec::new() } else { fresh }
    }
}

/// Whether the user's notification settings let `row` through.
pub(crate) fn should_notify(prefs: &PreferencesState, row: &NewMailRow) -> bool {
    if !prefs.notifications_enabled {
        return false;
    }
    if !prefs.smart_notifications {
        return true;
    }
    let from_vip = row.from_address.as_deref().is_some_and(|address| {
        let address = address.trim().to_lowercase();
        prefs.vip_senders.contains(&address)
    });
    from_vip || prefs.notify_categories.contains(&row.bundle)
}

/// Identifier of a banner that opens `thread_id` of `account_id` on click.
fn identifier(account_id: &str, thread_id: &str, message_id: &str) -> String {
    format!("mail\n{account_id}\n{thread_id}\n{message_id}")
}

/// The account and thread a clicked banner points at.
pub(crate) fn thread_of(identifier: &str) -> Option<(String, String)> {
    let mut parts = identifier.split('\n');
    if parts.next()? != "mail" {
        return None;
    }
    let account_id = parts.next()?.to_owned();
    let thread_id = parts.next()?.to_owned();
    Some((account_id, thread_id))
}

/// Banners for `rows`, newest first as the query returns them: one per
/// message, or one summary pointing at the newest when there are many.
pub(crate) fn banners(rows: &[NewMailRow]) -> Vec<DesktopNotification> {
    let Some(newest) = rows.first() else {
        return Vec::new();
    };
    if rows.len() > MAX_SEPARATE_BANNERS {
        return vec![DesktopNotification {
            identifier: identifier(&newest.account_id, &newest.thread_id, &newest.message_id),
            group: "courly-new-mail".into(),
            title: t!("main.notifications.new_messages", count = rows.len()).into_owned(),
            subtitle: String::new(),
            body: t!(
                "main.notifications.latest_from",
                sender = sender_of(newest),
                subject = subject_of(newest)
            )
            .into_owned(),
        }];
    }
    rows.iter()
        .rev()
        .map(|row| DesktopNotification {
            identifier: identifier(&row.account_id, &row.thread_id, &row.message_id),
            group: format!("{}\n{}", row.account_id, row.thread_id),
            title: sender_of(row),
            subtitle: subject_of(row),
            body: row.snippet.clone().unwrap_or_default(),
        })
        .collect()
}

fn sender_of(row: &NewMailRow) -> String {
    row.from_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .or(row.from_address.as_deref())
        .map_or_else(|| t!("main.notifications.unknown_sender").into_owned(), str::to_owned)
}

fn subject_of(row: &NewMailRow) -> String {
    row.subject
        .as_deref()
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
        .map_or_else(|| t!("main.notifications.no_subject").into_owned(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::settings::Settings;

    fn row(message_id: &str, from: &str, bundle: &str) -> NewMailRow {
        NewMailRow {
            account_id: "acc".into(),
            message_id: message_id.into(),
            thread_id: format!("t-{message_id}"),
            from_name: None,
            from_address: Some(from.into()),
            subject: Some("Hello".into()),
            snippet: Some("Body".into()),
            bundle: bundle.into(),
        }
    }

    #[test]
    fn the_first_look_only_records_what_is_there() {
        let mut watch = NewMailWatch::new(1_000_000_000);
        assert!(watch.needs_priming());
        assert!(watch.take_new(vec![row("m1", "a@x.io", "Primary")]).is_empty());
        assert!(!watch.needs_priming());
        let fresh = watch.take_new(vec![
            row("m2", "b@x.io", "Primary"),
            row("m1", "a@x.io", "Primary"),
        ]);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].message_id, "m2");
        assert!(watch.take_new(vec![row("m2", "b@x.io", "Primary")]).is_empty());
    }

    #[test]
    fn smart_notifications_keep_vips_and_chosen_categories() {
        let mut prefs = Settings::default().committed_preferences;
        prefs.vip_senders = vec!["boss@x.io".into()];
        assert!(should_notify(&prefs, &row("1", "someone@x.io", "Primary")));
        assert!(!should_notify(&prefs, &row("2", "shop@x.io", "Promotions")));
        assert!(should_notify(&prefs, &row("3", " Boss@X.io ", "Promotions")));

        prefs.smart_notifications = false;
        assert!(should_notify(&prefs, &row("4", "shop@x.io", "Promotions")));

        prefs.notifications_enabled = false;
        assert!(!should_notify(&prefs, &row("5", "boss@x.io", "Primary")));
    }

    #[test]
    fn many_messages_collapse_into_one_banner_that_opens_the_newest() {
        let rows: Vec<_> = (0..5).map(|i| row(&i.to_string(), "a@x.io", "Primary")).collect();
        let summary = banners(&rows);
        assert_eq!(summary.len(), 1);
        assert_eq!(
            thread_of(&summary[0].identifier),
            Some(("acc".into(), "t-0".into()))
        );

        let few = banners(&rows[..2]);
        assert_eq!(few.len(), 2);
        assert_eq!(few[0].title, "a@x.io");
        assert_eq!(thread_of(&few[0].identifier), Some(("acc".into(), "t-1".into())));
        assert_eq!(thread_of("other"), None);
    }
}
