//! Dialogs behind the sidebar folder menu: name a new or renamed folder,
//! pick where a folder moves, confirm emptying or deleting one.

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Element, Length};
use rust_i18n::t;

use crate::ui::dialog::{DialogAction, alert_dialog, form_dialog};
use crate::ui::layout::{
    DROPDOWN_ITEM_HEIGHT, PAD_INPUT, PAD_NAV_ITEM, SPACE_MD, SPACE_SM, SPACE_XXS, TEXT_MD,
    TEXT_SM, TYPEAHEAD_MAX_HEIGHT,
};
use crate::ui::sidebar::MoveTarget;
use crate::ui::theme;

pub const NAME_INPUT_ID: &str = "folder-dialog-name";

/// What deleting a folder does on the account's provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteEffect {
    /// Microsoft 365: the folder goes to Deleted Items.
    DeletedItems,
    /// Gmail: only the label goes, the messages stay in All Mail.
    LabelOnly,
    /// JMAP: the server refuses a folder that still holds messages.
    MustBeEmpty,
    /// IMAP and anything unknown: the folder and its messages are gone.
    Permanent,
}

impl DeleteEffect {
    pub fn for_provider(provider: Option<&str>) -> Self {
        match provider {
            Some("graph") => Self::DeletedItems,
            Some("gmail_api") => Self::LabelOnly,
            Some("jmap") => Self::MustBeEmpty,
            _ => Self::Permanent,
        }
    }
}

#[derive(Debug, Clone)]
pub enum FolderDialogKind {
    Create {
        account_id: String,
        /// Parent folder (id, name); `None` creates at the top level.
        parent: Option<(String, String)>,
    },
    Rename {
        account_id: String,
        folder_id: String,
        old_name: String,
    },
    Move {
        account_id: String,
        folder_id: String,
        name: String,
        targets: Vec<MoveTarget>,
        picked: Option<usize>,
    },
    Empty {
        account_id: String,
        folder_id: String,
        name: String,
    },
    Delete {
        account_id: String,
        folder_id: String,
        name: String,
        effect: DeleteEffect,
    },
}

#[derive(Debug, Clone)]
pub struct FolderDialog {
    pub kind: FolderDialogKind,
    /// The name being typed, for Create and Rename.
    pub name: String,
    /// A service call is running; the dialog can't be submitted or closed.
    pub pending: bool,
    pub error: Option<String>,
}

/// What a successful folder write returned.
#[derive(Debug, Clone)]
pub enum FolderDone {
    /// Create and Rename: the folder's id, which a rename can change.
    Written(String),
    /// Empty: how many messages went to the trash.
    Moved(usize),
    Done,
}

#[derive(Debug, Clone)]
pub enum FolderDialogMessage {
    NameChanged(String),
    Pick(usize),
    Submit,
    Cancel,
    Done(Result<FolderDone, String>),
}

impl FolderDialog {
    pub fn new(kind: FolderDialogKind) -> Self {
        let name = match &kind {
            FolderDialogKind::Rename { old_name, .. } => old_name.clone(),
            _ => String::new(),
        };
        Self {
            kind,
            name,
            pending: false,
            error: None,
        }
    }

    /// Whether the dialog has a name field, which handles Enter itself.
    pub fn takes_text(&self) -> bool {
        matches!(
            self.kind,
            FolderDialogKind::Create { .. } | FolderDialogKind::Rename { .. }
        )
    }

    pub fn can_submit(&self) -> bool {
        if self.pending {
            return false;
        }
        let name = self.name.trim();
        match &self.kind {
            FolderDialogKind::Create { .. } => !name.is_empty(),
            FolderDialogKind::Rename { old_name, .. } => !name.is_empty() && name != old_name,
            FolderDialogKind::Move { picked, .. } => picked.is_some(),
            FolderDialogKind::Empty { .. } | FolderDialogKind::Delete { .. } => true,
        }
    }

