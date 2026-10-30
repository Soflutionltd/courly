use std::borrow::Cow;
use std::collections::HashSet;

use iced::widget::{Space, column, container, row, scrollable};
use iced::{Alignment, Element, Length, Task};
use rust_i18n::t;

use crate::component::Component;
use crate::db::{Account, PinnedPublicFolder, PinnedSearch, SharedMailbox};
use crate::ui::layout::*;
use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::{FolderKind, NavigationFolder, NavigationState};
use rtsk::scope::ViewScope;
use types::{FolderId, SidebarSelection};

mod chats;
mod folders;
mod labels;
mod nav;
pub(crate) mod order;
mod pinned_searches;
mod public_folders;
mod scope;
mod search_here;
mod smart;

use chats::chats_section;
use labels::labels_section;
use nav::nav_items;
use order::{SidebarEntry, SidebarLayouts};
pub(crate) use pinned_searches::format_relative_time;
use public_folders::pinned_public_folders_section;
use scope::scope_dropdown;
use smart::smart_folders;

// ── Messages & Events ──────────────────────────────────

#[derive(Debug, Clone)]
pub enum SidebarMessage {
    SelectAccount(usize),
    SelectAllAccounts,
    CycleAccount,
    Select(SidebarSelection),
    ToggleScopeDropdown,
    ToggleLabelsSection,
    ToggleSmartFoldersSection,
    ToggleFolderExpand(String),
    Compose,
    AddAccount,
    ToggleSettings,
    SelectPinnedSearch(i64),
    DismissPinnedSearch(i64),
    RefreshPinnedSearch(i64),
    ToggleChatsSection,
    SelectChat(String),
    ToggleMode,
    /// Right-click "Search here" on a label/folder.
    SearchHere(String),
    /// Click a smart folder - execute its query via the unified search pipeline.
    SelectSmartFolder {
        id: String,
        query: String,
    },
    /// Select a shared/delegated mailbox in the scope dropdown.
    /// Carries (account_id, mailbox_id).
    SelectSharedMailbox(String, String),
    /// Select a pinned public folder.
    /// Carries (account_id, folder_id).
    SelectPublicFolder(String, String),
    /// Left press on a folder-section row: a click or the start of a drag.
    RowPress(usize),
    RowRelease(usize),
    RowHover(usize),
    /// Left release anywhere in the folder section: drops a moved row.
    DragEnd,
    DragCancel,
    OpenRowMenu(usize),
    CloseRowMenu,
    AddSeparatorBelow(usize),
    RemoveSeparator(u32),
    ResetOrder,
}

/// A press on a folder-section row, tracked until release.
#[derive(Debug, Clone)]
pub struct SidebarDrag {
    pub from: SidebarEntry,
    pub target: Option<SidebarEntry>,
    /// The cursor reached another row, so release drops instead of clicking.
    pub moved: bool,
}

/// Events the sidebar emits upward to the App.
#[derive(Debug, Clone)]
#[allow(dead_code)] // shared-mailbox / public-folder routing is wired one variant at a time
pub enum SidebarEvent {
    AccountSelected(usize),
    AllAccountsSelected,
    SelectionChanged(SidebarSelection),
    Compose,
    AddAccount,
    ToggleSettings,
    PinnedSearchSelected(i64),
    PinnedSearchDismissed(i64),
    PinnedSearchRefreshed(i64),
    ChatSelected(String),
    ModeToggled,
    /// "Search here" - prefill search bar with a scope query prefix.
    SearchHere {
        query_prefix: String,
    },
    /// Smart folder selected - execute its query via the unified search pipeline.
    SmartFolderSelected {
        id: String,
        query: String,
    },
    /// Shared mailbox selected in scope dropdown.
    /// Carries (account_id, mailbox_id).
    SharedMailboxSelected {
        account_id: String,
        mailbox_id: String,
    },
    /// Pinned public folder selected.
    /// Carries (account_id, folder_id).
    PublicFolderSelected {
        account_id: String,
        folder_id: String,
    },
    /// The folder arrangement changed; carries the `sidebar_layout` JSON.
    LayoutChanged(String),
}

