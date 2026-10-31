use crate::app::{AppMode, ReadyApp};
use crate::component::Component;
use crate::db;
use crate::handlers;
use crate::message::Message;
use crate::pop_out::{self, PopOutWindow, compose::ComposeMode};
use crate::ui;
use crate::ui::add_account::AddAccountMessage;
use crate::ui::reading_pane::{ReadingPaneEvent, ReadingPaneMessage};
use crate::ui::settings::{SettingsEvent, SettingsMessage};
use crate::ui::sidebar::{SidebarEvent, SidebarMessage};
use crate::ui::status_bar::{
    AccountWarning, StatusBarEvent, StatusBarMessage, SyncEvent, WarningKind,
};
use crate::ui::thread_list::{ThreadListEvent, ThreadListMessage};
use cmdk::CommandId;
use iced::Task;
use rtsk::scope::ViewScope;
use rust_i18n::t;
use service_api::SettingValue;
use std::sync::Arc;

impl ReadyApp {
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn handle_sidebar(&mut self, msg: SidebarMessage) -> Task<Message> {
        let (task, event) = self.sidebar.update(msg);
        let mut tasks = vec![task.map(Message::Sidebar)];
        if let Some(evt) = event {
            tasks.push(self.handle_sidebar_event(evt));
        }
        Task::batch(tasks)
    }

    pub(crate) fn handle_sidebar_event(&mut self, event: SidebarEvent) -> Task<Message> {
        match event {
            SidebarEvent::AccountSelected(_idx) => {
                self.reset_view_state();
                Task::batch([
                    self.load_navigation_and_threads(),
                    self.focus_selected_folder(),
                ])
            }
            SidebarEvent::AllAccountsSelected => {
                self.reset_view_state();
                Task::batch([
                    self.load_navigation_and_threads(),
                    self.focus_selected_folder(),
                ])
            }
            SidebarEvent::SelectionChanged(_sel) => {
                self.reset_view_state();
                let token = self.nav_generation.next();
                Task::batch([
                    self.load_threads_for_current_view(token),
                    self.focus_selected_folder(),
                ])
            }
            SidebarEvent::AccountsReordered(orders) => self.handle_reorder_accounts(orders),
            SidebarEvent::Compose => self.update(Message::Compose),
            SidebarEvent::AddAccount => self.handle_open_add_account_wizard(),
            SidebarEvent::ToggleSettings => self.update(Message::ToggleSettings),
            SidebarEvent::PinnedSearchSelected(id) => self.update(Message::SelectPinnedSearch(id)),
            SidebarEvent::PinnedSearchDismissed(id) => {
                self.update(Message::DismissPinnedSearch(id))
            }
            SidebarEvent::ModeToggled => self.update(Message::ToggleAppMode),
            SidebarEvent::SearchHere { query_prefix } => {
                self.update(Message::SearchHere(query_prefix))
            }
            SidebarEvent::SmartFolderSelected { id, query } => {
                self.handle_smart_folder_selected(id, query)
            }
            SidebarEvent::PinnedSearchRefreshed(id) => {
                self.update(Message::RefreshPinnedSearch(id))
            }
            SidebarEvent::SharedMailboxSelected { .. } => {
                self.reset_view_state();
                Task::batch([
                    self.load_navigation_and_threads(),
                    self.focus_selected_folder(),
                ])
            }
            SidebarEvent::PublicFolderSelected { .. } => {
                self.reset_view_state();
                self.load_navigation_and_threads()
            }
            SidebarEvent::ChatSelected(email) => self.enter_chat_view(&email),
            SidebarEvent::LayoutChanged(json) => self.persist_sidebar_layout(json),
            SidebarEvent::Folder(command) => self.handle_folder_command(command),
        }
    }

