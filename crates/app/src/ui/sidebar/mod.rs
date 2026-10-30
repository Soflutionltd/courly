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
mod tree;

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
    /// Click on a folder row of the section for this scope.
    SelectIn(ViewScope, SidebarSelection),
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
    RowPress(ViewScope, usize),
    RowRelease(ViewScope, usize),
    RowHover(ViewScope, usize),
    /// Left release anywhere in the folder section: drops a moved row.
    DragEnd,
    DragCancel,
    OpenRowMenu(ViewScope, usize),
    /// Right-click on a subfolder row.
    OpenChildMenu {
        scope: ViewScope,
        folder_id: String,
        root_index: usize,
    },
    /// Right-click anywhere else in the sidebar, at this window position.
    OpenBackgroundMenu(iced::Point),
    CloseRowMenu,
    AddSeparatorBelow(ViewScope, usize),
    RemoveSeparator(ViewScope, u32),
    ResetOrder(ViewScope),
    /// Fold or unfold a section of the all-accounts tree, by scope key.
    ToggleSection(String),
    /// Left press on an account header: a fold toggle or the start of a
    /// drag that reorders the accounts.
    SectionPress(String),
    SectionRelease(String),
    SectionHover(String),
    SectionDragEnd,
    SectionDragCancel,
}

/// A press on a folder-section row, tracked until release.
#[derive(Debug, Clone)]
pub struct SidebarDrag {
    pub scope: ViewScope,
    pub from: SidebarEntry,
    pub target: Option<SidebarEntry>,
    /// The cursor reached another row, so release drops instead of clicking.
    pub moved: bool,
}

/// A press on an account header, tracked until release.
#[derive(Debug, Clone)]
pub struct SectionDrag {
    pub from: String,
    pub target: Option<String>,
    pub moved: bool,
}

/// Folders for the all-accounts tree: the unified section, then one
/// section per account, keyed by account id.
#[derive(Debug, Clone)]
pub struct NavigationTree {
    pub all: NavigationState,
    pub accounts: Vec<(String, NavigationState)>,
}

/// Whether the sidebar lays its folders out like Outlook: a unified
/// section, then one collapsible section per account.
pub fn shows_tree(scope: &ViewScope, account_count: usize) -> bool {
    account_count >= 2 && matches!(scope, ViewScope::AllAccounts | ViewScope::Account(_))
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
    /// Account sections were dragged into a new order: `(id, sort_order)`.
    AccountsReordered(Vec<(String, i64)>),
}

// ── State ──────────────────────────────────────────────

pub struct Sidebar {
    pub accounts: Vec<Account>,
    pub nav_state: Option<NavigationState>,
    pub tree: Option<NavigationTree>,
    /// Folded tree sections, by scope key.
    pub collapsed_sections: HashSet<String>,
    pub section_drag: Option<SectionDrag>,
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
    pub row_menu: Option<RowMenuAnchor>,
}

/// Where the folder-section right-click menu is open.
#[derive(Debug, Clone, PartialEq)]
pub enum RowMenuAnchor {
    /// A top-level row, by its index in its section's folder order.
    Entry(ViewScope, usize),
    /// A subfolder row; separators go below its top-level ancestor, since
    /// only top-level rows can be reordered.
    Child {
        scope: ViewScope,
        folder_id: String,
        root_index: usize,
    },
    /// Empty space or a non-folder row, at the clicked window position.
    Background(iced::Point),
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            accounts: Vec::new(),
            nav_state: None,
            tree: None,
            collapsed_sections: HashSet::new(),
            section_drag: None,
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

    fn entry_at(&self, scope: &ViewScope, index: usize) -> Option<SidebarEntry> {
        nav::current_order(self, scope).into_iter().nth(index)
    }

    fn save_order(
        &mut self,
        scope: &ViewScope,
        order: Vec<SidebarEntry>,
    ) -> (Task<SidebarMessage>, Option<SidebarEvent>) {
        self.row_menu = None;
        self.layouts.set(order::scope_key(scope), order);
        (
            Task::none(),
            Some(SidebarEvent::LayoutChanged(self.layouts.to_json())),
        )
    }

    /// Whether the folders show as the all-accounts tree.
    pub fn shows_tree(&self) -> bool {
        self.tree.is_some()
            && shows_tree(
                &self.selected_scope,
                self.accounts.iter().filter(|a| !a.is_deleting).count(),
            )
    }

