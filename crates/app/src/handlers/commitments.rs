//! Commitments: writing one down from a mail, and storing it.

use iced::Task;
use rust_i18n::t;

use crate::app::ReadyApp;
use crate::message::Message;
use crate::ui::commitment_dialog::{
    CommitmentDialog, CommitmentDialogMessage, CommitmentDraft, Direction, TEXT_INPUT_ID,
};
use crate::ui::commitment_list::{CLOSED_SHOWN_FOR, CommitmentList, CommitmentListMessage};
use rtsk::db::types::DbCommitment;

/// The request that stores `item` as it is, open or closed as `done` says.
fn save_params_of(item: &DbCommitment, done: bool) -> service_api::CommitmentSaveParams {
    service_api::CommitmentSaveParams {
        id: item.id.clone(),
        account_id: item.account_id.clone(),
        direction: item.direction.clone(),
        text: item.text.clone(),
        person_name: item.person_name.clone(),
        person_address: item.person_address.clone(),
        due_date: item.due_date.clone(),
        thread_id: item.thread_id.clone(),
        message_id: item.message_id.clone(),
        done,
    }
}

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
                if self.commitment_list.is_some() {
                    return self.load_commitments();
                }
            }
        }
        Task::none()
    }

    pub(crate) fn open_commitments(&mut self) -> Task<Message> {
        self.commitment_list = Some(CommitmentList::new(jiff::Zoned::now().date()));
        self.load_commitments()
    }

    fn load_commitments(&self) -> Task<Message> {
        let db = std::sync::Arc::clone(&self.db);
        let closed_since = jiff::Timestamp::now().as_second() - CLOSED_SHOWN_FOR;
        Task::perform(
            async move {
                db.with_read(move |conn| {
                    rtsk::db::queries_extra::list_commitments(conn, closed_since)
                })
                .await
            },
            |result| Message::CommitmentList(CommitmentListMessage::Loaded(result)),
        )
    }

    pub(crate) fn handle_commitment_list(&mut self, msg: CommitmentListMessage) -> Task<Message> {
        let Some(list) = self.commitment_list.as_mut() else {
            return Task::none();
        };
        match msg {
            CommitmentListMessage::Loaded(Ok(items)) => list.set_items(items),
            CommitmentListMessage::Loaded(Err(error))
            | CommitmentListMessage::Written(Err(error)) => list.error = Some(error),
            CommitmentListMessage::Written(Ok(())) => return self.load_commitments(),
            CommitmentListMessage::Close => self.commitment_list = None,
            CommitmentListMessage::SetDone(id, done) => {
                let Some(params) = list.find(&id).map(|item| save_params_of(item, done)) else {
                    return Task::none();
                };
                let Some(client) = self.service_client.clone() else {
                    list.error = Some(t!("main.commitment.service_not_connected").into_owned());
                    return Task::none();
                };
                return Task::perform(
                    async move {
                        client
                            .save_commitment(params)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    |result| Message::CommitmentList(CommitmentListMessage::Written(result)),
                );
            }
            CommitmentListMessage::Delete(id) => {
                let Some(client) = self.service_client.clone() else {
                    list.error = Some(t!("main.commitment.service_not_connected").into_owned());
                    return Task::none();
                };
                return Task::perform(
                    async move {
                        client
                            .delete_commitment(id)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    |result| Message::CommitmentList(CommitmentListMessage::Written(result)),
                );
            }
            CommitmentListMessage::Edit(id) => {
                let Some(item) = list.find(&id) else {
                    return Task::none();
                };
                let Some(direction) = Direction::from_stored(&item.direction) else {
                    return Task::none();
                };
                let draft = CommitmentDraft {
                    id: Some(item.id.clone()),
                    account_id: item.account_id.clone(),
                    direction,
                    text: item.text.clone(),
                    person_name: item.person_name.clone(),
                    person_address: item.person_address.clone(),
                    due_date: item.due_date.as_deref().and_then(|day| day.parse().ok()),
                    thread_id: item.thread_id.clone(),
                    message_id: item.message_id.clone(),
                    done: item.done_at.is_some(),
                };
                self.commitment_dialog =
                    Some(CommitmentDialog::new(draft, jiff::Zoned::now().date()));
                return iced::widget::operation::focus(TEXT_INPUT_ID);
            }
            CommitmentListMessage::OpenMail(id) => {
                let Some((account_id, thread_id)) = list
                    .find(&id)
                    .and_then(|item| Some((item.account_id.clone(), item.thread_id.clone()?)))
                else {
                    return Task::none();
                };
                self.commitment_list = None;
                return self.open_thread_of(account_id, thread_id);
            }
        }
        Task::none()
    }
}
