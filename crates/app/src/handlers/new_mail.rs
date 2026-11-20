use iced::Task;

use crate::app::ReadyApp;
use crate::message::Message;
use crate::ui::sidebar::SidebarMessage;
use rtsk::db::queries_extra::new_mail::{NewMailRow, unread_inbox_messages_since_sync};

impl ReadyApp {
    /// Read the unread inbox mail that could need a desktop notification,
    /// for `account_id` or, until the starting state is recorded, for every
    /// account.
    pub(crate) fn look_for_new_mail(&self, account_id: Option<&str>) -> Task<Message> {
        let account_id = if self.new_mail.needs_priming() {
            None
        } else {
            account_id.map(str::to_owned)
        };
        let since_ms = self.new_mail.since_ms();
        let db = self.db.read_db_state();
        Task::perform(
            async move {
                db.with_read(move |conn| {
                    unread_inbox_messages_since_sync(
                        conn,
                        account_id.as_deref(),
                        since_ms,
                        crate::new_mail::LOOK_LIMIT,
                    )
                })
                .await
            },
            Message::NewMailLoaded,
        )
    }

    pub(crate) fn handle_new_mail_loaded(&mut self, result: Result<Vec<NewMailRow>, String>) {
        let rows = match result {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("new mail check failed: {e}");
                return;
            }
        };
        let first_look = self.new_mail.needs_priming();
        let prefs = &self.settings.committed_preferences;
        if first_look && prefs.notifications_enabled {
            crate::desktop_notifications::request_authorization();
        }
        let wanted: Vec<NewMailRow> = self
            .new_mail
            .take_new(rows)
            .into_iter()
            .filter(|row| crate::new_mail::should_notify(prefs, row))
            .collect();
        crate::desktop_notifications::post(&crate::new_mail::banners(&wanted));
    }

    /// Show the thread a clicked notification points at: select it when the
    /// list already holds it, otherwise open that account's inbox first.
    pub(crate) fn open_notified_thread(&mut self, identifier: &str) -> Task<Message> {
        let Some((account_id, thread_id)) = crate::new_mail::thread_of(identifier) else {
            return Task::none();
        };
        if self.show_settings {
            self.close_settings();
        }
        if let Some(index) = self.notified_thread_index(&account_id, &thread_id) {
            self.thread_list.clear_multi_select();
            return self.handle_select_thread(index);
        }
        let Some(account_index) = self
            .sidebar
            .accounts
            .iter()
            .position(|account| account.id == account_id)
        else {
            return Task::none();
        };
        self.pending_notified_thread = Some((account_id, thread_id));
        self.handle_sidebar(SidebarMessage::SelectAccount(account_index))
    }

    /// Select the thread a notification click asked for once the list that
    /// holds it has loaded.
    pub(crate) fn select_pending_notified_thread(&mut self) -> Task<Message> {
        let Some((account_id, thread_id)) = self.pending_notified_thread.take() else {
            return Task::none();
        };
        match self.notified_thread_index(&account_id, &thread_id) {
            Some(index) => self.handle_select_thread(index),
            None => Task::none(),
        }
    }

    fn notified_thread_index(&self, account_id: &str, thread_id: &str) -> Option<usize> {
        self.thread_list
            .threads
            .iter()
            .position(|thread| thread.account_id == account_id && thread.id == thread_id)
    }
}
