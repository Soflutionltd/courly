use iced::Element;
use rust_i18n::t;

use crate::ui::dialog::{DialogAction, alert_dialog};

use super::messages::CalendarMessage;

/// Confirmation dialog before deleting an event.
pub(super) fn delete_confirm_card(title: &str) -> Element<'_, CalendarMessage> {
    let display_title = if title.is_empty() {
        t!("calendar.dialog.untitled")
    } else {
        title.into()
    };
    alert_dialog(
        t!("calendar.dialog.delete_title"),
        t!("calendar.dialog.delete_body", title = display_title),
        vec![
            DialogAction::default_action(t!("calendar.editor.cancel"), CalendarMessage::CloseModal),
            DialogAction::destructive(t!("calendar.delete"), CalendarMessage::DeleteEvent),
        ],
        None,
    )
}

/// Confirmation dialog before discarding unsaved editor changes.
pub(super) fn discard_confirm_card(_title: &str) -> Element<'_, CalendarMessage> {
    alert_dialog(
        t!("calendar.dialog.discard_title"),
        t!("calendar.dialog.discard_body"),
        vec![
            DialogAction::default_action(
                t!("calendar.dialog.keep_editing"),
                CalendarMessage::CloseModal,
            ),
            DialogAction::destructive(
                t!("calendar.dialog.discard"),
                CalendarMessage::DiscardChanges,
            ),
        ],
        None,
    )
}
