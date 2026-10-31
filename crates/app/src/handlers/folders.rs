//! Provider-folder operations from the sidebar folder menu.

use std::sync::Arc;

use iced::Task;
use rust_i18n::t;
use service_api::{
    FolderContentsParams, FolderCreateParams, FolderDeleteParams, FolderMoveParams,
    FolderRenameParams,
};
use types::{FolderId, SidebarSelection};

use crate::app::ReadyApp;
use crate::message::Message;
use crate::service_client::ServiceClient;
use crate::ui::folder_dialog::{
    DeleteEffect, FolderDialog, FolderDialogKind, FolderDialogMessage, FolderDone, NAME_INPUT_ID,
};
use crate::ui::sidebar::FolderCommand;

/// One service call a submitted dialog makes.
enum FolderRequest {
    Create(FolderCreateParams),
    Rename(FolderRenameParams),
    Move(FolderMoveParams),
    Empty(FolderContentsParams),
    Delete(FolderDeleteParams),
}

impl FolderRequest {
    fn from_dialog(dialog: &FolderDialog) -> Option<Self> {
        let name = dialog.name.trim().to_string();
        Some(match &dialog.kind {
            FolderDialogKind::Create { account_id, parent } => Self::Create(FolderCreateParams {
                account_id: account_id.clone(),
                name,
                parent_id: parent.as_ref().map(|(id, _)| id.clone()),
            }),
            FolderDialogKind::Rename {
                account_id,
                folder_id,
                ..
            } => Self::Rename(FolderRenameParams {
                account_id: account_id.clone(),
                folder_id: folder_id.clone(),
                name,
            }),
            FolderDialogKind::Move {
                account_id,
                folder_id,
                targets,
                picked,
                ..
            } => Self::Move(FolderMoveParams {
                account_id: account_id.clone(),
                folder_id: folder_id.clone(),
                parent_id: targets.get((*picked)?)?.parent_id.clone(),
            }),
            FolderDialogKind::Empty {
                account_id,
                folder_id,
                ..
            } => Self::Empty(FolderContentsParams {
                account_id: account_id.clone(),
                folder_id: folder_id.clone(),
            }),
            FolderDialogKind::Delete {
                account_id,
                folder_id,
                ..
            } => Self::Delete(FolderDeleteParams {
                account_id: account_id.clone(),
                folder_id: folder_id.clone(),
            }),
        })
    }

    async fn send(self, client: Arc<ServiceClient>) -> Result<FolderDone, String> {
        let done = match self {
            Self::Create(params) => client.create_folder(params).await.map(FolderDone::Written),
            Self::Rename(params) => client.rename_folder(params).await.map(FolderDone::Written),
            Self::Move(params) => client.move_folder(params).await.map(|()| FolderDone::Done),
            Self::Empty(params) => client.empty_folder(params).await.map(FolderDone::Moved),
            Self::Delete(params) => client
                .delete_folder(params)
                .await
                .map(|()| FolderDone::Done),
        };
        done.map_err(|e| e.to_string())
    }
}

impl ReadyApp {
    pub(crate) fn handle_folder_command(&mut self, command: FolderCommand) -> Task<Message> {
        let kind = match command {
            FolderCommand::MarkAllRead {
                account_ids,
                folder_id,
            } => return self.mark_folder_read(account_ids, folder_id),
            FolderCommand::Create { account_id, parent } => {
                FolderDialogKind::Create { account_id, parent }
            }
            FolderCommand::Rename {
                account_id,
                folder_id,
                name,
            } => FolderDialogKind::Rename {
                account_id,
                folder_id,
                old_name: name,
            },
            FolderCommand::Move {
                account_id,
                folder_id,
                name,
            } => FolderDialogKind::Move {
                targets: self.sidebar.move_targets(&account_id, &folder_id),
                account_id,
                folder_id,
                name,
                picked: None,
            },
            FolderCommand::Empty {
                account_id,
                folder_id,
                name,
            } => FolderDialogKind::Empty {
                account_id,
                folder_id,
                name,
            },
            FolderCommand::Delete {
                account_id,
                folder_id,
                name,
            } => {
                let provider = self
                    .sidebar
                    .accounts
                    .iter()
                    .find(|a| a.id == account_id)
                    .map(|a| a.provider.as_str());
                FolderDialogKind::Delete {
                    effect: DeleteEffect::for_provider(provider),
                    account_id,
                    folder_id,
                    name,
                }
            }
        };
        let dialog = FolderDialog::new(kind);
        let focus = dialog.takes_text();
        self.folder_dialog = Some(dialog);
        if focus {
            iced::widget::operation::focus(NAME_INPUT_ID)
        } else {
            Task::none()
        }
    }

