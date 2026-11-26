use std::borrow::Cow;

use cmdk::CommandId;
use iced::Task;
use rust_i18n::t;

use crate::action_resolve::MailActionIntent;
use crate::command_dispatch::{self, ComposeAction};
use crate::ui::palette::PaletteMessage;
use crate::ui::thread_menu::{RowMenuEntry, RowMenuItem, RowMenuPage, RowMenuPages};
use crate::{Message, ReadyApp};

/// Row menu actions that work from the messages of the conversation,
/// which the reading pane loads after the row is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowMessageAction {
    Open,
    Reply,
    ReplyAll,
    Forward,
    CreateEvent,
    UseAsSignature,
}

impl ReadyApp {
    pub(crate) fn handle_row_menu_opened(
        &mut self,
        index: usize,
        selection_changed: bool,
    ) -> Task<Message> {
        let select = if selection_changed {
            self.handle_select_thread(index)
        } else {
            Task::none()
        };
        let pages = self.row_menu_pages();
        self.thread_list.set_row_menu_pages(pages);
        select
    }

    fn row_menu_pages(&self) -> RowMenuPages {
        let ctx = command_dispatch::build_context(self);
        let thread = self.menu_thread();
        let single = self.thread_list.selection_count() == 1;
        let has_sender = thread
            .and_then(|t| t.from_address.as_deref())
            .is_some_and(|a| !a.is_empty());
        let has_subject = thread
            .and_then(|t| t.subject.as_deref())
            .is_some_and(|s| !s.trim().is_empty());
        let in_trash = ctx.thread_in_trash == Some(true);

        let command = |item: RowMenuItem, label: Cow<'_, str>, id: CommandId| RowMenuEntry::Item {
            item,
            label: label.into_owned(),
            shortcut: self.binding_table.display_binding(id),
            enabled: self.is_command_available(id),
        };
        let single_command = |item: RowMenuItem, label: Cow<'_, str>, id: CommandId| match command(
            item, label, id,
        ) {
            RowMenuEntry::Item {
                item,
                label,
                shortcut,
                enabled,
            } => RowMenuEntry::Item {
                item,
                label,
                shortcut,
                enabled: enabled && single,
            },
            other => other,
        };
        let plain = |item: RowMenuItem, label: Cow<'_, str>, enabled: bool| RowMenuEntry::Item {
            item,
            label: label.into_owned(),
            shortcut: None,
            enabled,
        };
        let page = |page: RowMenuPage, label: Cow<'_, str>, enabled: bool| RowMenuEntry::Page {
            page,
            label: label.into_owned(),
            enabled,
        };
        let toggle = |on: Option<bool>, when_on: Cow<'static, str>, when_off: Cow<'static, str>| {
            if on == Some(true) { when_on } else { when_off }
        };

        let (delete_id, delete_label) = if in_trash {
            (
                CommandId::EmailPermanentDelete,
                t!("main.row_menu.delete_forever"),
            )
        } else {
            (CommandId::EmailTrash, t!("main.row_menu.delete"))
        };
        let delete = RowMenuEntry::Item {
            item: RowMenuItem::Delete,
            label: delete_label.into_owned(),
            shortcut: Some(t!("main.row_menu.delete_key").into_owned()),
            enabled: self.is_command_available(delete_id),
        };

        let main = vec![
            plain(
                RowMenuItem::Open,
                t!("main.row_menu.open"),
                single && thread.is_some(),
            ),
            RowMenuEntry::Divider,
            single_command(
                RowMenuItem::Reply,
                t!("main.row_menu.reply"),
                CommandId::ComposeReply,
            ),
            single_command(
                RowMenuItem::ReplyAll,
                t!("main.row_menu.reply_all"),
                CommandId::ComposeReplyAll,
            ),
            single_command(
                RowMenuItem::Forward,
                t!("main.row_menu.forward"),
                CommandId::ComposeForward,
            ),
            RowMenuEntry::Divider,
            plain(
                RowMenuItem::CreateEvent,
                t!("main.row_menu.create_event"),
                single && thread.is_some(),
            ),
            plain(
                RowMenuItem::UseAsSignature,
                t!("main.row_menu.use_as_signature"),
                single && thread.is_some(),
            ),
            RowMenuEntry::Divider,
            command(
                RowMenuItem::ToggleRead,
                toggle(
                    ctx.thread_is_read,
                    t!("main.row_menu.mark_unread"),
                    t!("main.row_menu.mark_read"),
                ),
                CommandId::EmailMarkRead,
            ),
            page(
                RowMenuPage::Snooze,
                t!("main.row_menu.snooze"),
                self.is_command_available(CommandId::EmailSnooze),
            ),
            RowMenuEntry::Divider,
            page(
                RowMenuPage::Related,
                t!("main.row_menu.related"),
                single && (has_sender || has_subject),
            ),
            page(
                RowMenuPage::Sender,
                t!("main.row_menu.sender"),
                single && has_sender,
            ),
            RowMenuEntry::Divider,
            command(
                RowMenuItem::ToggleFlag,
                toggle(
                    ctx.thread_is_starred,
                    t!("main.row_menu.unflag"),
                    t!("main.row_menu.flag"),
                ),
                CommandId::EmailStar,
            ),
            command(
                RowMenuItem::TogglePin,
                toggle(
                    ctx.thread_is_pinned,
                    t!("main.row_menu.unpin"),
                    t!("main.row_menu.pin"),
                ),
                CommandId::EmailPin,
            ),
            command(
                RowMenuItem::Categorize,
                t!("main.row_menu.categorize"),
                CommandId::EmailAddLabel,
            ),
            command(
                RowMenuItem::Uncategorize,
                t!("main.row_menu.uncategorize"),
                CommandId::EmailRemoveLabel,
            ),
            RowMenuEntry::Divider,
            command(
                RowMenuItem::ToggleSpam,
                toggle(
                    ctx.thread_in_spam,
                    t!("main.row_menu.not_junk"),
                    t!("main.row_menu.junk"),
                ),
                CommandId::EmailSpam,
            ),
            plain(
                RowMenuItem::BlockSender,
                t!("main.row_menu.block_sender"),
                single && has_sender && self.service_client.is_some(),
            ),
            command(
                RowMenuItem::Move,
                t!("main.row_menu.move"),
                CommandId::EmailMoveToFolder,
            ),
            RowMenuEntry::Divider,
            command(
                RowMenuItem::ToggleMute,
                toggle(
                    ctx.thread_is_muted,
                    t!("main.row_menu.unmute"),
                    t!("main.row_menu.mute"),
                ),
                CommandId::EmailMute,
            ),
            delete,
            command(
                RowMenuItem::Archive,
                t!("main.row_menu.archive"),
                CommandId::EmailArchive,
            ),
        ];

        let snooze = crate::ui::palette::snooze_preset_options()
            .into_iter()
            .filter_map(|option| {
                let until = option.id.parse::<i64>().ok()?;
                Some(plain(
                    RowMenuItem::Snooze { until },
                    Cow::Owned(option.label),
                    true,
                ))
            })
            .collect();

        let related = vec![
            plain(
                RowMenuItem::SearchSameSubject,
                t!("main.row_menu.same_subject"),
                has_subject,
            ),
            plain(
                RowMenuItem::SearchFromSender,
                t!("main.row_menu.from_sender"),
                has_sender,
            ),
        ];

        let sender = vec![
            plain(
                RowMenuItem::SearchFromSender,
                t!("main.row_menu.search_sender"),
                has_sender,
            ),
            plain(
                RowMenuItem::AddSenderToContacts,
                t!("main.row_menu.add_to_contacts"),
                has_sender,
            ),
            plain(
                RowMenuItem::CopySenderAddress,
                t!("main.row_menu.copy_address"),
                has_sender,
            ),
        ];

        RowMenuPages {
            main,
            snooze,
            related,
            sender,
        }
    }

    pub(crate) fn handle_row_menu_picked(&mut self, item: RowMenuItem) -> Task<Message> {
        match item {
            RowMenuItem::Open => self.run_or_hold_row_action(RowMessageAction::Open),
            RowMenuItem::Reply => self.run_or_hold_row_action(RowMessageAction::Reply),
            RowMenuItem::ReplyAll => self.run_or_hold_row_action(RowMessageAction::ReplyAll),
            RowMenuItem::Forward => self.run_or_hold_row_action(RowMessageAction::Forward),
            RowMenuItem::CreateEvent => self.run_or_hold_row_action(RowMessageAction::CreateEvent),
            RowMenuItem::UseAsSignature => {
                self.run_or_hold_row_action(RowMessageAction::UseAsSignature)
            }
            RowMenuItem::ToggleRead => self.handle_email_action(MailActionIntent::ToggleRead),
            RowMenuItem::Snooze { until } => {
                self.handle_email_action(MailActionIntent::Snooze { until })
            }
            RowMenuItem::ToggleFlag => self.handle_email_action(MailActionIntent::ToggleStar),
            RowMenuItem::TogglePin => self.handle_email_action(MailActionIntent::TogglePin),
            RowMenuItem::ToggleSpam => self.handle_email_action(MailActionIntent::ToggleSpam),
            RowMenuItem::BlockSender => self.block_menu_sender(),
            RowMenuItem::ToggleMute => self.handle_email_action(MailActionIntent::ToggleMute),
            RowMenuItem::Archive => self.handle_email_action(MailActionIntent::Archive),
            RowMenuItem::Delete => {
                let in_trash = command_dispatch::build_context(self).thread_in_trash == Some(true);
                self.handle_email_action(super::keyboard::delete_key_intent(in_trash))
            }
            RowMenuItem::Categorize => self.open_category_menu(),
            RowMenuItem::Uncategorize => self.open_palette_on(CommandId::EmailRemoveLabel),
            RowMenuItem::Move => self.open_move_menu(),
            RowMenuItem::SearchSameSubject => {
                let subject = self.menu_thread().and_then(|t| t.subject.as_deref());
                let words = rtsk::threading::normalize_subject(subject).replace('"', " ");
                self.search_from_row_menu(words)
            }
            RowMenuItem::SearchFromSender => match self.menu_sender() {
                Some(address) => {
                    let name = self
                        .menu_thread()
                        .and_then(|thread| thread.from_name.clone())
                        .filter(|name| !name.is_empty());
                    if let Some(name) = name {
                        self.search_sender_name = Some((address.clone(), name));
                    }
                    self.show_mail_sent_by(address)
                }
                None => Task::none(),
            },
            RowMenuItem::AddSenderToContacts => match self.menu_sender() {
                Some(address) => self.open_contact_editor_for_email(address),
                None => Task::none(),
            },
            RowMenuItem::CopySenderAddress => match self.menu_sender() {
                Some(address) => {
                    self.status_bar
                        .show_confirmation(t!("main.row_menu.address_copied").into_owned());
                    iced::clipboard::write(address).map(|_| Message::Noop)
                }
                None => Task::none(),
            },
        }
    }

    /// Run the held row-menu action once its thread has loaded.
    pub(crate) fn run_pending_row_action(&mut self) -> Task<Message> {
        let Some((account_id, thread_id, action)) = self.pending_row_action.take() else {
            return Task::none();
        };
        if self.reading_pane.has_loaded_thread(&account_id, &thread_id) {
            return self.run_row_message_action(action);
        }
        self.pending_row_action = Some((account_id, thread_id, action));
        Task::none()
    }

    fn run_or_hold_row_action(&mut self, action: RowMessageAction) -> Task<Message> {
        let Some(thread) = self.menu_thread() else {
            return Task::none();
        };
        let (account_id, thread_id) = (thread.account_id.clone(), thread.id.clone());
        if self.reading_pane.has_loaded_thread(&account_id, &thread_id) {
            return self.run_row_message_action(action);
        }
        self.pending_row_action = Some((account_id, thread_id, action));
        Task::none()
    }

    /// Messages are newest first: index 0 is the latest one.
    fn run_row_message_action(&mut self, action: RowMessageAction) -> Task<Message> {
        match action {
            RowMessageAction::Open => self.open_message_view_window(0),
            RowMessageAction::Reply => self.handle_compose_action(&ComposeAction::Reply),
            RowMessageAction::ReplyAll => self.handle_compose_action(&ComposeAction::ReplyAll),
            RowMessageAction::Forward => self.handle_compose_action(&ComposeAction::Forward),
            RowMessageAction::CreateEvent => self.create_event_from_email(0),
            RowMessageAction::UseAsSignature => self.use_message_as_signature(0),
        }
    }

    fn open_palette_on(&mut self, id: CommandId) -> Task<Message> {
        self.handle_palette(PaletteMessage::OpenCommand(
            id,
            cmdk::CommandContext::default(),
        ))
    }

    fn search_from_row_menu(&mut self, query: String) -> Task<Message> {
        let focus = self.handle_search_here(query);
        Task::batch([focus, self.update(Message::SearchExecute)])
    }

    fn menu_thread(&self) -> Option<&crate::db::Thread> {
        self.thread_list
            .selected_thread
            .and_then(|index| self.thread_list.threads.get(index))
    }

    /// Block the selected thread's sender on its account: the thread goes
    /// to Spam now through the regular spam action, and the service's
    /// blocked sender sweep sends that address's later mail there too.
    /// Reached from the row menu, the toolbar, the menu bar and the palette.
    pub(crate) fn block_menu_sender(&mut self) -> Task<Message> {
        let Some(address) = self.menu_sender() else {
            return Task::none();
        };
        let Some(account_id) = self.menu_thread().map(|thread| thread.account_id.clone()) else {
            return Task::none();
        };
        let Some(client) = self.service_client.clone() else {
            self.status_bar.show_confirmation(
                t!(
                    "main.row_menu.block_failed",
                    error = t!("main.side_panel.templates.service_unavailable")
                )
                .into_owned(),
            );
            return Task::none();
        };
        let to_spam = if command_dispatch::build_context(self).thread_in_spam == Some(true) {
            Task::none()
        } else {
            self.handle_email_action(MailActionIntent::ToggleSpam)
        };
        let block = Task::perform(
            async move {
                client
                    .block_sender(account_id, address.clone())
                    .await
                    .map(|_| address)
                    .map_err(|e| e.to_string())
            },
            Message::SenderBlocked,
        );
        Task::batch([to_spam, block])
    }

    pub(crate) fn handle_sender_blocked(
        &mut self,
        result: Result<String, String>,
    ) -> Task<Message> {
        let (text, reload) = match result {
            Ok(address) => (
                t!("main.row_menu.sender_blocked", address = address),
                self.load_blocked_senders(),
            ),
            Err(error) => (
                t!("main.row_menu.block_failed", error = error),
                Task::none(),
            ),
        };
        self.status_bar.show_confirmation(text.into_owned());
        reload
    }

    /// Remove an address from an account's blocked list from Settings.
    pub(crate) fn unblock_sender(&mut self, account_id: String, address: String) -> Task<Message> {
        let Some(client) = self.service_client.clone() else {
            self.status_bar.show_confirmation(
                t!(
                    "main.row_menu.unblock_failed",
                    error = t!("main.side_panel.templates.service_unavailable")
                )
                .into_owned(),
            );
            return Task::none();
        };
        Task::perform(
            async move {
                client
                    .unblock_sender(account_id, address.clone())
                    .await
                    .map(|_| address)
                    .map_err(|e| e.to_string())
            },
            Message::SenderUnblocked,
        )
    }

    pub(crate) fn handle_sender_unblocked(
        &mut self,
        result: Result<String, String>,
    ) -> Task<Message> {
        let text = match result {
            Ok(address) => t!("main.row_menu.sender_unblocked", address = address),
            Err(error) => t!("main.row_menu.unblock_failed", error = error),
        };
        self.status_bar.show_confirmation(text.into_owned());
        self.load_blocked_senders()
    }

    /// Read every account's blocked senders for the Settings list.
    pub(crate) fn load_blocked_senders(&self) -> Task<Message> {
        let db = std::sync::Arc::clone(&self.db);
        Task::perform(
            async move {
                let rows = rtsk::db::queries_extra::db_get_all_blocked_senders(&db.read_db_state())
                    .await?;
                Ok(rows
                    .into_iter()
                    .map(|row| crate::ui::settings::BlockedSender {
                        account_id: row.account_id,
                        account_email: row.account_email,
                        address: row.sender_address,
                    })
                    .collect())
            },
            Message::BlockedSendersLoaded,
        )
    }

    fn menu_sender(&self) -> Option<String> {
        self.menu_thread()
            .and_then(|t| t.from_address.clone())
            .filter(|address| !address.is_empty())
    }
}
