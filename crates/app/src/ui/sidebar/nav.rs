use iced::widget::{button, column, container, mouse_area, row, text};
use iced::{Alignment, Element, Length};
use rust_i18n::t;

use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::{FolderKind, NavigationFolder};
use rtsk::scope::ViewScope;
use types::SidebarSelection;

use super::folders::{AccountForest, account_forest, folder_row_content, is_folder_active};
use super::order::{SidebarEntry, default_order, resolve, scope_key};
use super::search_here::{build_search_here_folder_prefix, build_search_here_user_folder_prefix};
use super::{
    RowMenuAnchor, Sidebar, SidebarMessage, folder_display_name, universal_folder_selection,
};

// ── Nav items ───────────────────────────────────────────

fn universal_folders<'a>(sidebar: &'a Sidebar, scope: &ViewScope) -> Vec<&'a NavigationFolder> {
    let all_accounts = matches!(scope, ViewScope::AllAccounts);
    sidebar
        .section_nav(scope)
        .map(|ns| &ns.folders[..])
        .unwrap_or(&[])
        .iter()
        .filter(|f| matches!(f.folder_kind, FolderKind::Universal))
        // All Mail is the per-account "literally everything" view - it
        // doesn't aggregate meaningfully, so single-account only.
        .filter(|f| !(all_accounts && f.id == "all-mail"))
        .collect()
}

fn order_for(
    sidebar: &Sidebar,
    scope: &ViewScope,
    universal: &[&NavigationFolder],
    forest: &AccountForest<'_>,
) -> Vec<SidebarEntry> {
    let universal_ids: Vec<&str> = universal.iter().map(|f| f.id.as_str()).collect();
    let roots: Vec<(&str, &str)> = forest
        .roots
        .iter()
        .map(|root| (root.folder.id.as_str(), root.folder.name.as_str()))
        .collect();
    resolve(
        default_order(&universal_ids, &roots),
        sidebar.layouts.get(&scope_key(scope)),
    )
}

/// The folders of the section for `scope`, as currently arranged.
pub(super) fn current_order(sidebar: &Sidebar, scope: &ViewScope) -> Vec<SidebarEntry> {
    order_for(
        sidebar,
        scope,
        &universal_folders(sidebar, scope),
        &account_forest(sidebar, scope),
    )
}

