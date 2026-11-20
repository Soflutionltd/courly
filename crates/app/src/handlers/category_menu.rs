//! The ribbon's Categorize menu.

use iced::Task;
use rtsk::command_palette_queries::get_thread_label_groups_for_palette;
use rtsk::db::queries_extra::navigation::query_label_groups_for_settings;
use rust_i18n::t;
use types::LabelGroupId;

use crate::action_resolve::MailActionIntent;
use crate::app::ReadyApp;
use crate::message::Message;
use crate::ui::category_menu::{
    CategoryEntry, CategoryMenu, CategoryMenuMessage, selection_coverage,
};
use crate::ui::label_paint::LabelPaint;
use crate::ui::settings::{SettingsMessage, Tab};

/// Whether the account of every selected conversation has this category.
/// `memberships` pairs a category with an account that has it; `selected`
/// pairs each selected conversation's account with its id.
fn offered_to(group_id: i64, memberships: &[(i64, String)], selected: &[(String, String)]) -> bool {
    selected.iter().all(|(account_id, _)| {
        memberships
            .iter()
            .any(|(group, account)| *group == group_id && account == account_id)
    })
}

impl ReadyApp {
    /// Open the Categorize menu over the selected conversations.
    pub(crate) fn open_category_menu(&mut self) -> Task<Message> {
        let selected: Vec<(String, String)> = self
            .thread_list
            .selected_indices()
            .into_iter()
            .filter_map(|index| self.thread_list.threads.get(index))
            .map(|thread| (thread.account_id.clone(), thread.id.clone()))
            .collect();
        if selected.is_empty() {
            return Task::none();
        }
        let loaded = self.db.with_read_sync(|conn| {
            // Categories belong to a mailbox, as in Outlook: only the ones
            // every selected conversation's account has are offered.
            let mut statement = conn
                .prepare("SELECT DISTINCT group_id, account_id FROM label_group_members")
                .map_err(|e| format!("prepare category accounts: {e}"))?;
            let memberships = statement
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| format!("query category accounts: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("read category accounts: {e}"))?;
            let groups = query_label_groups_for_settings(conn)?
                .into_iter()
                .filter(|group| offered_to(group.id, &memberships, &selected))
                .collect::<Vec<_>>();
            let mut per_thread: Vec<Vec<i64>> = Vec::with_capacity(selected.len());
            for (account_id, thread_id) in &selected {
                let carried = get_thread_label_groups_for_palette(conn, account_id, thread_id)?;
                let mut ids = Vec::with_capacity(carried.len());
                for group in carried {
                    ids.push(group.id.parse::<i64>().map_err(|e| {
                        format!("label group id `{}` is not a number: {e}", group.id)
                    })?);
                }
                per_thread.push(ids);
            }
            Ok((groups, per_thread))
        });
        match loaded {
            Ok((groups, per_thread)) => {
                let (on_all, carried) = selection_coverage(&per_thread);
                let entries = groups
                    .into_iter()
                    .map(|group| CategoryEntry {
                        group_id: LabelGroupId::from(group.id),
                        applied: on_all.contains(&group.id),
                        paint: LabelPaint::from_hex_pair(&group.color_bg, &group.color_fg),
                        name: group.name,
                    })
                    .collect();
                let carried = carried.into_iter().map(LabelGroupId::from).collect();
                self.toolbar.overflow_open = false;
                self.move_menu = None;
                self.category_menu = Some(CategoryMenu::new(entries, carried));
            }
            Err(error) => {
                log::error!("Categorize menu: loading categories failed: {error}");
                self.status_bar.show_confirmation(
                    t!("main.category_menu.load_failed", error = error).into_owned(),
                );
            }
        }
        Task::none()
    }

    pub(crate) fn handle_category_menu(&mut self, message: CategoryMenuMessage) -> Task<Message> {
        let Some(menu) = self.category_menu.take() else {
            return Task::none();
        };
        match message {
            CategoryMenuMessage::Toggle(group_id) => {
                let intent = if menu.is_applied(group_id) {
                    MailActionIntent::RemoveLabelGroup { group_id }
                } else {
                    MailActionIntent::ApplyLabelGroup { group_id }
                };
                self.update(Message::EmailAction(intent))
            }
            CategoryMenuMessage::ClearAll => Task::batch(
                menu.carried()
                    .iter()
                    .map(|&group_id| {
                        self.update(Message::EmailAction(MailActionIntent::RemoveLabelGroup {
                            group_id,
                        }))
                    })
                    .collect::<Vec<_>>(),
            ),
            CategoryMenuMessage::New => {
                let open = self.open_settings(Tab::Labels);
                let editor =
                    self.update(Message::Settings(SettingsMessage::OpenLabelGroupEditor {
                        group_id: None,
                    }));
                Task::batch([open, editor])
            }
            CategoryMenuMessage::Manage => self.open_settings(Tab::Labels),
            CategoryMenuMessage::Close => Task::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::offered_to;

    fn selection(accounts: &[&str]) -> Vec<(String, String)> {
        accounts
            .iter()
            .map(|account| ((*account).to_owned(), "thread".to_owned()))
            .collect()
    }

    #[test]
    fn a_category_of_another_mailbox_is_not_offered() {
        let memberships = vec![(1, "work".to_owned()), (2, "home".to_owned())];
        assert!(offered_to(1, &memberships, &selection(&["work"])));
        assert!(!offered_to(2, &memberships, &selection(&["work"])));
    }

    #[test]
    fn a_selection_across_mailboxes_gets_the_categories_they_share() {
        let memberships = vec![
            (1, "work".to_owned()),
            (1, "home".to_owned()),
            (2, "home".to_owned()),
        ];
        let both = selection(&["work", "home"]);
        assert!(offered_to(1, &memberships, &both));
        assert!(!offered_to(2, &memberships, &both));
    }
}
