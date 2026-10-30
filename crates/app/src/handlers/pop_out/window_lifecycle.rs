use iced::{Size, Task};

use crate::pop_out::PopOutWindow;
use crate::pop_out::compose::{ComposeMode, ComposeState, ReplySource};
use crate::pop_out::message_view::{MessageViewMessage, MessageViewState};
use crate::ui::layout::{
    COMPOSE_DEFAULT_HEIGHT, COMPOSE_DEFAULT_WIDTH, COMPOSE_MIN_HEIGHT, COMPOSE_MIN_WIDTH,
    MESSAGE_VIEW_DEFAULT_HEIGHT, MESSAGE_VIEW_DEFAULT_WIDTH, MESSAGE_VIEW_MIN_HEIGHT,
    MESSAGE_VIEW_MIN_WIDTH,
};
use crate::{Message, ReadyApp};

impl ReadyApp {
    /// Open a message view pop-out for the message at `message_index` in the
    /// reading pane's thread messages list.
    pub(crate) fn open_message_view_window(&mut self, message_index: usize) -> Task<Message> {
        let Some(msg) = self
            .reading_pane
            .thread_messages
            .get(message_index)
            .cloned()
        else {
            return Task::none();
        };

        let generation = self.next_pop_out_generation();
        let source_selection = Some(self.sidebar.selection.clone());
        let state = MessageViewState::from_thread_message(
            &msg,
            generation,
            source_selection,
            self.settings.default_rendering_mode,
        );
        let account_id = state.account_id.clone();
        let message_id = state.message_id.clone();

        let settings = iced::window::Settings {
            size: Size::new(MESSAGE_VIEW_DEFAULT_WIDTH, MESSAGE_VIEW_DEFAULT_HEIGHT),
            min_size: Some(Size::new(MESSAGE_VIEW_MIN_WIDTH, MESSAGE_VIEW_MIN_HEIGHT)),
            exit_on_close_request: false,
            ..Default::default()
        };

        let (window_id, open_task) = iced::window::open(settings);
        self.pop_out_windows
            .insert(window_id, PopOutWindow::MessageView(Box::new(state)));

        self.dispatch_message_view_loads(window_id, generation, account_id, message_id, open_task)
    }

    /// Open a message-view pop-out for a chat-timeline message.
    ///
    /// This is the "View as email" path from chat view: the user wants to
    /// see one of the bubbles rendered in classic email format (full
    /// headers, signatures, quoted history) without leaving the chat
    /// timeline. Works the same as `open_message_view_window` from the
    /// thread reading pane, but pulls header data off `ChatMessage`
    /// rather than `reading_pane.thread_messages`.
    pub(crate) fn open_chat_message_view_window(
        &mut self,
        msg: &rtsk::chat::ChatMessage,
    ) -> Task<Message> {
        let generation = self.next_pop_out_generation();
        // Chat view has no sidebar selection of its own (active_chat sits
        // outside ViewScope), so we pass through whatever was selected
        // before the user entered chat. That keeps any subsequent
        // pop-out-initiated action (archive, etc.) resolving against the
        // user's last folder context, not nothing.
        let source_selection = Some(self.sidebar.selection.clone());
        let state = MessageViewState::from_chat_message(
            msg,
            generation,
            source_selection,
            self.settings.default_rendering_mode,
        );
        let account_id = state.account_id.clone();
        let message_id = state.message_id.clone();

        let settings = iced::window::Settings {
            size: Size::new(MESSAGE_VIEW_DEFAULT_WIDTH, MESSAGE_VIEW_DEFAULT_HEIGHT),
            min_size: Some(Size::new(MESSAGE_VIEW_MIN_WIDTH, MESSAGE_VIEW_MIN_HEIGHT)),
            exit_on_close_request: false,
            ..Default::default()
        };

        let (window_id, open_task) = iced::window::open(settings);
        self.pop_out_windows
            .insert(window_id, PopOutWindow::MessageView(Box::new(state)));

        self.dispatch_message_view_loads(window_id, generation, account_id, message_id, open_task)
    }

    /// Open a compose window with the given mode.
    pub(crate) fn open_compose_window(&mut self, mode: ComposeMode) -> Task<Message> {
        let state = ComposeState::new(&self.sidebar.accounts);
        self.open_compose_window_with_state(state, mode)
    }