    pub fn view(&self) -> Element<'_, FolderDialogMessage> {
        let cancel = DialogAction::default_action(
            t!("main.folder_dialog.cancel"),
            FolderDialogMessage::Cancel,
        )
        .disabled_when(self.pending);
        let submit = |label: std::borrow::Cow<'_, str>, destructive: bool| {
            let action = if destructive {
                DialogAction::destructive(label, FolderDialogMessage::Submit)
            } else {
                DialogAction::suggested(label, FolderDialogMessage::Submit)
            };
            action.disabled_when(!self.can_submit())
        };

        match &self.kind {
            FolderDialogKind::Create { parent, .. } => {
                let title = match parent {
                    Some((_, parent_name)) => {
                        t!("main.folder_dialog.create_in_title", parent = parent_name)
                    }
                    None => t!("main.folder_dialog.create_title"),
                };
                form_dialog(
                    title,
                    self.name_body(),
                    vec![cancel, submit(t!("main.folder_dialog.create"), false)],
                    None,
                )
            }
            FolderDialogKind::Rename { old_name, .. } => form_dialog(
                t!("main.folder_dialog.rename_title", name = old_name),
                self.name_body(),
                vec![cancel, submit(t!("main.folder_dialog.rename"), false)],
                None,
            ),
            FolderDialogKind::Move {
                name,
                targets,
                picked,
                ..
            } => form_dialog(
                t!("main.folder_dialog.move_title", name = name),
                self.with_error(move_list(targets, *picked)),
                vec![cancel, submit(t!("main.folder_dialog.move"), false)],
                None,
            ),
            FolderDialogKind::Empty { name, .. } => form_dialog(
                t!("main.folder_dialog.empty_title", name = name),
                self.with_error(body_text(t!("main.folder_dialog.empty_body"))),
                vec![cancel, submit(t!("main.folder_dialog.empty"), true)],
                None,
            ),
            FolderDialogKind::Delete { name, effect, .. } => {
                let body = match effect {
                    DeleteEffect::DeletedItems => t!("main.folder_dialog.delete_body_graph"),
                    DeleteEffect::LabelOnly => t!("main.folder_dialog.delete_body_gmail"),
                    DeleteEffect::MustBeEmpty => t!("main.folder_dialog.delete_body_jmap"),
                    DeleteEffect::Permanent => t!("main.folder_dialog.delete_body_permanent"),
                };
                if self.error.is_none() {
                    return alert_dialog(
                        t!("main.folder_dialog.delete_title", name = name),
                        body,
                        vec![cancel, submit(t!("main.folder_dialog.delete"), true)],
                        None,
                    );
                }
                form_dialog(
                    t!("main.folder_dialog.delete_title", name = name),
                    self.with_error(body_text(body)),
                    vec![cancel, submit(t!("main.folder_dialog.delete"), true)],
                    None,
                )
            }
        }
    }

    fn name_body(&self) -> Element<'_, FolderDialogMessage> {
        let mut input = text_input(t!("main.folder_dialog.name_placeholder"), &self.name)
            .id(NAME_INPUT_ID)
            .size(TEXT_MD)
            .padding(PAD_INPUT);
        if !self.pending {
            input = input
                .on_input(FolderDialogMessage::NameChanged)
                .on_submit(FolderDialogMessage::Submit);
        }
        self.with_error(input.into())
    }

    fn with_error<'a>(
        &'a self,
        body: Element<'a, FolderDialogMessage>,
    ) -> Element<'a, FolderDialogMessage> {
        let mut col = column![body].spacing(SPACE_SM);
        if let Some(error) = &self.error {
            col = col.push(
                text(t!("main.folder_dialog.failed", error = error))
                    .size(TEXT_SM)
                    .style(text::danger),
            );
        }
        col.into()
    }
}

fn body_text<'a>(body: std::borrow::Cow<'a, str>) -> Element<'a, FolderDialogMessage> {
    text(body).size(TEXT_MD).style(text::secondary).into()
}

/// The folders a folder can move under, indented by depth; the first
/// target is the top level.
fn move_list(targets: &[MoveTarget], picked: Option<usize>) -> Element<'_, FolderDialogMessage> {
    let mut list = column![].spacing(SPACE_XXS);
    for (index, target) in targets.iter().enumerate() {
        let label = if target.parent_id.is_none() {
            t!("main.folder_dialog.top_level")
        } else {
            std::borrow::Cow::Borrowed(target.name.as_str())
        };
        let indent = SPACE_MD * f32::from(target.depth);
        list = list.push(
            button(
                row![
                    Space::new().width(indent),
                    text(label).size(TEXT_MD).style(text::base),
                ]
                .align_y(iced::Alignment::Center),
            )
            .on_press(FolderDialogMessage::Pick(index))
            .padding(PAD_NAV_ITEM)
            .height(DROPDOWN_ITEM_HEIGHT)
            .style(
                theme::ButtonClass::Dropdown {
                    selected: picked == Some(index),
                }
                .style(),
            )
            .width(Length::Fill),
        );
    }
    container(scrollable(list).height(Length::Shrink))
        .height(Length::Fit.max(TYPEAHEAD_MAX_HEIGHT))
        .width(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename(name: &str) -> FolderDialog {
        let mut dialog = FolderDialog::new(FolderDialogKind::Rename {
            account_id: "acc".into(),
            folder_id: "f".into(),
            old_name: "Résa".into(),
        });
        dialog.name = name.into();
        dialog
    }

    #[test]
    fn rename_needs_a_new_non_blank_name() {
        assert!(!rename("Résa").can_submit());
        assert!(!rename("   ").can_submit());
        assert!(rename("Réservations").can_submit());
        let mut pending = rename("Réservations");
        pending.pending = true;
        assert!(!pending.can_submit());
    }

    #[test]
    fn move_needs_a_picked_target() {
        let mut dialog = FolderDialog::new(FolderDialogKind::Move {
            account_id: "acc".into(),
            folder_id: "f".into(),
            name: "Résa".into(),
            targets: Vec::new(),
            picked: None,
        });
        assert!(!dialog.can_submit());
        if let FolderDialogKind::Move { picked, .. } = &mut dialog.kind {
            *picked = Some(0);
        }
        assert!(dialog.can_submit());
    }

    #[test]
    fn unknown_providers_are_warned_as_permanent() {
        assert_eq!(DeleteEffect::for_provider(Some("graph")), DeleteEffect::DeletedItems);
        assert_eq!(DeleteEffect::for_provider(Some("imap")), DeleteEffect::Permanent);
        assert_eq!(DeleteEffect::for_provider(None), DeleteEffect::Permanent);
    }
}
