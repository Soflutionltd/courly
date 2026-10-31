use std::collections::{HashMap, HashSet};

use iced::widget::{Space, button, container, mouse_area, row, text};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::move_menu::MoveDestination;
use crate::ui::theme;
use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::{FolderKind, NavigationFolder, NavigationState};
use rtsk::scope::ViewScope;
use types::{FolderId, SidebarSelection};

use super::nav::{FolderMenu, MenuFolder};
use super::search_here::build_search_here_user_folder_prefix;
use super::{MoveTarget, RowMenuAnchor, Sidebar, SidebarMessage};

/// A node in the depth-first tree traversal.
struct TreeNode<'a> {
    folder: &'a NavigationFolder,
    depth: u16,
}

/// Maximum nesting depth to prevent cycles from crashing the renderer.
const MAX_TREE_DEPTH: u16 = 10;

/// Indent step per tree depth level.
const TREE_INDENT: f32 = SPACE_MD;

/// One top-level user folder with its visible-or-not subtree.
pub(super) struct AccountRoot<'a> {
    pub folder: &'a NavigationFolder,
    descendants: Vec<TreeNode<'a>>,
}

/// A section's user-created folders as top-level roots, each carrying its
/// subtree. Empty for the All-Accounts section (user folders don't
/// aggregate across accounts).
pub(super) struct AccountForest<'a> {
    folders: Vec<&'a NavigationFolder>,
    pub roots: Vec<AccountRoot<'a>>,
}

pub(super) fn account_forest<'a>(sidebar: &'a Sidebar, scope: &ViewScope) -> AccountForest<'a> {
    if matches!(scope, ViewScope::AllAccounts) {
        return AccountForest {
            folders: Vec::new(),
            roots: Vec::new(),
        };
    }

    let folders: Vec<&NavigationFolder> = sidebar
        .section_nav(scope)
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[])
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::AccountFolder))
        .collect();

    let mut roots: Vec<AccountRoot<'_>> = Vec::new();
    for node in tree_sort(&folders) {
        match roots.last_mut() {
            Some(root) if node.depth > 0 => root.descendants.push(node),
            _ => roots.push(AccountRoot {
                folder: node.folder,
                descendants: Vec::new(),
            }),
        }
    }
    AccountForest { folders, roots }
}

impl AccountForest<'_> {
    fn has_children(&self, folder: &NavigationFolder) -> bool {
        self.folders
            .iter()
            .any(|f| f.parent_id.as_deref() == Some(&folder.id))
    }

    /// Chevron state for a root row, `None` when it has no subfolders.
    pub(super) fn root_tree_state(
        &self,
        sidebar: &Sidebar,
        root: &AccountRoot<'_>,
    ) -> Option<(bool, bool)> {
        self.has_children(root.folder)
            .then(|| (true, sidebar.collapsed_folders.contains(&root.folder.id)))
    }

    /// Rows for the subfolders of `root`, skipping collapsed branches.
    pub(super) fn descendant_rows<'a>(
        &self,
        sidebar: &'a Sidebar,
        scope: &ViewScope,
        root: &AccountRoot<'a>,
        root_index: usize,
    ) -> Vec<Element<'a, SidebarMessage>> {
        let id_to_folder: HashMap<&str, &NavigationFolder> =
            self.folders.iter().map(|f| (f.id.as_str(), *f)).collect();

        root.descendants
            .iter()
            .filter(|node| {
                !is_hidden_by_collapsed_ancestor(
                    node.folder,
                    &id_to_folder,
                    &sidebar.collapsed_folders,
                )
            })
            .map(|node| {
                let tree_state = Some((
                    self.has_children(node.folder),
                    sidebar.collapsed_folders.contains(&node.folder.id),
                ));
                let indent = TREE_INDENT * f32::from(node.depth);
                folder_row(
                    sidebar,
                    (scope, root_index),
                    node.folder,
                    indent,
                    tree_state,
                )
            })
            .collect()
    }
}

pub(super) fn is_folder_active(
    sidebar: &Sidebar,
    scope: &ViewScope,
    folder: &NavigationFolder,
) -> bool {
    sidebar.selected_scope == *scope
        && sidebar.active_pinned_search.is_none()
        && matches!(
            &sidebar.selection,
            SidebarSelection::ProviderFolder(fid) if fid.as_str() == folder.id
        )
}

