use iced::widget::{Space, button, column, container, mouse_area, row, text};
use iced::{Alignment, Element, Length, mouse};
use rust_i18n::t;

use crate::db::Account;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use rtsk::scope::ViewScope;

use super::nav::nav_items;
use super::order::scope_key;
use super::{Sidebar, SidebarMessage};

/// Outlook-style folders: the unified section, then one section per
/// account in account order. Dragging an account header moves the whole
/// section.
pub(super) fn tree_view(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let mut sections = column![all_accounts_section(sidebar)].spacing(SIDEBAR_SECTION_GAP);
    for account in sidebar.accounts.iter().filter(|a| !a.is_deleting) {
        sections = sections.push(account_section(sidebar, account));
    }
    mouse_area(sections)
        .on_release(SidebarMessage::SectionDragEnd)
        .on_exit(SidebarMessage::SectionDragCancel)
        .into()
}

fn all_accounts_section(sidebar: &Sidebar) -> Element<'_, SidebarMessage> {
    let scope = ViewScope::AllAccounts;
    let key = scope_key(&scope);
    let expanded = !sidebar.collapsed_sections.contains(&key);
    let header = button(title_row(expanded, t!("main.sidebar.all_accounts")))
        .on_press(SidebarMessage::ToggleSection(key))
        .padding(PAD_SIDEBAR_SECTION_TITLE)
        .style(theme::ButtonClass::Ghost.style())
        .width(Length::Fill);
    let mut section = column![header].spacing(SPACE_XXS);
    if expanded {
        section = section.push(nav_items(sidebar, &scope));
    }
    section.into()
}

fn account_section<'a>(sidebar: &'a Sidebar, account: &'a Account) -> Element<'a, SidebarMessage> {
    let scope = ViewScope::Account(account.id.clone());
    let expanded = !sidebar.collapsed_sections.contains(&account.id);
    let label = account
        .account_name
        .as_deref()
        .or(account.display_name.as_deref())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&account.email);

    let mut title = column![title_row(expanded, label)].spacing(SPACE_XXXS);
    if label != account.email {
        title = title.push(
            row![
                Space::new().width(SIDEBAR_SECTION_CHEVRON_SLOT),
                text(&account.email)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Muted.style()),
            ]
            .align_y(Alignment::Center),
        );
    }

    let mut header: Element<'a, SidebarMessage> = mouse_area(
        container(title)
            .padding(PAD_SIDEBAR_SECTION_TITLE)
            .width(Length::Fill),
    )
    .on_press(SidebarMessage::SectionPress(account.id.clone()))
    .on_release(SidebarMessage::SectionRelease(account.id.clone()))
    .interaction(mouse::Interaction::Pointer)
    .into();
    if let Some(drag) = sidebar.section_drag.as_ref().filter(|drag| drag.moved) {
        let class = if drag.from == account.id {
            Some(theme::ContainerClass::DraggingRow)
        } else if drag.target.as_deref() == Some(account.id.as_str()) {
            Some(theme::ContainerClass::DropTarget)
        } else {
            None
        };
        if let Some(class) = class {
            header = container(header)
                .style(class.style())
                .width(Length::Fill)
                .into();
        }
    }

    let mut section = column![header].spacing(SPACE_XXS);
    if expanded {
        section = section.push(nav_items(sidebar, &scope));
    }
    mouse_area(section)
        .on_enter(SidebarMessage::SectionHover(account.id.clone()))
        .into()
}

/// Fold chevron and bold section name.
fn title_row<'a>(
    expanded: bool,
    label: impl text::IntoFragment<'a>,
) -> Element<'a, SidebarMessage> {
    let chevron = if expanded {
        icon::chevron_down()
    } else {
        icon::chevron_right()
    };
    row![
        container(
            chevron
                .size(ICON_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .width(SIDEBAR_SECTION_CHEVRON_SLOT)
        .align_x(Alignment::Center),
        text(label)
            .size(TEXT_LG)
            .font(crate::font::text_bold())
            .style(text::base),
    ]
    .align_y(Alignment::Center)
    .into()
}