    /// Open a compose window with pre-built state and mode.
    /// If a compose window already exists for the same reply thread,
    /// focuses it instead of opening a duplicate.
    pub(crate) fn open_compose_window_with_state(
        &mut self,
        mut state: ComposeState,
        mode: ComposeMode,
    ) -> Task<Message> {
        if let Some(existing) = self.existing_compose_for_thread(&state) {
            return existing;
        }
        self.prepare_compose_state(&mut state, mode);
        let (window_id, open_task) = self.open_compose_window_for(state);

        // Dispatch initial signature resolution for the default From account.
        let sig_task = self.resolve_compose_signature(window_id);

        Task::batch([open_task, sig_task])
    }

    /// Reply or forward in the main window, in place of the reading pane,
    /// the way Outlook does. A reply already open for another conversation
    /// is kept as a draft.
    pub(crate) fn open_inline_compose(
        &mut self,
        mut state: ComposeState,
        mode: ComposeMode,
    ) -> Task<Message> {
        if let Some(existing) = self.existing_compose_for_thread(&state) {
            return existing;
        }
        if let Some(previous) = self.inline_compose {
            let has_content = matches!(
                self.pop_out_windows.get(&previous),
                Some(PopOutWindow::Compose(s)) if s.has_user_content()
            );
            if has_content && !self.save_compose_draft_sync(previous) {
                log::warn!("Inline compose draft save failed, keeping it open");
                return Task::none();
            }
            self.pop_out_windows.remove(&previous);
        }
        self.prepare_compose_state(&mut state, mode);
        state.inline = true;
        let id = iced::window::Id::unique();
        self.pop_out_windows
            .insert(id, PopOutWindow::Compose(Box::new(state)));
        self.inline_compose = Some(id);
        self.resolve_compose_signature(id)
    }

    /// Move the inline compose into its own window.
    pub(crate) fn pop_out_inline_compose(&mut self, id: iced::window::Id) -> Task<Message> {
        if self.inline_compose != Some(id) {
            return Task::none();
        }
        self.inline_compose = None;
        let Some(PopOutWindow::Compose(mut state)) = self.pop_out_windows.remove(&id) else {
            return Task::none();
        };
        state.inline = false;
        self.open_compose_window_for(*state).1
    }

    /// Remove a compose and close its window; an inline compose just leaves
    /// the reading pane.
    pub(crate) fn close_compose(&mut self, id: iced::window::Id) -> Task<Message> {
        self.pop_out_windows.remove(&id);
        if self.inline_compose == Some(id) {
            self.inline_compose = None;
            return Task::none();
        }
        iced::window::close(id)
    }

    /// Focus the compose already open for the same conversation, if any.
    fn existing_compose_for_thread(&self, state: &ComposeState) -> Option<Task<Message>> {
        let tid = state.reply_thread_id.as_deref()?;
        let (&existing_id, _) = self.pop_out_windows.iter().find(|(_, w)| {
            matches!(w, PopOutWindow::Compose(s) if s.reply_thread_id.as_deref() == Some(tid))
        })?;
        if self.inline_compose == Some(existing_id) {
            return Some(Task::none());
        }
        Some(iced::window::gain_focus(existing_id))
    }

    fn prepare_compose_state(&self, state: &mut ComposeState, mode: ComposeMode) {
        // Auto-select shared mailbox identity only for reply/forward flows
        // originating from shared-mailbox scope, and only when mailbox rights
        // do not explicitly deny submit.
        if state.reply_thread_id.is_some()
            && self.current_mailbox_may_submit().unwrap_or(true)
            && let rtsk::scope::ViewScope::SharedMailbox {
                ref account_id,
                ref mailbox_id,
            } = self.sidebar.selected_scope
            && let Ok(Some(shared_email)) = self.db.get_shared_mailbox_email(account_id, mailbox_id)
        {
            state.set_shared_mailbox_from(account_id, &shared_email);
        }

        state.mode = mode;
        state.subject = state.mode.prefixed_subject();
    }