/// Depth-first ordering for tree rendering. Orphans (parent_id pointing
/// at a non-existent folder) are appended as roots.
fn tree_sort<'a>(folders: &[&'a NavigationFolder]) -> Vec<TreeNode<'a>> {
    let mut children_of: HashMap<Option<&str>, Vec<&NavigationFolder>> = HashMap::new();
    for f in folders {
        children_of
            .entry(f.parent_id.as_deref())
            .or_default()
            .push(f);
    }

    let mut result = Vec::with_capacity(folders.len());

    fn walk<'a>(
        parent: Option<&str>,
        depth: u16,
        children_of: &HashMap<Option<&str>, Vec<&'a NavigationFolder>>,
        result: &mut Vec<TreeNode<'a>>,
    ) {
        if depth > MAX_TREE_DEPTH {
            return;
        }
        let Some(children) = children_of.get(&parent) else {
            return;
        };
        for child in children {
            result.push(TreeNode {
                folder: child,
                depth,
            });
            walk(Some(&child.id), depth + 1, children_of, result);
        }
    }

    walk(None, 0, &children_of, &mut result);

    let in_tree: HashSet<&str> = result.iter().map(|n| n.folder.id.as_str()).collect();
    for f in folders {
        if !in_tree.contains(f.id.as_str()) {
            result.push(TreeNode {
                folder: f,
                depth: 0,
            });
        }
    }

    result
}

fn is_hidden_by_collapsed_ancestor(
    folder: &NavigationFolder,
    id_to_folder: &HashMap<&str, &NavigationFolder>,
    collapsed: &HashSet<String>,
) -> bool {
    let mut current_parent = folder.parent_id.as_deref();
    let mut depth = 0u16;
    while let Some(pid) = current_parent {
        if collapsed.contains(pid) {
            return true;
        }
        if depth >= MAX_TREE_DEPTH {
            break;
        }
        depth += 1;
        current_parent = id_to_folder.get(pid).and_then(|f| f.parent_id.as_deref());
    }
    false
}

/// Padded content of one folder row: optional indent / chevron, the folder
/// name, and the unread badge. Matches the visual weight of
/// `widgets::nav_button` in the surrounding section.
pub(super) fn folder_row_content<'a>(
    folder: &'a NavigationFolder,
    indent: f32,
    tree_state: Option<(bool, bool)>,
    active: bool,
) -> Element<'a, SidebarMessage> {
    let mut item_row = row![].spacing(SPACE_XXS).align_y(Alignment::Center);

    if indent > 0.0 {
        item_row = item_row.push(Space::new().width(indent));
    }

    if let Some((has_children, is_collapsed)) = tree_state
        && has_children
    {
        let chevron = if is_collapsed {
            icon::chevron_right()
        } else {
            icon::chevron_down()
        };
        item_row = item_row.push(
            button(
                chevron
                    .size(ICON_XS)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .on_press(SidebarMessage::ToggleFolderExpand(folder.id.clone()))
            .padding(SPACE_XXXS)
            .style(theme::ButtonClass::Ghost.style()),
        );
    }

    let lbl_style: fn(&iced::Theme) -> text::Style = if active {
        text::primary
    } else {
        theme::TextClass::Muted.style()
    };
    item_row = item_row.push(text(&folder.name).size(TEXT_MD).style(lbl_style));

    if folder.unread_count() > 0 {
        item_row = item_row
            .push(Space::new().width(Length::Fill))
            .push(widgets::count_badge(folder.unread_count()));
    }

    container(item_row)
        .padding(PAD_NAV_ITEM)
        .width(Length::Fill)
        .into()
}

