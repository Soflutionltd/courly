use iced::Task;

use crate::action_resolve::MailActionIntent;
use crate::command_dispatch::{self, KeyEventMessage};
use crate::pop_out::PopOutWindow;
use crate::ui::thread_list::{ThreadListMessage, TypeaheadDirection};
use crate::{AppMode, Message, PendingChord, ReadyApp};
use cmdk::{Chord, CommandId, ResolveResult};

/// What the Delete key does. Outside the trash it moves the selection
/// there (undoable). Inside the trash it deletes for good, which is what
/// Outlook does with the same key.
pub(crate) fn delete_key_intent(in_trash: bool) -> MailActionIntent {
    if in_trash {
        MailActionIntent::PermanentDelete
    } else {
        MailActionIntent::Trash
    }
}

fn is_plain_delete_key(key: &iced::keyboard::Key, modifiers: iced::keyboard::Modifiers) -> bool {
    matches!(
        key,
        iced::keyboard::Key::Named(
            iced::keyboard::key::Named::Backspace | iced::keyboard::key::Named::Delete
        )
    ) && !modifiers.command()
        && !modifiers.control()
        && !modifiers.shift()
        && !modifiers.alt()
}

/// Select all, copy, paste, cut, undo and redo belong to the text field
/// that captured them, never to the thread list behind it.
fn is_text_editing_chord(key: &iced::keyboard::Key, modifiers: iced::keyboard::Modifiers) -> bool {
    let iced::keyboard::Key::Character(character) = key else {
        return false;
    };
    modifiers.command()
        && !modifiers.alt()
        && ["a", "c", "v", "x", "z"]
            .iter()
            .any(|editing| character.eq_ignore_ascii_case(editing))
}