    fn persist_sidebar_layout(&self, json: String) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            log::warn!("settings.set: no ServiceClient yet; sidebar layout not persisted");
            return Task::none();
        };
        Task::perform(
            async move {
                client
                    .set_settings(vec![SettingValue::SidebarLayout(json)])
                    .await
            },
            |result| {
                if let Err(e) = result {
                    log::warn!("settings.set sidebar_layout failed: {e}");
                }
                Message::Noop
            },
        )
    }

    /// Full view-transition reset: clear search, pinned search, thread
    /// Reset view state: clear search, thread selection, chat, bump
    /// generations, and update thread list context.
    /// Call before loading threads/navigation for the new view.
    pub(crate) fn reset_view_state(&mut self) {
        self.clear_search_state();
        self.clear_pinned_search_context();
        self.active_chat = None;
        self.sidebar.active_chat = None;
        self.clear_thread_selection();
        self.chat_timeline = None;
        let _ = self.nav_generation.next();
        let _ = self.thread_generation.next();
        self.update_thread_list_context_from_sidebar();
    }

    /// Clear thread selection and reading pane together. Every code path that
    /// deselects threads must use this to prevent stale reading pane content.
    pub(crate) fn clear_thread_selection(&mut self) {
        self.thread_list.selected_thread = None;
        self.thread_list.clear_multi_select();
        self.reading_pane.set_thread(None);
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn handle_thread_list(&mut self, msg: ThreadListMessage) -> Task<Message> {
        let (task, event) = self.thread_list.update(msg);
        let mut tasks = vec![task.map(Message::ThreadList)];
        if let Some(evt) = event {
            tasks.push(self.handle_thread_list_event(evt));
        }
        Task::batch(tasks)
    }

    pub(crate) fn handle_thread_list_event(&mut self, event: ThreadListEvent) -> Task<Message> {
        match event {
            ThreadListEvent::ThreadSelected(idx) => {
                // Check modifier keys for multi-select behavior.
                if self.current_modifiers.control() {
                    return self.handle_thread_list(ThreadListMessage::ToggleThread(idx));
                }
                if self.current_modifiers.shift() {
                    return self.handle_thread_list(ThreadListMessage::RangeSelectThread(idx));
                }
                // Plain click: clear multi-select, single-select.
                self.thread_list.clear_multi_select();
                self.handle_select_thread(idx)
            }
            ThreadListEvent::RowPressed(idx) => {
                self.start_thread_drag(idx);
                Task::none()
            }
            ThreadListEvent::PressedRowLeft => {
                self.begin_thread_drag();
                Task::none()
            }
            ThreadListEvent::SearchQueryChanged(query) => {
                self.update(Message::SearchQueryChanged(query))
            }
            ThreadListEvent::SearchExecute => self.update(Message::SearchSuggest(
                crate::ui::search_suggest::SearchSuggestMessage::Activate,
            )),
            ThreadListEvent::SearchUndo => {
                if let Some(text) = self.search_query.undo() {
                    let query = text.to_owned();
                    self.thread_list.search_query.clone_from(&query);
                    return self.apply_search_debounce();
                }
                Task::none()
            }
            ThreadListEvent::SearchRedo => {
                if let Some(text) = self.search_query.redo() {
                    let query = text.to_owned();
                    self.thread_list.search_query.clone_from(&query);
                    return self.apply_search_debounce();
                }
                Task::none()
            }
            ThreadListEvent::ThreadDeselected => {
                self.clear_thread_selection();
                Task::none()
            }
            ThreadListEvent::WidenSearchScope => {
                self.set_search_scope(crate::ui::search_scope::SearchScopeChoice::AllMailboxes)
            }
            ThreadListEvent::TypeaheadQuery { .. } => Task::none(),
            ThreadListEvent::TypeaheadSelected(idx) => self.handle_typeahead_select(idx),
            ThreadListEvent::MultiSelectionChanged(_count) => {
                // Selection count changed - no action needed yet.
                Task::none()
            }
            ThreadListEvent::AutoAdvance { new_index } => {
                if let Some(idx) = new_index {
                    self.handle_select_thread(idx)
                } else {
                    self.clear_thread_selection();
                    Task::none()
                }
            }
            ThreadListEvent::BatchAction(_indices) => {
                // Batch email actions not yet wired to providers.
                Task::none()
            }
            ThreadListEvent::RowAction { index, action } => {
                use crate::action_resolve::MailActionIntent;
                use crate::ui::widgets::ThreadRowAction;

                let select = self.handle_select_thread(index);
                let in_trash =
                    crate::command_dispatch::build_context(self).thread_in_trash == Some(true);
                let (intent, command) = match action {
                    ThreadRowAction::Pin => (MailActionIntent::TogglePin, CommandId::EmailPin),
                    ThreadRowAction::Flag => (MailActionIntent::ToggleStar, CommandId::EmailStar),
                    ThreadRowAction::Trash => {
                        let intent = super::keyboard::delete_key_intent(in_trash);
                        let command = match intent {
                            MailActionIntent::PermanentDelete => CommandId::EmailPermanentDelete,
                            _ => CommandId::EmailTrash,
                        };
                        (intent, command)
                    }
                };
                if !self.is_command_available(command) {
                    return select;
                }
                Task::batch([select, self.handle_email_action(intent)])
            }
            ThreadListEvent::RowMenuOpened {
                index,
                selection_changed,
            } => self.handle_row_menu_opened(index, selection_changed),
            ThreadListEvent::RowMenuPicked(item) => self.handle_row_menu_picked(item),
        }
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn handle_reading_pane(&mut self, msg: ReadingPaneMessage) -> Task<Message> {
        let (task, event) = self.reading_pane.update(msg);
        let mut tasks = vec![task.map(Message::ReadingPane)];
        if let Some(evt) = event {
            tasks.push(self.handle_reading_pane_event(evt));
        }
        Task::batch(tasks)
    }

    pub(crate) fn handle_reading_pane_event(&mut self, event: ReadingPaneEvent) -> Task<Message> {
        match event {
            ReadingPaneEvent::OpenMessagePopOut { message_index } => {
                self.open_message_view_window(message_index)
            }
            ReadingPaneEvent::PreviewAttachment(params) => self.handle_preview_attachment(params),
            ReadingPaneEvent::OpenAttachment(params) => self.handle_open_attachment(params),
            ReadingPaneEvent::SaveAttachment(params) => self.handle_save_attachment(params),
            ReadingPaneEvent::ReplyToMessage { message_index } => self.handle_reading_pane_compose(
                message_index,
                ComposeMode::Reply {
                    original_subject: self.current_subject(),
                },
            ),
            ReadingPaneEvent::ReplyAllToMessage { message_index } => self
                .handle_reading_pane_compose(
                    message_index,
                    ComposeMode::ReplyAll {
                        original_subject: self.current_subject(),
                    },
                ),
            ReadingPaneEvent::ForwardMessage { message_index } => self.handle_reading_pane_compose(
                message_index,
                ComposeMode::Forward {
                    original_subject: self.current_subject(),
                },
            ),
            ReadingPaneEvent::EditContact { email } => {
                // Open the contact editor in settings for this email.
                // Find or create the contact, then open settings with editor.
                self.open_contact_editor_for_email(email)
            }
            ReadingPaneEvent::CreateEventFromEmail { message_index } => {
                self.create_event_from_email(message_index)
            }
            ReadingPaneEvent::RespondToMeetingInvite {
                message_id,
                account_id,
                event_id,
                response,
                reply,
            } => self.respond_to_meeting_invite(message_id, account_id, event_id, response, reply),
            ReadingPaneEvent::ToggleStar => {
                self.update(Message::ExecuteCommand(CommandId::EmailStar))
            }
        }
    }

    /// Resolve each open meeting request against the synced calendar so
    /// its card can show the slot, the day's conflicts and RSVP controls.
    pub(crate) fn load_meeting_invites(&mut self) -> Task<Message> {
        let accounts = &self.sidebar.accounts;
        let lookups = self.reading_pane.pending_invite_lookups(|account_id| {
            accounts
                .iter()
                .any(|account| account.id == account_id && account.provider == "graph")
        });
        Task::batch(lookups.into_iter().map(|(message_id, account_id, uid)| {
            let db = Arc::clone(&self.db);
            Task::perform(
                async move { db.load_meeting_invite(account_id, uid).await },
                move |result| {
                    Message::ReadingPane(ReadingPaneMessage::InviteLoaded(
                        message_id.clone(),
                        result.map(|details| details.map(Box::new)),
                    ))
                },
            )
        }))
    }

    /// Send an invite response through the calendar-action pipeline.
    fn respond_to_meeting_invite(
        &mut self,
        message_id: String,
        account_id: String,
        event_id: String,
        response: service_api::RsvpResponse,
        reply: service_api::WireRsvpReply,
    ) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            self.reading_pane.finish_invite_response(
                &message_id,
                Err(t!("main.errors.cannot_respond_not_connected").into_owned()),
            );
            return Task::none();
        };
        let notified = reply.send_response;
        let plan = service_api::CalendarActionPlan {
            plan_id: service_api::PlanId::new_v7(),
            operations: vec![service_api::CalendarActionWireOperation {
                operation_id: service_api::OperationId(0),
                account_id,
                operation: service_api::WireCalendarOperation::RsvpEvent {
                    event_id,
                    response,
                    reply,
                },
            }],
        };
        let plan_id = plan.plan_id;
        Task::perform(
            async move {
                client
                    .execute_calendar_plan(plan)
                    .await
                    .map_err(|e| e.to_string())?;
                let completed = client
                    .subscribe_or_consume_calendar_action(plan_id)
                    .await
                    .map_err(|e| e.to_string())?;
                crate::handlers::calendar::completion_to_result(&completed)
            },
            move |result| Message::MeetingInviteResponded {
                message_id: message_id.clone(),
                notified,
                result,
            },
        )
    }

    /// Create a calendar event pre-filled from the given email message.
    pub(crate) fn create_event_from_email(&mut self, message_index: usize) -> Task<Message> {
        use crate::ui::calendar::{CalendarEventData, CalendarWorkflow, EditorSession};

        let msg = self.reading_pane.thread_messages.get(message_index);
        let Some(msg) = msg else { return Task::none() };

        let title = msg.subject.clone().unwrap_or_default();
        let description = msg.snippet.clone().unwrap_or_default();

        // Pre-fill attendees from To/Cc addresses.
        let now = jiff::Zoned::now();
        let today = now.date();
        let hour = u32::from(now.hour().unsigned_abs());
        let mut event = CalendarEventData::new_at(today, hour.min(22));
        event.title = title;
        event.description = description;

        // Set the account_id from the message's actual account.
        let account_id = Some(msg.account_id.clone());
        event.account_id = account_id.clone();

        // Pre-assign calendar when unambiguous for this account.
        if event.calendar_id.is_none()
            && let Some(ref acct) = account_id
        {
            let account_cals: Vec<_> = self
                .calendar
                .calendars
                .iter()
                .filter(|c| &c.account_id == acct)
                .collect();
            if account_cals.len() == 1 {
                event.calendar_id = Some(account_cals[0].id.clone());
            }
        }

        let session = EditorSession::new(event);
        // Workflow first, then surface.
        self.calendar.workflow = CalendarWorkflow::CreatingEvent {
            account_id,
            session,
        };
        self.calendar.sync_surfaces();

        // If calendar is popped out, focus that window instead of switching main to calendar.
        if let Some((&win_id, _)) = self
            .pop_out_windows
            .iter()
            .find(|(_, w)| matches!(w, crate::pop_out::PopOutWindow::Calendar(_)))
        {
            return iced::window::gain_focus(win_id);
        }

        // Otherwise switch to calendar mode to show the editor.
        self.app_mode = AppMode::Calendar;
        self.reload_calendar_events()
    }

    /// Returns the window ID of the calendar pop-out, if one exists.
    pub(crate) fn calendar_pop_out_id(&self) -> Option<iced::window::Id> {
        self.pop_out_windows
            .iter()
            .find(|(_, w)| matches!(w, PopOutWindow::Calendar(_)))
            .map(|(&id, _)| id)
    }

    /// Dismiss all mutually exclusive overlays (palette, settings, calendar
    /// overlays, add-account wizard). Call before opening a new overlay.
    pub(crate) fn dismiss_overlays(&mut self) {
        if self.palette.is_open() {
            self.palette.close();
        }
        if self.show_settings {
            self.close_settings();
        }
        self.calendar.workflow = crate::ui::calendar::CalendarWorkflow::Idle;
        self.calendar.sync_surfaces();
        self.add_account_wizard = None;
    }

    /// Open settings to a specific tab. Handles the full protocol:
    /// dismiss conflicting overlays, show_settings, sheet reset, animation
    /// reset, tab, begin_editing.
    pub(crate) fn open_settings(&mut self, tab: crate::ui::settings::Tab) {
        self.dismiss_overlays();
        self.show_settings = true;
        self.settings.active_sheet = None;
        self.settings
            .sheet_anim
            .go_mut(false, iced::time::Instant::now());
        self.settings.active_tab = tab;
        self.settings.begin_editing();
    }

    /// Write the committed preferences when they differ from what was last
    /// persisted. Runs after every message, so every way of closing settings
    /// (Escape, the toggle, switching mode, opening another overlay) saves.
    /// Nothing is written before the boot snapshot has been applied.
    ///
    /// Atomicity is preserved Service-side: the handler writes all values
    /// in one `WriteConn::transaction`. Failure policy is log-only; the next
    /// boot shows the old values.
    pub(crate) fn persist_preferences_if_changed(&mut self) -> Task<Message> {
        let prefs = self.settings.committed_preferences.clone();
        if self.settings.persisted_preferences.as_ref() == Some(&prefs)
            || self.settings.persisted_preferences.is_none()
        {
            return Task::none();
        }
        let Some(client) = self.service_client.as_ref().cloned() else {
            log::warn!("settings.set: no ServiceClient yet; preferences not persisted");
            return Task::none();
        };
        self.settings.persisted_preferences = Some(prefs.clone());
        let values = vec![
            SettingValue::ShowSyncStatus(prefs.sync_status_bar),
            SettingValue::AutoUpdate(prefs.auto_update),
            SettingValue::BlockRemoteImages(prefs.block_remote_images),
            SettingValue::PhishingDetectionEnabled(prefs.phishing_detection),
            SettingValue::PhishingSensitivity(prefs.phishing_sensitivity.clone()),
            SettingValue::Theme(prefs.theme.clone()),
            SettingValue::FontSize(prefs.font_size.clone()),
            SettingValue::ReadingPanePosition(prefs.reading_pane_position.clone()),
            SettingValue::EmailZoom(prefs.email_zoom.clone()),
            SettingValue::ThreadDateGroups(prefs.thread_date_groups),
            SettingValue::MailCheckInterval(prefs.mail_check_interval.clone()),
            SettingValue::EmailBodyBackground(prefs.email_body_background.label().into()),
        ];
        Task::perform(async move { client.set_settings(values).await }, |result| {
            if let Err(e) = result {
                log::warn!("settings.set failed: {e}");
            }
            Message::Noop
        })
    }

    /// Re-render HTML bodies when the theme or the body background
    /// preference changes what they are painted on.
    pub(crate) fn sync_email_appearance(&mut self) -> Task<Message> {
        let appearance = crate::ui::theme::email_body_appearance(&self.theme());
        if crate::ui::mail_body::reading_appearance() == appearance {
            return Task::none();
        }
        crate::ui::mail_body::set_reading_appearance(appearance);
        Task::batch([
            self.reading_pane
                .set_reading_appearance(appearance)
                .map(Message::ReadingPane),
            self.settings
                .set_signature_preview_appearance(appearance)
                .map(Message::Settings),
            self.set_pop_out_appearance(appearance),
        ])
    }

    /// Apply a keyboard zoom step to the reading zoom preference.
    pub(crate) fn step_email_zoom(
        &mut self,
        step: crate::ui::settings::types::EmailZoomStep,
    ) -> Task<Message> {
        use crate::ui::settings::types::{
            ACTUAL_SIZE_EMAIL_ZOOM, EmailZoomStep, email_zoom_factor, step_email_zoom,
        };
        let label = match step {
            EmailZoomStep::In => step_email_zoom(&self.settings.email_zoom, 1),
            EmailZoomStep::Out => step_email_zoom(&self.settings.email_zoom, -1),
            EmailZoomStep::ActualSize => ACTUAL_SIZE_EMAIL_ZOOM,
        };
        let Some(zoom) = email_zoom_factor(label) else {
            return Task::none();
        };
        self.settings.email_zoom = label.into();
        self.settings.committed_preferences.email_zoom = label.into();
        if let Some(prefs) = self.settings.editing_preferences.as_mut() {
            prefs.email_zoom = label.into();
        }
        crate::ui::mail_body::set_reading_zoom(zoom);
        self.reading_pane
            .set_reading_zoom(zoom)
            .map(Message::ReadingPane)
    }

    /// Close settings, committing preference changes.
    pub(crate) fn close_settings(&mut self) {
        self.settings.commit_preferences();
        self.show_settings = false;
    }

    /// Open the contact editor in settings for a specific email address.
    /// Navigates to Settings > People and opens the editor, creating a
    /// new local contact if none exists for that email.
    pub(crate) fn open_contact_editor_for_email(&mut self, email: String) -> Task<Message> {
        self.open_settings(crate::ui::settings::types::Tab::People);

        // Look up existing contact or create new editor state
        let found = self
            .settings
            .contacts
            .iter()
            .find(|c| c.email.eq_ignore_ascii_case(&email));

        if let Some(contact) = found {
            let id = contact.id.clone();
            self.settings.open_contact_editor(&id);
        } else {
            // Create a new editor pre-populated with the email
            self.settings.open_new_contact_editor();
            if let Some(ref mut editor) = self.settings.contact_editor {
                editor.email.set_text(email);
            }
        }

        // Load contacts for the settings view
        self.handle_load_contacts(self.settings.contact_filter.clone())
    }

    /// Get the subject of the currently selected thread.
    pub(crate) fn current_subject(&self) -> String {
        self.thread_list
            .selected_thread
            .and_then(|idx| self.thread_list.threads.get(idx))
            .and_then(|t| t.subject.clone())
            .unwrap_or_default()
    }

    /// Open a compose window from a reading pane Reply/ReplyAll/Forward action.
    pub(crate) fn handle_reading_pane_compose(
        &mut self,
        message_index: usize,
        mode: ComposeMode,
    ) -> Task<Message> {
        let thread_id = self
            .thread_list
            .selected_thread
            .and_then(|idx| self.thread_list.threads.get(idx))
            .map(|t| t.id.as_str());
        let msg = self.reading_pane.thread_messages.get(message_index);
        let source = msg
            .map(|m| pop_out::compose::ReplySource::from_thread_message(m, thread_id))
            .unwrap_or_default();
        let state =
            pop_out::compose::ComposeState::new_reply(&self.sidebar.accounts, &mode, &source);

        self.open_inline_compose(state, mode)
    }

    pub(crate) fn handle_status_bar(&mut self, msg: StatusBarMessage) -> Task<Message> {
        let (task, event) = self.status_bar.update(msg);
        let mut tasks = vec![task.map(Message::StatusBar)];
        if let Some(evt) = event {
            tasks.push(self.handle_status_bar_event(evt));
        }
        Task::batch(tasks)
    }

    pub(crate) fn handle_status_bar_event(&mut self, event: StatusBarEvent) -> Task<Message> {
        match event {
            StatusBarEvent::RequestReauth { account_id } => {
                self.handle_open_reauth_wizard(account_id)
            }
        }
    }

    pub(crate) fn handle_sync_event(&mut self, event: SyncEvent) {
        match event {
            SyncEvent::Progress {
                account_id,
                phase,
                current,
                total,
            } => {
                log::info!("Sync progress: account={account_id} phase={phase} {current}/{total}");
                let email = self.email_for_account(&account_id);
                self.status_bar
                    .report_sync_progress(account_id, email, current, total, phase);
            }
            SyncEvent::Complete { account_id } => {
                log::info!("Sync complete: account={account_id}");
                self.status_bar.report_sync_complete(&account_id);
                // Clear connection failure warnings on successful sync.
                self.status_bar.clear_warning(&account_id);
            }
            SyncEvent::Error { account_id, error } => {
                log::warn!("Sync error: account={account_id} error={error}");
                let email = self.email_for_account(&account_id);
                self.status_bar.set_warning(AccountWarning {
                    account_id,
                    email,
                    kind: WarningKind::ConnectionFailure { message: error },
                });
            }
        }
    }

    /// Look up the email address for an account ID from the sidebar's
    /// account list. Returns the account ID itself if not found.
    pub(crate) fn email_for_account(&self, account_id: &str) -> String {
        self.sidebar
            .accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| a.email.clone())
            .unwrap_or_else(|| account_id.to_string())
    }

    /// Spawn an async task that checks whether any account has an active
    /// auto-reply and delivers the result as `Message::AutoReplyChecked`.
    pub(crate) fn check_auto_reply_status(&self) -> Task<Message> {
        let db = std::sync::Arc::clone(&self.db);
        Task::perform(
            async move { db.any_auto_response_active().await },
            |result| Message::AutoReplyChecked(result.unwrap_or(false)),
        )
    }

    pub(crate) fn handle_settings(&mut self, msg: SettingsMessage) -> Task<Message> {
        let (task, event) = self.settings.update(msg);
        let mut tasks = vec![task.map(Message::Settings)];
        if let Some(evt) = event {
            tasks.push(self.handle_settings_event(evt));
        }
        Task::batch(tasks)
    }

    pub(crate) fn handle_settings_event(&mut self, event: SettingsEvent) -> Task<Message> {
        match event {
            SettingsEvent::Closed => {
                self.close_settings();
                Task::none()
            }
            // Persisted by `persist_preferences_if_changed` after the message.
            SettingsEvent::PreferencesCommitted | SettingsEvent::PreferencesDiscarded => {
                // Live fields are already restored to committed state by Settings.
                Task::none()
            }
            SettingsEvent::DateDisplayChanged(display) => {
                self.reading_pane.date_display = display;
                Task::none()
            }
            SettingsEvent::EmailZoomChanged(zoom) => self
                .reading_pane
                .set_reading_zoom(zoom)
                .map(Message::ReadingPane),
            SettingsEvent::OpenAddAccountWizard => self.handle_open_add_account_wizard(),
            SettingsEvent::CheckForUpdates => {
                self.handle_updater(crate::updater::UpdaterMessage::CheckRequestedByUser)
            }
            SettingsEvent::OpenGithub => {
                crate::ui::reading_pane::open_url_in_browser(&format!(
                    "https://github.com/{}",
                    crate::updater::REPOSITORY
                ));
                Task::none()
            }
            SettingsEvent::DeleteAccount(account_id) => self.handle_delete_account(account_id),
            SettingsEvent::SaveAccountChanges { account_id, params } => {
                self.handle_save_account_changes(account_id, params)
            }
            SettingsEvent::SaveSignature(req) => {
                handlers::signatures::handle_save_signature(self.service_client.clone(), req)
                    .map(Message::SignatureOp)
            }
            SettingsEvent::DeleteSignature(id) => {
                handlers::signatures::handle_delete_signature(self.service_client.clone(), id)
                    .map(Message::SignatureOp)
            }
            SettingsEvent::ReorderSignatures(ordered_ids) => {
                handlers::signatures::handle_reorder_signatures(
                    self.service_client.clone(),
                    ordered_ids,
                )
                .map(Message::SignatureOp)
            }
            SettingsEvent::LoadContacts(filter) => self.handle_load_contacts(filter),
            SettingsEvent::LoadGroups(filter) => self.handle_load_groups(filter),
            SettingsEvent::SaveContact(entry) => self.handle_save_contact(entry),
            SettingsEvent::DeleteContact(id) => self.handle_delete_contact(id),
            SettingsEvent::SaveGroup(group, members) => self.handle_save_group(group, members),
            SettingsEvent::DeleteGroup(id) => self.handle_delete_group(id),
            SettingsEvent::LoadGroupMembers(group_id) => self.handle_load_group_members(group_id),
            SettingsEvent::LoadLabelGroupMembers(group_id) => {
                handlers::labels::load_label_group_members_async(&self.db, group_id)
            }
            SettingsEvent::ReorderLabelGroups(orders) => self.handle_reorder_label_groups(orders),
            SettingsEvent::SaveLabel {
                account_ids,
                label_id,
                name,
                color,
            } => self.handle_save_label(account_ids, label_id, name, color),
            SettingsEvent::DeleteLabel {
                account_id,
                label_id,
            } => self.handle_delete_label(account_id, label_id),
            SettingsEvent::ExecuteContactImport {
                prepared,
                account_id,
                update_existing,
            } => self.handle_import_contacts(prepared, account_id, update_existing),
            SettingsEvent::ReorderAccounts(orders) => self.handle_reorder_accounts(orders),
            SettingsEvent::ReauthenticateAccount(account_id) => {
                self.handle_open_reauth_wizard(account_id)
            }
            SettingsEvent::ResyncAccount(account_id) => self.handle_resync_account(account_id),
        }
    }

    fn handle_resync_account(&mut self, account_id: String) -> Task<Message> {
        let Some(service_client) = self.service_client.clone() else {
            self.status_bar
                .show_confirmation(t!("settings.labels.service_not_connected").into_owned());
            return Task::none();
        };
        self.status_bar
            .show_confirmation(t!("settings.accounts.resync_started").into_owned());
        Task::perform(
            async move {
                service_client
                    .resync_sync_account(account_id)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            },
            Message::AccountResyncComplete,
        )
    }

    pub(crate) fn handle_add_account(&mut self, msg: AddAccountMessage) -> Task<Message> {
        let wizard = match self.add_account_wizard.as_mut() {
            Some(w) => w,
            None => return Task::none(),
        };

        let (task, event) = wizard.update(msg);
        let mut tasks = vec![task.map(Message::AddAccount)];

        if let Some(evt) = event {
            tasks.push(self.handle_add_account_event(evt));
        }
        Task::batch(tasks)
    }

    pub(crate) fn handle_signature_op(
        &mut self,
        result: handlers::SignatureResult,
    ) -> Task<Message> {
        // Phase 6a: each IPC method has its own ack variant. The
        // post-ack behavior is the same on the happy path
        // (re-list so the settings UI reflects the canonical
        // Service-committed state), but per-variant arms keep the
        // door open for per-method handling later (e.g. surfacing a
        // toast on create with the new id, or rolling back a
        // failed reorder).
        match result {
            handlers::SignatureResult::Loaded(Ok(sigs)) => {
                self.settings.signatures = sigs;
                Task::none()
            }
            handlers::SignatureResult::Loaded(Err(e)) => {
                log::error!("Failed to load signatures: {e}");
                Task::none()
            }
            handlers::SignatureResult::CreatedAck(_)
            | handlers::SignatureResult::UpdatedAck(_)
            | handlers::SignatureResult::DeletedAck(_)
            | handlers::SignatureResult::ReorderedAck(_) => {
                handlers::signatures::load_signatures_async(&self.db).map(Message::SignatureOp)
            }
        }
    }

    pub(crate) fn handle_label_op(&mut self, op: handlers::LabelOp) -> Task<Message> {
        match op {
            handlers::LabelOp::Loaded(Ok(groups)) => {
                self.settings.labels_by_account = groups;
                Task::none()
            }
            handlers::LabelOp::Loaded(Err(e)) => {
                log::error!("Failed to load per-account labels: {e}");
                Task::none()
            }
            handlers::LabelOp::GroupsLoaded(Ok(groups)) => {
                self.settings.label_groups = groups;
                Task::none()
            }
            handlers::LabelOp::GroupsLoaded(Err(e)) => {
                log::error!("Failed to load label groups: {e}");
                Task::none()
            }
            handlers::LabelOp::ReorderAck(Ok(())) => {
                // Refresh both the settings list and the sidebar so the
                // new order shows up everywhere it's rendered.
                Task::batch([
                    handlers::labels::load_label_groups_async(&self.db).map(Message::LabelOp),
                    self.load_navigation_and_threads(),
                ])
            }
            handlers::LabelOp::ReorderAck(Err(e)) => {
                log::error!("Failed to persist label group order: {e}");
                Task::none()
            }
            handlers::LabelOp::WriteAck(result) => {
                if let Err(ref e) = result {
                    log::error!("Label write failed: {e}");
                }
                self.settings.finish_label_write(result);
                // Reload even on failure: a create fanned out to several
                // accounts may have succeeded on some of them.
                Task::batch([
                    handlers::labels::load_visible_labels_async(&self.db).map(Message::LabelOp),
                    handlers::labels::load_label_groups_async(&self.db).map(Message::LabelOp),
                    self.load_navigation_and_threads(),
                ])
            }
        }
    }

    pub(crate) fn handle_save_label(
        &mut self,
        account_ids: Vec<String>,
        label_id: Option<String>,
        name: String,
        color: Option<(String, String)>,
    ) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            return Task::done(Message::LabelOp(handlers::LabelOp::WriteAck(Err(t!(
                "settings.labels.service_not_connected"
            )
            .into_owned()))));
        };
        let account_names: Vec<String> = account_ids
            .iter()
            .map(|id| {
                self.settings
                    .managed_accounts
                    .iter()
                    .find(|a| &a.id == id)
                    .map_or_else(|| id.clone(), |a| a.email.clone())
            })
            .collect();
        Task::perform(
            async move {
                let mut failures = Vec::new();
                for (account_id, account_name) in account_ids.into_iter().zip(account_names) {
                    let params = service_api::LabelSaveParams {
                        account_id,
                        label_id: label_id.clone(),
                        name: name.clone(),
                        color: color.clone(),
                    };
                    if let Err(e) = client.save_label(params).await {
                        failures.push(format!("{account_name}: {e}"));
                    }
                }
                if failures.is_empty() {
                    Ok(())
                } else {
                    Err(failures.join("\n"))
                }
            },
            |result| Message::LabelOp(handlers::LabelOp::WriteAck(result)),
        )
    }

    pub(crate) fn handle_delete_label(
        &mut self,
        account_id: String,
        label_id: String,
    ) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            return Task::done(Message::LabelOp(handlers::LabelOp::WriteAck(Err(t!(
                "settings.labels.service_not_connected"
            )
            .into_owned()))));
        };
        Task::perform(
            async move {
                client
                    .delete_label(account_id, label_id)
                    .await
                    .map_err(|e| e.to_string())
            },
            |result| Message::LabelOp(handlers::LabelOp::WriteAck(result)),
        )
    }

    pub(crate) fn handle_delete_account(&mut self, account_id: String) -> Task<Message> {
        // Phase 6a-part-2: account.delete is now a single Service-side
        // IPC that folds cancel-and-await for sync/push/calendar
        // runners + delete_account_orchestrate + external-store
        // cleanup (body / inline / search / attachment cache). The
        // UI no longer holds the cleanup orchestration - the
        // runner-quiescence invariant closes Service-side, so a
        // future caller cannot delete while runners hold writer
        // references. The 60 s wire timeout absorbs cleanup on a
        // heavily-cached account.

        // Close compose and message-view pop-outs belonging to the deleted account.
        let windows_to_close: Vec<iced::window::Id> = self
            .pop_out_windows
            .iter()
            .filter(|(_, w)| match w {
                PopOutWindow::Compose(state) => state
                    .from_account
                    .as_ref()
                    .is_some_and(|a| a.id == account_id),
                PopOutWindow::MessageView(state) => state.account_id == account_id,
                PopOutWindow::Calendar(_) => false,
            })
            .map(|(&id, _)| id)
            .collect();
        let mut close_tasks: Vec<Task<Message>> = Vec::new();
        for win_id in windows_to_close {
            close_tasks.push(self.close_compose(win_id));
        }

        // If the deleted account is referenced by the current scope, revert to All Accounts
        let scope_references_account = match &self.sidebar.selected_scope {
            ViewScope::Account(id) => *id == account_id,
            ViewScope::SharedMailbox {
                account_id: aid, ..
            }
            | ViewScope::PublicFolder {
                account_id: aid, ..
            } => *aid == account_id,
            ViewScope::AllAccounts => false,
        };
        if scope_references_account {
            self.sidebar.selected_scope = ViewScope::AllAccounts;
        }

        let Some(client) = self.service_client.as_ref().cloned() else {
            log::warn!("account.delete: no ServiceClient yet; ignoring delete request");
            return Task::batch(close_tasks);
        };

        let delete_task = Task::perform(
            async move {
                client
                    .delete_account(account_id)
                    .await
                    .map(|ack| {
                        log::info!(
                            "account.delete ack: {} bodies, {} inline images, {} cache files; \
                             search_cleaned={}",
                            ack.bodies_deleted,
                            ack.inline_images_deleted,
                            ack.cache_files_deleted,
                            ack.search_cleaned,
                        );
                        if !ack.cache_file_errors.is_empty() {
                            log::warn!(
                                "account.delete: {} cache file errors",
                                ack.cache_file_errors.len(),
                            );
                        }
                    })
                    .map_err(|e| e.to_string())
            },
            Message::AccountDeleted,
        );

        close_tasks.push(delete_task);
        Task::batch(close_tasks)
    }

    pub(crate) fn handle_save_account_changes(
        &mut self,
        account_id: String,
        params: rtsk::db::queries_extra::UpdateAccountParams,
    ) -> Task<Message> {
        // Phase 6a: account.update IPC. Convert the existing internal
        // UpdateAccountParams (carries no id; takes the id from the
        // function arg) into the wire shape (id-bearing).
        let Some(client) = self.service_client.as_ref().cloned() else {
            log::warn!("account.update: no ServiceClient yet; ignoring update");
            return Task::none();
        };
        let wire = service_api::AccountUpdateParams {
            id: account_id,
            account_name: params.account_name,
            display_name: params.display_name,
            account_color: params.account_color,
            caldav_url: params.caldav_url,
            caldav_username: params.caldav_username,
            caldav_password: params.caldav_password,
            cache_attachments_enabled: params.cache_attachments_enabled,
        };
        Task::perform(
            async move { client.update_account(wire).await.map_err(|e| e.to_string()) },
            Message::AccountUpdated,
        )
    }

    pub(crate) fn handle_reorder_label_groups(&mut self, orders: Vec<(i64, i64)>) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            log::warn!("label_group.reorder: no ServiceClient yet; ignoring reorder");
            return Task::none();
        };
        Task::perform(
            async move {
                client
                    .reorder_label_groups(orders)
                    .await
                    .map_err(|e| e.to_string())
            },
            |result| Message::LabelOp(handlers::LabelOp::ReorderAck(result)),
        )
    }

    pub(crate) fn handle_reorder_accounts(&mut self, orders: Vec<(String, i64)>) -> Task<Message> {
        // Phase 6a: account.reorder IPC. Same staleness tolerance as
        // signature.reorder - rapid drag-reorder clicks may land out
        // of order; next reload reconciles. Per-entity ordering token
        // is the documented escape hatch if a real bug shows up.
        let Some(client) = self.service_client.as_ref().cloned() else {
            log::warn!("account.reorder: no ServiceClient yet; ignoring reorder");
            return Task::none();
        };
        Task::perform(
            async move {
                client
                    .reorder_accounts(orders)
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::AccountUpdated,
        )
    }

    pub(crate) fn handle_clear_all_pinned_searches(&mut self) -> Task<Message> {
        // Eager local clear before the IPC: the toggle feels instant
        // and the staleness window is bounded by the next sidebar
        // reload. No-rollback policy on Err - we log and accept that
        // a Service-side failure leaves rows on disk that the next
        // refresh path will surface again. Matches the documented
        // signature-reorder precedent for staleness-tolerant ops.
        self.pinned_searches.clear();
        self.sidebar.active_pinned_search = None;
        self.sidebar.pinned_searches.clear();
        let Some(client) = self.service_client.as_ref().cloned() else {
            log::warn!(
                "pinned_search.delete_all: no ServiceClient yet; \
                 clear not persisted (next reload reconciles)"
            );
            return Task::none();
        };
        Task::perform(
            async move {
                client
                    .delete_all_pinned_searches()
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::PinnedSearchDeleteAllAck,
        )
    }

    pub(crate) fn handle_window_close(&mut self, id: iced::window::Id) -> Task<Message> {
        if id == self.main_window_id {
            log::info!("Main window closing, saving state");
            let data_dir = crate::APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
            self.window.sidebar_width = self.sidebar_width;
            self.window.thread_list_width = self.thread_list_width;
            self.window.side_panel_open = self.side_panel.open;
            self.window.side_panel_module = self.side_panel.module;
            self.window.save(data_dir);
            self.save_session_state();
            // Save dirty compose drafts synchronously before destroying windows
            let compose_ids: Vec<_> = self
                .pop_out_windows
                .iter()
                .filter_map(|(&win_id, w)| {
                    matches!(w, PopOutWindow::Compose(s) if s.draft_dirty).then_some(win_id)
                })
                .collect();
            for win_id in compose_ids {
                self.save_compose_draft_sync(win_id);
            }
            let inline = self.inline_compose.take();
            let mut tasks: Vec<Task<Message>> = self
                .pop_out_windows
                .keys()
                .filter(|&&win_id| Some(win_id) != inline)
                .map(|&win_id| iced::window::close(win_id))
                .collect();
            self.pop_out_windows.clear();
            tasks.push(iced::window::close(id));
            if let Some(service_client) = self.service_client.clone() {
                tasks.push(Task::perform(
                    async move {
                        service_client
                            .shutdown()
                            .await
                            .map_err(|error| error.to_string())
                    },
                    Message::ServiceShutdownComplete,
                ));
            } else {
                tasks.push(iced::exit());
            }
            return Task::batch(tasks);
        }

        if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&id) {
            if state.has_user_content() && !state.discard_confirm_open {
                // Show discard confirmation instead of closing silently
                state.discard_confirm_open = true;
                return Task::none();
            }
            // Either no content or user already confirmed - save and close
            if !self.save_compose_draft_sync(id) {
                log::warn!("Compose draft save failed, aborting window close");
                return Task::none();
            }
        }
        // Calendar pop-out closing - calendar becomes available in main window again.
        // (No state change needed - mode toggle just works.)
        self.pop_out_windows.remove(&id);
        iced::window::close(id)
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn handle_select_thread(&mut self, idx: usize) -> Task<Message> {
        self.pending_row_action = None;
        self.release_new_message_compose();
        let thread = self.thread_list.threads.get(idx);
        if let Some(t) = thread {
            log::debug!("Thread selected: {}", t.id);
        }

        // Local drafts reopen in the compose instead of the message view.
        if let Some(t) = thread
            && t.is_local_draft
        {
            let draft_id = t.id.clone();
            let db = Arc::clone(&self.db);
            return Task::perform(
                async move {
                    let core_db = db.read_db_state();
                    rtsk::db::queries_extra::db_get_local_draft(&core_db, draft_id).await
                },
                Message::LocalDraftLoaded,
            );
        }

        self.reading_pane.set_thread(thread);

        // Public folder items aren't real threads - skip detail loading.
        if matches!(self.sidebar.selected_scope, ViewScope::PublicFolder { .. }) {
            return Task::none();
        }

        // Set search highlight terms when in search mode
        if self.thread_list.mode == ui::thread_list::ThreadListMode::Search {
            let query = self.search_query.text().to_string();
            let parsed = smart_folder::parse_query(&query);
            self.reading_pane.search_highlight_terms = parsed
                .free_text
                .split_whitespace()
                .map(String::from)
                .collect();
        } else {
            self.reading_pane.search_highlight_terms.clear();
        }

        let thread_gen = self.thread_generation.next();
        if let Some(thread) = thread {
            let account_id = thread.account_id.clone();
            let thread_id = thread.id.clone();
            let load_gen = thread_gen;

            // Use core's thread detail if body store is available,
            // otherwise fall back to the old separate queries.
            if let Some(ref body_store) = self.body_store {
                let db = Arc::clone(&self.db);
                let bs = body_store.clone();
                let iis = self.inline_image_store.clone();
                return Task::perform(
                    async move {
                        let r = db::threads::load_thread_detail(
                            &db,
                            &bs,
                            iis.as_ref(),
                            account_id,
                            thread_id,
                        )
                        .await;
                        (load_gen, r)
                    },
                    |(g, result)| Message::ThreadDetailLoaded(g, result),
                );
            }
        }
        Task::none()
    }

    /// Phase 6a-part-2 (encryption-key handle): apply the cold-boot
    /// bootstrap snapshots returned by `internal.read_bootstrap_snapshots`.
    /// On Err the UI keeps the default preferences and logs the failure;
    /// the next boot retries. Per-field decrypt warnings are surfaced
    /// from the IPC's `warnings` slot today only as log lines.
    pub(crate) fn handle_bootstrap_snapshots_loaded(
        &mut self,
        result: Result<service_api::ReadBootstrapSnapshotsAck, String>,
    ) {
        let ack = match result {
            Ok(ack) => ack,
            Err(e) => {
                log::warn!(
                    "bootstrap snapshots IPC failed; using default preferences this boot: {e}",
                );
                return;
            }
        };
        for warning in &ack.warnings {
            log::warn!("bootstrap snapshot decrypt warning: {warning}");
        }
        let ui_snap: rtsk::db::queries::UiBootstrapSnapshot = match serde_json::from_value(ack.ui) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("could not parse ui bootstrap snapshot: {e}");
                return;
            }
        };
        let settings_snap: rtsk::db::queries::SettingsBootstrapSnapshot =
            match serde_json::from_value(ack.settings) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("could not parse settings bootstrap snapshot: {e}");
                    return;
                }
            };
        self.settings.apply_bootstrap(&ui_snap, &settings_snap);
        self.sidebar.layouts =
            crate::ui::sidebar::order::SidebarLayouts::parse(ui_snap.sidebar_layout.as_deref());
        self.toolbar.layout =
            crate::ui::toolbar::ToolbarLayout::parse(ui_snap.toolbar_layout.as_deref());
        self.search_scope.choice =
            crate::ui::search_scope::SearchScopeChoice::parse(ui_snap.search_scope.as_deref());
        self.settings.persisted_preferences = Some(self.settings.committed_preferences.clone());
    }

    pub(crate) fn handle_accounts_loaded(&mut self, accounts: Vec<db::Account>) -> Task<Message> {
        let first_load = self.sidebar.accounts.is_empty();
        self.sidebar.accounts = accounts;
        if self.sidebar.accounts.is_empty() {
            self.status = "No account yet".to_string();
            return Task::none();
        }
        self.no_accounts = false;
        self.settings.managed_accounts = self
            .sidebar
            .accounts
            .iter()
            .map(|a| ui::settings::ManagedAccount {
                id: a.id.clone(),
                email: a.email.clone(),
                provider: a.provider.clone(),
                account_name: a.account_name.clone(),
                account_color: a.account_color.clone(),
                display_name: a.display_name.clone(),
                last_sync_at: a.last_sync_at,
                health: ui::settings::compute_health(
                    a.last_sync_at,
                    a.token_expires_at,
                    a.is_active,
                ),
            })
            .collect();
        let scope_still_exists = match &self.sidebar.selected_scope {
            ViewScope::Account(id)
            | ViewScope::SharedMailbox { account_id: id, .. }
            | ViewScope::PublicFolder { account_id: id, .. } => {
                self.sidebar.accounts.iter().any(|a| a.id == *id)
            }
            ViewScope::AllAccounts => true,
        };
        if (first_load || !scope_still_exists)
            && let Some(first) = self.sidebar.accounts.first()
        {
            self.sidebar.selected_scope = ViewScope::Account(first.id.clone());
            self.sidebar.selection = types::SidebarSelection::Inbox;
        }
        self.status = format!("Loaded {} accounts", self.sidebar.accounts.len());
        let sig_task =
            handlers::signatures::load_signatures_async(&self.db).map(Message::SignatureOp);
        let labels_task =
            handlers::labels::load_visible_labels_async(&self.db).map(Message::LabelOp);
        let label_groups_task =
            handlers::labels::load_label_groups_async(&self.db).map(Message::LabelOp);
        let sync_task = self.sync_all_accounts();
        let auto_reply_task = self.check_auto_reply_status();
        // Per-account thread_participants backfill used to run from here.
        // As of Phase 1.5 it runs Service-side during the boot sequence
        // (BootPhase::BackfillingThreadParticipants); the helper is
        // idempotent so future-account-creation backfill becomes a Phase 2
        // action-pipeline concern.
        //
        // JMAP push setup also used to run here; as of Phase 4 it runs
        // Service-side from a post-`boot.ready` task in
        // `dispatch/post_ready.rs::spawn_post_ready_push_startup`, which
        // starts the resident sync engine's ingress and attaches
        // accounts (the engine owns the per-account push bridges). The
        // `sync.start_account` IPC piggybacks push setup for newly
        // added accounts (Phase 4 task 6).
        Task::batch([
            self.load_navigation_and_threads(),
            sig_task,
            labels_task,
            label_groups_task,
            sync_task,
            auto_reply_task,
        ])
    }

    pub(crate) fn update_thread_list_context_from_sidebar(&mut self) {
        let folder_name = self
            .sidebar
            .selection
            .navigation_folder_id()
            .and_then(|nav_id| {
                self.sidebar.nav_state.as_ref().and_then(|ns| {
                    ns.folders
                        .iter()
                        .find(|f| f.id == nav_id)
                        .map(|f| ui::sidebar::folder_display_name(f).into_owned())
                })
            })
            .unwrap_or_else(|| t!("main.folder.inbox").into_owned());
        let scope_name = match &self.sidebar.selected_scope {
            ViewScope::AllAccounts => t!("main.thread_list.scope_all").into_owned(),
            ViewScope::Account(id) => self
                .sidebar
                .accounts
                .iter()
                .find(|a| a.id == *id)
                .and_then(|a| a.display_name.as_deref().or(Some(a.email.as_str())))
                .map_or_else(
                    || t!("main.thread_list.scope_account").into_owned(),
                    str::to_string,
                ),
            ViewScope::SharedMailbox { mailbox_id, .. } => self
                .sidebar
                .shared_mailboxes
                .iter()
                .find(|sm| sm.mailbox_id == *mailbox_id)
                .and_then(|sm| sm.display_name.as_deref())
                .unwrap_or(mailbox_id.as_str())
                .to_string(),
            ViewScope::PublicFolder { folder_id, .. } => self
                .sidebar
                .pinned_public_folders
                .iter()
                .find(|pf| pf.folder_id == *folder_id)
                .map(|pf| pf.display_name.as_str())
                .unwrap_or(folder_id.as_str())
                .to_string(),
        };
        self.thread_list.set_context(folder_name, scope_name);
    }
}
