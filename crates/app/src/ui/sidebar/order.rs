//! User-arranged order of the sidebar folder section: system folders first by
//! default, drag-and-drop moves, separators. Saved per scope in the
//! `sidebar_layout` setting.

use std::collections::{HashMap, HashSet};

use rtsk::scope::ViewScope;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum SidebarEntry {
    Folder(String),
    Separator(u32),
}

/// Saved arrangements keyed by [`scope_key`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SidebarLayouts(HashMap<String, Vec<SidebarEntry>>);

impl SidebarLayouts {
    pub fn parse(json: Option<&str>) -> Self {
        let Some(json) = json else {
            return Self::default();
        };
        serde_json::from_str(json).unwrap_or_else(|e| {
            log::warn!("sidebar_layout: unreadable saved layout, using defaults: {e}");
            Self::default()
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("string-keyed map of plain enums serializes")
    }

    pub fn get(&self, scope: &str) -> Option<&[SidebarEntry]> {
        self.0.get(scope).map(Vec::as_slice)
    }

    pub fn set(&mut self, scope: String, entries: Vec<SidebarEntry>) {
        self.0.insert(scope, entries);
    }

    pub fn reset(&mut self, scope: &str) -> bool {
        self.0.remove(scope).is_some()
    }
}

pub fn scope_key(scope: &ViewScope) -> String {
    match scope {
        ViewScope::AllAccounts => "*".to_string(),
        ViewScope::Account(id) => id.clone(),
        ViewScope::SharedMailbox {
            account_id,
            mailbox_id,
        } => format!("shared:{account_id}:{mailbox_id}"),
        ViewScope::PublicFolder {
            account_id,
            folder_id,
        } => format!("public:{account_id}:{folder_id}"),
    }
}

/// Default order of the universal folders: the mailbox folders first, then
/// the virtual views. Unknown ids follow in their incoming order.
const UNIVERSAL_RANK: &[&str] = &[
    "INBOX", "SENT", "DRAFT", "TRASH", "SPAM", "archive", "STARRED", "SNOOZED", "all-mail",
];

/// Display names of the Exchange outbox. Graph syncs it as a plain folder
/// (no well-known role reaches the app), so it is recognised by name to sit
/// with the system folders by default.
const OUTBOX_NAMES: &[&str] = &[
    "outbox",
    "boîte d'envoi",
    "postausgang",
    "bandeja de salida",
    "posta in uscita",
    "caixa de saída",
    "postvak uit",
];

fn is_outbox_name(name: &str) -> bool {
    let name = name.trim().to_lowercase().replace('\u{2019}', "'");
    OUTBOX_NAMES.contains(&name.as_str())
}

/// Default arrangement: universal folders by [`UNIVERSAL_RANK`], the outbox
/// right after Inbox, then the account's own folders in provider order.
pub fn default_order(universal_ids: &[&str], account_roots: &[(&str, &str)]) -> Vec<SidebarEntry> {
    let mut universal: Vec<&str> = universal_ids.to_vec();
    universal.sort_by_key(|id| {
        UNIVERSAL_RANK
            .iter()
            .position(|known| known == id)
            .unwrap_or(UNIVERSAL_RANK.len())
    });
    let (outboxes, user_folders): (Vec<_>, Vec<_>) = account_roots
        .iter()
        .partition(|(_, name)| is_outbox_name(name));

    let mut order: Vec<SidebarEntry> = universal
        .iter()
        .map(|id| SidebarEntry::Folder((*id).to_string()))
        .collect();
    let after_inbox = usize::from(universal.first() == Some(&"INBOX"));
    for (offset, (id, _)) in outboxes.iter().enumerate() {
        order.insert(
            after_inbox + offset,
            SidebarEntry::Folder((*id).to_string()),
        );
    }
    order.extend(
        user_folders
            .iter()
            .map(|(id, _)| SidebarEntry::Folder((*id).to_string())),
    );
    order
}

/// Apply a saved arrangement to the folders that exist now: saved entries
/// for vanished folders drop out, new folders join at the end.
pub fn resolve(default: Vec<SidebarEntry>, saved: Option<&[SidebarEntry]>) -> Vec<SidebarEntry> {
    let Some(saved) = saved else {
        return default;
    };
    let existing: HashSet<&SidebarEntry> = default.iter().collect();
    let mut order: Vec<SidebarEntry> = saved
        .iter()
        .filter(|entry| matches!(entry, SidebarEntry::Separator(_)) || existing.contains(entry))
        .cloned()
        .collect();
    let placed: HashSet<SidebarEntry> = order.iter().cloned().collect();
    order.extend(default.into_iter().filter(|entry| !placed.contains(entry)));
    order
}

/// Move the entry at `from` into the slot of the entry at `to`.
pub fn move_entry(order: &mut Vec<SidebarEntry>, from: usize, to: usize) {
    if from >= order.len() || from == to {
        return;
    }
    let entry = order.remove(from);
    order.insert(to.min(order.len()), entry);
}

pub fn insert_separator_after(order: &mut Vec<SidebarEntry>, index: usize) {
    let next_id = order
        .iter()
        .filter_map(|entry| match entry {
            SidebarEntry::Separator(id) => Some(*id + 1),
            SidebarEntry::Folder(_) => None,
        })
        .max()
        .unwrap_or(0);
    let at = (index + 1).min(order.len());
    order.insert(at, SidebarEntry::Separator(next_id));
}

pub fn remove_separator(order: &mut Vec<SidebarEntry>, id: u32) {
    order.retain(|entry| entry != &SidebarEntry::Separator(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: &str) -> SidebarEntry {
        SidebarEntry::Folder(id.to_string())
    }

    #[test]
    fn default_puts_system_folders_first_with_outbox_after_inbox() {
        let order = default_order(
            &[
                "INBOX", "STARRED", "SNOOZED", "SENT", "DRAFT", "archive", "TRASH", "SPAM",
            ],
            &[
                ("g-hotel", "Hotel"),
                ("g-out", "Boîte d'envoi"),
                ("g-immo", "Immobilier"),
            ],
        );
        assert_eq!(
            order,
            vec![
                folder("INBOX"),
                folder("g-out"),
                folder("SENT"),
                folder("DRAFT"),
                folder("TRASH"),
                folder("SPAM"),
                folder("archive"),
                folder("STARRED"),
                folder("SNOOZED"),
                folder("g-hotel"),
                folder("g-immo"),
            ]
        );
    }

    #[test]
    fn outbox_name_ignores_case_and_curly_apostrophe() {
        assert!(is_outbox_name("Boîte d\u{2019}envoi"));
        assert!(is_outbox_name(" OUTBOX "));
        assert!(!is_outbox_name("Hotel"));
    }

    #[test]
    fn resolve_keeps_saved_order_drops_missing_and_appends_new() {
        let default = vec![folder("INBOX"), folder("SENT"), folder("new")];
        let saved = vec![
            folder("SENT"),
            SidebarEntry::Separator(0),
            folder("gone"),
            folder("INBOX"),
        ];
        assert_eq!(
            resolve(default, Some(&saved)),
            vec![
                folder("SENT"),
                SidebarEntry::Separator(0),
                folder("INBOX"),
                folder("new"),
            ]
        );
    }

    #[test]
    fn move_entry_takes_the_target_slot_both_ways() {
        let mut order = vec![folder("a"), folder("b"), folder("c")];
        move_entry(&mut order, 0, 2);
        assert_eq!(order, vec![folder("b"), folder("c"), folder("a")]);
        move_entry(&mut order, 2, 0);
        assert_eq!(order, vec![folder("a"), folder("b"), folder("c")]);
    }

    #[test]
    fn separators_get_fresh_ids_and_can_be_removed() {
        let mut order = vec![folder("a"), folder("b")];
        insert_separator_after(&mut order, 0);
        insert_separator_after(&mut order, 2);
        assert_eq!(
            order,
            vec![
                folder("a"),
                SidebarEntry::Separator(0),
                folder("b"),
                SidebarEntry::Separator(1),
            ]
        );
        remove_separator(&mut order, 0);
        assert_eq!(
            order,
            vec![folder("a"), folder("b"), SidebarEntry::Separator(1)]
        );
    }

    #[test]
    fn layouts_round_trip_and_survive_garbage() {
        let mut layouts = SidebarLayouts::default();
        layouts.set(
            "acc".to_string(),
            vec![folder("INBOX"), SidebarEntry::Separator(3)],
        );
        let parsed = SidebarLayouts::parse(Some(&layouts.to_json()));
        assert_eq!(parsed, layouts);
        assert_eq!(
            SidebarLayouts::parse(Some("not json")),
            SidebarLayouts::default()
        );
    }
}