// ── State ──────────────────────────────────────────────

pub struct Sidebar {
    pub accounts: Vec<Account>,
    pub nav_state: Option<NavigationState>,
    pub selected_scope: ViewScope,
    pub selection: SidebarSelection,
    pub scope_dropdown_open: bool,
    pub labels_expanded: bool,
    pub smart_folders_expanded: bool,
    /// Set of folder IDs whose children are collapsed (hidden).
    pub collapsed_folders: HashSet<String>,
    /// Pinned searches, set by parent App before each view.
    pub pinned_searches: Vec<PinnedSearch>,
    /// Currently selected pinned search, set by parent App.
    pub active_pinned_search: Option<i64>,
    /// Shared/delegated mailboxes discovered via Autodiscover.
    pub shared_mailboxes: Vec<SharedMailbox>,
    /// Pinned public folders.
    pub pinned_public_folders: Vec<PinnedPublicFolder>,
    /// Designated chat contacts, set by parent App after each load.
    pub chat_contacts: Vec<rtsk::chat::ChatContactSummary>,
    /// Email of the chat contact whose timeline is currently active, if any.
    /// Pushed by App::view before each Sidebar::view call.
    pub active_chat: Option<String>,
    /// Whether the CHATS section is expanded.
    pub chats_expanded: bool,
    /// Saved folder arrangements per scope.
    pub layouts: SidebarLayouts,
    pub drag: Option<SidebarDrag>,
    /// Folder-section row whose right-click menu is open.
    pub row_menu: Option<usize>,
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            accounts: Vec::new(),
            nav_state: None,
            selected_scope: ViewScope::AllAccounts,
            selection: SidebarSelection::Inbox,
            scope_dropdown_open: false,
            labels_expanded: true,
            smart_folders_expanded: true,
            collapsed_folders: HashSet::new(),
            pinned_searches: Vec::new(),
            active_pinned_search: None,
            shared_mailboxes: Vec::new(),
            pinned_public_folders: Vec::new(),
            chat_contacts: Vec::new(),
            active_chat: None,
            chats_expanded: true,
            layouts: SidebarLayouts::default(),
            drag: None,
            row_menu: None,
        }
    }

    fn entry_at(&self, index: usize) -> Option<SidebarEntry> {
        nav::current_order(self).into_iter().nth(index)
    }

    fn save_order(
        &mut self,
        order: Vec<SidebarEntry>,
    ) -> (Task<SidebarMessage>, Option<SidebarEvent>) {
        self.row_menu = None;
        self.layouts
            .set(order::scope_key(&self.selected_scope), order);
        (
            Task::none(),
            Some(SidebarEvent::LayoutChanged(self.layouts.to_json())),
        )
    }

    pub fn is_all_accounts(&self) -> bool {
        matches!(self.selected_scope, ViewScope::AllAccounts)
    }

    /// Return the selected account index, if scope is a single account.
    pub fn selected_account_index(&self) -> Option<usize> {
        if let ViewScope::Account(ref id) = self.selected_scope {
            self.accounts.iter().position(|a| a.id == *id)
        } else {
            None
        }
    }
}

// ── Component impl ─────────────────────────────────────

impl Component for Sidebar {
    type Message = SidebarMessage;
    type Event = SidebarEvent;