    pub(crate) fn handle_folder_dialog(&mut self, msg: FolderDialogMessage) -> Task<Message> {
        let Some(dialog) = self.folder_dialog.as_mut() else {
            return Task::none();
        };
        match msg {
            FolderDialogMessage::NameChanged(name) => {
                if !dialog.pending {
                    dialog.name = name;
                }
            }
            FolderDialogMessage::Pick(index) => {
                if let FolderDialogKind::Move { picked, .. } = &mut dialog.kind
                    && !dialog.pending
                {
                    *picked = Some(index);
                }
            }
            FolderDialogMessage::Cancel => {
                if !dialog.pending {
                    self.folder_dialog = None;
                }
            }
            FolderDialogMessage::Submit => {
                if !dialog.can_submit() {
                    return Task::none();
                }
                let Some(request) = FolderRequest::from_dialog(dialog) else {
                    return Task::none();
                };
                let Some(client) = self.service_client.clone() else {
                    dialog.error =
                        Some(t!("main.folder_dialog.service_not_connected").into_owned());
                    return Task::none();
                };
                dialog.pending = true;
                dialog.error = None;
                return Task::perform(request.send(client), |result| {
                    Message::FolderDialog(FolderDialogMessage::Done(result))
                });
            }
            FolderDialogMessage::Done(Err(error)) => {
                dialog.pending = false;
                dialog.error = Some(error);
            }
            FolderDialogMessage::Done(Ok(done)) => {
                let Some(dialog) = self.folder_dialog.take() else {
                    return Task::none();
                };
                return self.finish_folder_write(&dialog.kind, done);
            }
        }
        Task::none()
    }

    /// Keep the selection on a renamed folder (whose id can change) and off
    /// a deleted one, then reload folders and threads.
    fn finish_folder_write(&mut self, kind: &FolderDialogKind, done: FolderDone) -> Task<Message> {
        let selected = |folder_id: &str| {
            matches!(
                &self.sidebar.selection,
                SidebarSelection::ProviderFolder(id) if id.as_str() == folder_id
            )
        };
        match (kind, done) {
            (FolderDialogKind::Rename { folder_id, .. }, FolderDone::Written(new_id))
                if selected(folder_id) =>
            {
                self.sidebar.selection = SidebarSelection::ProviderFolder(FolderId::from(new_id));
            }
            (FolderDialogKind::Delete { folder_id, .. }, _) if selected(folder_id) => {
                self.sidebar.selection = SidebarSelection::Inbox;
                self.reset_view_state();
            }
            (FolderDialogKind::Empty { name, .. }, FolderDone::Moved(count)) => {
                self.status_bar.show_confirmation(
                    t!("main.folder_dialog.emptied", count = count, name = name).into_owned(),
                );
            }
            _ => {}
        }
        self.load_navigation_and_threads()
    }

    fn mark_folder_read(&mut self, account_ids: Vec<String>, folder_id: String) -> Task<Message> {
        let Some(client) = self.service_client.clone() else {
            self.status_bar
                .show_confirmation(t!("main.folder_dialog.service_not_connected").into_owned());
            return Task::none();
        };
        Task::perform(
            async move {
                let mut marked = 0;
                let mut failures = Vec::new();
                for account_id in account_ids {
                    let params = FolderContentsParams {
                        account_id,
                        folder_id: folder_id.clone(),
                    };
                    match client.mark_folder_read(params).await {
                        Ok(count) => marked += count,
                        Err(e) => failures.push(e.to_string()),
                    }
                }
                if failures.is_empty() {
                    Ok(marked)
                } else {
                    Err(failures.join("\n"))
                }
            },
            Message::FolderMarkedRead,
        )
    }

    pub(crate) fn finish_folder_mark_read(
        &mut self,
        result: Result<usize, String>,
    ) -> Task<Message> {
        let toast = match result {
            Ok(count) => t!("main.folder_dialog.marked_read", count = count),
            Err(error) => t!("main.folder_dialog.mark_read_failed", error = error),
        };
        self.status_bar.show_confirmation(toast.into_owned());
        self.load_navigation_and_threads()
    }
}
