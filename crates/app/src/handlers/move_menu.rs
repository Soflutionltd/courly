//! The ribbon's Move menu.

use std::collections::BTreeSet;

use iced::Task;
use rtsk::db::queries_extra::navigation::get_folder_navigation;
use rtsk::db::types::AccountScope;
use rust_i18n::t;
use service_api::actions::FolderId;

use crate::action_resolve::MailActionIntent;
use crate::app::ReadyApp;
use crate::message::Message;
use crate::ui::move_menu::{MoveMenu, MoveMenuMessage, SEARCH_INPUT_ID};

impl ReadyApp {
    /// Open the Move menu over the selected messages. Messages of one
    /// account can go to its own folders too; a selection spanning
    /// accounts gets the folders they all have.
    pub(crate) fn open_move_menu(&mut self) -> Task<Message> {
        let accounts: BTreeSet<String> = self
            .thread_list
            .selected_indices()
            .into_iter()
            .filter_map(|index| self.thread_list.threads.get(index))
            .map(|thread| thread.account_id.clone())
            .collect();
        let mut accounts = accounts.into_iter();
        let scope = match (accounts.next(), accounts.next()) {
            (None, _) => return Task::none(),
            (Some(account_id), None) => AccountScope::Single(account_id),
            _ => AccountScope::All,
        };
        let current = self
            .sidebar
            .selected_folder()
            .map(|folder| folder.folder_id);
        match self
            .db
            .with_read_sync(|conn| get_folder_navigation(conn, &scope))
        {
            Ok(nav) => {
                let destinations =
                    crate::ui::sidebar::message_destinations(&nav, current.as_deref());
                self.toolbar.overflow_open = false;
                self.move_menu = Some(MoveMenu::new(destinations));
                iced::widget::operation::focus(SEARCH_INPUT_ID)
            }
            Err(error) => {
                log::error!("Move menu: loading folders failed: {error}");
                self.status_bar.show_confirmation(
                    t!("main.move_menu.load_failed", error = error).into_owned(),
                );
                Task::none()
            }
        }
    }

    pub(crate) fn handle_move_menu(&mut self, message: MoveMenuMessage) -> Task<Message> {
        match message {
            MoveMenuMessage::Query(query) => {
                if let Some(menu) = self.move_menu.as_mut() {
                    menu.set_query(query);
                }
                Task::none()
            }
            MoveMenuMessage::Submit => {
                let picked = self
                    .move_menu
                    .as_ref()
                    .and_then(MoveMenu::first_match)
                    .map(|destination| destination.id.clone());
                match picked {
                    Some(folder_id) => self.move_selection_to(folder_id),
                    None => Task::none(),
                }
            }
            MoveMenuMessage::Pick(folder_id) => self.move_selection_to(folder_id),
            MoveMenuMessage::Close => {
                self.move_menu = None;
                Task::none()
            }
        }
    }

    fn move_selection_to(&mut self, folder_id: String) -> Task<Message> {
        self.move_menu = None;
        self.update(Message::EmailAction(MailActionIntent::move_to(
            FolderId::from(folder_id),
        )))
    }
}