    /// Open a compose window holding `state`.
    fn open_compose_window_for(
        &mut self,
        state: ComposeState,
    ) -> (iced::window::Id, Task<Message>) {
        let settings = iced::window::Settings {
            size: Size::new(COMPOSE_DEFAULT_WIDTH, COMPOSE_DEFAULT_HEIGHT),
            min_size: Some(Size::new(COMPOSE_MIN_WIDTH, COMPOSE_MIN_HEIGHT)),
            exit_on_close_request: false,
            ..Default::default()
        };
        let (window_id, open_task) = iced::window::open(settings);
        self.pop_out_windows
            .insert(window_id, PopOutWindow::Compose(Box::new(state)));
        (window_id, open_task.discard())
    }

    fn current_mailbox_may_submit(&self) -> Option<bool> {
        self.sidebar
            .selection
            .navigation_folder_id()
            .and_then(|nav_id| {
                self.sidebar
                    .nav_state
                    .as_ref()?
                    .folders
                    .iter()
                    .find(|folder| folder.id == nav_id)
                    .and_then(|folder| folder.rights.as_ref())
                    .and_then(|rights| rights.may_submit)
            })
    }

    /// Open a compose window from a message view's Reply/ReplyAll/Forward.
    pub(crate) fn open_compose_from_message_view(
        &mut self,
        window_id: iced::window::Id,
        action: &MessageViewMessage,
    ) -> Task<Message> {
        let Some(PopOutWindow::MessageView(mv)) = self.pop_out_windows.get(&window_id) else {
            return Task::none();
        };

        let subject = mv.subject.clone().unwrap_or_default();
        let mode = match *action {
            MessageViewMessage::Reply => ComposeMode::Reply {
                original_subject: subject,
            },
            MessageViewMessage::ReplyAll => ComposeMode::ReplyAll {
                original_subject: subject,
            },
            MessageViewMessage::Forward => ComposeMode::Forward {
                original_subject: subject,
            },
            _ => return Task::none(),
        };

        let source = ReplySource {
            from_name: mv.from_name.as_deref(),
            from_address: mv.from_address.as_deref(),
            to_addresses: mv.to_addresses.as_deref(),
            cc_addresses: mv.cc_addresses.as_deref(),
            subject: mv.subject.as_deref(),
            date: mv.date,
            body_html: mv.body_html.as_deref(),
            body_text: mv.body_text.as_deref(),
            snippet: mv.snippet.as_deref(),
            thread_id: Some(&mv.thread_id),
            message_id: Some(&mv.message_id),
            message_id_header: mv.message_id_header.as_deref(),
        };
        let state = ComposeState::new_reply(&self.sidebar.accounts, &mode, &source);

        self.open_compose_window_with_state(state, mode)
    }

    /// Handle compose actions from command dispatch (Reply/ReplyAll/Forward
    /// from the main window's reading pane context).
    pub(crate) fn handle_compose_action(
        &mut self,
        action: &crate::command_dispatch::ComposeAction,
    ) -> Task<Message> {
        let selected_thread = self
            .thread_list
            .selected_thread
            .and_then(|idx| self.thread_list.threads.get(idx));
        let last_message = self.reading_pane.thread_messages.first();

        let subject = selected_thread
            .and_then(|t| t.subject.clone())
            .unwrap_or_default();

        let mode = match *action {
            crate::command_dispatch::ComposeAction::Reply => ComposeMode::Reply {
                original_subject: subject,
            },
            crate::command_dispatch::ComposeAction::ReplyAll => ComposeMode::ReplyAll {
                original_subject: subject,
            },
            crate::command_dispatch::ComposeAction::Forward => ComposeMode::Forward {
                original_subject: subject,
            },
        };

        let thread_id = selected_thread.map(|t| t.id.as_str());
        let source = last_message
            .map(|m| ReplySource::from_thread_message(m, thread_id))
            .unwrap_or_default();
        let state = ComposeState::new_reply(&self.sidebar.accounts, &mode, &source);

        self.open_inline_compose(state, mode)
    }

    /// Increment and return the next pop-out generation token.
    pub(crate) fn next_pop_out_generation(
        &mut self,
    ) -> rtsk::generation::GenerationToken<rtsk::generation::PopOut> {
        self.pop_out_generation.next()
    }
}