    /// Folders of the section for `scope`: a tree section, or the whole
    /// sidebar when it shows the selected scope alone.
    pub(super) fn section_nav(&self, scope: &ViewScope) -> Option<&NavigationState> {
        if let Some(tree) = self.tree.as_ref().filter(|_| self.shows_tree()) {
            return match scope {
                ViewScope::AllAccounts => Some(&tree.all),
                ViewScope::Account(id) => tree
                    .accounts
                    .iter()
                    .find(|(account_id, _)| account_id == id)
                    .map(|(_, nav)| nav),
                _ => None,
            };
        }
        self.nav_state
            .as_ref()
            .filter(|_| *scope == self.selected_scope)
    }

    /// Select `selection` in the section for `scope`, switching scope
    /// when the row belongs to another section of the tree.
    fn select_in(
        &mut self,
        scope: ViewScope,
        selection: SidebarSelection,
    ) -> (Task<SidebarMessage>, Option<SidebarEvent>) {
        if scope == self.selected_scope {
            return self.update(SidebarMessage::Select(selection));
        }
        self.row_menu = None;
        self.scope_dropdown_open = false;
        self.selection = selection;
        let event = match &scope {
            ViewScope::Account(id) => self
                .accounts
                .iter()
                .position(|a| a.id == *id)
                .map(SidebarEvent::AccountSelected),
            _ => Some(SidebarEvent::AllAccountsSelected),
        };
        self.selected_scope = scope;
        (Task::none(), event)
    }