impl ReadyApp {
    pub(crate) fn handle_key_event(&mut self, msg: KeyEventMessage) -> Task<Message> {
        match msg {
            KeyEventMessage::KeyPressed {
                key,
                modifiers,
                status,
                window_id,
            } => {
                if key == iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)
                    && window_id != self.main_window_id
                    && let Some(PopOutWindow::Compose(state)) =
                        self.pop_out_windows.get_mut(&window_id)
                {
                    if state.link_dialog_open {
                        state.link_dialog_open = false;
                        state.link_url.clear();
                        state.link_text.clear();
                        return Task::none();
                    }
                    if state.discard_confirm_open {
                        state.discard_confirm_open = false;
                        return Task::none();
                    }
                    if state.save_group_dialog_open {
                        state.save_group_dialog_open = false;
                        state.save_group_name.clear();
                        state.save_group_error = None;
                        state.save_group_in_flight = false;
                        return Task::none();
                    }
                }

                // Escape in a pop-out window closes that window
                if key == iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)
                    && window_id != self.main_window_id
                    && self.pop_out_windows.contains_key(&window_id)
                {
                    return self.handle_window_close(window_id);
                }
                self.handle_key_pressed(&key, modifiers, status, window_id)
            }
            KeyEventMessage::PendingChordTimeout => {
                self.pending_chord = None;
                Task::none()
            }
        }
    }

    fn handle_key_pressed(
        &mut self,
        key: &iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
        status: iced::event::Status,
        window_id: iced::window::Id,
    ) -> Task<Message> {
        use iced::keyboard::{Key, key::Named};

        // 0. The permanent-delete confirmation takes Enter and Escape, and
        //    holds back every other shortcut while it is up.
        if self.pending_permanent_delete.is_some() && window_id == self.main_window_id {
            return match key {
                Key::Named(Named::Enter) => self.update(Message::ConfirmPermanentDelete),
                Key::Named(Named::Escape) => self.update(Message::CancelPermanentDelete),
                _ => Task::none(),
            };
        }
        if let Some(dialog) = self.folder_dialog.as_ref()
            && window_id == self.main_window_id
        {
            use crate::ui::folder_dialog::FolderDialogMessage;
            return match key {
                Key::Named(Named::Escape) => {
                    self.update(Message::FolderDialog(FolderDialogMessage::Cancel))
                }
                Key::Named(Named::Enter) if !dialog.takes_text() => {
                    self.update(Message::FolderDialog(FolderDialogMessage::Submit))
                }
                _ => Task::none(),
            };
        }
        if self.thread_drag.is_some() && *key == Key::Named(Named::Escape) {
            return self.update(Message::ThreadDrag(
                crate::handlers::thread_drag::ThreadDragMessage::Cancel,
            ));
        }
        if self.move_menu.is_some() && window_id == self.main_window_id {
            if *key == Key::Named(Named::Escape) {
                self.move_menu = None;
            }
            return Task::none();
        }
        if self.thread_list.row_menu.is_some() && *key == Key::Named(Named::Escape) {
            self.thread_list.row_menu = None;
            return Task::none();
        }
        if *key == Key::Named(Named::Escape)
            && window_id == self.main_window_id
            && self.toolbar.escape()
        {
            return Task::none();
        }

        // 1. If palette is open, route to palette-specific handler
        if self.palette.is_open() {
            return self.handle_palette_key(key);
        }

        // 2. If settings is open, Escape closes the active sheet first if one is
        //    open, otherwise closes settings entirely. Filter inputs intercept
        //    Escape first so it clears the active filter without cascading up
        //    to close the sheet / settings.
        if self.show_settings {
            if *key == iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) {
                // Escape inside the discard-changes confirm dialog cancels
                // the deferred dismissal (returns the user to the editor)
                // instead of re-triggering the same close request.
                if self.settings.pending_discard.is_some() {
                    return self.update(Message::Settings(
                        crate::ui::settings::SettingsMessage::CancelDiscardEditorChanges,
                    ));
                }
                if let Some(filter_id) = self.settings.focused_filter {
                    return self.update(Message::Settings(
                        crate::ui::settings::SettingsMessage::FilterCleared(filter_id),
                    ));
                }
                let msg = if self.settings.active_sheet.is_some() {
                    crate::ui::settings::SettingsMessage::CloseSheet
                } else {
                    crate::ui::settings::SettingsMessage::Close
                };
                return self.update(Message::Settings(msg));
            }
            // Don't process other shortcuts while settings is open.
            return Task::none();
        }

        // 2a. If typeahead is visible, intercept arrow keys and Tab/Enter/Escape.
        if self.thread_list.typeahead.visible {
            match key {
                iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowUp) => {
                    return self.update(Message::ThreadList(ThreadListMessage::TypeaheadNavigate(
                        TypeaheadDirection::Up,
                    )));
                }
                iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown) => {
                    return self.update(Message::ThreadList(ThreadListMessage::TypeaheadNavigate(
                        TypeaheadDirection::Down,
                    )));
                }
                iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab) => {
                    // Tab accepts the selected typeahead item.
                    let idx = self.thread_list.typeahead.selected;
                    return self
                        .update(Message::ThreadList(ThreadListMessage::TypeaheadSelect(idx)));
                }
                iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
                    return self.update(Message::ThreadList(ThreadListMessage::TypeaheadDismiss));
                }
                _ => {}
            }
        }

        // 2b. The search suggestions under the title-bar field.
        if self.search_suggest.open {
            use crate::ui::search_suggest::SearchSuggestMessage;
            let message = match key {
                iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowUp) => {
                    Some(SearchSuggestMessage::Navigate(TypeaheadDirection::Up))
                }
                iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown) => {
                    Some(SearchSuggestMessage::Navigate(TypeaheadDirection::Down))
                }
                iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
                    Some(SearchSuggestMessage::Close)
                }
                _ => None,
            };
            if let Some(message) = message {
                return self.update(Message::SearchSuggest(message));
            }
        }

        // 2b. If a text input or other widget captured the event, skip
        //     (unless it's a modifier-chord like Ctrl+K)
        if status == iced::event::Status::Captured
            && (!command_dispatch::has_command_modifier(&modifiers)
                || is_text_editing_chord(key, modifiers))
        {
            return Task::none();
        }

        // The Mac key labelled Delete sends Backspace; forward delete sends
        // Delete. Both move the selection to the trash, and delete it for
        // good once it is already there. A text field that captured the key
        // (search, compose) never reaches this point.
        if is_plain_delete_key(key, modifiers)
            && window_id == self.main_window_id
            && self.app_mode == AppMode::Mail
        {
            let in_trash = command_dispatch::build_context(self).thread_in_trash == Some(true);
            let intent = delete_key_intent(in_trash);
            let command = match intent {
                MailActionIntent::PermanentDelete => CommandId::EmailPermanentDelete,
                _ => CommandId::EmailTrash,
            };
            if self.is_command_available(command) {
                return self.handle_email_action(intent);
            }
            return Task::none();
        }

        // 3. Convert iced key to cmdk Chord
        let Some(chord) = command_dispatch::iced_key_to_chord(key, &modifiers) else {
            return Task::none();
        };

        // 4. If we're in pending chord state, resolve the sequence
        if let Some(pending) = self.pending_chord.take() {
            log::debug!(
                "Resolving chord sequence: {:?} + {:?}",
                pending.first,
                chord
            );
            if let Some(id) = self.binding_table.resolve_sequence(&pending.first, &chord) {
                if self.is_command_available(id) {
                    return self.update(Message::ExecuteCommand(id));
                }
                return Task::none();
            }
            // Second chord didn't match any sequence - re-process as fresh first chord
            return self.try_resolve_single_chord(chord);
        }

        // 5. Resolve single chord
        self.try_resolve_single_chord(chord)
    }

    /// Check if a command is currently available given app context.
    pub(crate) fn is_command_available(&self, id: CommandId) -> bool {
        let ctx = command_dispatch::build_context(self);
        self.registry
            .get(id)
            .is_some_and(|desc| (desc.is_available)(&ctx))
    }

    /// When the palette is open, intercept Escape/ArrowUp/ArrowDown/Enter.
    fn handle_palette_key(&mut self, key: &iced::keyboard::Key) -> Task<Message> {
        use crate::ui::palette::PaletteMessage;
        let default_ctx = cmdk::CommandContext::default();
        match key {
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
                self.update(Message::Palette(PaletteMessage::Close(default_ctx)))
            }
            iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown) => {
                self.update(Message::Palette(PaletteMessage::SelectNext))
            }
            iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowUp) => {
                self.update(Message::Palette(PaletteMessage::SelectPrev))
            }
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter) => {
                self.update(Message::Palette(PaletteMessage::Confirm(default_ctx)))
            }
            _ => Task::none(),
        }
    }

    /// Try to resolve a single chord, checking availability before dispatch.
    fn try_resolve_single_chord(&mut self, chord: Chord) -> Task<Message> {
        match self.binding_table.resolve_chord(&chord) {
            ResolveResult::Command(id) => {
                if self.is_command_available(id) {
                    self.update(Message::ExecuteCommand(id))
                } else {
                    Task::none()
                }
            }
            ResolveResult::Pending => {
                self.pending_chord = Some(PendingChord { first: chord });
                Task::none()
            }
            ResolveResult::NoMatch => Task::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{delete_key_intent, is_text_editing_chord};
    use crate::action_resolve::MailActionIntent;

    #[test]
    fn the_delete_key_trashes_and_only_destroys_what_is_already_in_the_trash() {
        assert!(matches!(delete_key_intent(false), MailActionIntent::Trash));
        assert!(matches!(
            delete_key_intent(true),
            MailActionIntent::PermanentDelete
        ));
    }

    #[test]
    fn text_fields_keep_their_editing_chords() {
        use iced::keyboard::{Key, Modifiers};
        let character = |c: &str| Key::Character(c.into());

        assert!(is_text_editing_chord(&character("a"), Modifiers::COMMAND));
        assert!(is_text_editing_chord(
            &character("Z"),
            Modifiers::COMMAND | Modifiers::SHIFT
        ));
        assert!(!is_text_editing_chord(&character("a"), Modifiers::empty()));
        assert!(!is_text_editing_chord(&character("k"), Modifiers::COMMAND));
    }
}