    fn update(&mut self, message: SidebarMessage) -> (Task<SidebarMessage>, Option<SidebarEvent>) {
        match message {
            SidebarMessage::SelectAccount(idx) => {
                let account_id = self
                    .accounts
                    .get(idx)
                    .map(|a| a.id.clone())
                    .unwrap_or_default();
                self.selected_scope = ViewScope::Account(account_id);
                self.selection = SidebarSelection::Inbox;
                self.scope_dropdown_open = false;
                (Task::none(), Some(SidebarEvent::AccountSelected(idx)))
            }
            SidebarMessage::SelectAllAccounts => {
                self.selected_scope = ViewScope::AllAccounts;
                self.selection = SidebarSelection::Inbox;
                self.scope_dropdown_open = false;
                (Task::none(), Some(SidebarEvent::AllAccountsSelected))
            }
            SidebarMessage::CycleAccount => {
                if self.accounts.len() > 1 {
                    let current_idx = self.selected_account_index();
                    let next = match current_idx {
                        Some(idx) => (idx + 1) % self.accounts.len(),
                        None => 0,
                    };
                    let account_id = self.accounts[next].id.clone();
                    self.selected_scope = ViewScope::Account(account_id);
                    self.selection = SidebarSelection::Inbox;
                    self.scope_dropdown_open = false;
                    (Task::none(), Some(SidebarEvent::AccountSelected(next)))
                } else {
                    (Task::none(), None)
                }
            }
            SidebarMessage::Select(sel) => {
                self.row_menu = None;
                self.selection = sel.clone();
                (Task::none(), Some(SidebarEvent::SelectionChanged(sel)))
            }
            SidebarMessage::ToggleScopeDropdown => {
                self.scope_dropdown_open = !self.scope_dropdown_open;
                (Task::none(), None)
            }
            SidebarMessage::ToggleLabelsSection => {
                self.labels_expanded = !self.labels_expanded;
                (Task::none(), None)
            }
            SidebarMessage::ToggleSmartFoldersSection => {
                self.smart_folders_expanded = !self.smart_folders_expanded;
                (Task::none(), None)
            }
            SidebarMessage::ToggleFolderExpand(folder_id) => {
                if !self.collapsed_folders.remove(&folder_id) {
                    self.collapsed_folders.insert(folder_id);
                }
                (Task::none(), None)
            }
            SidebarMessage::Compose => (Task::none(), Some(SidebarEvent::Compose)),
            SidebarMessage::AddAccount => (Task::none(), Some(SidebarEvent::AddAccount)),
            SidebarMessage::ToggleSettings => (Task::none(), Some(SidebarEvent::ToggleSettings)),
            SidebarMessage::SelectPinnedSearch(id) => {
                (Task::none(), Some(SidebarEvent::PinnedSearchSelected(id)))
            }
            SidebarMessage::DismissPinnedSearch(id) => {
                (Task::none(), Some(SidebarEvent::PinnedSearchDismissed(id)))
            }
            SidebarMessage::RefreshPinnedSearch(id) => {
                (Task::none(), Some(SidebarEvent::PinnedSearchRefreshed(id)))
            }
            SidebarMessage::ToggleChatsSection => {
                self.chats_expanded = !self.chats_expanded;
                (Task::none(), None)
            }
            SidebarMessage::SelectChat(email) => {
                (Task::none(), Some(SidebarEvent::ChatSelected(email)))
            }
            SidebarMessage::ToggleMode => (Task::none(), Some(SidebarEvent::ModeToggled)),
            SidebarMessage::SearchHere(query_prefix) => {
                self.row_menu = None;
                (
                    Task::none(),
                    Some(SidebarEvent::SearchHere { query_prefix }),
                )
            }
            SidebarMessage::RowPress(index) => {
                self.row_menu = None;
                self.drag = self.entry_at(index).map(|from| SidebarDrag {
                    from,
                    target: None,
                    moved: false,
                });
                (Task::none(), None)
            }
            SidebarMessage::RowHover(index) => {
                let entry = self.drag.as_ref().and_then(|_| self.entry_at(index));
                if let (Some(drag), Some(entry)) = (self.drag.as_mut(), entry) {
                    drag.moved |= entry != drag.from;
                    if drag.moved {
                        drag.target = Some(entry);
                    }
                }
                (Task::none(), None)
            }
            SidebarMessage::RowRelease(index) => {
                let clicked = self.drag.as_ref().is_some_and(|drag| {
                    !drag.moved && Some(&drag.from) == self.entry_at(index).as_ref()
                });
                if !clicked {
                    return (Task::none(), None);
                }
                self.drag = None;
                match self.entry_at(index) {
                    Some(SidebarEntry::Folder(id)) => {
                        self.update(SidebarMessage::Select(universal_folder_selection(&id)))
                    }
                    _ => (Task::none(), None),
                }
            }
            SidebarMessage::DragEnd => {
                let Some(drag) = self.drag.take() else {
                    return (Task::none(), None);
                };
                let Some(target) = drag.target.filter(|_| drag.moved) else {
                    return (Task::none(), None);
                };
                let mut order = nav::current_order(self);
                let from = order.iter().position(|e| *e == drag.from);
                let to = order.iter().position(|e| *e == target);
                match (from, to) {
                    (Some(from), Some(to)) if from != to => {
                        order::move_entry(&mut order, from, to);
                        self.save_order(order)
                    }
                    _ => (Task::none(), None),
                }
            }
            SidebarMessage::DragCancel => {
                self.drag = None;
                (Task::none(), None)
            }
            SidebarMessage::OpenRowMenu(index) => {
                self.drag = None;
                self.row_menu = Some(index);
                (Task::none(), None)
            }
            SidebarMessage::CloseRowMenu => {
                self.row_menu = None;
                (Task::none(), None)
            }
            SidebarMessage::AddSeparatorBelow(index) => {
                let mut order = nav::current_order(self);
                order::insert_separator_after(&mut order, index);
                self.save_order(order)
            }
            SidebarMessage::RemoveSeparator(id) => {
                let mut order = nav::current_order(self);
                order::remove_separator(&mut order, id);
                self.save_order(order)
            }
            SidebarMessage::ResetOrder => {
                self.row_menu = None;
                if !self.layouts.reset(&order::scope_key(&self.selected_scope)) {
                    return (Task::none(), None);
                }
                (
                    Task::none(),
                    Some(SidebarEvent::LayoutChanged(self.layouts.to_json())),
                )
            }
            SidebarMessage::SelectSmartFolder { id, query } => {
                self.selection = SidebarSelection::SmartFolder { id: id.clone() };
                (
                    Task::none(),
                    Some(SidebarEvent::SmartFolderSelected { id, query }),
                )
            }
            SidebarMessage::SelectSharedMailbox(account_id, mailbox_id) => {
                self.selected_scope = ViewScope::SharedMailbox {
                    account_id: account_id.clone(),
                    mailbox_id: mailbox_id.clone(),
                };
                self.selection = SidebarSelection::Inbox;
                self.scope_dropdown_open = false;
                (
                    Task::none(),
                    Some(SidebarEvent::SharedMailboxSelected {
                        account_id,
                        mailbox_id,
                    }),
                )
            }
            SidebarMessage::SelectPublicFolder(account_id, folder_id) => {
                self.selected_scope = ViewScope::PublicFolder {
                    account_id: account_id.clone(),
                    folder_id: folder_id.clone(),
                };
                self.selection = SidebarSelection::Inbox;
                self.scope_dropdown_open = false;
                (
                    Task::none(),
                    Some(SidebarEvent::PublicFolderSelected {
                        account_id,
                        folder_id,
                    }),
                )
            }
        }
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn view(&self) -> Element<'_, SidebarMessage> {
        let header_row = row![
            container(scope_dropdown(self)).width(Length::Fill),
            widgets::add_button(SidebarMessage::AddAccount),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center);

        let mut scroll_content = column![Space::new().height(SPACE_XXS)]
            .spacing(0)
            .width(Length::Fill);

        // Chats (above universal folders, only if non-empty, scope-independent)
        if !self.chat_contacts.is_empty() {
            scroll_content = scroll_content.push(chats_section(self));
            scroll_content = scroll_content.push(Space::new().height(SPACE_XXS));
        }

        scroll_content = scroll_content.push(Space::new().height(SPACE_XS));
        scroll_content = scroll_content.push(nav_items(self));
        scroll_content = scroll_content.push(widgets::section_break());
        scroll_content = scroll_content.push(smart_folders(self));

        let has_label_groups = self.nav_state.as_ref().is_some_and(|ns| {
            ns.folders
                .iter()
                .any(|f| matches!(f.folder_kind, FolderKind::LabelGroup))
        });
        if has_label_groups {
            scroll_content = scroll_content.push(widgets::section_break::<SidebarMessage>());
            scroll_content = scroll_content.push(labels_section(self));
        }

        // Pinned public folders (if any)
        if !self.pinned_public_folders.is_empty() {
            scroll_content = scroll_content.push(widgets::section_break::<SidebarMessage>());
            scroll_content = scroll_content.push(pinned_public_folders_section(self));
        }

        container(
            column![
                header_row,
                scrollable(scroll_content)
                    .spacing(SCROLLBAR_SPACING)
                    .height(Length::Fill),
            ]
            .spacing(SPACE_XS),
        )
        .padding(PAD_SIDEBAR)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

/// Truncates a query string for display, adding ellipsis if needed.
pub fn truncate_query(query: &str, max_chars: usize) -> String {
    if query.len() <= max_chars {
        query.to_string()
    } else {
        format!("{}...", &query[..query.floor_char_boundary(max_chars)])
    }
}

/// Name shown for a sidebar folder. Universal folders carry the app's own
/// English names, so they follow the interface language; provider folder
/// names are shown as the server sends them.
pub(crate) fn folder_display_name(folder: &NavigationFolder) -> Cow<'_, str> {
    if !matches!(folder.folder_kind, FolderKind::Universal) {
        return Cow::Borrowed(&folder.name);
    }
    match folder.id.as_str() {
        "INBOX" => t!("main.folder.inbox"),
        "STARRED" => t!("main.folder.starred"),
        "SNOOZED" => t!("main.folder.snoozed"),
        "SENT" => t!("main.folder.sent"),
        "DRAFT" => t!("main.folder.drafts"),
        "archive" => t!("main.folder.archive"),
        "TRASH" => t!("main.folder.trash"),
        "SPAM" => t!("main.folder.spam"),
        "all-mail" => t!("main.folder.all_mail"),
        _ => Cow::Borrowed(&folder.name),
    }
}

/// Map a universal folder's DB ID to the corresponding `SidebarSelection`.
pub(crate) fn universal_folder_selection(id: &str) -> SidebarSelection {
    use types::{Bundle, FeatureView, SystemFolder, VirtualView};
    match id {
        "INBOX" => SidebarSelection::Inbox,
        "STARRED" => SidebarSelection::VirtualView(VirtualView::Starred),
        "SENT" => SidebarSelection::Folder(SystemFolder::Sent),
        "DRAFT" => SidebarSelection::Folder(SystemFolder::Draft),
        "SNOOZED" => SidebarSelection::VirtualView(VirtualView::Snoozed),
        "TRASH" => SidebarSelection::Folder(SystemFolder::Trash),
        "SPAM" => SidebarSelection::Folder(SystemFolder::Spam),
        "all-mail" => SidebarSelection::VirtualView(VirtualView::AllMail),
        "BUNDLE_PRIMARY" => SidebarSelection::Bundle(Bundle::Primary),
        "BUNDLE_UPDATES" => SidebarSelection::Bundle(Bundle::Updates),
        "BUNDLE_PROMOTIONS" => SidebarSelection::Bundle(Bundle::Promotions),
        "BUNDLE_SOCIAL" => SidebarSelection::Bundle(Bundle::Social),
        "BUNDLE_NEWSLETTERS" => SidebarSelection::Bundle(Bundle::Newsletters),
        "TASKS" => SidebarSelection::FeatureView(FeatureView::Tasks),
        "ATTACHMENTS" => SidebarSelection::FeatureView(FeatureView::Attachments),
        other => SidebarSelection::ProviderFolder(FolderId::from(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::order::SidebarEntry;
    use super::*;
    use rtsk::db::queries_extra::navigation::NavigationFolder;
    use rtsk::db::types::AccountScope;

    fn folder(id: &str, name: &str, kind: &str) -> NavigationFolder {
        let unread = if kind == "Universal" {
            serde_json::json!({ "universal": 0 })
        } else {
            serde_json::json!({ "general": 0 })
        };
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "folderKind": kind,
            "unreadCount": unread,
            "accountId": null,
            "parentId": null,
            "query": null,
        }))
        .expect("valid navigation folder")
    }

    fn sidebar() -> Sidebar {
        let mut sidebar = Sidebar::new();
        sidebar.selected_scope = ViewScope::Account("acc".to_string());
        sidebar.nav_state = Some(NavigationState {
            scope: AccountScope::Single("acc".to_string()),
            folders: vec![
                folder("INBOX", "Inbox", "Universal"),
                folder("SENT", "Sent", "Universal"),
                folder("TRASH", "Trash", "Universal"),
                folder("g-hotel", "Hotel", "AccountFolder"),
            ],
        });
        sidebar
    }

    fn ids(sidebar: &Sidebar) -> Vec<String> {
        nav::current_order(sidebar)
            .into_iter()
            .map(|entry| match entry {
                SidebarEntry::Folder(id) => id,
                SidebarEntry::Separator(id) => format!("sep{id}"),
            })
            .collect()
    }

    #[test]
    fn press_and_release_on_one_row_selects_it() {
        let mut sidebar = sidebar();
        let _ = sidebar.update(SidebarMessage::RowPress(3));
        let (_, event) = sidebar.update(SidebarMessage::RowRelease(3));
        assert!(matches!(
            event,
            Some(SidebarEvent::SelectionChanged(SidebarSelection::ProviderFolder(ref id)))
                if id.as_str() == "g-hotel"
        ));
        assert!(sidebar.drag.is_none());
    }

    #[test]
    fn dragging_trash_onto_inbox_moves_it_first_and_saves() {
        let mut sidebar = sidebar();
        assert_eq!(ids(&sidebar), ["INBOX", "SENT", "TRASH", "g-hotel"]);
        let _ = sidebar.update(SidebarMessage::RowPress(2));
        let _ = sidebar.update(SidebarMessage::RowHover(1));
        let _ = sidebar.update(SidebarMessage::RowHover(0));
        let (_, release) = sidebar.update(SidebarMessage::RowRelease(0));
        assert!(release.is_none(), "a drag release must not select");
        let (_, event) = sidebar.update(SidebarMessage::DragEnd);
        assert_eq!(ids(&sidebar), ["TRASH", "INBOX", "SENT", "g-hotel"]);
        let Some(SidebarEvent::LayoutChanged(json)) = event else {
            panic!("expected LayoutChanged");
        };
        assert_eq!(order::SidebarLayouts::parse(Some(&json)), sidebar.layouts);
    }

    #[test]
    fn separators_and_reset_round_trip() {
        let mut sidebar = sidebar();
        let _ = sidebar.update(SidebarMessage::AddSeparatorBelow(2));
        assert_eq!(ids(&sidebar), ["INBOX", "SENT", "TRASH", "sep0", "g-hotel"]);
        let _ = sidebar.update(SidebarMessage::RemoveSeparator(0));
        assert_eq!(ids(&sidebar), ["INBOX", "SENT", "TRASH", "g-hotel"]);
        let (_, event) = sidebar.update(SidebarMessage::ResetOrder);
        assert!(matches!(event, Some(SidebarEvent::LayoutChanged(_))));
        assert_eq!(sidebar.layouts.get("acc"), None);
    }
}
