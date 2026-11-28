//! Commitments: writing one down from a mail, and storing it.

use iced::Task;
use rust_i18n::t;

use crate::app::ReadyApp;
use crate::message::Message;
use crate::ui::commitment_dialog::{
    CommitmentDialog, CommitmentDialogMessage, CommitmentDraft, Direction, TEXT_INPUT_ID,
};

impl ReadyApp {
    /// Open the dialog on the sentence the reader selected in the message
    /// at `message_index` of the open conversation.
    pub(crate) fn note_commitment(
        &mut self,
        message_index: usize,
        text: &str,
        direction: Direction,
    ) -> Task<Message> {
        let Some(message) = self.reading_pane.thread_messages.get(message_index) else {
            return Task::none();
        };
        // The other person: whoever wrote the mail, or the first one it
        // was written to when the reader wrote it.
        let (person_name, person_address) = if message.is_own_message {
            let first = crate::ui::invite_card::display_names(message.to_addresses.as_deref())
                .into_iter()
                .next();
            (first, None)
        } else {
            (
                message
                    .from_name
                    .clone()
                    .filter(|name| !name.trim().is_empty()),
                message.from_address.clone(),
            )
        };
        let draft = CommitmentDraft {
            id: None,
            account_id: message.account_id.clone(),
            direction,
            // A selection runs over line ends; a commitment is one line.
            text: text.split_whitespace().collect::<Vec<_>>().join(" "),
            person_name,
            person_address,
            due_date: None,
            thread_id: Some(message.thread_id.clone()),
            message_id: Some(message.id.clone()),
            done: false,
        };
        self.commitment_dialog = Some(CommitmentDialog::new(draft, jiff::Zoned::now().date()));
        iced::widget::operation::focus(TEXT_INPUT_ID)
    }

    pub(crate) fn handle_commitment_dialog(
        &mut self,
        msg: CommitmentDialogMessage,
    ) -> Task<Message> {
        let Some(dialog) = self.commitment_dialog.as_mut() else {
            return Task::none();
        };
        match msg {
            CommitmentDialogMessage::TextChanged(text) => dialog.set_text(text),
            CommitmentDialogMessage::DirectionPicked(direction) => dialog.set_direction(direction),
            CommitmentDialogMessage::DuePicked(due) => dialog.set_due(due),
            CommitmentDialogMessage::DateTyped(typed) => dialog.type_date(typed),
            CommitmentDialogMessage::Cancel => {
                if !dialog.pending {
                    self.commitment_dialog = None;
                }
            }
            CommitmentDialogMessage::Submit => {
                if !dialog.can_submit() {
                    return Task::none();
                }
                let Some(client) = self.service_client.clone() else {
                    dialog.error = Some(t!("main.commitment.service_not_connected").into_owned());
                    return Task::none();
                };
                dialog.pending = true;
                dialog.error = None;
                let params = dialog.save_params();
                return Task::perform(
                    async move {
                        client
                            .save_commitment(params)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    |result| Message::CommitmentDialog(CommitmentDialogMessage::Done(result)),
                );
            }
            CommitmentDialogMessage::Done(Err(error)) => {
                dialog.pending = false;
                dialog.error = Some(error);
            }
            CommitmentDialogMessage::Done(Ok(())) => {
                self.commitment_dialog = None;
                self.status_bar
                    .show_confirmation(t!("main.commitment.saved").into_owned());
            }
        }
        Task::none()
    }
}
