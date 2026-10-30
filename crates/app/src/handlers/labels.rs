//! Per-account raw labels (settings > Mail Rules > Labels).
//!
//! Read path lives here. Writes go through the `label.save` and
//! `label.delete` Service IPC (see `App::handle_save_label`).
//! Every per-account label is keyed on `(account_id, label_id)`; cross-
//! account labels are explicit user-created groups (`label_groups`), not
//! auto-collapse by normalised name. See `reference/glossary/folders-labels.md`.

use std::sync::Arc;

use iced::Task;

use crate::db::Db;

use super::LabelOp;

/// Load all raw provider labels grouped by account, sorted alphabetically.
/// The sidebar's LABELS section renders explicit `label_groups`, not the
/// per-account rows this settings view exposes.
pub fn load_visible_labels_async(db: &Arc<Db>) -> Task<LabelOp> {
    let db = Arc::clone(db);
    Task::perform(
        async move {
            let core_db = db.read_db_state();
            core_db
                .with_read(move |conn| {
                    rtsk::db::queries_extra::navigation::query_labels_by_account(conn)
                })
                .await
        },
        |result| {
            if let Ok(ref groups) = result {
                let total: usize = groups.iter().map(|g| g.labels.len()).sum();
                log::info!(
                    "Labels loaded: {total} labels across {} accounts",
                    groups.len()
                );
            }
            LabelOp::Loaded(result)
        },
    )
}

/// Load `(account_id, label_id)` members for one `label_groups` row.
/// Dispatched when the user opens the label-group editor sheet on an
/// existing group; populates `LabelGroupEditorState::members`.
pub fn load_label_group_members_async(db: &Arc<Db>, group_id: i64) -> Task<crate::Message> {
    let db = Arc::clone(db);
    Task::perform(
        async move {
            let core_db = db.read_db_state();
            core_db
                .with_read(move |conn| {
                    rtsk::db::queries_extra::navigation::query_label_group_members(conn, group_id)
                })
                .await
        },
        move |result| {
            crate::Message::Settings(
                crate::ui::settings::SettingsMessage::LabelGroupMembersLoaded(group_id, result),
            )
        },
    )
}

/// Load user-visible label groups for the Settings > Labels top section.
pub fn load_label_groups_async(db: &Arc<Db>) -> Task<LabelOp> {
    let db = Arc::clone(db);
    Task::perform(
        async move {
            let core_db = db.read_db_state();
            core_db
                .with_read(move |conn| {
                    rtsk::db::queries_extra::navigation::query_label_groups_for_settings(conn)
                })
                .await
        },
        |result| {
            if let Ok(ref groups) = result {
                log::info!("Label groups loaded: {} groups", groups.len());
            }
            LabelOp::GroupsLoaded(result)
        },
    )
}