/// System folders and the account's own folders in one reorderable list:
/// drag a row onto another to take its place, right-click for separators,
/// search-here and reset.
pub(super) fn nav_items<'a>(
    sidebar: &'a Sidebar,
    scope: &ViewScope,
) -> Element<'a, SidebarMessage> {
    let universal = universal_folders(sidebar, scope);
    let forest = account_forest(sidebar, scope);
    let order = order_for(sidebar, scope, &universal, &forest);
    let in_scope = sidebar.selected_scope == *scope;

    let mut col = column![].spacing(SPACE_XXS);
    for (index, entry) in order.iter().enumerate() {
        match entry {
            SidebarEntry::Separator(id) => {
                let line = container(
                    iced::widget::rule::horizontal(SIDEBAR_SEPARATOR_THICKNESS)
                        .style(theme::RuleClass::SidebarSeparator.style()),
                )
                .padding(PAD_SIDEBAR_SEPARATOR)
                .width(Length::Fill);
                col = col.push(reorderable_row(
                    sidebar,
                    (scope, index),
                    entry,
                    line.into(),
                    false,
                    SidebarMessage::CloseRowMenu,
                    RowMenu::Separator(*id),
                ));
            }
            SidebarEntry::Folder(id) => {
                if let Some(folder) = universal.iter().find(|f| f.id == *id) {
                    let selection = universal_folder_selection(&folder.id);
                    let active = in_scope && sidebar_nav_selection_is_active(sidebar, &selection);
                    let content = widgets::nav_button_content(
                        Some(universal_folder_icon(&folder.id)),
                        folder_display_name(folder),
                        active,
                        widgets::NavSize::Compact,
                        folder
                            .universal_unread_count()
                            .map(widgets::NavBadge::Universal),
                    );
                    col = col.push(reorderable_row(
                        sidebar,
                        (scope, index),
                        entry,
                        content,
                        active,
                        SidebarMessage::SelectIn(scope.clone(), selection),
                        RowMenu::Folder(build_search_here_folder_prefix(
                            &folder.name,
                            sidebar,
                            scope,
                        )),
                    ));
                } else if let Some(root) = forest.roots.iter().find(|r| r.folder.id == *id) {
                    let active = is_folder_active(sidebar, scope, root.folder);
                    let content = if super::order::is_outbox_name(&root.folder.name) {
                        widgets::nav_button_content(
                            Some(icon::outbox()),
                            folder_display_name(root.folder),
                            active,
                            widgets::NavSize::Compact,
                            Some(widgets::NavBadge::General(root.folder.unread_count())),
                        )
                    } else {
                        folder_row_content(
                            root.folder,
                            0.0,
                            forest.root_tree_state(sidebar, root),
                            active,
                        )
                    };
                    col = col.push(reorderable_row(
                        sidebar,
                        (scope, index),
                        entry,
                        content,
                        active,
                        SidebarMessage::SelectIn(
                            scope.clone(),
                            universal_folder_selection(&root.folder.id),
                        ),
                        RowMenu::Folder(build_search_here_user_folder_prefix(
                            &root.folder.name,
                            sidebar,
                            scope,
                        )),
                    ));
                    for child in forest.descendant_rows(sidebar, scope, root, index) {
                        col = col.push(child);
                    }
                }
            }
        }
    }

    mouse_area(col)
        .on_release(SidebarMessage::DragEnd)
        .on_exit(SidebarMessage::DragCancel)
        .into()
}

enum RowMenu {
    /// A folder row; carries its search-here query prefix.
    Folder(String),
    Separator(u32),
}

/// One draggable row. The inner `mouse_area` takes the press (so a press
/// can start a drag) and the release (a click selects); the outer button
/// only supplies the hover and active styling.
fn reorderable_row<'a>(
    sidebar: &'a Sidebar,
    (scope, index): (&ViewScope, usize),
    entry: &SidebarEntry,
    content: Element<'a, SidebarMessage>,
    active: bool,
    on_press: SidebarMessage,
    menu: RowMenu,
) -> Element<'a, SidebarMessage> {
    let hit = mouse_area(content)
        .on_press(SidebarMessage::RowPress(scope.clone(), index))
        .on_release(SidebarMessage::RowRelease(scope.clone(), index))
        .on_enter(SidebarMessage::RowHover(scope.clone(), index))
        .on_right_press(SidebarMessage::OpenRowMenu(scope.clone(), index));
    let mut row_el: Element<'a, SidebarMessage> = button(hit)
        .on_press(on_press)
        .padding(0)
        .style(theme::ButtonClass::Nav { active }.style())
        .width(Length::Fill)
        .into();

    if let Some(drag) = sidebar
        .drag
        .as_ref()
        .filter(|drag| drag.moved && drag.scope == *scope)
    {
        let class = if drag.from == *entry {
            Some(theme::ContainerClass::DraggingRow)
        } else if drag.target.as_ref() == Some(entry) {
            Some(theme::ContainerClass::DropTarget)
        } else {
            None
        };
        if let Some(class) = class {
            row_el = container(row_el)
                .style(class.style())
                .width(Length::Fill)
                .into();
        }
    }

    if sidebar.row_menu != Some(RowMenuAnchor::Entry(scope.clone(), index)) {
        return row_el;
    }

    let items = match menu {
        RowMenu::Folder(query_prefix) => folder_menu_items(scope, query_prefix, index),
        RowMenu::Separator(id) => column![menu_item(
            icon::x(),
            t!("main.sidebar.remove_separator"),
            SidebarMessage::RemoveSeparator(scope.clone(), id),
        )]
        .spacing(SPACE_XXS),
    };
    with_row_menu(scope, row_el, items)
}