    /// Move the account `from` to the place of `to` and renumber.
    fn move_account(&mut self, from: &str, to: &str) -> Option<SidebarEvent> {
        let from = self.accounts.iter().position(|a| a.id == from)?;
        let to = self.accounts.iter().position(|a| a.id == to)?;
        if from == to {
            return None;
        }
        let account = self.accounts.remove(from);
        self.accounts.insert(to, account);
        let mut orders = Vec::with_capacity(self.accounts.len());
        for (position, account) in (0_i64..).zip(self.accounts.iter_mut()) {
            account.sort_order = position;
            orders.push((account.id.clone(), position));
        }
        Some(SidebarEvent::AccountsReordered(orders))
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
            SidebarMessage::SelectIn(scope, selection) => self.select_in(scope, selection),
            SidebarMessage::RowPress(scope, index) => {
                self.row_menu = None;
                self.drag = self.entry_at(&scope, index).map(|from| SidebarDrag {
                    scope,
                    from,
                    target: None,
                    moved: false,
                });
                (Task::none(), None)
            }
            SidebarMessage::RowHover(scope, index) => {
                let entry = self
                    .drag
                    .as_ref()
                    .filter(|drag| drag.scope == scope)
                    .and_then(|_| self.entry_at(&scope, index));
                if let (Some(drag), Some(entry)) = (self.drag.as_mut(), entry) {
                    drag.moved |= entry != drag.from;
                    if drag.moved {
                        drag.target = Some(entry);
                    }
                }
                (Task::none(), None)
            }
            SidebarMessage::RowRelease(scope, index) => {
                let clicked = self.drag.as_ref().is_some_and(|drag| {
                    drag.scope == scope
                        && !drag.moved
                        && Some(&drag.from) == self.entry_at(&scope, index).as_ref()
                });
                if !clicked {
                    return (Task::none(), None);
                }
                self.drag = None;
                match self.entry_at(&scope, index) {
                    Some(SidebarEntry::Folder(id)) => {
                        self.select_in(scope, universal_folder_selection(&id))
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
                let mut order = nav::current_order(self, &drag.scope);
                let from = order.iter().position(|e| *e == drag.from);
                let to = order.iter().position(|e| *e == target);
                match (from, to) {
                    (Some(from), Some(to)) if from != to => {
                        order::move_entry(&mut order, from, to);
                        self.save_order(&drag.scope, order)
                    }
                    _ => (Task::none(), None),
                }
            }
            SidebarMessage::DragCancel => {
                self.drag = None;
                (Task::none(), None)
            }
            SidebarMessage::OpenRowMenu(scope, index) => {
                self.drag = None;
                self.row_menu = Some(RowMenuAnchor::Entry(scope, index));
                (Task::none(), None)
            }
            SidebarMessage::OpenChildMenu {
                scope,
                folder_id,
                root_index,
            } => {
                self.drag = None;
                self.row_menu = Some(RowMenuAnchor::Child {
                    scope,
                    folder_id,
                    root_index,
                });
                (Task::none(), None)
            }
            SidebarMessage::OpenBackgroundMenu(point) => {
                self.drag = None;
                self.row_menu = Some(RowMenuAnchor::Background(point));
                (Task::none(), None)
            }
            SidebarMessage::CloseRowMenu => {
                self.row_menu = None;
                (Task::none(), None)
            }
            SidebarMessage::AddSeparatorBelow(scope, index) => {
                let mut order = nav::current_order(self, &scope);
                order::insert_separator_after(&mut order, index);
                self.save_order(&scope, order)
            }
            SidebarMessage::RemoveSeparator(scope, id) => {
                let mut order = nav::current_order(self, &scope);
                order::remove_separator(&mut order, id);
                self.save_order(&scope, order)
            }
            SidebarMessage::ToggleSection(key) => {
                if !self.collapsed_sections.remove(&key) {
                    self.collapsed_sections.insert(key);
                }
                (Task::none(), None)
            }
            SidebarMessage::SectionPress(id) => {
                self.row_menu = None;
                self.section_drag = Some(SectionDrag {
                    from: id,
                    target: None,
                    moved: false,
                });
                (Task::none(), None)
            }
            SidebarMessage::SectionHover(id) => {
                if let Some(drag) = self.section_drag.as_mut() {
                    drag.moved |= id != drag.from;
                    if drag.moved {
                        drag.target = Some(id);
                    }
                }
                (Task::none(), None)
            }
            SidebarMessage::SectionRelease(id) => {
                let clicked = self
                    .section_drag
                    .as_ref()
                    .is_some_and(|drag| !drag.moved && drag.from == id);
                if clicked {
                    self.section_drag = None;
                    return self.update(SidebarMessage::ToggleSection(id));
                }
                (Task::none(), None)
            }
            SidebarMessage::SectionDragEnd => {
                let Some(drag) = self.section_drag.take() else {
                    return (Task::none(), None);
                };
                match drag.target.filter(|_| drag.moved) {
                    Some(target) => (Task::none(), self.move_account(&drag.from, &target)),
                    None => (Task::none(), None),
                }
            }
            SidebarMessage::SectionDragCancel => {
                self.section_drag = None;
                (Task::none(), None)
            }
            SidebarMessage::ResetOrder(scope) => {
                self.row_menu = None;
                if !self.layouts.reset(&order::scope_key(&scope)) {
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
        scroll_content = scroll_content.push(if self.shows_tree() {
            tree::tree_view(self)
        } else {
            nav_items(self, &self.selected_scope)
        });
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

        let mut scroll_area = crate::ui::anchored_overlay::anchored_overlay(
            scrollable(scroll_content)
                .spacing(SCROLLBAR_SPACING)
                .height(Length::Fill),
        )
        .on_right_press(SidebarMessage::OpenBackgroundMenu);
        if let Some(RowMenuAnchor::Background(point)) = self.row_menu {
            scroll_area = scroll_area
                .popup(nav::background_menu(self))
                .anchor_point(point)
                .popup_width(SIDEBAR_MIN_WIDTH)
                .on_dismiss(SidebarMessage::CloseRowMenu);
        }

        container(column![header_row, scroll_area].spacing(SPACE_XS))
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

    fn acc() -> ViewScope {
        ViewScope::Account("acc".to_string())
    }

    fn ids(sidebar: &Sidebar) -> Vec<String> {
        nav::current_order(sidebar, &acc())
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
        let _ = sidebar.update(SidebarMessage::RowPress(acc(), 3));
        let (_, event) = sidebar.update(SidebarMessage::RowRelease(acc(), 3));
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
        let _ = sidebar.update(SidebarMessage::RowPress(acc(), 2));
        let _ = sidebar.update(SidebarMessage::RowHover(acc(), 1));
        let _ = sidebar.update(SidebarMessage::RowHover(acc(), 0));
        let (_, release) = sidebar.update(SidebarMessage::RowRelease(acc(), 0));
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
        let _ = sidebar.update(SidebarMessage::AddSeparatorBelow(acc(), 2));
        assert_eq!(ids(&sidebar), ["INBOX", "SENT", "TRASH", "sep0", "g-hotel"]);
        let _ = sidebar.update(SidebarMessage::RemoveSeparator(acc(), 0));
        assert_eq!(ids(&sidebar), ["INBOX", "SENT", "TRASH", "g-hotel"]);
        let (_, event) = sidebar.update(SidebarMessage::ResetOrder(acc()));
        assert!(matches!(event, Some(SidebarEvent::LayoutChanged(_))));
        assert_eq!(sidebar.layouts.get("acc"), None);
    }

    fn account(id: &str, sort_order: i64) -> Account {
        Account {
            id: id.to_string(),
            email: format!("{id}@example.com"),
            display_name: None,
            provider: "imap".to_string(),
            account_name: Some(id.to_uppercase()),
            account_color: None,
            last_sync_at: None,
            token_expires_at: None,
            is_active: true,
            sort_order,
            is_deleting: false,
        }
    }

    fn nav(scope: AccountScope, folders: Vec<NavigationFolder>) -> NavigationState {
        NavigationState { scope, folders }
    }

    fn tree_sidebar() -> Sidebar {
        let mut sidebar = Sidebar::new();
        sidebar.accounts = vec![account("antoine", 0), account("villart", 1)];
        sidebar.selected_scope = ViewScope::AllAccounts;
        let universal = || {
            vec![
                folder("INBOX", "Inbox", "Universal"),
                folder("SENT", "Sent", "Universal"),
            ]
        };
        let mut villart = universal();
        villart.push(folder("v-hotel", "Hotel", "AccountFolder"));
        sidebar.tree = Some(NavigationTree {
            all: nav(AccountScope::All, universal()),
            accounts: vec![
                (
                    "antoine".to_string(),
                    nav(AccountScope::Single("antoine".to_string()), universal()),
                ),
                (
                    "villart".to_string(),
                    nav(AccountScope::Single("villart".to_string()), villart),
                ),
            ],
        });
        sidebar
    }

    #[test]
    fn clicking_a_folder_of_another_account_switches_to_that_account() {
        let mut sidebar = tree_sidebar();
        assert!(sidebar.shows_tree());
        let villart = ViewScope::Account("villart".to_string());
        assert_eq!(
            nav::current_order(&sidebar, &villart),
            [
                SidebarEntry::Folder("INBOX".to_string()),
                SidebarEntry::Folder("SENT".to_string()),
                SidebarEntry::Folder("v-hotel".to_string()),
            ]
        );
        let _ = sidebar.update(SidebarMessage::RowPress(villart.clone(), 2));
        let (_, event) = sidebar.update(SidebarMessage::RowRelease(villart.clone(), 2));
        assert!(matches!(event, Some(SidebarEvent::AccountSelected(1))));
        assert_eq!(sidebar.selected_scope, villart);
        assert!(matches!(
            &sidebar.selection,
            SidebarSelection::ProviderFolder(id) if id.as_str() == "v-hotel"
        ));
        assert!(sidebar.shows_tree(), "the tree stays after the switch");
    }

    #[test]
    fn dragging_an_account_header_reorders_the_accounts() {
        let mut sidebar = tree_sidebar();
        let _ = sidebar.update(SidebarMessage::SectionPress("villart".to_string()));
        let _ = sidebar.update(SidebarMessage::SectionHover("antoine".to_string()));
        let _ = sidebar.update(SidebarMessage::SectionRelease("antoine".to_string()));
        let (_, event) = sidebar.update(SidebarMessage::SectionDragEnd);
        let Some(SidebarEvent::AccountsReordered(orders)) = event else {
            panic!("expected AccountsReordered");
        };
        assert_eq!(
            orders,
            [("villart".to_string(), 0), ("antoine".to_string(), 1)]
        );
        let ids: Vec<&str> = sidebar.accounts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["villart", "antoine"]);
    }

    #[test]
    fn clicking_an_account_header_folds_its_section() {
        let mut sidebar = tree_sidebar();
        let _ = sidebar.update(SidebarMessage::SectionPress("antoine".to_string()));
        let _ = sidebar.update(SidebarMessage::SectionRelease("antoine".to_string()));
        let (_, event) = sidebar.update(SidebarMessage::SectionDragEnd);
        assert!(event.is_none());
        assert!(sidebar.collapsed_sections.contains("antoine"));
        let _ = sidebar.update(SidebarMessage::SectionPress("antoine".to_string()));
        let _ = sidebar.update(SidebarMessage::SectionRelease("antoine".to_string()));
        assert!(!sidebar.collapsed_sections.contains("antoine"));
    }
}