/// A subfolder row: selectable, right-click opens the folder menu, whose
/// separator goes below its top-level ancestor at `root_index`.
fn folder_row<'a>(
    sidebar: &'a Sidebar,
    (scope, root_index): (&ViewScope, usize),
    folder: &'a NavigationFolder,
    indent: f32,
    tree_state: Option<(bool, bool)>,
) -> Element<'a, SidebarMessage> {
    let active = is_folder_active(sidebar, scope, folder);
    let label_btn: Element<'_, SidebarMessage> =
        button(folder_row_content(folder, indent, tree_state, active))
            .on_press(SidebarMessage::SelectIn(
                scope.clone(),
                SidebarSelection::ProviderFolder(FolderId::from(folder.id.clone())),
            ))
            .padding(0)
            .style(theme::ButtonClass::Nav { active }.style())
            .width(Length::Fill)
            .into();

    let query_prefix = build_search_here_user_folder_prefix(&folder.name, sidebar, scope);
    let mut row_el: Element<'a, SidebarMessage> = mouse_area(label_btn)
        .on_right_press(SidebarMessage::OpenChildMenu {
            scope: scope.clone(),
            folder_id: folder.id.clone(),
            root_index,
        })
        .on_enter(SidebarMessage::FolderEnter(
            scope.clone(),
            folder.id.clone(),
        ))
        .on_exit(SidebarMessage::FolderExit(scope.clone(), folder.id.clone()))
        .into();
    if sidebar.is_drop_target(scope, &folder.id) {
        row_el = container(row_el)
            .style(theme::ContainerClass::DropTarget.style())
            .width(Length::Fill)
            .into();
    }
    let menu_open = matches!(
        &sidebar.row_menu,
        Some(RowMenuAnchor::Child { scope: menu_scope, folder_id, .. })
            if *folder_id == folder.id && menu_scope == scope
    );
    if !menu_open {
        return row_el;
    }
    let menu = FolderMenu {
        query_prefix,
        root_index,
        shift: None,
        folder: MenuFolder::own(scope, folder),
    };
    super::nav::with_row_menu(
        scope,
        row_el,
        super::nav::folder_menu_items(scope, menu, root_index),
    )
}

/// Where messages can move from the folder `current`: Inbox, Archive,
/// Trash and Junk, then the account's own folders in tree order.
pub(crate) fn message_destinations(
    nav: &NavigationState,
    current: Option<&str>,
) -> Vec<MoveDestination> {
    const SYSTEM: [&str; 4] = ["INBOX", "archive", "TRASH", "SPAM"];
    let system = SYSTEM.iter().filter_map(|id| {
        nav.folders
            .iter()
            .find(|f| matches!(f.folder_kind, FolderKind::Universal) && f.id == *id)
    });
    let own: Vec<&NavigationFolder> = nav
        .folders
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::AccountFolder))
        .collect();
    system
        .map(|folder| (folder, 0))
        .chain(
            tree_sort(&own)
                .into_iter()
                .map(|node| (node.folder, node.depth)),
        )
        .filter(|(folder, _)| current != Some(folder.id.as_str()))
        .map(|(folder, depth)| MoveDestination {
            id: folder.id.clone(),
            name: super::folder_display_name(folder).into_owned(),
            depth,
            system: matches!(folder.folder_kind, FolderKind::Universal),
        })
        .collect()
}

impl Sidebar {
    /// Where `folder_id` can move: the top level, then every folder of the
    /// account outside its own subtree, in tree order.
    pub fn move_targets(&self, account_id: &str, folder_id: &str) -> Vec<MoveTarget> {
        let scope = ViewScope::Account(account_id.to_string());
        let folders: Vec<&NavigationFolder> = self
            .section_nav(&scope)
            .map(|ns| &ns.folders[..])
            .unwrap_or(&[])
            .iter()
            .filter(|f| matches!(f.folder_kind, FolderKind::AccountFolder))
            .collect();
        let mut targets = vec![MoveTarget {
            parent_id: None,
            name: String::new(),
            depth: 0,
        }];
        let mut moved_subtree: Option<u16> = None;
        for node in tree_sort(&folders) {
            if let Some(depth) = moved_subtree {
                if node.depth > depth {
                    continue;
                }
                moved_subtree = None;
            }
            if node.folder.id == folder_id {
                moved_subtree = Some(node.depth);
                continue;
            }
            targets.push(MoveTarget {
                parent_id: Some(node.folder.id.clone()),
                name: node.folder.name.clone(),
                depth: node.depth,
            });
        }
        targets
    }
}