/// "Search Here" and "Add Separator Below", the separator going after the
/// top-level row at `separator_after`.
pub(super) fn folder_menu_items<'a>(
    scope: &ViewScope,
    query_prefix: String,
    separator_after: usize,
) -> iced::widget::Column<'a, SidebarMessage> {
    column![
        menu_item(
            icon::search(),
            t!("main.sidebar.search_here"),
            SidebarMessage::SearchHere(query_prefix),
        ),
        menu_item(
            icon::minus(),
            t!("main.sidebar.add_separator_below"),
            SidebarMessage::AddSeparatorBelow(scope.clone(), separator_after),
        ),
    ]
    .spacing(SPACE_XXS)
}

/// The right-click menu card, ending with "Reset Folder Order".
fn menu_card<'a>(
    scope: &ViewScope,
    items: iced::widget::Column<'a, SidebarMessage>,
) -> Element<'a, SidebarMessage> {
    let items = items.push(widgets::divider()).push(menu_item(
        icon::undo(),
        t!("main.sidebar.reset_folder_order"),
        SidebarMessage::ResetOrder(scope.clone()),
    ));
    container(items)
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style())
        .into()
}

/// Menu for a right-click outside the folder rows: the separator goes
/// after the last folder of the selected section, from where it can be
/// dragged.
pub(super) fn background_menu(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let scope = &sidebar.selected_scope;
    let mut items = column![].spacing(SPACE_XXS);
    if let Some(last) = current_order(sidebar, scope).len().checked_sub(1) {
        items = items.push(menu_item(
            icon::minus(),
            t!("main.sidebar.add_separator"),
            SidebarMessage::AddSeparatorBelow(scope.clone(), last),
        ));
    }
    menu_card(scope, items)
}

/// Float the right-click menu below `row_el`.
pub(super) fn with_row_menu<'a>(
    scope: &ViewScope,
    row_el: Element<'a, SidebarMessage>,
    items: iced::widget::Column<'a, SidebarMessage>,
) -> Element<'a, SidebarMessage> {
    crate::ui::anchored_overlay::anchored_overlay(row_el)
        .popup(menu_card(scope, items))
        .popup_width(SIDEBAR_MIN_WIDTH)
        .position(crate::ui::anchored_overlay::AnchorPosition::Below)
        .on_dismiss(SidebarMessage::CloseRowMenu)
        .into()
}

fn menu_item<'a>(
    ico: iced::widget::Text<'a>,
    label: std::borrow::Cow<'a, str>,
    on_press: SidebarMessage,
) -> Element<'a, SidebarMessage> {
    button(
        row![
            container(ico.size(ICON_MD).style(text::secondary))
                .width(SLOT_DROPDOWN)
                .height(SLOT_DROPDOWN)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
            container(text(label).size(TEXT_MD).style(text::base)).align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_NAV_ITEM)
    .height(DROPDOWN_ITEM_HEIGHT)
    .style(theme::ButtonClass::Dropdown { selected: false }.style())
    .width(Length::Fill)
    .into()
}

pub(super) fn sidebar_nav_selection_is_active(
    sidebar: &Sidebar,
    selection: &SidebarSelection,
) -> bool {
    sidebar.active_pinned_search.is_none() && sidebar.selection == *selection
}

fn universal_folder_icon<'a>(id: &str) -> iced::widget::Text<'a> {
    match id {
        "INBOX" => icon::inbox(),
        "STARRED" => icon::star(),
        "SNOOZED" => icon::clock(),
        "SENT" => icon::send(),
        "DRAFT" => icon::file_text(),
        "archive" => icon::archive(),
        "TRASH" => icon::trash(),
        "SPAM" => icon::alert_triangle(),
        "all-mail" => icon::mail(),
        _ => icon::folder(),
    }
}
